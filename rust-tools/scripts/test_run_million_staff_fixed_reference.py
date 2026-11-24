#!/usr/bin/env python3
"""Offline unit tests for run_million_staff_fixed_reference.py."""

import json
import os
import sys
import unittest
from unittest.mock import MagicMock, patch

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import run_million_staff_fixed_reference as runner  # noqa: E402


class TestRunMillionStaffFixedReference(unittest.TestCase):
    def setUp(self):
        self._orig = {
            k: getattr(runner.harness, k)
            for k in (
                "EXPECTED_STACK_ID",
                "EXPECTED_BRANCH",
                "DEFAULT_STATE_FILE",
                "DEFAULT_NATIVE_CORE",
                "EXPECTED_TOTAL_NATIVE_DOCS",
                "SOLR_CONTAINER",
                "BACKEND_CONTAINER",
                "DB_CONTAINER",
                "run_with_peak_rss",
            )
        }

    def tearDown(self):
        for k, v in self._orig.items():
            setattr(runner.harness, k, v)

    def test_parse_runner_args_defaults_and_help_safe(self):
        args = runner.parse_runner_args([])
        self.assertEqual(args.native_core, "stage_staff_20261124_010605_9807_36038e70_native")
        self.assertEqual(args.concurrency, 12)
        self.assertFalse(args.dry_run)

        with self.assertRaises(SystemExit) as cm:
            runner.parse_runner_args(["--help"])
        self.assertEqual(cm.exception.code, 0)

    def test_configure_harness_sets_fixed_reference_without_fresh_native(self):
        args = runner.parse_runner_args(["--dry-run"])
        argv = runner.configure_harness(args)
        self.assertIn("--full", argv)
        self.assertIn("--dry-run", argv)
        self.assertNotIn("--fresh-native", argv)
        self.assertEqual(argv[argv.index("--concurrency") + 1], "12")
        self.assertEqual(
            argv[argv.index("--native-core") + 1],
            "stage_staff_20261124_010605_9807_36038e70_native",
        )
        self.assertEqual(runner.harness.EXPECTED_STACK_ID, "844ef09c")
        self.assertEqual(runner.harness.EXPECTED_BRANCH, "codex/million-snapshot-seed")
        self.assertEqual(runner.harness.EXPECTED_TOTAL_NATIVE_DOCS, 1036427)

    def test_configure_harness_rejects_accidental_empty_core_as_native_ref(self):
        args = runner.parse_runner_args(
            ["--native-core", runner.ACCIDENTAL_EMPTY_CANDIDATE_CORE]
        )
        with self.assertRaisesRegex(ValueError, "accidental empty candidate core"):
            runner.configure_harness(args)

    def test_verify_no_active_benchmark_processes_rejects_running_job(self):
        fake_ps_conflict = (
            "100 systemd /sbin/init\n"
            "3353972 agy agy --model gemini-4-argon-high --print-timeout 1800s -p run_million_staff_benchmark.py\n"
            "22222 aspace-cli /home/user/rust-tools/target/release/aspace-cli staff-full-candidate\n"
        )
        with patch("run_million_staff_fixed_reference.subprocess.run") as mock_run:
            mock_run.return_value = MagicMock(returncode=0, stdout=fake_ps_conflict)
            with self.assertRaisesRegex(RuntimeError, "another benchmark process is active"):
                runner.verify_no_active_benchmark_processes(current_pid=99999)

        fake_ps_clean = (
            "100 systemd /sbin/init\n"
            "3353968 bash /bin/bash -c cat > /tmp/prompt.md run_million_staff_benchmark.py\n"
            "3353972 agy agy --model gemini-4-argon-high --print-timeout 1800s -p run_million_staff_benchmark.py\n"
        )
        with patch("run_million_staff_fixed_reference.subprocess.run") as mock_run:
            mock_run.return_value = MagicMock(returncode=0, stdout=fake_ps_clean)
            runner.verify_no_active_benchmark_processes(current_pid=99999)

    def test_verify_retained_empty_candidate_core_rejects_non_empty_or_missing(self):
        def make_resp(payload):
            m = MagicMock()
            m.read.return_value = json.dumps(payload).encode("utf-8")
            m.__enter__.return_value = m
            return m

        # Pass case: present and numFound == 0
        with patch(
            "urllib.request.urlopen",
            side_effect=[
                make_resp({"status": {runner.ACCIDENTAL_EMPTY_CANDIDATE_CORE: {"name": runner.ACCIDENTAL_EMPTY_CANDIDATE_CORE}}}),
                make_resp({"response": {"numFound": 0}}),
            ],
        ):
            runner.verify_retained_empty_candidate_core("http://127.0.0.1:28983/solr")

        # Fail case: numFound != 0
        with patch(
            "urllib.request.urlopen",
            side_effect=[
                make_resp({"status": {runner.ACCIDENTAL_EMPTY_CANDIDATE_CORE: {"name": runner.ACCIDENTAL_EMPTY_CANDIDATE_CORE}}}),
                make_resp({"response": {"numFound": 5}}),
            ],
        ):
            with self.assertRaisesRegex(RuntimeError, "numFound=5"):
                runner.verify_retained_empty_candidate_core("http://127.0.0.1:28983/solr")

        # Fail case: missing core
        with patch(
            "urllib.request.urlopen",
            side_effect=[make_resp({"status": {}})],
        ):
            with self.assertRaisesRegex(RuntimeError, "is missing"):
                runner.verify_retained_empty_candidate_core("http://127.0.0.1:28983/solr")

    def test_validate_watermark_in_cli_summary_enforces_positive_int(self):
        self.assertEqual(
            runner.validate_watermark_in_cli_summary(
                json.dumps({"full_reindex_started_at_epoch_sec": 1795499000, "committed": True})
            ),
            1795499000,
        )
        for bad in ({}, {"full_reindex_started_at_epoch_sec": 0}, {"full_reindex_started_at_epoch_sec": True}, {"full_reindex_started_at_epoch_sec": "1795499000"}):
            with self.assertRaisesRegex(RuntimeError, "full_reindex_started_at_epoch_sec"):
                runner.validate_watermark_in_cli_summary(json.dumps(bad))


if __name__ == "__main__":
    unittest.main()
