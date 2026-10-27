#!/usr/bin/env python3
"""Offline Unit Tests for Gate 3 STAFF Resource Slice Parity Harness"""

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


if __name__ == "__main__":
    unittest.main()
