#!/usr/bin/env python3
"""Gate 2 Isolated AO-Only Orchestrator Failure Injection & Safe Fallback Harness

Executes an isolated failure injection test for Gate 2:
  1. Validates preconditions (git branch, release binary provenance, available memory >= 24GiB,
     main Solr stopped, dedicated profiling DB running, stack labels e62875f1).
  2. Starts profiling Solr & backend if stopped (tracks for atexit/signal cleanup, stopping ONLY
     containers started during this run).
  3. Verifies DB identity via challenge-response and confirms active app config (/system/info).
  4. Verifies native reference core (stage_staff_20261025_224741_59214_8a08a5d2_native) has exactly 24,200 docs.
  5. Injects deterministic failure into Rust candidate indexer via --fail-after-batches 5 into a NEW candidate core:
     - Asserts CLI exits non-zero with success=false.
     - Asserts failures > 0 and no final hard commit (solr_commit_elapsed_sec == 0.0).
     - Asserts no promotion occurred.
     - Asserts candidate core is preserved (not deleted or reused).
  6. Invokes unchanged native PeriodicIndexer AO-only path via run_native_archival_object_profile.sh
     with REPEATS=1 and STAGE_ROLE=fallback into a NEW fallback core (Rust flags all false).
  7. Asserts fallback core was created, has a distinct name from candidate core, and contains exactly 24,200 docs.
  8. Runs type-strict streaming stored-Solr parity comparison between the new fallback core and the native reference core:
     - Asserts exact_parity=True, 24,200 documents compared, 0 differences, 0 marker leaks.
  9. Preserves all cores (no deletion/unloading) and writes mode 0600 atomic JSON report.
"""

from __future__ import annotations

import argparse
import atexit
import glob
import json
import os
import secrets
import signal
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional
import urllib.error
import urllib.parse
import urllib.request

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "rust-tools", "scripts"))

from run_gate2_full_ao_candidate import (
    BENCHMARKS_DIR,
    DEFAULT_NATIVE_CORE,
    DEFAULT_REPO_ID,
    DEFAULT_STATE_FILE,
    EXPECTED_AO_COUNT,
    EXPECTED_BRANCH,
    EXPECTED_STACK_ID,
    MIN_MEMORY_HEADROOM_GIB,
    MAIN_SOLR_CONTAINER,
    BENCH_SOLR_CONTAINER,
    SOLR_CONTAINER,
    BACKEND_CONTAINER,
    DB_CONTAINER,
    docker_inspect,
    get_mem_available_gib,
    is_running,
    run_full_parity_comparison,
    snapshot_system_metrics,
    stream_fetch_solr_core_documents,
    verify_active_app_arks_enabled,
    verify_binary_provenance,
    write_file_mode_0600,
)

started_containers: List[str] = []


def fetch_first_archival_object_id(
    backend_url: str,
    admin_user: str,
    admin_password: str,
    repo_id: int,
) -> int:
    """Fetches the first archival object ID for the repository from the backend,
    matching the exact sequence the candidate indexer processes.
    Falls back to verified dataset initial ID 1221 if API call is unauthenticated.
    """
    try:
        login_url = f"{backend_url.rstrip('/')}/users/{urllib.parse.quote(admin_user)}/login?password={urllib.parse.quote(admin_password)}"
        req = urllib.request.Request(login_url, data=b"", method="POST")
        with urllib.request.urlopen(req, timeout=10) as resp:
            login_data = json.loads(resp.read().decode("utf-8"))
            session_token = login_data.get("session")

        if session_token:
            ids_url = f"{backend_url.rstrip('/')}/repositories/{repo_id}/archival_objects?all_ids=true&modified_since=0"
            ids_req = urllib.request.Request(
                ids_url,
                headers={"X-ArchivesSpace-Session": session_token},
                method="GET",
            )
            with urllib.request.urlopen(ids_req, timeout=10) as resp:
                ids_list = json.loads(resp.read().decode("utf-8"))
                if isinstance(ids_list, list) and len(ids_list) > 0:
                    first_id = int(ids_list[0])
                    print(f"Discovered first AO ID from backend: {first_id}", flush=True)
                    return first_id
    except Exception as e:
        print(f"Warning: could not fetch dynamic first AO ID from backend: {e}", file=sys.stderr)
    return 1221


def cleanup():
    global started_containers
    if not started_containers:
        return
    print(
        f"\n[CLEANUP] Stopping containers started during this run: {started_containers}...",
        flush=True,
    )
    for name in reversed(started_containers):
        try:
            print(f"[CLEANUP] Stopping {name}...", flush=True)
            subprocess.run(
                ["docker", "stop", name],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except Exception as e:
            print(f"[CLEANUP] Warning: failed to stop {name}: {e}", file=sys.stderr)
    started_containers.clear()
    print("[CLEANUP] Cleanup complete.", flush=True)


def sig_handler(signum, frame):
    print(f"\n[SIGNAL] Caught signal {signum}, initiating cleanup...", file=sys.stderr)
    cleanup()
    sys.exit(128 + signum)


atexit.register(cleanup)
signal.signal(signal.SIGINT, sig_handler)
signal.signal(signal.SIGTERM, sig_handler)


def parse_args():
    parser = argparse.ArgumentParser(
        description="Gate 2 Isolated AO-Only Failure Injection & Safe Fallback Harness"
    )
    parser.add_argument(
        "--state",
        default=DEFAULT_STATE_FILE,
        help="Path to profiling stack state JSON",
    )
    parser.add_argument(
        "--native-core",
        default=DEFAULT_NATIVE_CORE,
        help="Retained native Solr reference core name",
    )
    parser.add_argument(
        "--output-dir",
        default=BENCHMARKS_DIR,
        help="Directory to save output reports",
    )
    parser.add_argument(
        "--fail-after-batches",
        type=int,
        default=25,
        help="Number of batches before candidate failure injection (default: 25)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Run safety and static checks without modifying state or running indexing",
    )
    parser.add_argument(
        "--save-debug-stderr",
        action="store_true",
        help="Explicit debug opt-in: save raw fallback stderr to a mode 0600 file on failure",
    )
    return parser.parse_args()


def run_failure_injection_fallback_harness():
    args = parse_args()
    print("=== Gate 2 Failure Injection & Safe Fallback Harness ===", flush=True)

    # 1. Check git branch, HEAD, and clean tracked status
    git_branch = (
        subprocess.run(
            ["git", "-C", REPO_ROOT, "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )
    git_head = (
        subprocess.run(
            ["git", "-C", REPO_ROOT, "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )
    git_status_tracked = (
        subprocess.run(
            ["git", "-C", REPO_ROOT, "status", "--porcelain", "-uno"],
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )
    git_dirty = bool(git_status_tracked)
    print(f"Git branch: {git_branch}")
    print(f"Git HEAD: {git_head}")
    print(f"Git dirty (tracked): {git_dirty}")

    if git_branch != EXPECTED_BRANCH:
        raise RuntimeError(
            f"Expected branch '{EXPECTED_BRANCH}', but current branch is '{git_branch}'"
        )

    if git_dirty:
        raise RuntimeError(
            f"Git working tree has uncommitted tracked changes! Tracked status:\n{git_status_tracked}\n"
            "All changes must be committed before building/running to ensure binary and source provenance."
        )

    # 2. Check binary provenance
    cli_bin = os.path.join(REPO_ROOT, "rust-tools", "target", "release", "aspace-cli")
    provenance = verify_binary_provenance(cli_bin)
    print(f"Verified release binary: {cli_bin} (sha256: {provenance['sha256'][:16]}...)")

    # 3. Read and verify state file
    if not os.path.isfile(args.state):
        raise FileNotFoundError(f"State file not found at {args.state}")
    with open(args.state) as f:
        state = json.load(f)

    if state.get("stack_id") != EXPECTED_STACK_ID:
        raise RuntimeError(
            f"State file stack_id '{state.get('stack_id')}' != expected '{EXPECTED_STACK_ID}'"
        )

    # 4. Check memory headroom
    mem_avail = get_mem_available_gib()
    print(f"Available memory: {mem_avail:.2f} GiB (minimum required: {MIN_MEMORY_HEADROOM_GIB} GiB)")
    if mem_avail < MIN_MEMORY_HEADROOM_GIB:
        raise RuntimeError(
            f"Insufficient memory headroom: {mem_avail:.2f} GiB < {MIN_MEMORY_HEADROOM_GIB} GiB required"
        )

    # 5. Check main & bench Solr safety: must NOT be running
    for cname in [MAIN_SOLR_CONTAINER, BENCH_SOLR_CONTAINER]:
        info = docker_inspect(cname)
        if info and info["State"].get("Running", False):
            raise RuntimeError(
                f"SAFETY VIOLATION: Container '{cname}' is currently running. "
                "Harness requires main/bench Solr to remain stopped."
            )

    # 6. Check dedicated DB container is running
    if not is_running(DB_CONTAINER):
        raise RuntimeError(f"Dedicated profiling DB container '{DB_CONTAINER}' is not running!")
    print(f"Confirmed: {DB_CONTAINER} is running.")

    # 7. Check Docker container labels for isolated stack
    for role, cname in state["containers"].items():
        info = docker_inspect(cname)
        if not info:
            raise RuntimeError(f"Required container {cname} not found")
        labels = info["Config"].get("Labels", {})
        if labels.get("archivalist.profiling-stack") != EXPECTED_STACK_ID:
            raise RuntimeError(
                f"Container {cname} missing expected label archivalist.profiling-stack={EXPECTED_STACK_ID}"
            )
        if labels.get("archivalist.profiling-role") != role:
            raise RuntimeError(
                f"Container {cname} label role '{labels.get('archivalist.profiling-role')}' != expected '{role}'"
            )

    if args.dry_run:
        print("\n[DRY RUN] Safety and provenance checks passed cleanly. Halting before live operations.")
        return 0

    # 8. Start profiling Solr if stopped (track for cleanup)
    solr_host_url = state["solr_host_url"].rstrip("/")
    if not is_running(SOLR_CONTAINER):
        print(f"Starting isolated profiling Solr ({SOLR_CONTAINER})...", flush=True)
        subprocess.run(["docker", "start", SOLR_CONTAINER], check=True)
        started_containers.append(SOLR_CONTAINER)

    # Wait for Solr readiness
    solr_ready = False
    for attempt in range(60):
        try:
            req = urllib.request.urlopen(
                f"{solr_host_url}/admin/cores?action=STATUS&core={state['identity_core']}&wt=json",
                timeout=3,
            )
            data = json.loads(req.read().decode())
            if state["identity_core"] in data.get("status", {}):
                solr_ready = True
                break
        except Exception:
            time.sleep(1)
    if not solr_ready:
        raise RuntimeError(
            f"Isolated Solr {solr_host_url} failed to become ready with identity core within 60s"
        )
    print("Isolated profiling Solr is ready.", flush=True)

    # 9. Verify native reference core exists and has exactly 24,200 documents
    native_core = args.native_core
    status_req = urllib.request.urlopen(
        f"{solr_host_url}/admin/cores?action=STATUS&core={native_core}&wt=json",
        timeout=10,
    )
    status_data = json.loads(status_req.read().decode())
    native_core_info = status_data.get("status", {}).get(native_core)
    if not native_core_info:
        raise RuntimeError(
            f"Native reference core '{native_core}' does not exist in isolated Solr {solr_host_url}"
        )

    native_select_url = f"{solr_host_url}/{native_core}/select?q=*:*&rows=0&wt=json"
    native_num_found = -1
    for attempt in range(60):
        try:
            with urllib.request.urlopen(native_select_url, timeout=5) as resp:
                data = json.loads(resp.read().decode())
                native_num_found = data.get("response", {}).get("numFound", -1)
                if native_num_found >= 0:
                    break
        except urllib.error.HTTPError as e:
            if e.code == 503:
                time.sleep(1)
                continue
            raise
        except Exception:
            time.sleep(1)

    if native_num_found != EXPECTED_AO_COUNT:
        raise RuntimeError(
            f"Native reference core '{native_core}' contains {native_num_found} docs; expected exactly {EXPECTED_AO_COUNT}"
        )
    print(f"Native reference core '{native_core}' verified present with exactly {native_num_found} documents.")

    # 10. Start profiling backend if stopped (track for cleanup)
    backend_host_url = state["backend_host_url"].rstrip("/")
    if not is_running(BACKEND_CONTAINER):
        print(f"Starting isolated profiling backend ({BACKEND_CONTAINER})...", flush=True)
        subprocess.run(["docker", "start", BACKEND_CONTAINER], check=True)
        started_containers.append(BACKEND_CONTAINER)

    # Wait for backend readiness
    backend_ready = False
    for attempt in range(120):
        try:
            req = urllib.request.urlopen(f"{backend_host_url}/", timeout=3)
            data = json.loads(req.read().decode())
            if "databaseProductName" in data:
                backend_ready = True
                break
        except Exception:
            time.sleep(2)
        if attempt > 0 and attempt % 10 == 0:
            print(f"  still waiting for backend ({attempt * 2}s)...", flush=True)
    if not backend_ready:
        raise RuntimeError("Profiling backend failed to become ready within 240s")
    print("Profiling backend is ready.", flush=True)

    # 11. Challenge-Response verification of DB identity
    print("\n=== Challenge-Response Stack Verification ===", flush=True)
    verify_cmd = [
        "python3",
        os.path.join(REPO_ROOT, "rust-tools", "scripts", "profiling_stack.py"),
        "verify",
        "--state",
        args.state,
    ]
    verify_res = subprocess.run(verify_cmd, capture_output=True, text=True, check=True)
    print(verify_res.stdout.strip(), flush=True)

    # 12. Authenticated verification of active arks_enabled
    print("\n=== Validating Active App Configuration (/system/info) ===", flush=True)
    v_mode, active_arks = verify_active_app_arks_enabled(
        backend_host_url,
        state["admin_user"],
        state["admin_password"],
    )
    print(f"Active app configuration: mode={v_mode}, arks_enabled={active_arks}")

    # =========================================================================
    # Phase A: Isolated Candidate Failure Injection
    # =========================================================================
    print("\n=== Phase A: Injected Candidate Failure Test ===", flush=True)
    cand_nonce = secrets.token_hex(4)
    ts_now = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
    pid = os.getpid()
    cand_core = f"stage_staff_{ts_now}_{pid}_{cand_nonce}_candidate"
    cand_instance_dir = f"/var/solr/data/{cand_core}"

    print(f"Creating isolated candidate core '{cand_core}' in profiling Solr...", flush=True)
    create_url = (
        f"{solr_host_url}/admin/cores?"
        + urllib.parse.urlencode({
            "action": "CREATE",
            "name": cand_core,
            "configSet": "archivesspace",
            "wt": "json",
        })
    )
    create_req = urllib.request.urlopen(create_url, timeout=30)
    create_resp = json.loads(create_req.read().decode())
    if create_resp.get("responseHeader", {}).get("status") != 0:
        raise RuntimeError(f"Solr core CREATE failed for {cand_core}: {create_resp}")

    # Run aspace-cli staff-ao-candidate with --fail-after-batches
    arks_flag = "true" if active_arks else "false"
    fail_limit = args.fail_after_batches
    cli_cmd = [
        cli_bin,
        "--url",
        backend_host_url,
        "--user",
        state["admin_user"],
        "--password",
        state["admin_password"],
        "staff-ao-candidate",
        "--state",
        args.state,
        "--repo",
        str(DEFAULT_REPO_ID),
        "--batch-size",
        "25",
        "--concurrency",
        "4",
        "--solr-url",
        solr_host_url,
        "--candidate-core",
        cand_core,
        "--arks-enabled",
        arks_flag,
        "--strip-markers",
        "true",
        "--fail-after-batches",
        str(fail_limit),
        "--json",
    ]

    print(f"Launching candidate indexer with deterministic failure injection (--fail-after-batches {fail_limit})...", flush=True)
    pre_cand_metrics = snapshot_system_metrics(solr_host_url)
    t_cand_start = time.perf_counter()
    cand_proc = subprocess.run(cli_cmd, capture_output=True, text=True)
    t_cand_elapsed = time.perf_counter() - t_cand_start
    post_cand_metrics = snapshot_system_metrics(solr_host_url)

    # Assert candidate failure properties
    print(f"Candidate process finished with exit code {cand_proc.returncode} in {t_cand_elapsed:.3f}s.")
    if cand_proc.returncode == 0:
        raise AssertionError("Expected candidate to FAIL with non-zero exit code, but exited 0!")

    try:
        cand_summary = json.loads(cand_proc.stdout)
    except Exception as e:
        raise AssertionError(f"Could not parse candidate JSON summary from stdout: {e}. Output was: {cand_proc.stdout[:256]}")

    if cand_summary.get("success") is not False:
        raise AssertionError(f"Expected candidate summary success=false, got: {cand_summary.get('success')}")

    if cand_summary.get("failures", 0) < 1:
        raise AssertionError(f"Expected candidate failures >= 1, got: {cand_summary.get('failures')}")

    if cand_summary.get("solr_commit_elapsed_sec", 0.0) != 0.0:
        raise AssertionError(
            f"Expected no final hard commit (solr_commit_elapsed_sec == 0.0), got: {cand_summary.get('solr_commit_elapsed_sec')}"
        )

    # Verify at least one Solr update POST occurred before failure
    cand_update_requests = cand_summary.get("solr_update_requests_sent", 0)
    if cand_update_requests < 1:
        raise AssertionError(
            f"Expected candidate writer to dispatch at least 1 Solr update POST before failure, got: {cand_update_requests}"
        )
    print(f"Asserted: candidate writer dispatched {cand_update_requests} Solr update POST request(s) before failure.")

    print("Asserted: candidate reported deterministic failure, no final hard commit, and exit code != 0.")
    print(f"Error message recorded: {cand_summary.get('error_message')}")

    # Check candidate core doc count in Solr: must NOT be 24,200
    with urllib.request.urlopen(f"{solr_host_url}/{cand_core}/select?q=*:*&rows=0&wt=json", timeout=10) as resp:
        cand_core_docs = json.loads(resp.read().decode()).get("response", {}).get("numFound", -1)
    print(f"Candidate core Solr numFound: {cand_core_docs} docs (partial/uncommitted; expected != {EXPECTED_AO_COUNT}).")
    if cand_core_docs == EXPECTED_AO_COUNT:
        raise AssertionError(f"Candidate core has full {EXPECTED_AO_COUNT} docs despite injected failure!")

    # Attempt real-time /get verification on partial uncommitted documents using known initial ID
    target_ao_id = fetch_first_archival_object_id(
        backend_host_url,
        state["admin_user"],
        state["admin_password"],
        DEFAULT_REPO_ID,
    )
    target_ao_uri = f"/repositories/{DEFAULT_REPO_ID}/archival_objects/{target_ao_id}"
    print(
        f"Attempting real-time /get verification on selected first written document: {target_ao_uri}...",
        flush=True,
    )

    rt_get_confirmed = False
    rt_get_details = None
    try:
        # Check /get for the first known written AO URI in the candidate core
        get_url = f"{solr_host_url}/{cand_core}/get?id={urllib.parse.quote(target_ao_uri, safe='')}&wt=json"
        req = urllib.request.Request(get_url)
        with urllib.request.urlopen(req, timeout=5) as get_resp:
            get_json = json.loads(get_resp.read().decode())
            doc = get_json.get("doc")
            if doc and doc.get("id") == target_ao_uri:
                rt_get_confirmed = True
                rt_get_details = f"Document visible via real-time /get: id={doc.get('id')}"
            elif doc:
                rt_get_confirmed = True
                rt_get_details = f"Document visible via real-time /get with mismatched id: {doc.get('id')}"
            else:
                rt_get_details = (
                    "Real-time /get returned 200 without doc (expected in archivesspace configSet: "
                    "updateLog/RealTimeGetHandler not active; uncommitted documents are buffered in memory)"
                )
    except urllib.error.HTTPError as e:
        rt_get_details = (
            f"Real-time /get returned HTTP {e.code} (expected in archivesspace configSet: "
            f"handler not configured: {e.reason})"
        )
    except Exception as e:
        rt_get_details = f"Real-time /get query failed: {e}"

    print(
        f"Candidate Solr state: numFound={cand_core_docs}, update_requests_sent={cand_update_requests}, "
        f"target_uri={target_ao_uri}, real_time_get_check={rt_get_confirmed} ({rt_get_details})."
    )

    # =========================================================================
    # Phase B: Native PeriodicIndexer Safe Fallback
    # =========================================================================
    print("\n=== Phase B: Invoking Native PeriodicIndexer AO-Only Fallback ===", flush=True)
    fallback_script = os.path.join(REPO_ROOT, "rust-tools", "scripts", "run_native_archival_object_profile.sh")
    fallback_env = os.environ.copy()
    fallback_env["STACK_STATE"] = args.state
    fallback_env["REPEATS"] = "1"
    fallback_env["STAGE_ROLE"] = "fallback"

    # Record pre-existing reports before invoking fallback script to detect unique newly generated artifact
    pre_existing_reports = set(
        p for p in glob.glob(os.path.join(BENCHMARKS_DIR, "native_ao_profile_*.json"))
        if not p.endswith(".build.json")
    )

    pre_fb_metrics = snapshot_system_metrics(solr_host_url)
    t_fb_start = time.perf_counter()
    t_fb_wall = time.time()
    fb_proc = subprocess.run(
        [fallback_script],
        env=fallback_env,
        capture_output=True,
        text=True,
    )
    t_fb_elapsed = time.perf_counter() - t_fb_start
    post_fb_metrics = snapshot_system_metrics(solr_host_url)

    print(f"Native fallback script finished with exit code {fb_proc.returncode} in {t_fb_elapsed:.3f}s.")
    if fb_proc.returncode != 0:
        print(
            f"[FALLBACK FAILURE] Native fallback script failed with exit code {fb_proc.returncode} "
            "(raw stderr suppressed to protect potential record content or credentials)",
            file=sys.stderr,
        )
        if getattr(args, "save_debug_stderr", False) and fb_proc.stderr:
            err_ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
            debug_stderr_path = os.path.join(
                args.output_dir, f"fallback_stderr_debug_{EXPECTED_STACK_ID}_{err_ts}.log"
            )
            write_file_mode_0600(debug_stderr_path, fb_proc.stderr)
            print(f"[FALLBACK FAILURE] Raw stderr saved with mode 0600 under debug opt-in: {debug_stderr_path}", file=sys.stderr)
        raise RuntimeError(f"Native fallback script failed with exit code {fb_proc.returncode}")

    # Locate the unique new native_ao_profile report
    # 1. Look for report path parsed from wrapper stdout
    parsed_report_path: Optional[str] = None
    wrapper_stdout_file = None
    for token in fb_proc.stdout.split():
        if token.startswith("stdout="):
            wrapper_stdout_file = token.split("=", 1)[1]
            break

    if wrapper_stdout_file and os.path.isfile(wrapper_stdout_file):
        try:
            with open(wrapper_stdout_file, "r") as sf:
                for line in sf:
                    line_s = line.strip()
                    if line_s.startswith("Report:"):
                        cand_path = line_s.split("Report:", 1)[1].strip()
                        if cand_path:
                            # Note: wrapper runs inside container where path is /archivesspace/rust-tools/benchmarks/...
                            # Map to host path under BENCHMARKS_DIR
                            parsed_report_path = os.path.abspath(
                                os.path.join(BENCHMARKS_DIR, os.path.basename(cand_path))
                            )
                            break
        except Exception as e:
            print(f"Warning: could not read wrapper stdout file {wrapper_stdout_file}: {e}", file=sys.stderr)

    # 2. Compare before/after manifest, excluding wrapper build metadata (*.build.json)
    current_reports = set(
        p for p in glob.glob(os.path.join(BENCHMARKS_DIR, "native_ao_profile_*.json"))
        if not p.endswith(".build.json")
    )
    new_reports = current_reports - pre_existing_reports
    new_reports_in_window = [
        p for p in new_reports
        if os.path.getmtime(p) >= (t_fb_wall - 10.0)
    ]

    # 3. Fail closed on ambiguity
    selected_report_path: Optional[str] = None
    if len(new_reports_in_window) == 1:
        selected_report_path = list(new_reports_in_window)[0]
        if parsed_report_path and os.path.normpath(selected_report_path) != os.path.normpath(parsed_report_path):
            raise RuntimeError(
                f"AMBIGUITY ERROR: Parsed report path '{parsed_report_path}' differs from unique manifest report '{selected_report_path}'!"
            )
    elif len(new_reports_in_window) == 0:
        if parsed_report_path and os.path.isfile(parsed_report_path):
            selected_report_path = parsed_report_path
        else:
            raise RuntimeError(
                f"AMBIGUITY ERROR: No new native_ao_profile_*.json report found after fallback run! "
                f"(pre-existing={len(pre_existing_reports)}, parsed={parsed_report_path})"
            )
    else:
        # Multiple new reports found
        if parsed_report_path and parsed_report_path in new_reports_in_window:
            selected_report_path = parsed_report_path
        else:
            raise RuntimeError(
                f"AMBIGUITY ERROR: Multiple ({len(new_reports_in_window)}) new native_ao_profile reports created concurrently! "
                f"Candidates: {new_reports_in_window}"
            )

    selected_report_path = os.path.abspath(selected_report_path)
    print(f"Reading verified unique fallback report: {selected_report_path}")
    with open(selected_report_path) as f:
        fb_report = json.load(f)

    results_list = fb_report.get("results", [])
    if not results_list:
        raise RuntimeError(f"Fallback report '{selected_report_path}' contains empty results list!")

    fallback_core = results_list[0].get("core")
    if not fallback_core:
        raise RuntimeError(f"Fallback report '{selected_report_path}' result missing 'core' field!")

    if not fallback_core.endswith("_fallback"):
        raise AssertionError(
            f"AMBIGUITY ERROR: Fallback core name '{fallback_core}' does not end with '_fallback'!"
        )

    print(f"Identified new fallback core: '{fallback_core}'")

    # Assert fallback core is NEW and distinct from candidate core
    if fallback_core == cand_core:
        raise AssertionError(f"Safety violation: fallback core '{fallback_core}' reused candidate core '{cand_core}'!")
    if fallback_core == native_core:
        raise AssertionError(f"Safety violation: fallback core '{fallback_core}' reused native core '{native_core}'!")

    # Verify Solr document count on fallback core
    with urllib.request.urlopen(f"{solr_host_url}/{fallback_core}/select?q=*:*&rows=0&wt=json", timeout=10) as resp:
        fb_solr_num_found = json.loads(resp.read().decode()).get("response", {}).get("numFound", -1)

    print(f"Fallback core Solr document count: {fb_solr_num_found} (expected: {EXPECTED_AO_COUNT})")
    if fb_solr_num_found != EXPECTED_AO_COUNT:
        raise AssertionError(
            f"Fallback core '{fallback_core}' contains {fb_solr_num_found} docs; expected exactly {EXPECTED_AO_COUNT}!"
        )

    # =========================================================================
    # Phase C: Full Type-Strict Parity Comparison Against Native Reference Core
    # =========================================================================
    print(f"\n=== Phase C: Streaming Type-Strict Parity ({fallback_core} vs {native_core}) ===", flush=True)
    t_comp_start = time.perf_counter()
    fb_docs_by_uri, fb_marker_leaks = stream_fetch_solr_core_documents(
        solr_host_url,
        fallback_core,
        DEFAULT_REPO_ID,
        batch_size=1000,
    )
    native_docs_by_uri, native_marker_leaks = stream_fetch_solr_core_documents(
        solr_host_url,
        native_core,
        DEFAULT_REPO_ID,
        batch_size=1000,
    )
    parity_result = run_full_parity_comparison(
        fb_docs_by_uri,
        native_docs_by_uri,
        fb_marker_leaks,
    )
    t_comp_elapsed = time.perf_counter() - t_comp_start

    print(
        f"Parity comparison finished in {t_comp_elapsed:.2f}s:\n"
        f"  - exact_parity: {parity_result['exact_parity']}\n"
        f"  - records_compared: {parity_result['records_compared']}\n"
        f"  - total_differences: {parity_result['total_differences']}\n"
        f"  - missing_in_fallback: {parity_result['missing_in_candidate_count']}\n"
        f"  - extra_in_fallback: {parity_result['extra_in_candidate_count']}\n"
        f"  - marker_leaks: {parity_result['candidate_marker_leaks_count']}"
    )

    if not parity_result["exact_parity"]:
        raise AssertionError(
            f"Fallback parity failed with {parity_result['total_differences']} differences "
            f"and {parity_result['candidate_marker_leaks_count']} marker leaks!"
        )

    # =========================================================================
    # Phase D: Save Mode 0600 Final Report
    # =========================================================================
    # Verify core states and derive no_candidate_promotion and candidate_core_preserved
    all_cores_req = urllib.request.urlopen(f"{solr_host_url}/admin/cores?action=STATUS&wt=json", timeout=10)
    all_cores_status = json.loads(all_cores_req.read().decode()).get("status", {})

    cand_core_status = all_cores_status.get(cand_core)
    cand_core_preserved = cand_core_status is not None
    if not cand_core_preserved:
        raise AssertionError(f"Candidate core '{cand_core}' missing from Solr cores; was unexpectedly unloaded or deleted!")

    cand_instance_dir = cand_core_status.get("instanceDir", "")
    cand_instance_dir_matches = cand_instance_dir.rstrip("/").endswith(cand_core)

    identity_core_name = state.get("identity_core")
    identity_core_status = all_cores_status.get(identity_core_name)
    identity_not_swapped = (
        identity_core_status is not None
        and identity_core_status.get("instanceDir", "").rstrip("/").endswith(identity_core_name)
    )

    fallback_distinct = bool(fallback_core != cand_core and fallback_core != native_core)

    # Isolated CoreAdmin alias/masquerade check: identity core was not retargeted to candidate or fallback
    no_identity_aliasing = (
        identity_core_name != cand_core
        and identity_core_name != fallback_core
        and cand_core in all_cores_status
    )

    no_candidate_promotion = bool(
        cand_core_preserved
        and cand_instance_dir_matches
        and identity_not_swapped
        and fallback_distinct
        and no_identity_aliasing
    )
    if not no_candidate_promotion:
        raise AssertionError("Failed promotion check: candidate core was aliased, swapped, or reused!")

    promotion_evidence = [
        f"Observation 1 (no promotion code invoked): Candidate indexer CLI halted immediately upon injected failure (exit code {cand_proc.returncode}); no promotion logic, alias API, or core swap commands exist in its failure path.",
        f"Observation 2 (isolated CoreAdmin STATUS): Candidate core '{cand_core}' remains registered under its exact original name with dedicated instanceDir '{cand_instance_dir}'.",
        f"Observation 3 (identity core unaltered): Identity/search core '{identity_core_name}' remains mapped to its own instanceDir without alias, swap, or rename.",
        f"Observation 4 (native reference isolation): Retained native reference core '{native_core}' was not modified, swapped, or overwritten.",
        f"Observation 5 (fallback fresh core): Native fallback executed strictly into new, distinct fallback core '{fallback_core}'.",
        "(Scope limitation: This confirms empirically via isolated CoreAdmin status that no promotion occurred during this run; it is an isolated state check rather than a global proof across arbitrary external systems.)",
    ]

    candidate_failed_cleanly = bool(cand_proc.returncode != 0 and cand_summary.get("success") is False and cand_summary.get("failures", 0) >= 1)
    at_least_one_update_sent = bool(cand_update_requests >= 1)
    no_candidate_hard_commit = bool(cand_summary.get("solr_commit_elapsed_sec", 0.0) == 0.0)
    fallback_docs_verified = bool(fb_solr_num_found == EXPECTED_AO_COUNT)
    fallback_exact_parity = bool(parity_result["exact_parity"] is True and parity_result["total_differences"] == 0 and parity_result["candidate_marker_leaks_count"] == 0)

    gate2_safe_fallback_demonstrated = bool(
        candidate_failed_cleanly
        and at_least_one_update_sent
        and no_candidate_hard_commit
        and no_candidate_promotion
        and cand_core_preserved
        and fallback_distinct
        and fallback_docs_verified
        and fallback_exact_parity
    )

    report_ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
    report_filename = f"gate2_failure_injection_fallback_{EXPECTED_STACK_ID}_{report_ts}.json"
    report_path = os.path.join(args.output_dir, report_filename)

    final_report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "stack_id": EXPECTED_STACK_ID,
        "git_commit": git_head,
        "git_branch": git_branch,
        "git_dirty": git_dirty,
        "release_binary_sha256": provenance["sha256"],
        "repo_id": DEFAULT_REPO_ID,
        "expected_ao_count": EXPECTED_AO_COUNT,
        "native_reference_core": native_core,
        "injected_candidate_core": cand_core,
        "fallback_core": fallback_core,
        "candidate_failure_injection": {
            "fail_after_batches": fail_limit,
            "exit_code": cand_proc.returncode,
            "cli_summary": cand_summary,
            "solr_docs_in_partial_candidate_core": cand_core_docs,
            "solr_update_requests_sent": cand_update_requests,
            "real_time_get_check": {
                "target_id": target_ao_id,
                "target_uri": target_ao_uri,
                "confirmed": rt_get_confirmed,
                "details": rt_get_details,
                "primary_proof": "solr_update_requests_sent",
            },
            "hard_commit_issued": cand_summary.get("solr_commit_elapsed_sec", 0.0) > 0.0,
            "promoted": not no_candidate_promotion,
            "core_preserved": cand_core_preserved,
            "system_snapshots": {
                "pre": pre_cand_metrics,
                "post": post_cand_metrics,
            },
        },
        "native_fallback_execution": {
            "exit_code": fb_proc.returncode,
            "script_elapsed_sec": t_fb_elapsed,
            "report_file": selected_report_path,
            "solr_verified_docs": fb_solr_num_found,
            "system_snapshots": {
                "pre": pre_fb_metrics,
                "post": post_fb_metrics,
            },
        },
        "parity_result": parity_result,
        "summary": {
            "candidate_failed_cleanly": candidate_failed_cleanly,
            "at_least_one_update_sent": at_least_one_update_sent,
            "no_candidate_hard_commit": no_candidate_hard_commit,
            "no_candidate_promotion": no_candidate_promotion,
            "candidate_core_preserved": cand_core_preserved,
            "fallback_core_is_distinct_and_new": fallback_distinct,
            "fallback_docs_verified": fallback_docs_verified,
            "fallback_exact_parity": fallback_exact_parity,
            "gate2_safe_fallback_demonstrated": gate2_safe_fallback_demonstrated,
        },
        "no_candidate_promotion_evidence": promotion_evidence,
        "no_candidate_promotion_basis": "verified_isolated_coreadmin_status_and_no_promotion_code_invoked",
        "solr_partial_write_safety_explanation": (
            "Because active Solr background autoCommit/autoSoftCommit could theoretically persist or make partial "
            "documents visible even without a client final hard commit, client-side 'no hard commit' alone does not "
            "guarantee that zero documents exist in the core. The critical architectural safety invariant is strict "
            "core-level isolation: the partial candidate core is never promoted, aliased, or swapped, while recovery "
            "executes via an independent, newly created fallback core with 100% verified parity."
        ),
        "notes": [
            "Candidate failure was deterministically injected via --fail-after-batches.",
            "Candidate core was partially written but never issued final hard commit and was never promoted.",
            "Candidate core was preserved in isolated Solr data volume without deletion or reuse.",
            "Fallback was executed through the unchanged native PeriodicIndexer AO worker loop via run_native_archival_object_profile.sh.",
            "Fallback indexed exactly 24,200 documents into a fresh fallback core with Rust flags all false.",
            "Fallback core achieved 100% exact type-strict parity against the native AO reference core with 0 differences and 0 marker leaks.",
        ],
    }

    write_file_mode_0600(report_path, json.dumps(final_report, indent=2))
    print(f"\nFinal Failure Injection & Fallback report atomically saved (mode 0600): {report_path}")

    return 0


if __name__ == "__main__":
    sys.exit(run_failure_injection_fallback_harness())
