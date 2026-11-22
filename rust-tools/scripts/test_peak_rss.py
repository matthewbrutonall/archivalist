#!/usr/bin/env python3
"""Offline tests for peak_rss.py (per-child peak RSS capture and comparison)."""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from peak_rss import compare_peak_memory, run_with_peak_rss

ALLOC_MIB = 200


class TestRunWithPeakRss(unittest.TestCase):
    def test_captures_output_exit_code_and_child_peak(self):
        code = (
            "import sys\n"
            f"block = bytearray({ALLOC_MIB} * 1024 * 1024)\n"
            "for i in range(0, len(block), 4096): block[i] = 1\n"
            "print('out-line'); print('err-line', file=sys.stderr)\n"
            "sys.exit(3)\n"
        )
        result = run_with_peak_rss([sys.executable, "-c", code])
        self.assertEqual(result.returncode, 3)
        self.assertEqual(result.stdout.strip(), "out-line")
        self.assertEqual(result.stderr.strip(), "err-line")
        # Peak must include the touched allocation, but not be wildly larger.
        self.assertGreaterEqual(result.peak_rss_kib, ALLOC_MIB * 1024)
        self.assertLess(result.peak_rss_kib, (ALLOC_MIB + 300) * 1024)

    def test_small_child_is_not_charged_for_earlier_large_child(self):
        big = "b = bytearray(300 * 1024 * 1024)\nfor i in range(0, len(b), 4096): b[i] = 1\n"
        run_with_peak_rss([sys.executable, "-c", big])
        small = run_with_peak_rss([sys.executable, "-c", "pass"])
        self.assertEqual(small.returncode, 0)
        self.assertLess(small.peak_rss_kib, 150 * 1024)

    def test_env_is_passed_and_large_output_does_not_deadlock(self):
        code = "import os, sys\nsys.stdout.write(os.environ['PEAK_RSS_TEST'] * 2_000_000)\n"
        env = dict(os.environ, PEAK_RSS_TEST="x")
        result = run_with_peak_rss([sys.executable, "-c", code], env=env)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout), 2_000_000)

    def test_signal_exit_is_negative(self):
        code = "import os, signal\nos.kill(os.getpid(), signal.SIGTERM)\n"
        result = run_with_peak_rss([sys.executable, "-c", code])
        self.assertEqual(result.returncode, -15)


class TestComparePeakMemory(unittest.TestCase):
    def native(self, hwm):
        return {"results": [{"mode": "staff", "process_memory": {"vm_hwm_after_kib": hwm}}]}

    def test_reduction_and_target(self):
        summary = compare_peak_memory(1_000_000, self.native(2_000_000), "-Xmx4g")
        self.assertEqual(summary["reduction_pct"], 50.0)
        self.assertTrue(summary["meets_target"])
        self.assertEqual(summary["native_java_opts"], "-Xmx4g")

    def test_below_target_fails(self):
        summary = compare_peak_memory(1_900_000, self.native(2_000_000))
        self.assertEqual(summary["reduction_pct"], 5.0)
        self.assertFalse(summary["meets_target"])

    def test_missing_measurement_is_unknown_not_pass_or_fail(self):
        for cand, native in [
            (None, self.native(2_000_000)),
            (1_000_000, None),
            (1_000_000, {"results": []}),
            (1_000_000, {"results": [{}, {}]}),
            (0, self.native(2_000_000)),
        ]:
            summary = compare_peak_memory(cand, native)
            self.assertIsNone(summary["meets_target"])
            self.assertIsNone(summary["reduction_pct"])


if __name__ == "__main__":
    unittest.main()
