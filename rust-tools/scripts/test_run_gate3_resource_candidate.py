#!/usr/bin/env python3
"""Offline Unit Tests for Gate 3 STAFF Resource Slice Parity Harness"""

import contextlib
import io
import json
import os
import sys
import unittest
from unittest.mock import MagicMock, patch

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "rust-tools", "scripts"))

import run_gate3_resource_candidate as harness


class TestRunGate3ResourceCandidate(unittest.TestCase):
    def test_canonical_json_key_and_strict_equality(self):
        self.assertTrue(harness.json_values_strictly_equal(1, 1))
        self.assertFalse(harness.json_values_strictly_equal(1, 1.0))
        self.assertFalse(harness.json_values_strictly_equal(1, "1"))
        self.assertFalse(harness.json_values_strictly_equal(True, 1))

        d1 = {"a": 1, "b": [1, 2]}
        d2 = {"b": [1, 2], "a": 1}
        self.assertTrue(harness.json_values_strictly_equal(d1, d2))

        d3 = {"a": 1, "b": [2, 1]}
        self.assertFalse(harness.json_values_strictly_equal(d1, d3))

    def test_check_for_marker_leaks(self):
        # Clean document
        clean_doc = {
            "id": "/repositories/4/resources/1",
            "uri": "/repositories/4/resources/1",
            "title": "Clean Title",
            "json": '{"title":"Clean Title"}',
        }
        self.assertEqual(harness.check_for_marker_leaks(clean_doc), [])

        # Top-level key leak
        key_leak_doc = {
            "id": "/repositories/4/resources/1",
            "uri": "/repositories/4/resources/1",
            "_archivalist_test": "leaked",
            "json": '{"title":"Clean Title"}',
        }
        leaks = harness.check_for_marker_leaks(key_leak_doc)
        self.assertEqual(len(leaks), 1)
        self.assertIn("key:_archivalist_test", leaks[0])

        # Embedded JSON string leak
        embedded_leak_doc = {
            "id": "/repositories/4/resources/1",
            "uri": "/repositories/4/resources/1",
            "json": '{"title":"Clean Title","_archivalist_internal":"leaked"}',
        }
        leaks = harness.check_for_marker_leaks(embedded_leak_doc)
        self.assertEqual(len(leaks), 1)
        self.assertIn("embedded_json:_archivalist_marker_found", leaks[0])

    def test_collect_diffs_detects_mismatches_and_redacts(self):
        c_doc = {
            "uri": "/repositories/4/resources/1",
            "title": "Secret Candidate Title",
            "level": "collection",
        }
        n_doc = {
            "uri": "/repositories/4/resources/1",
            "title": "Secret Native Title",
            "level": "recordgrp",
        }

        diffs = harness.collect_same_stage_stored_diffs(c_doc, n_doc)
        self.assertEqual(len(diffs), 2)
        paths = {d["path"] for d in diffs}
        self.assertIn("title", paths)
        self.assertIn("level", paths)

        for d in diffs:
            # Confirm NO plaintext content is leaked
            self.assertNotIn("Secret Candidate Title", str(d))
            self.assertNotIn("Secret Native Title", str(d))
            self.assertIn("candidate_hash", d)
            self.assertIn("native_hash", d)

    def test_run_slice_parity_comparison_clean_match(self):
        cand_docs = {
            "/repositories/4/resources/1": {
                "id": "/repositories/4/resources/1",
                "uri": "/repositories/4/resources/1",
                "title": "Res 1",
                "json": '{"title":"Res 1"}',
            },
            "/repositories/4/collection_management/1": {
                "id": "/repositories/4/collection_management/1",
                "uri": "/repositories/4/collection_management/1",
                "parent_id": "/repositories/4/resources/1",
                "parent_type": "resource",
                "json": '{"processing_priority":"high"}',
            },
        }
        native_docs = {
            "/repositories/4/resources/1": {
                "id": "/repositories/4/resources/1",
                "uri": "/repositories/4/resources/1",
                "title": "Res 1",
                "json": '{"title":"Res 1"}',
                "_version_": 12345,
            },
            "/repositories/4/collection_management/1": {
                "id": "/repositories/4/collection_management/1",
                "uri": "/repositories/4/collection_management/1",
                "parent_id": "/repositories/4/resources/1",
                "parent_type": "resource",
                "json": '{"processing_priority":"high"}',
                "_version_": 67890,
            },
        }

        res = harness.run_slice_parity_comparison(cand_docs, native_docs, [])
        self.assertTrue(res["exact_parity"])
        self.assertEqual(res["total_differences"], 0)
        self.assertEqual(res["candidate_marker_leaks_count"], 0)
        self.assertEqual(res["embedded_json_mismatches_count"], 0)
        self.assertEqual(res["records_with_exact_equality"], 2)

    def test_run_slice_parity_comparison_embedded_json_mismatch(self):
        cand_docs = {
            "/repositories/4/resources/1": {
                "id": "/repositories/4/resources/1",
                "uri": "/repositories/4/resources/1",
                "title": "Res 1",
                "json": '{"title":"Res 1","extra":1}',
            },
        }
        native_docs = {
            "/repositories/4/resources/1": {
                "id": "/repositories/4/resources/1",
                "uri": "/repositories/4/resources/1",
                "title": "Res 1",
                "json": '{"title":"Res 1"}',
                "_version_": 12345,
            },
        }

        res = harness.run_slice_parity_comparison(cand_docs, native_docs, [])
        self.assertFalse(res["exact_parity"])
        self.assertEqual(res["embedded_json_mismatches_count"], 1)

    def test_preflight_checks_fail_on_low_memory(self):
        with patch("run_gate3_resource_candidate.get_mem_available_gib", return_value=12.0):
            with patch("run_gate3_resource_candidate.parse_args") as mock_args:
                mock_args.return_value = MagicMock(dry_run=True, state=harness.DEFAULT_STATE_FILE)
                with patch("run_gate3_resource_candidate.subprocess.run") as mock_subproc:
                    mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/gate3-full-staff\n")
                    with patch(
                        "run_gate3_resource_candidate.verify_binary_provenance",
                        return_value={"sha256": "mock", "path": "mock"},
                    ):
                        with self.assertRaises(RuntimeError) as cm:
                            harness.run_harness()
                        self.assertIn("Insufficient memory headroom", str(cm.exception))

    def test_build_candidate_cli_command_omits_password_and_secret(self):
        sentinel = "SUPER_SECRET_SENTINEL_XYZ_98765"
        cmd = harness.build_candidate_cli_command(
            cli_bin="/path/to/bin",
            backend_host_url="http://127.0.0.1:4567",
            admin_user="admin",
            state_path="/path/to/state.json",
            repo_id=4,
            solr_host_url="http://127.0.0.1:8983/solr",
            cand_core="cand_core_1",
            active_arks=True,
        )
        self.assertNotIn("--password", cmd)
        self.assertNotIn("-P", cmd)
        self.assertNotIn(sentinel, cmd)
        self.assertNotIn(sentinel, " ".join(cmd))
        self.assertNotIn(sentinel, str(cmd))

    def test_build_candidate_cli_env_sets_aspace_password_without_modifying_base(self):
        sentinel = "SUPER_SECRET_SENTINEL_XYZ_98765"
        base_env = {"EXISTING_VAR": "val1"}
        env = harness.build_candidate_cli_env(admin_password=sentinel, base_env=base_env)
        self.assertEqual(env["ASPACE_PASSWORD"], sentinel)
        self.assertEqual(env["EXISTING_VAR"], "val1")
        self.assertNotIn("ASPACE_PASSWORD", base_env)

    def test_redact_secrets_filters_plain_and_url_encoded_variants(self):
        sentinel = "P@ssw0rd with spaces & symbols!"
        harness.register_secret(sentinel)
        raw_err = f"Error connecting with password {sentinel} to server"
        redacted = harness.redact_secrets(raw_err)
        self.assertNotIn(sentinel, redacted)
        self.assertIn("<REDACTED>", redacted)

        quoted = "P%40ssw0rd%20with%20spaces%20%26%20symbols%21"
        raw_url = f"http://localhost:8089/login?password={quoted}"
        redacted_url = harness.redact_secrets(raw_url)
        self.assertNotIn(quoted, redacted_url)
        self.assertIn("<REDACTED>", redacted_url)

    def test_verify_active_app_arks_enabled_login_url_has_no_secret_in_url(self):
        sentinel = "SECRET_SENTINEL_PASSWORD_456"
        captured_requests = []

        def mock_urlopen(req, timeout=10):
            captured_requests.append(req)
            resp = MagicMock()
            resp.read.return_value = json.dumps({"session": "mock_session"}).encode("utf-8")
            resp.__enter__.return_value = resp
            return resp

        with patch("urllib.request.urlopen", side_effect=mock_urlopen):
            harness.verify_active_app_arks_enabled(
                "http://127.0.0.1:4567",
                "admin",
                sentinel,
            )

        self.assertTrue(len(captured_requests) >= 1)
        login_req = captured_requests[0]
        self.assertEqual(login_req.full_url, "http://127.0.0.1:4567/users/admin/login")
        self.assertNotIn(sentinel, login_req.full_url)
        self.assertNotIn("password=", login_req.full_url)
        self.assertIn(b"password=", login_req.data)

    def test_cli_error_diagnostics_redact_secret_in_stderr_and_stdout(self):
        sentinel = "CRITICAL_SENTINEL_SECRET_789"
        harness.register_secret(sentinel)

        bad_stderr = f"fatal: auth failed for password '{sentinel}'"
        redacted = harness.redact_secrets(bad_stderr)
        self.assertNotIn(sentinel, redacted)
        self.assertIn("<REDACTED>", redacted)

        bad_stdout = f"raw trace dump with {sentinel} inside"
        redacted_out = harness.redact_secrets(bad_stdout)
        self.assertNotIn(sentinel, redacted_out)
        self.assertIn("<REDACTED>", redacted_out)

        ex = RuntimeError(f"Command failed with {sentinel}")
        redacted_ex = harness.redact_secrets(ex)
        self.assertNotIn(sentinel, redacted_ex)
        self.assertIn("<REDACTED>", redacted_ex)

    def test_harness_cli_invocation_never_logs_or_passes_sentinel(self):
        sentinel = "HARNESS_SENTINEL_SECRET_XYZ"
        harness.register_secret(sentinel)

        cmd = harness.build_candidate_cli_command(
            cli_bin="/path/to/bin",
            backend_host_url="http://127.0.0.1:4567",
            admin_user="admin",
            state_path="/tmp/mock_state.json",
            repo_id=4,
            solr_host_url="http://127.0.0.1:8983/solr",
            cand_core="stage_staff_cand",
            active_arks=False,
        )
        env = harness.build_candidate_cli_env(admin_password=sentinel)

        # Assert no password arg in argv
        self.assertNotIn("--password", cmd)
        self.assertNotIn("-P", cmd)
        self.assertNotIn(sentinel, cmd)

        # Assert rendered command string has no sentinel or password flag
        rendered = " ".join(cmd)
        logged_cmd = f"Running CLI command: {harness.redact_secrets(rendered)}"
        self.assertNotIn(sentinel, logged_cmd)
        self.assertNotIn("--password", logged_cmd)

        # Simulate subprocess run capturing output with sentinel in stderr
        mock_proc = MagicMock(
            returncode=1,
            stdout="{}",
            stderr=f"Error connecting with {sentinel}",
        )
        stderr_io = io.StringIO()
        with contextlib.redirect_stderr(stderr_io):
            redacted_err = harness.redact_secrets(mock_proc.stderr)
            print(f"CLI stderr: {redacted_err}", file=sys.stderr)

        captured_stderr = stderr_io.getvalue()
        self.assertNotIn(sentinel, captured_stderr)
        self.assertIn("<REDACTED>", captured_stderr)


if __name__ == "__main__":
    unittest.main()
