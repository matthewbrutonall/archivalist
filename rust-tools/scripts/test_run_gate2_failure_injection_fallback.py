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


if __name__ == "__main__":
    unittest.main()
