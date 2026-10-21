import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import summarize_indexing_profile as sip


def rep(wall, cats):
    return {"wall_ms": wall, "summary": {"region_ms": wall, "by_category_exclusive_ms": cats}}


class SummarizeTest(unittest.TestCase):
    def test_medians_overhead_and_padding(self):
        mode = {
            "uninstrumented_median_ms": 100.0,
            "profiled_median_ms": 110.0,
            "profiled_repeats": [
                rep(100.0, {"a": 60.0, "b": 40.0}),
                rep(110.0, {"a": 70.0, "b": 40.0}),
                rep(120.0, {"a": 80.0}),  # b absent -> counts as 0
            ],
        }
        out = sip.summarize_mode(mode)
        self.assertEqual(out["profiling_overhead_ratio"], 1.1)
        self.assertEqual(out["region_median_ms"], 110.0)
        self.assertEqual(out["categories"]["a"]["median_exclusive_ms"], 70.0)
        self.assertEqual(out["categories"]["b"]["median_exclusive_ms"], 40.0)
        self.assertEqual(list(out["categories"]), ["a", "b"])
        self.assertAlmostEqual(out["categories"]["a"]["share_of_region_median"], 70.0 / 110.0, places=3)

    def test_empty_mode(self):
        out = sip.summarize_mode({})
        self.assertIsNone(out["profiling_overhead_ratio"])
        self.assertEqual(out["categories"], {})


if __name__ == "__main__":
    unittest.main()
