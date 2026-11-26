#!/usr/bin/env python3
"""Offline Unit Tests for Gate 2 Full AO Candidate Vertical Slice Harness"""

import json
import os
import sys
import unittest
from unittest.mock import MagicMock, patch

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "rust-tools", "scripts"))

import run_gate2_full_ao_candidate as harness


class TestRunGate2FullAoCandidate(unittest.TestCase):
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
        self.assertTrue(harness.is_array_reordered(d1["b"], d3["b"]))

    def test_collect_diffs_detects_mismatches_and_redacts(self):
        c_doc = {"uri": "/repositories/4/archival_objects/1", "title": "Secret Candidate Title", "level": "item"}
        n_doc = {"uri": "/repositories/4/archival_objects/1", "title": "Secret Native Title", "level": "series"}

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

    def test_run_full_parity_comparison_success_and_marker_leak(self):
        cand_docs = {
            "/repositories/4/archival_objects/1": {"id": "1", "uri": "/repositories/4/archival_objects/1", "title": "T1"},
            "/repositories/4/archival_objects/2": {"id": "2", "uri": "/repositories/4/archival_objects/2", "title": "T2"},
        }
        native_docs = {
            "/repositories/4/archival_objects/1": {"id": "1", "uri": "/repositories/4/archival_objects/1", "title": "T1", "_version_": 123},
            "/repositories/4/archival_objects/2": {"id": "2", "uri": "/repositories/4/archival_objects/2", "title": "T2", "_version_": 456},
        }

        # 1. Clean match
        res = harness.run_full_parity_comparison(cand_docs, native_docs, [])
        self.assertTrue(res["exact_parity"])
        self.assertEqual(res["total_differences"], 0)
        self.assertEqual(res["candidate_marker_leaks_count"], 0)

        # 2. Marker leak detected
        res_leak = harness.run_full_parity_comparison(
            cand_docs, native_docs, ["/repositories/4/archival_objects/1:_archivalist_marker"]
        )
        self.assertFalse(res_leak["exact_parity"])
        self.assertEqual(res_leak["candidate_marker_leaks_count"], 1)

    def test_preflight_checks_fail_on_low_memory(self):
        with patch("run_gate2_full_ao_candidate.get_mem_available_gib", return_value=12.0):
            with patch("run_gate2_full_ao_candidate.parse_args") as mock_args:
                mock_args.return_value = MagicMock(dry_run=True, state=harness.DEFAULT_STATE_FILE)
                with patch("run_gate2_full_ao_candidate.subprocess.run") as mock_subproc:
                    mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                    with patch("run_gate2_full_ao_candidate.verify_binary_provenance", return_value={"sha256": "mock", "path": "mock"}):
                        with self.assertRaises(RuntimeError) as cm:
                            harness.run_harness()
                        self.assertIn("Insufficient memory headroom", str(cm.exception))

    def test_preflight_checks_fail_if_main_solr_running(self):
        with patch("run_gate2_full_ao_candidate.get_mem_available_gib", return_value=32.0):
            with patch("run_gate2_full_ao_candidate.parse_args") as mock_args:
                mock_args.return_value = MagicMock(dry_run=True, state=harness.DEFAULT_STATE_FILE)
                with patch("run_gate2_full_ao_candidate.subprocess.run") as mock_subproc:
                    mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                    with patch("run_gate2_full_ao_candidate.verify_binary_provenance", return_value={"sha256": "mock", "path": "mock"}):
                        with patch("run_gate2_full_ao_candidate.docker_inspect") as mock_inspect:
                            def inspect_side_effect(name):
                                if name == harness.MAIN_SOLR_CONTAINER:
                                    return {"State": {"Running": True}}
                                return {"State": {"Running": False}}
                            mock_inspect.side_effect = inspect_side_effect
                            with self.assertRaises(RuntimeError) as cm:
                                harness.run_harness()
                            self.assertIn("SAFETY VIOLATION", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
