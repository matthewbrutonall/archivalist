#!/usr/bin/env python3
"""Offline Unit and Safety Tests for Gate 2 Cross-Stage Observation Probe."""

import io
import json
import os
import stat
import sys
import tempfile
import time
import unittest
from unittest.mock import MagicMock, patch

sys.path.insert(0, os.path.dirname(__file__))

from probe_gate2_ao_parity_gap import (
    strip_capability_markers,
    normalize_solr_doc,
    collect_cross_stage_observations,
    normalize_path,
    value_sha256_and_size,
    json_values_strictly_equal,
    is_array_reordered,
    sanitize_proc_output,
    handle_builder_failure,
    verify_active_app_arks_enabled,
    verify_binary_provenance,
)


class TestCrossStageProbe(unittest.TestCase):
    def test_strip_capability_markers(self):
        doc = {
            "id": "/repositories/4/archival_objects/1",
            "title": "Sample Title",
            "_archivalist_rust_fields": ["common_metadata"],
            "_archivalist_collection_management_documents": [],
            "_archivalist_pui_routing": {"type": "pui"},
            "repository": "/repositories/4",
        }
        stripped = strip_capability_markers(doc)
        self.assertEqual(stripped["id"], "/repositories/4/archival_objects/1")
        self.assertEqual(stripped["title"], "Sample Title")
        self.assertEqual(stripped["repository"], "/repositories/4")
        self.assertNotIn("_archivalist_rust_fields", stripped)
        self.assertNotIn("_archivalist_collection_management_documents", stripped)
        self.assertNotIn("_archivalist_pui_routing", stripped)

    def test_normalize_solr_doc(self):
        solr_doc = {
            "id": "/repositories/4/archival_objects/1",
            "title": "Native Title",
            "_version_": 181395819230582,
            "suppressed": False,
        }
        normalized = normalize_solr_doc(solr_doc)
        self.assertEqual(normalized["id"], "/repositories/4/archival_objects/1")
        self.assertEqual(normalized["title"], "Native Title")
        self.assertEqual(normalized["suppressed"], False)
        self.assertNotIn("_version_", normalized)

    def test_normalize_path(self):
        self.assertEqual(normalize_path("ancestors[0]"), "ancestors[]")
        self.assertEqual(normalize_path("notes[3].subnotes[12]"), "notes[].subnotes[]")
        self.assertEqual(normalize_path("title"), "title")

    def test_collect_cross_stage_observations_missing_and_extra(self):
        builder = {
            "id": "/repositories/4/archival_objects/1",
            "only_in_builder": "foo",
        }
        native = {
            "id": "/repositories/4/archival_objects/1",
            "only_in_native": "bar",
        }
        diffs = collect_cross_stage_observations(builder, native)
        categories = {d["category"] for d in diffs}
        self.assertEqual(categories, {"cross_stage_missing_in_builder", "cross_stage_extra_in_builder"})

        missing = next(d for d in diffs if d["category"] == "cross_stage_missing_in_builder")
        self.assertEqual(missing["field"], "only_in_native")
        self.assertEqual(missing["native_type"], "str")
        self.assertIn("native_hash", missing)
        self.assertIn("native_size", missing)
        # Verify raw payload is not in gap report
        self.assertNotIn("bar", missing.values())

        extra = next(d for d in diffs if d["category"] == "cross_stage_extra_in_builder")
        self.assertEqual(extra["field"], "only_in_builder")
        self.assertEqual(extra["builder_type"], "str")
        self.assertIn("builder_hash", extra)
        self.assertIn("builder_size", extra)
        self.assertNotIn("foo", extra.values())

    def test_collect_cross_stage_observations_value_mismatch(self):
        builder = {"title": "Builder Title"}
        native = {"title": "Native Title"}
        diffs = collect_cross_stage_observations(builder, native)
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["category"], "cross_stage_value_mismatch")
        self.assertEqual(diffs[0]["path"], "title")
        self.assertIn("builder_hash", diffs[0])
        self.assertIn("native_hash", diffs[0])
        # Value strings must not appear directly in report
        self.assertNotIn("Builder Title", diffs[0].values())
        self.assertNotIn("Native Title", diffs[0].values())

    def test_collect_cross_stage_observations_type_mismatch(self):
        builder = {"identifier": 123}
        native = {"identifier": "123"}
        diffs = collect_cross_stage_observations(builder, native)
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["category"], "cross_stage_type_mismatch")
        self.assertEqual(diffs[0]["builder_type"], "int")
        self.assertEqual(diffs[0]["native_type"], "str")

    def test_collect_cross_stage_observations_nested_array_and_order(self):
        builder = {"subjects": ["Art", "History"]}
        native = {"subjects": ["History", "Art"]}
        diffs = collect_cross_stage_observations(builder, native)
        self.assertTrue(any(d["category"] == "cross_stage_array_order_change" for d in diffs))

    def test_sanitize_proc_output_redacts_passwords_and_payloads(self):
        secret_pw = "super_secret_admin_pw_123"
        raw_stderr = (
            f"Error: failed connecting with password {secret_pw}\n"
            "Server returned response body:\n"
            '{"uri": "/repositories/4/archival_objects/1", "title": "Sensitive Document"}\n'
            "Connection terminated.\n"
        )
        sanitized = sanitize_proc_output(raw_stderr, password_to_redact=secret_pw)
        self.assertNotIn(secret_pw, sanitized)
        self.assertIn("[REDACTED_PASSWORD]", sanitized)
        self.assertNotIn("Sensitive Document", sanitized)
        self.assertIn("[PAYLOAD REDACTED]", sanitized)
        self.assertIn("Connection terminated.", sanitized)

    def test_verify_binary_provenance_fresh_vs_stale(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            dummy_bin = os.path.join(tmpdir, "dummy_cli")
            with open(dummy_bin, "w") as f:
                f.write("binary content")

            # Set binary mtime to the future so it is newer than any source
            future_mtime = time.time() + 1000
            os.utime(dummy_bin, (future_mtime, future_mtime))

            info = verify_binary_provenance(dummy_bin)
            self.assertEqual(info["path"], dummy_bin)
            self.assertIn("sha256", info)

            # Now set binary mtime to old timestamp (stale)
            os.utime(dummy_bin, (1000.0, 1000.0))
            with self.assertRaises(RuntimeError) as ctx:
                verify_binary_provenance(dummy_bin)
            self.assertIn("is stale", str(ctx.exception))

    def test_mode_0600_creation(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            test_file = os.path.join(tmpdir, "test_mode.json")
            fd = os.open(test_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(fd, "w") as f:
                json.dump({"test": "value"}, f)

            st = os.stat(test_file)
            mode = stat.S_IMODE(st.st_mode)
            self.assertEqual(mode, 0o600)


    def test_handle_builder_failure_generic_message_no_leak(self):
        secret_pw = "super_secret_admin_pw_123"
        raw_stderr = (
            f"Panic in builder with credential {secret_pw}\n"
            "Raw JSON: {\"uri\": \"/archival_objects/99\", \"title\": \"Secret AO Title\"}\n"
        )
        stderr_capture = io.StringIO()
        stdout_capture = io.StringIO()
        with patch("sys.stderr", stderr_capture), patch("sys.stdout", stdout_capture):
            res_path = handle_builder_failure(1, raw_stderr, write_debug_log=False)

        self.assertIsNone(res_path)
        err_out = stderr_capture.getvalue()
        std_out = stdout_capture.getvalue()

        # Stdout must have no output
        self.assertEqual(std_out, "")
        # Stderr must contain generic failure notice
        self.assertIn("Rust STAFF AO builder failed with exit code 1", err_out)
        self.assertIn("Process output suppressed", err_out)
        # Secrets and payloads must NOT leak to stderr
        self.assertNotIn(secret_pw, err_out)
        self.assertNotIn("Secret AO Title", err_out)

    def test_handle_builder_failure_debug_log_0600_when_requested(self):
        secret_pw = "super_secret_admin_pw_123"
        raw_stderr = f"Error with {secret_pw}"
        with tempfile.TemporaryDirectory() as tmpdir:
            stderr_capture = io.StringIO()
            with patch("sys.stderr", stderr_capture):
                log_path = handle_builder_failure(2, raw_stderr, log_dir=tmpdir, write_debug_log=True)

            self.assertIsNotNone(log_path)
            self.assertTrue(os.path.exists(log_path))
            st = os.stat(log_path)
            mode = stat.S_IMODE(st.st_mode)
            self.assertEqual(mode, 0o600)

            with open(log_path) as f:
                content = f.read()
            self.assertIn("WARNING: This file contains raw stderr", content)
            self.assertIn(secret_pw, content)

    def test_verify_active_app_arks_enabled_genuine_active_process(self):
        login_resp = MagicMock()
        login_resp.read.return_value = json.dumps({"session": "mock-session"}).encode("utf-8")
        login_resp.__enter__.return_value = login_resp

        info_resp = MagicMock()
        info_resp.read.return_value = json.dumps({
            "version": "v3.5.0",
            "appconfig": {"arks_enabled": False}
        }).encode("utf-8")
        info_resp.__enter__.return_value = info_resp

        with patch("urllib.request.urlopen", side_effect=[login_resp, info_resp]):
            status, val = verify_active_app_arks_enabled(
                "http://127.0.0.1:14567", "admin", "admin", "mock-container"
            )
            self.assertEqual(status, "genuine_active_process")
            self.assertIs(val, False)

    def test_verify_active_app_arks_enabled_best_effort_fallback(self):
        # HTTP fails
        with patch("urllib.request.urlopen", side_effect=Exception("Connection refused")):
            mock_proc = MagicMock()
            mock_proc.returncode = 0
            mock_proc.stdout = "false\n"
            with patch("subprocess.run", return_value=mock_proc):
                status, val = verify_active_app_arks_enabled(
                    "http://127.0.0.1:14567", "admin", "admin", "mock-container"
                )
                self.assertEqual(status, "best_effort_config_file")
                self.assertIs(val, False)

    def test_verify_active_app_arks_enabled_unverified_on_failure(self):
        # Both HTTP and container inspection fail
        with patch("urllib.request.urlopen", side_effect=Exception("Connection refused")):
            mock_proc = MagicMock()
            mock_proc.returncode = 1
            mock_proc.stdout = ""
            with patch("subprocess.run", return_value=mock_proc):
                status, val = verify_active_app_arks_enabled(
                    "http://127.0.0.1:14567", "admin", "admin", "mock-container"
                )
                self.assertEqual(status, "unverified")
                self.assertIsNone(val)


if __name__ == "__main__":
    unittest.main()
