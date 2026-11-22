#!/usr/bin/env python3
"""Offline Unit and Safety Tests for Gate 2 Pre-Solr Comparison Harness."""

import hashlib
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))

from compare_gate2_ao_pre_solr import (
    collect_pre_solr_doc_diffs,
    compare_embedded_json_string,
    json_values_strictly_equal,
    normalize_path,
    run_pre_solr_comparison,
    value_sha256_and_size,
)


class TestPreSolrComparison(unittest.TestCase):
    def test_json_values_strictly_equal(self):
        # Type strict: int vs bool
        self.assertFalse(json_values_strictly_equal(1, True))
        self.assertFalse(json_values_strictly_equal(0, False))

        # Type strict: int vs float
        self.assertFalse(json_values_strictly_equal(1, 1.0))

        # Dict equality
        self.assertTrue(json_values_strictly_equal({"a": [1, 2]}, {"a": [1, 2]}))
        self.assertFalse(json_values_strictly_equal({"a": [1, 2]}, {"a": [1, "2"]}))

    def test_compare_embedded_json_string(self):
        # Exact match
        s1 = '{"title":"Test","id":1}'
        res = compare_embedded_json_string(s1, s1)
        self.assertTrue(res["exact_bytes_match"])
        self.assertTrue(res["parsed_equality"])
        self.assertEqual(res["parsed_diffs_count"], 0)

        # Same JSON parsed, different key order / formatting
        s2 = '{"id":1,"title":"Test"}'
        res2 = compare_embedded_json_string(s1, s2)
        self.assertFalse(res2["exact_bytes_match"])
        self.assertTrue(res2["parsed_equality"])
        self.assertEqual(res2["parsed_diffs_count"], 0)

        # Content difference
        s3 = '{"id":1,"title":"Different"}'
        res3 = compare_embedded_json_string(s1, s3)
        self.assertFalse(res3["exact_bytes_match"])
        self.assertFalse(res3["parsed_equality"])
        self.assertGreater(res3["parsed_diffs_count"], 0)

    def test_collect_pre_solr_doc_diffs_missing_and_extra(self):
        cand = {
            "id": "/repositories/4/archival_objects/1",
            "title": "Title 1",
            "extra_field": "foo",
        }
        native = {
            "id": "/repositories/4/archival_objects/1",
            "title": "Title 1",
            "missing_field": "bar",
        }
        diffs = collect_pre_solr_doc_diffs(cand, native)
        categories = {d["category"] for d in diffs}
        self.assertEqual(categories, {"missing_in_candidate", "extra_in_candidate"})

    def test_collect_pre_solr_doc_diffs_array_reordered_and_length(self):
        cand = {"tags": ["b", "a"]}
        native = {"tags": ["a", "b"]}
        diffs = collect_pre_solr_doc_diffs(cand, native)
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["category"], "array_reordered")

        cand_len = {"tags": ["a"]}
        native_len = {"tags": ["a", "b"]}
        diffs_len = collect_pre_solr_doc_diffs(cand_len, native_len)
        self.assertEqual(len(diffs_len), 1)
        self.assertEqual(diffs_len[0]["category"], "array_length_mismatch")

    def test_run_pre_solr_comparison_exact_match(self):
        native_docs = [
            {
                "uri": "/repositories/4/archival_objects/100",
                "title": "Archival item 100",
                "types": ["archival_object"],
                "fullrecord": ["unpublished note"],
                "extents": ["1 box"],
                "json": '{"title":"Archival item 100"}',
            }
        ]
        cand_docs = [
            {
                "uri": "/repositories/4/archival_objects/100",
                "title": "Archival item 100",
                "types": ["archival_object"],
                "fullrecord": ["unpublished note"],
                "extents": ["1 box"],
                "json": '{"title":"Archival item 100"}',
            }
        ]
        result = run_pre_solr_comparison(native_docs, cand_docs)
        self.assertEqual(result["records_compared"], 1)
        self.assertEqual(result["records_with_exact_equality"], 1)
        self.assertEqual(result["records_with_differences"], 0)
        self.assertEqual(result["embedded_json_bytes_matches"], 1)
        self.assertEqual(result["embedded_json_parsed_matches"], 1)


if __name__ == "__main__":
    unittest.main()
