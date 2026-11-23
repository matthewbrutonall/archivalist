#!/usr/bin/env python3
"""Offline Unit Tests for Gate 3 STAFF Combined Slice Parity Harness"""

import contextlib
import io
import json
import os
import sys
import unittest
from unittest.mock import MagicMock, patch

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "rust-tools", "scripts"))

import run_gate3_combined_candidate as harness


class TestRunGate3CombinedCandidate(unittest.TestCase):
    def test_native_command_matches_staff_threads_and_records_effective_java_opts(self):
        state = {
            "backend_network_url": "http://archivalist-profiling-backend-844ef09c:4567",
            "solr_network_url": "http://archivalist-profiling-solr-844ef09c:8983/solr",
            "stack_id": "844ef09c",
            "identity_repo_id": 2,
            "identity_core": "archivalist_profiling_identity_844ef09c",
            "app_container": "archivesspaceai-app-1",
        }
        command = harness.build_native_profile_command(state, "a" * 32, "-Xmx8g", 12)
        self.assertIn("STAFF_THREADS=12", command)
        self.assertIn("JAVA_OPTS=-Xmx8g -Xss16m", command)
        self.assertIn("MODES=staff", command)
        self.assertIn("TIMERS=0", command)

    def test_native_staff_threads_argument_rejects_out_of_range_value(self):
        with patch.object(sys, "argv", ["harness", "--native-staff-threads", "33"]):
            with self.assertRaises(SystemExit):
                harness.parse_args()

    def test_mixed_type_coverage_requires_every_type_and_child_parent(self):
        docs = {kind: {"primary_type": kind} for kind in harness.EXPECTED_STAFF_TYPES}
        for parent in harness.EXPECTED_CHILD_PARENTS:
            docs[f"child-{parent}"] = {"primary_type": "collection_management", "parent_type": parent}
        types, parents = harness.mixed_type_counts(docs)
        harness.require_mixed_type_coverage("native", types, parents)
        del docs["job"]
        del docs["child-accession"]
        types, parents = harness.mixed_type_counts(docs)
        with self.assertRaisesRegex(RuntimeError, "job"):
            harness.require_mixed_type_coverage("native", types, parents)

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
            "/repositories/4/archival_objects/101": {
                "id": "/repositories/4/archival_objects/101",
                "uri": "/repositories/4/archival_objects/101",
                "title": "AO 101",
                "json": '{"title":"AO 101"}',
            },
            "/repositories/4/collection_management/1": {
                "id": "/repositories/4/collection_management/1",
                "uri": "/repositories/4/collection_management/1",
                "parent_id": "/repositories/4/resources/1",
                "parent_type": "resource",
                "json": '{"processing_priority":"high"}',
            },
            "/repositories/4/digital_objects/501": {
                "id": "/repositories/4/digital_objects/501",
                "uri": "/repositories/4/digital_objects/501",
                "title": "DO 501",
                "json": '{"title":"DO 501"}',
            },
            "/repositories/4/top_containers/1001": {
                "id": "/repositories/4/top_containers/1001",
                "uri": "/repositories/4/top_containers/1001",
                "indicator": "Box 1",
                "json": '{"indicator":"Box 1"}',
            },
            "/repositories/4/collection_management/2": {
                "id": "/repositories/4/collection_management/2",
                "uri": "/repositories/4/collection_management/2",
                "parent_id": "/repositories/4/digital_objects/501",
                "parent_type": "digital_object",
                "json": '{"processing_priority":"medium"}',
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
            "/repositories/4/archival_objects/101": {
                "id": "/repositories/4/archival_objects/101",
                "uri": "/repositories/4/archival_objects/101",
                "title": "AO 101",
                "json": '{"title":"AO 101"}',
                "_version_": 54321,
            },
            "/repositories/4/collection_management/1": {
                "id": "/repositories/4/collection_management/1",
                "uri": "/repositories/4/collection_management/1",
                "parent_id": "/repositories/4/resources/1",
                "parent_type": "resource",
                "json": '{"processing_priority":"high"}',
                "_version_": 67890,
            },
            "/repositories/4/digital_objects/501": {
                "id": "/repositories/4/digital_objects/501",
                "uri": "/repositories/4/digital_objects/501",
                "title": "DO 501",
                "json": '{"title":"DO 501"}',
                "_version_": 98765,
            },
            "/repositories/4/top_containers/1001": {
                "id": "/repositories/4/top_containers/1001",
                "uri": "/repositories/4/top_containers/1001",
                "indicator": "Box 1",
                "json": '{"indicator":"Box 1"}',
                "_version_": 11223,
            },
            "/repositories/4/collection_management/2": {
                "id": "/repositories/4/collection_management/2",
                "uri": "/repositories/4/collection_management/2",
                "parent_id": "/repositories/4/digital_objects/501",
                "parent_type": "digital_object",
                "json": '{"processing_priority":"medium"}',
                "_version_": 87654,
            },
        }

        res = harness.run_slice_parity_comparison(cand_docs, native_docs, [])
        self.assertTrue(res["exact_parity"])
        self.assertEqual(res["total_differences"], 0)
        self.assertEqual(res["candidate_marker_leaks_count"], 0)
        self.assertEqual(res["embedded_json_mismatches_count"], 0)
        self.assertEqual(res["records_with_exact_equality"], 6)

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
        with patch("run_gate3_combined_candidate.get_mem_available_gib", return_value=12.0):
            with patch("run_gate3_combined_candidate.parse_args") as mock_args:
                mock_args.return_value = MagicMock(dry_run=True, state=harness.DEFAULT_STATE_FILE)
                mock_args.return_value.full = False
                mock_args.return_value.fresh_native = False
                mock_args.return_value.require_mixed_types = False
                mock_args.return_value.inject_failure_after = None
                with patch("run_gate3_combined_candidate.subprocess.run") as mock_subproc:
                    mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/gate3-full-staff\n")
                    with patch(
                        "run_gate3_combined_candidate.verify_binary_provenance",
                        return_value={"sha256": "mock", "path": "mock"},
                    ):
                        with self.assertRaises(RuntimeError) as cm:
                            harness.run_harness()
                        self.assertIn("Insufficient memory headroom", str(cm.exception))

    def test_failure_injection_requires_full_native_fallback(self):
        with patch("run_gate3_combined_candidate.parse_args") as mock_args:
            args = MagicMock()
            args.full = True
            args.fresh_native = False
            args.inject_failure_after = 60
            mock_args.return_value = args
            with self.assertRaisesRegex(ValueError, "requires --full --fresh-native"):
                harness.run_harness()

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
            concurrency=12,
        )
        self.assertNotIn("--password", cmd)
        self.assertNotIn("-P", cmd)
        self.assertNotIn(sentinel, cmd)
        self.assertNotIn(sentinel, " ".join(cmd))
        self.assertNotIn(sentinel, str(cmd))
        self.assertIn("staff-combined-candidate", cmd)
        self.assertEqual(cmd[cmd.index("--concurrency") + 1], "12")

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


if __name__ == "__main__":
    unittest.main()
