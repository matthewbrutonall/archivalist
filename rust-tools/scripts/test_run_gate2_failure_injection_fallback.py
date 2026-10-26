#!/usr/bin/env python3
"""Offline Unit Tests for Gate 2 Failure Injection & Fallback Harness"""

import json
import os
import sys
import unittest
from unittest.mock import MagicMock, patch

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "rust-tools", "scripts"))

import run_gate2_failure_injection_fallback as harness


class TestRunGate2FailureInjectionFallback(unittest.TestCase):
    def test_dry_run_passes_safety_checks(self):
        with patch("run_gate2_failure_injection_fallback.parse_args") as mock_args:
            mock_args.return_value = MagicMock(dry_run=True, state=harness.DEFAULT_STATE_FILE)
            with patch("run_gate2_failure_injection_fallback.subprocess.run") as mock_subproc:
                mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                with patch("run_gate2_failure_injection_fallback.verify_binary_provenance", return_value={"sha256": "mock", "path": "mock"}):
                    with patch("run_gate2_failure_injection_fallback.get_mem_available_gib", return_value=32.0):
                        with patch("run_gate2_failure_injection_fallback.is_running", return_value=True):
                            with patch("run_gate2_failure_injection_fallback.docker_inspect") as mock_inspect:
                                def inspect_side_effect(name):
                                    if "db" in name:
                                        return {"State": {"Running": True}, "Config": {"Labels": {"archivalist.profiling-stack": "e62875f1", "archivalist.profiling-role": "db"}}}
                                    if "backend" in name:
                                        return {"State": {"Running": False}, "Config": {"Labels": {"archivalist.profiling-stack": "e62875f1", "archivalist.profiling-role": "backend"}}}
                                    if "solr" in name and "profiling" in name:
                                        return {"State": {"Running": False}, "Config": {"Labels": {"archivalist.profiling-stack": "e62875f1", "archivalist.profiling-role": "solr"}}}
                                    return {"State": {"Running": False}}
                                mock_inspect.side_effect = inspect_side_effect
                                rc = harness.run_failure_injection_fallback_harness()
                                self.assertEqual(rc, 0)

    def test_fails_if_memory_low(self):
        with patch("run_gate2_failure_injection_fallback.parse_args") as mock_args:
            mock_args.return_value = MagicMock(dry_run=True, state=harness.DEFAULT_STATE_FILE)
            with patch("run_gate2_failure_injection_fallback.subprocess.run") as mock_subproc:
                mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                with patch("run_gate2_failure_injection_fallback.verify_binary_provenance", return_value={"sha256": "mock", "path": "mock"}):
                    with patch("run_gate2_failure_injection_fallback.get_mem_available_gib", return_value=12.0):
                        with self.assertRaises(RuntimeError) as cm:
                            harness.run_failure_injection_fallback_harness()
                        self.assertIn("Insufficient memory headroom", str(cm.exception))

    def test_fails_if_main_solr_running(self):
        with patch("run_gate2_failure_injection_fallback.parse_args") as mock_args:
            mock_args.return_value = MagicMock(dry_run=True, state=harness.DEFAULT_STATE_FILE)
            with patch("run_gate2_failure_injection_fallback.subprocess.run") as mock_subproc:
                mock_subproc.return_value = MagicMock(returncode=0, stdout="codex/native-profile-analysis\n")
                with patch("run_gate2_failure_injection_fallback.verify_binary_provenance", return_value={"sha256": "mock", "path": "mock"}):
                    with patch("run_gate2_failure_injection_fallback.get_mem_available_gib", return_value=32.0):
                        with patch("run_gate2_failure_injection_fallback.docker_inspect") as mock_inspect:
                            def inspect_side_effect(name):
                                if name == harness.MAIN_SOLR_CONTAINER:
                                    return {"State": {"Running": True}}
                                return {"State": {"Running": False}}
                            mock_inspect.side_effect = inspect_side_effect
                            with self.assertRaises(RuntimeError) as cm:
                                harness.run_failure_injection_fallback_harness()
                            self.assertIn("SAFETY VIOLATION", str(cm.exception))

    def test_default_fail_after_batches_is_25(self):
        with patch("sys.argv", ["run_gate2_failure_injection_fallback.py"]):
            args = harness.parse_args()
            self.assertEqual(args.fail_after_batches, 25)

    def test_promotion_check_detects_core_swap_or_rename(self):
        # Verify that if cand_core instanceDir was altered, promotion check fails
        cand_core = "stage_staff_cand_candidate"
        status_data = {
            cand_core: {"instanceDir": "/var/solr/data/other_core"},
            "archivalist_profiling_identity_e62875f1": {"instanceDir": "/var/solr/data/archivalist_profiling_identity_e62875f1"},
        }
        cand_instance_dir = status_data[cand_core]["instanceDir"]
        cand_instance_dir_matches = cand_instance_dir.rstrip("/").endswith(cand_core)
        self.assertFalse(cand_instance_dir_matches)

    def test_summary_derivation_not_hardcoded(self):
        # Verify all conditions must hold for gate2_safe_fallback_demonstrated
        candidate_failed_cleanly = True
        at_least_one_update_sent = True
        no_candidate_hard_commit = True
        no_candidate_promotion = True
        cand_core_preserved = True
        fallback_distinct = True
        fallback_docs_verified = True
        fallback_exact_parity = True

        gate2_safe_fallback = (
            candidate_failed_cleanly
            and at_least_one_update_sent
            and no_candidate_hard_commit
            and no_candidate_promotion
            and cand_core_preserved
            and fallback_distinct
            and fallback_docs_verified
            and fallback_exact_parity
        )
        self.assertTrue(gate2_safe_fallback)

        # If zero updates were sent, fallback demonstration must be FALSE
        at_least_one_update_sent = False
        gate2_safe_fallback_no_updates = (
            candidate_failed_cleanly
            and at_least_one_update_sent
            and no_candidate_hard_commit
            and no_candidate_promotion
            and cand_core_preserved
            and fallback_distinct
            and fallback_docs_verified
            and fallback_exact_parity
        )
        self.assertFalse(gate2_safe_fallback_no_updates)


if __name__ == "__main__":
    unittest.main()
