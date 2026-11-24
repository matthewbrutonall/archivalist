#!/usr/bin/env python3
"""Run the full STAFF parity harness in fixed-reference mode against isolated stack 844ef09c.

Reuses run_gate3_combined_candidate.py in fixed-reference mode (--full without --fresh-native):
  - Stack ID: 844ef09c
  - Branch: codex/million-snapshot-seed
  - Fixed read-only native reference core: stage_staff_20261124_010605_9807_36038e70_native
  - Expected total native documents: 1,036,427
  - Candidate concurrency: 12
  - Preserves accidental empty core stage_staff_20261124_054050_3352672_b76bd497_candidate (verifies numFound == 0)
  - Refuses execution if another benchmark run is active
  - Verifies CLI success summary includes positive integer full_reindex_started_at_epoch_sec
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from typing import List, Mapping, Optional
import urllib.parse
import urllib.request

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", ".."))
sys.path.insert(0, SCRIPT_DIR)

import run_gate3_combined_candidate as harness  # noqa: E402
from peak_rss import PeakRssResult, run_with_peak_rss as _orig_run_with_peak_rss  # noqa: E402

EXPECTED_STACK_ID = "844ef09c"
EXPECTED_BRANCH = "codex/million-snapshot-seed"
DEFAULT_STATE_FILE = os.path.join(
    REPO_ROOT, "rust-tools", "benchmarks", "profiling_stack_844ef09c.json"
)
FIXED_NATIVE_CORE = "stage_staff_20261124_010605_9807_36038e70_native"
ACCIDENTAL_EMPTY_CANDIDATE_CORE = (
    "stage_staff_20261124_054050_3352672_b76bd497_candidate"
)
EXPECTED_TOTAL_NATIVE_DOCS = 1036427
DEFAULT_CONCURRENCY = 12


def parse_runner_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Full STAFF Rust candidate fixed-reference parity runner for stack 844ef09c "
            "(does NOT run a fresh native pass)."
        )
    )
    parser.add_argument(
        "--state",
        default=DEFAULT_STATE_FILE,
        help="Path to profiling_stack_844ef09c.json state file",
    )
    parser.add_argument(
        "--native-core",
        default=FIXED_NATIVE_CORE,
        help="Retained read-only native reference Solr core name",
    )
    parser.add_argument(
        "--concurrency",
        type=int,
        default=DEFAULT_CONCURRENCY,
        choices=range(1, 33),
        help="Concurrent Rust fetch/build batches in full mode (default: 12)",
    )
    parser.add_argument(
        "--output-dir",
        default=harness.BENCHMARKS_DIR,
        help="Directory to save mode-0600 output report",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Run safety and static preflight checks without writing to Solr",
    )
    return parser.parse_args(argv)


def verify_no_active_benchmark_processes(current_pid: Optional[int] = None) -> None:
    my_pid = os.getpid() if current_pid is None else current_pid
    parent_pid = os.getppid()
    proc = subprocess.run(
        ["ps", "-eo", "pid=,comm=,args="],
        capture_output=True,
        text=True,
        check=True,
    )
    python_runners = (
        "run_gate3_combined_candidate.py",
        "run_million_staff_benchmark.py",
        "run_million_staff_fixed_reference.py",
        "run_gate4_no_change_handoff.py",
        "run_gate4_mutation_during_pass_handoff.py",
    )
    conflicts: List[str] = []
    for raw_line in proc.stdout.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        parts = line.split(None, 2)
        if len(parts) < 3:
            continue
        try:
            pid = int(parts[0])
        except ValueError:
            continue
        if pid in (my_pid, parent_pid):
            continue
        comm, cmd = parts[1], parts[2]
        if comm in ("agy", "cat", "grep", "ps"):
            continue
        is_aspace_cli = comm == "aspace-cli" or (
            "aspace-cli" in cmd
            and ("staff-full-candidate" in cmd or "staff-combined-candidate" in cmd)
            and "--print-timeout" not in cmd
            and "cat >" not in cmd
        )
        is_python_runner = comm.startswith("python") and any(
            runner_name in cmd for runner_name in python_runners
        )
        is_native_profiler = comm in ("java", "jruby", "ruby") and (
            "native_indexing_profile.rb" in cmd
        )
        if is_aspace_cli or is_python_runner or is_native_profiler:
            conflicts.append(f"pid={pid} ({comm}): {cmd}")
    if conflicts:
        raise RuntimeError(
            "Refusing to start while another benchmark process is active: "
            + "; ".join(conflicts)
        )


def verify_retained_empty_candidate_core(
    solr_host_url: str,
    empty_core: str = ACCIDENTAL_EMPTY_CANDIDATE_CORE,
) -> None:
    clean_base = solr_host_url.rstrip("/")
    status_url = f"{clean_base}/admin/cores?" + urllib.parse.urlencode(
        {"action": "STATUS", "core": empty_core, "wt": "json"}
    )
    with urllib.request.urlopen(status_url, timeout=10) as resp:
        status_data = json.loads(resp.read().decode("utf-8"))
    core_info = status_data.get("status", {}).get(empty_core)
    if not core_info:
        raise RuntimeError(
            f"Retained accidental candidate core '{empty_core}' is missing from {clean_base}"
        )

    select_url = f"{clean_base}/{empty_core}/select?q=*:*&rows=0&wt=json"
    with urllib.request.urlopen(select_url, timeout=10) as resp:
        select_data = json.loads(resp.read().decode("utf-8"))
    num_found = select_data.get("response", {}).get("numFound", -1)
    if num_found != 0:
        raise RuntimeError(
            f"Retained accidental candidate core '{empty_core}' changed unexpectedly: "
            f"numFound={num_found} (expected 0)"
        )


def validate_watermark_in_cli_summary(raw_stdout: str) -> int:
    summary = json.loads(raw_stdout)
    epoch = summary.get("full_reindex_started_at_epoch_sec")
    if type(epoch) is not int or epoch <= 0:
        raise RuntimeError(
            "Candidate CLI summary is missing a positive integer "
            f"'full_reindex_started_at_epoch_sec' (got {epoch!r})"
        )
    return epoch


def run_with_peak_rss_and_watermark_check(
    cmd: List[str], env: Optional[Mapping[str, str]] = None
) -> PeakRssResult:
    result = _orig_run_with_peak_rss(cmd, env=env)
    if result.returncode == 0:
        epoch = validate_watermark_in_cli_summary(result.stdout)
        print(
            f"Verified candidate pre-fetch watermark: full_reindex_started_at_epoch_sec={epoch}",
            flush=True,
        )
    return result


def configure_harness(args: argparse.Namespace) -> List[str]:
    if args.native_core == ACCIDENTAL_EMPTY_CANDIDATE_CORE:
        raise ValueError(
            f"Refusing to use accidental empty candidate core '{ACCIDENTAL_EMPTY_CANDIDATE_CORE}' as native reference"
        )
    harness.EXPECTED_STACK_ID = EXPECTED_STACK_ID
    harness.EXPECTED_BRANCH = EXPECTED_BRANCH
    harness.DEFAULT_STATE_FILE = args.state
    harness.DEFAULT_NATIVE_CORE = args.native_core
    harness.EXPECTED_TOTAL_NATIVE_DOCS = EXPECTED_TOTAL_NATIVE_DOCS
    harness.SOLR_CONTAINER = f"archivalist-profiling-solr-{EXPECTED_STACK_ID}"
    harness.BACKEND_CONTAINER = f"archivalist-profiling-backend-{EXPECTED_STACK_ID}"
    harness.DB_CONTAINER = f"archivalist-profiling-db-{EXPECTED_STACK_ID}"
    harness.run_with_peak_rss = run_with_peak_rss_and_watermark_check

    harness_argv = [
        "run_gate3_combined_candidate.py",
        "--state",
        args.state,
        "--native-core",
        args.native_core,
        "--output-dir",
        args.output_dir,
        "--full",
        "--concurrency",
        str(args.concurrency),
    ]
    if args.dry_run:
        harness_argv.append("--dry-run")
    return harness_argv


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_runner_args(argv)
    verify_no_active_benchmark_processes()
    if not os.path.isfile(args.state):
        raise FileNotFoundError(f"State file not found at {args.state}")
    with open(args.state, encoding="utf-8") as f:
        state = json.load(f)
    if state.get("stack_id") != EXPECTED_STACK_ID:
        raise RuntimeError(
            f"State file stack_id '{state.get('stack_id')}' != expected '{EXPECTED_STACK_ID}'"
        )
    verify_retained_empty_candidate_core(state["solr_host_url"])
    print(
        f"Confirmed retained empty core '{ACCIDENTAL_EMPTY_CANDIDATE_CORE}' is present with numFound=0.",
        flush=True,
    )
    sys.argv = configure_harness(args)
    return harness.run_harness()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"\n[FATAL ERROR] {harness.redact_secrets(exc)}", file=sys.stderr)
        sys.exit(1)
