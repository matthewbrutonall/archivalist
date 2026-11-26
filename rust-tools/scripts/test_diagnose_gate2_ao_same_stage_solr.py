#!/usr/bin/env python3
"""Offline Unit and Safety Tests for Gate 2 Same-Stage Stored-Solr Diagnostic."""

import hashlib
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

from diagnose_gate2_ao_same_stage_solr import (
    collect_same_stage_stored_diffs,
    normalize_path,
    normalize_solr_doc,
    value_sha256_and_size,
    write_file_mode_0600,
    verify_active_app_arks_enabled,
    verify_binary_provenance,
)


class TestSameStageStoredSolrDiagnostic(unittest.TestCase):
    def test_normalize_solr_doc_only_version(self):
        doc = {
            "id": "/repositories/4/archival_objects/1221",
            "title": "Item 1221",
            "_version_": 181395819230582,
            "suppressed": False,
            "types": ["archival_object"],
            "notes": "some notes",
        }
        normalized = normalize_solr_doc(doc)
        self.assertEqual(normalized["id"], "/repositories/4/archival_objects/1221")
        self.assertEqual(normalized["title"], "Item 1221")
        self.assertEqual(normalized["suppressed"], False)
        self.assertEqual(normalized["types"], ["archival_object"])
        self.assertEqual(normalized["notes"], "some notes")
        self.assertNotIn("_version_", normalized)

    def test_collect_diffs_stored_missing_and_extra(self):
        cand = {
            "id": "/repositories/4/archival_objects/1221",
            "extra_field": ["alpha", "beta"],
        }
        native = {
            "id": "/repositories/4/archival_objects/1221",
            "native_field": "present_in_native",
        }
        diffs = collect_same_stage_stored_diffs(cand, native)
        categories = {d["category"] for d in diffs}
        self.assertEqual(
            categories,
            {"stored_missing_in_candidate", "stored_extra_in_candidate"},
        )

        missing = next(d for d in diffs if d["category"] == "stored_missing_in_candidate")
        self.assertEqual(missing["field"], "native_field")
        self.assertEqual(missing["native_type"], "str")
        self.assertIn("native_hash", missing)
        self.assertIn("native_size", missing)
        # Verify raw string payload is NOT present in diff
        self.assertNotIn("present_in_native", missing.values())

        extra = next(d for d in diffs if d["category"] == "stored_extra_in_candidate")
        self.assertEqual(extra["field"], "extra_field")
        self.assertEqual(extra["candidate_type"], "list")
        self.assertIn("candidate_hash", extra)
        self.assertIn("candidate_size", extra)

    def test_collect_diffs_stored_type_and_value_mismatch(self):
        cand = {
            "id": "/repositories/4/archival_objects/1221",
            "count": 42,
            "title": "Candidate Title",
        }
        native = {
            "id": "/repositories/4/archival_objects/1221",
            "count": 42.0,  # float vs int
            "title": "Native Title",
        }
        diffs = collect_same_stage_stored_diffs(cand, native)
        categories = {d["category"] for d in diffs}
        self.assertEqual(categories, {"stored_type_mismatch", "stored_value_mismatch"})

        type_diff = next(d for d in diffs if d["category"] == "stored_type_mismatch")
        self.assertEqual(type_diff["path"], "count")
        self.assertEqual(type_diff["candidate_type"], "int")
        self.assertEqual(type_diff["native_type"], "float")

        val_diff = next(d for d in diffs if d["category"] == "stored_value_mismatch")
        self.assertEqual(val_diff["path"], "title")
        self.assertIn("candidate_hash", val_diff)
        self.assertIn("native_hash", val_diff)
        # Redacted: raw titles never in diff
        self.assertNotIn("Candidate Title", val_diff.values())
        self.assertNotIn("Native Title", val_diff.values())

    def test_collect_diffs_stored_array_order_change(self):
        cand = {"tags": ["apple", "banana", "cherry"]}
        native = {"tags": ["cherry", "banana", "apple"]}
        diffs = collect_same_stage_stored_diffs(cand, native)
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["category"], "stored_array_order_change")
        self.assertEqual(diffs[0]["path"], "tags")
        self.assertEqual(diffs[0]["length"], 3)

    def test_collect_diffs_exact_match_yields_zero_diffs(self):
        cand = {
            "id": "/repositories/4/archival_objects/1221",
            "title": "Exact Title",
            "suppressed": False,
            "subjects": ["Subject A", "Subject B"],
            "nested": {"level": "item", "pos": 0},
        }
        native = {
            "id": "/repositories/4/archival_objects/1221",
            "title": "Exact Title",
            "suppressed": False,
            "subjects": ["Subject A", "Subject B"],
            "nested": {"level": "item", "pos": 0},
        }
        diffs = collect_same_stage_stored_diffs(cand, native)
        self.assertEqual(diffs, [])

    def test_no_clobber_atomic_write(self):
        with tempfile.TemporaryDirectory() as td:
            target = os.path.join(td, "report.json")
            write_file_mode_0600(target, '{"initial": true}')
            self.assertTrue(os.path.exists(target))

            mode = stat.S_IMODE(os.stat(target).st_mode)
            self.assertEqual(mode, 0o600)

            # Attempt overwrite should raise FileExistsError without clobbering
            with self.assertRaises(FileExistsError):
                write_file_mode_0600(target, '{"overwrite": true}')

            # Verify original content preserved
            with open(target) as f:
                content = f.read()
            self.assertEqual(content, '{"initial": true}')

    def test_wrong_stack_id_refusal(self):
        from diagnose_gate2_ao_same_stage_solr import run_diagnostic

        with tempfile.NamedTemporaryFile("w", suffix=".json") as f:
            json.dump({"stack_id": "wrong_stack_id_1234"}, f)
            f.flush()

            test_args = ["diagnose_gate2_ao_same_stage_solr.py", "--state", f.name, "--dry-run"]
            with patch("sys.argv", test_args):
                with patch("diagnose_gate2_ao_same_stage_solr.subprocess.run") as mock_subproc:
                    mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                    with patch("diagnose_gate2_ao_same_stage_solr.verify_binary_provenance") as mock_provenance:
                        mock_provenance.return_value = {"sha256": "mock_sha256", "size": 1234}
                        with self.assertRaises(RuntimeError) as cm:
                            run_diagnostic()
                        self.assertIn("wrong_stack_id_1234", str(cm.exception))

    def test_main_solr_running_refusal(self):
        from diagnose_gate2_ao_same_stage_solr import run_diagnostic

        fake_state = {
            "stack_id": "e62875f1",
            "containers": {
                "db": "archivalist-profiling-db-e62875f1",
                "solr": "archivalist-profiling-solr-e62875f1",
                "backend": "archivalist-profiling-backend-e62875f1",
            },
        }

        with tempfile.NamedTemporaryFile("w", suffix=".json") as f:
            json.dump(fake_state, f)
            f.flush()

            test_args = ["diagnose_gate2_ao_same_stage_solr.py", "--state", f.name, "--dry-run"]
            with patch("sys.argv", test_args):
                with patch("diagnose_gate2_ao_same_stage_solr.subprocess.run") as mock_subproc:
                    mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                    with patch("diagnose_gate2_ao_same_stage_solr.verify_binary_provenance") as mock_provenance:
                        mock_provenance.return_value = {"sha256": "mock_sha256", "size": 1234}
                        with patch("diagnose_gate2_ao_same_stage_solr.docker_inspect") as mock_inspect:
                            # Mock MAIN_SOLR_CONTAINER as running
                            def inspect_side_effect(name):
                                if name == "archivesspaceai-solr-1":
                                    return {"State": {"Running": True}}
                                return {
                                    "State": {"Running": False},
                                    "Config": {
                                        "Labels": {
                                            "archivalist.profiling-stack": "e62875f1",
                                            "archivalist.profiling-role": name.split("-")[2],
                                        }
                                    },
                                }

                            mock_inspect.side_effect = inspect_side_effect
                            with self.assertRaises(RuntimeError) as cm:
                                run_diagnostic()
                            self.assertIn("Main Solr container 'archivesspaceai-solr-1' is currently running", str(cm.exception))

    def test_candidate_core_already_exists_no_clobber_refusal(self):
        from diagnose_gate2_ao_same_stage_solr import run_diagnostic

        fake_state = {
            "stack_id": "e62875f1",
            "containers": {
                "db": "archivalist-profiling-db-e62875f1",
                "solr": "archivalist-profiling-solr-e62875f1",
                "backend": "archivalist-profiling-backend-e62875f1",
            },
            "solr_host_url": "http://127.0.0.1:18983/solr",
            "backend_host_url": "http://127.0.0.1:14567",
            "identity_core": "archivalist_profiling_identity_e62875f1",
            "admin_user": "admin",
            "admin_password": "admin",
        }

        with tempfile.NamedTemporaryFile("w", suffix=".json") as f:
            json.dump(fake_state, f)
            f.flush()

            test_args = ["diagnose_gate2_ao_same_stage_solr.py", "--state", f.name]
            with patch("sys.argv", test_args):
                with patch("diagnose_gate2_ao_same_stage_solr.verify_binary_provenance") as mock_provenance:
                    mock_provenance.return_value = {"sha256": "mock_sha256", "size": 1234}
                    with patch("diagnose_gate2_ao_same_stage_solr.time.sleep") as mock_sleep:
                        with patch("diagnose_gate2_ao_same_stage_solr.docker_inspect") as mock_inspect:
                            def inspect_mock(name):
                                if name == "archivesspaceai-solr-1":
                                    return {"State": {"Running": False}}
                                return {
                                    "State": {"Running": True},
                                    "Config": {
                                        "Labels": {
                                            "archivalist.profiling-stack": "e62875f1",
                                            "archivalist.profiling-role": name.split("-")[2] if "-" in name else "db",
                                        }
                                    },
                                }
                            mock_inspect.side_effect = inspect_mock
                            with patch("diagnose_gate2_ao_same_stage_solr.urllib.request.urlopen") as mock_urlopen:
                                with patch("diagnose_gate2_ao_same_stage_solr.verify_active_app_arks_enabled") as mock_arks:
                                    mock_arks.return_value = ("genuine_active_process", False)
                                    with patch("diagnose_gate2_ao_same_stage_solr.subprocess.run") as mock_subproc:
                                        executed_commands = []

                                        def subproc_side_effect(cmd, *a, **kw):
                                            executed_commands.append(cmd)
                                            if cmd[0] == "git":
                                                if "--abbrev-ref" in cmd:
                                                    return MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                                                return MagicMock(returncode=0, stdout="e48599e0a897cc816cfbfccb0e28c2f3429da299\n")
                                            if len(cmd) > 1 and "profiling_stack.py" in cmd[1]:
                                                return MagicMock(returncode=0, stdout="Verified: ok\n")
                                            raise AssertionError(f"Unexpected external subprocess execution: {cmd}")

                                        mock_subproc.side_effect = subproc_side_effect

                                        def make_mock_response(body_bytes: bytes) -> MagicMock:
                                            mock_res = MagicMock()
                                            mock_res.read.return_value = body_bytes
                                            mock_res.__enter__.return_value = mock_res
                                            mock_res.__exit__.return_value = None
                                            return mock_res

                                        def urlopen_side_effect(req, timeout=None):
                                            url = req.full_url if hasattr(req, "full_url") else str(req)
                                            if "candidate" in url and "action=STATUS" in url:
                                                # Simulates candidate core already present in Solr
                                                core_name = url.split("core=")[1].split("&")[0]
                                                return make_mock_response(json.dumps({
                                                    "responseHeader": {"status": 0},
                                                    "status": {core_name: {"name": core_name}}
                                                }).encode("utf-8"))
                                            if "action=STATUS" in url:
                                                return make_mock_response(json.dumps({
                                                    "responseHeader": {"status": 0},
                                                    "status": {
                                                        "stage_staff_20261025_224741_59214_8a08a5d2_native": {"name": "native"},
                                                        "archivalist_profiling_identity_e62875f1": {"name": "id"},
                                                    }
                                                }).encode("utf-8"))
                                            if "rows=0" in url:
                                                return make_mock_response(json.dumps({
                                                    "responseHeader": {"status": 0},
                                                    "response": {"numFound": 24200, "docs": []}
                                                }).encode("utf-8"))
                                            return make_mock_response(json.dumps({
                                                "databaseProductName": "MySQL"
                                            }).encode("utf-8"))

                                        mock_urlopen.side_effect = urlopen_side_effect
                                        with self.assertRaises(RuntimeError) as cm:
                                            run_diagnostic()
                                        self.assertIn("already exists before creation! No-clobber refusal", str(cm.exception))

                                        # Assertions for zero external/Docker calls and zero sleep retries
                                        self.assertEqual(mock_sleep.call_count, 0, "No retry sleep should occur with properly configured mock responses")
                                        self.assertGreater(len(executed_commands), 0)
                                        for cmd in executed_commands:
                                            self.assertNotIn("docker", cmd, f"Docker must not be invoked during unit test: {cmd}")


if __name__ == "__main__":
    unittest.main()
