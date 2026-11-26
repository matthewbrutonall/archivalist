#!/usr/bin/env python3
"""Run the full STAFF parity harness against the isolated fictional million-AO stack."""

import os
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", ".."))
sys.path.insert(0, SCRIPT_DIR)

import run_gate3_combined_candidate as harness

harness.EXPECTED_STACK_ID = "844ef09c"
harness.EXPECTED_BRANCH = "codex/million-snapshot-seed"
harness.SOLR_CONTAINER = "archivalist-profiling-solr-844ef09c"
harness.BACKEND_CONTAINER = "archivalist-profiling-backend-844ef09c"
harness.DB_CONTAINER = "archivalist-profiling-db-844ef09c"

sys.argv = [
    "run_gate3_combined_candidate.py",
    "--state",
    os.path.join(REPO_ROOT, "rust-tools", "benchmarks", "profiling_stack_844ef09c.json"),
    "--full",
    "--fresh-native",
    "--concurrency",
    "12",
    "--native-staff-threads",
    "12",
]

raise SystemExit(harness.run_harness())
