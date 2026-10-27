#!/usr/bin/env python3
"""Gate 2 25-AO Same-Stage Stored-Solr Diagnostic

Compares stored Solr documents between:
  1) A newly created empty isolated candidate core (stage_staff_<nonce>_candidate)
     populated with 25 Rust STAFF AO documents built via aspace-cli and written
     via aspace_client::IsolatedSolrStaffAoWriter with hard commit
  2) The existing retained native reference core
     stage_staff_20261025_224741_59214_8a08a5d2_native on isolated Solr stack e62875f1

This is a SAME-STAGE stored-Solr diagnostic. It does NOT claim exact parity,
full 24,200 run completion, timing inference, speedup, or production readiness.

Strict Safety Guards:
  - Verifies exact Docker container labels and state for stack e62875f1.
  - Verifies database challenge-response nonce via profiling_stack.py.
  - Verifies available system memory headroom before starting services.
  - Main Solr (archivesspaceai-solr-1) is NEVER started or touched.
  - Tracks started containers and on exit/interrupt stops ONLY those containers.
  - Candidate core created via Solr admin CREATE reusing native harness pattern.
  - Candidate core absence verified before creation; non-empty core refused.
  - No cores are ever deleted or unloaded.
  - Solr writes use aspace_client::IsolatedSolrStaffAoWriter inside aspace-cli.
  - Verifies genuine active backend arks_enabled config via authenticated /system/info.
  - Fetches stored documents from both cores, normalizes ONLY storage-only _version_.
  - Compares using strict JSON types; redacts all payload data in report.
  - Outputs mode 0600 report atomically without clobbering existing files.
"""

from __future__ import annotations

import argparse
import atexit
import hashlib
import json
import math
import os
import re
import secrets
import signal
import stat
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter
from typing import Any, Dict, List, Optional, Set, Tuple

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BENCHMARKS_DIR = os.path.join(REPO_ROOT, "rust-tools", "benchmarks")
DEFAULT_STATE_FILE = os.path.join(BENCHMARKS_DIR, "profiling_stack_e62875f1.json")
DEFAULT_EXPERIMENT_FILE = os.path.join(
    BENCHMARKS_DIR, "gate2_ao_normalization_experiment_e62875f1_20261026_085620.json"
)
DEFAULT_NATIVE_CORE = "stage_staff_20261025_224741_59214_8a08a5d2_native"
DEFAULT_REPO_ID = 4
EXPECTED_STACK_ID = "e62875f1"
EXPECTED_BRANCH = "codex/native-profile-analysis"
EXPECTED_AO_COUNT = 24200
SAMPLE_SIZE = 25
MIN_MEMORY_HEADROOM_GIB = 2.0

SOLR_CONTAINER = f"archivalist-profiling-solr-{EXPECTED_STACK_ID}"
BACKEND_CONTAINER = f"archivalist-profiling-backend-{EXPECTED_STACK_ID}"
DB_CONTAINER = f"archivalist-profiling-db-{EXPECTED_STACK_ID}"
MAIN_SOLR_CONTAINER = "archivesspaceai-solr-1"

# Explicitly documented Solr-added storage-only field to normalize
SOLR_STORAGE_ONLY_FIELDS = {"_version_"}

started_containers: List[str] = []


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


def docker_inspect(name: str) -> Optional[Dict[str, Any]]:
    res = subprocess.run(["docker", "container", "inspect", name], capture_output=True, text=True)
    if res.returncode != 0:
        return None
    return json.loads(res.stdout)[0]


def is_running(name: str) -> bool:
    info = docker_inspect(name)
    return info is not None and info["State"].get("Running", False)


def get_mem_available_gib() -> float:
    with open("/proc/meminfo") as f:
        for line in f:
            if line.startswith("MemAvailable:"):
                kb = int(line.split()[1])
                return kb / (1024 * 1024)
    return 0.0


def value_sha256_and_size(val: Any) -> Tuple[str, int]:
    try:
        raw_repr = json.dumps(val, sort_keys=True, ensure_ascii=False)
    except Exception:
        raw_repr = repr(val)
    encoded = raw_repr.encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()[:16], len(encoded)


def normalize_solr_doc(doc: Dict[str, Any]) -> Dict[str, Any]:
    """Normalize ONLY explicitly identified storage-only _version_, preserving all other fields."""
    return {k: v for k, v in doc.items() if k not in SOLR_STORAGE_ONLY_FIELDS}


def canonical_json_key(val: Any) -> Tuple[str, Any]:
    if val is None:
        return ("null", None)
    if type(val) is bool:
        return ("bool", val)
    if type(val) is int:
        return ("int", val)
    if type(val) is float:
        if math.isnan(val):
            return ("float", "nan")
        return ("float", val)
    if type(val) is str:
        return ("str", val)
    if isinstance(val, list):
        return ("list", tuple(canonical_json_key(item) for item in val))
    if isinstance(val, dict):
        return ("dict", tuple(sorted((str(k), canonical_json_key(v)) for k, v in val.items())))
    return ("unknown", type(val).__name__, repr(val))


def json_values_strictly_equal(v1: Any, v2: Any) -> bool:
    if type(v1) is not type(v2):
        return False
    if isinstance(v1, dict):
        if set(v1.keys()) != set(v2.keys()):
            return False
        return all(json_values_strictly_equal(v1[k], v2[k]) for k in v1)
    if isinstance(v1, list):
        if len(v1) != len(v2):
            return False
        return all(json_values_strictly_equal(x1, x2) for x1, x2 in zip(v1, v2))
    if isinstance(v1, float):
        if math.isnan(v1) and math.isnan(v2):
            return True
    return v1 == v2


def is_array_reordered(list1: List[Any], list2: List[Any]) -> bool:
    if len(list1) != len(list2):
        return False
    if json_values_strictly_equal(list1, list2):
        return False
    try:
        keys1 = Counter(canonical_json_key(x) for x in list1)
        keys2 = Counter(canonical_json_key(x) for x in list2)
        return keys1 == keys2
    except Exception:
        return False


def collect_same_stage_stored_diffs(
    candidate_val: Any,
    native_val: Any,
    path: str = "",
) -> List[Dict[str, Any]]:
    """Recursively collects field-level differences between candidate and native stored Solr documents.

    Redacts all payload data: reports path, category, types, sha256 hashes, and sizes only.
    """
    diffs: List[Dict[str, Any]] = []

    if type(candidate_val) is not type(native_val):
        h_c, s_c = value_sha256_and_size(candidate_val)
        h_n, s_n = value_sha256_and_size(native_val)
        diffs.append({
            "path": path or "<root>",
            "category": "stored_type_mismatch",
            "candidate_type": type(candidate_val).__name__,
            "native_type": type(native_val).__name__,
            "candidate_hash": h_c,
            "native_hash": h_n,
            "candidate_size": s_c,
            "native_size": s_n,
        })
        return diffs

    if isinstance(candidate_val, dict):
        c_keys = set(candidate_val.keys())
        n_keys = set(native_val.keys())

        # Missing in candidate (present in native)
        for k in sorted(n_keys - c_keys):
            subpath = f"{path}.{k}" if path else k
            h_n, s_n = value_sha256_and_size(native_val[k])
            diffs.append({
                "path": subpath,
                "category": "stored_missing_in_candidate",
                "field": k,
                "native_type": type(native_val[k]).__name__ if native_val[k] is not None else "null",
                "native_hash": h_n,
                "native_size": s_n,
            })

        # Extra in candidate (absent in native)
        for k in sorted(c_keys - n_keys):
            subpath = f"{path}.{k}" if path else k
            h_c, s_c = value_sha256_and_size(candidate_val[k])
            diffs.append({
                "path": subpath,
                "category": "stored_extra_in_candidate",
                "field": k,
                "candidate_type": type(candidate_val[k]).__name__ if candidate_val[k] is not None else "null",
                "candidate_hash": h_c,
                "candidate_size": s_c,
            })

        # Common keys
        for k in sorted(c_keys & n_keys):
            subpath = f"{path}.{k}" if path else k
            diffs.extend(
                collect_same_stage_stored_diffs(candidate_val[k], native_val[k], subpath)
            )
        return diffs

    if isinstance(candidate_val, list):
        if is_array_reordered(candidate_val, native_val):
            h_c, s_c = value_sha256_and_size(candidate_val)
            h_n, s_n = value_sha256_and_size(native_val)
            diffs.append({
                "path": path,
                "category": "stored_array_order_change",
                "length": len(candidate_val),
                "candidate_hash": h_c,
                "native_hash": h_n,
                "candidate_size": s_c,
                "native_size": s_n,
            })
            return diffs

        if len(candidate_val) != len(native_val):
            h_c, s_c = value_sha256_and_size(candidate_val)
            h_n, s_n = value_sha256_and_size(native_val)
            diffs.append({
                "path": path,
                "category": "stored_array_length_mismatch",
                "candidate_len": len(candidate_val),
                "native_len": len(native_val),
                "candidate_hash": h_c,
                "native_hash": h_n,
                "candidate_size": s_c,
                "native_size": s_n,
            })
            # Also compare elements up to min length
            min_len = min(len(candidate_val), len(native_val))
            for idx in range(min_len):
                subpath = f"{path}[{idx}]"
                diffs.extend(
                    collect_same_stage_stored_diffs(
                        candidate_val[idx], native_val[idx], subpath
                    )
                )
            return diffs

        for idx, (c_item, n_item) in enumerate(zip(candidate_val, native_val)):
            subpath = f"{path}[{idx}]"
            diffs.extend(collect_same_stage_stored_diffs(c_item, n_item, subpath))
        return diffs

    # Primitives comparison
    if not json_values_strictly_equal(candidate_val, native_val):
        h_c, s_c = value_sha256_and_size(candidate_val)
        h_n, s_n = value_sha256_and_size(native_val)
        diffs.append({
            "path": path,
            "category": "stored_value_mismatch",
            "candidate_type": type(candidate_val).__name__,
            "native_type": type(native_val).__name__,
            "candidate_hash": h_c,
            "native_hash": h_n,
            "candidate_size": s_c,
            "native_size": s_n,
        })

    return diffs


def normalize_path(path: str) -> str:
    return re.sub(r"\[\d+\]", "[]", path)


def write_file_mode_0600(path: str, content: str) -> None:
    """Atomically writes content to path with mode 0600 without world-readable window or overwrite clobber."""
    parent = os.path.dirname(os.path.abspath(path))
    os.makedirs(parent, exist_ok=True)
    if os.path.exists(path):
        raise FileExistsError(f"Target file {path} already exists (no-clobber refusal)")

    temp_path = os.path.join(
        parent,
        f".tmp_{os.getpid()}_{time.time_ns()}_{os.path.basename(path)}",
    )
    fd = os.open(temp_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(content)
            f.flush()
            os.fsync(f.fileno())
        os.link(temp_path, path)
    finally:
        if os.path.exists(temp_path):
            os.remove(temp_path)


def verify_active_app_arks_enabled(
    backend_url: str,
    admin_user: str,
    admin_password: str,
) -> Tuple[str, Optional[bool]]:
    """Verifies arks_enabled directly from active backend /system/info."""
    try:
        login_url = f"{backend_url.rstrip('/')}/users/{urllib.parse.quote(admin_user)}/login?password={urllib.parse.quote(admin_password)}"
        req = urllib.request.Request(login_url, data=b"", method="POST")
        with urllib.request.urlopen(req, timeout=10) as resp:
            login_data = json.loads(resp.read().decode("utf-8"))
            session_token = login_data.get("session")

        if session_token:
            info_url = f"{backend_url.rstrip('/')}/system/info"
            info_req = urllib.request.Request(
                info_url,
                headers={"X-ArchivesSpace-Session": session_token},
                method="GET",
            )
            with urllib.request.urlopen(info_req, timeout=10) as resp:
                info_data = json.loads(resp.read().decode("utf-8"))
                appconfig = info_data.get("appconfig")
                if isinstance(appconfig, dict) and "arks_enabled" in appconfig:
                    return ("genuine_active_process", bool(appconfig["arks_enabled"]))
    except Exception:
        pass
    return ("unverified", None)


def verify_binary_provenance(cli_bin: str) -> Dict[str, Any]:
    if not os.path.isfile(cli_bin):
        raise FileNotFoundError(f"CLI binary not found at {cli_bin}")

    with open(cli_bin, "rb") as f:
        bin_sha256 = hashlib.sha256(f.read()).hexdigest()
    bin_mtime = os.path.getmtime(cli_bin)

    src_dirs = [
        os.path.join(REPO_ROOT, "rust-tools", "crates", "aspace-cli", "src"),
        os.path.join(REPO_ROOT, "rust-tools", "crates", "aspace-client", "src"),
    ]
    newest_src_mtime = 0.0
    newest_src_file = ""
    for sdir in src_dirs:
        for root, _, files in os.walk(sdir):
            for fname in files:
                if fname.endswith(".rs"):
                    fpath = os.path.join(root, fname)
                    fmtime = os.path.getmtime(fpath)
                    if fmtime > newest_src_mtime:
                        newest_src_mtime = fmtime
                        newest_src_file = fpath

    if bin_mtime < newest_src_mtime:
        raise RuntimeError(
            f"Binary {cli_bin} is stale (binary mtime {bin_mtime:.2f} < source mtime {newest_src_mtime:.2f} for {newest_src_file}). "
            "Rebuild release binary from current HEAD before running diagnostic."
        )

    return {
        "path": cli_bin,
        "sha256": bin_sha256,
        "mtime": bin_mtime,
        "newest_source_file": newest_src_file,
        "newest_source_mtime": newest_src_mtime,
    }


def parse_args():
    parser = argparse.ArgumentParser(
        description="Gate 2 25-AO Same-Stage Stored-Solr Diagnostic"
    )
    parser.add_argument(
        "--state",
        default=DEFAULT_STATE_FILE,
        help="Path to profiling stack state JSON",
    )
    parser.add_argument(
        "--experiment-file",
        default=DEFAULT_EXPERIMENT_FILE,
        help="Path to normalization experiment JSON containing selected IDs",
    )
    parser.add_argument(
        "--native-core",
        default=DEFAULT_NATIVE_CORE,
        help="Retained native Solr core name",
    )
    parser.add_argument(
        "--output-dir",
        default=BENCHMARKS_DIR,
        help="Directory to save output diagnostic report",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Run safety and static checks without starting containers or writing Solr",
    )
    return parser.parse_args()


def run_diagnostic():
    args = parse_args()
    print("=== Gate 2 25-AO Same-Stage Stored-Solr Diagnostic ===", flush=True)

    # 1. Check git branch and HEAD
    git_branch_proc = subprocess.run(
        ["git", "-C", REPO_ROOT, "rev-parse", "--abbrev-ref", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    )
    git_branch = git_branch_proc.stdout.strip()
    git_head_proc = subprocess.run(
        ["git", "-C", REPO_ROOT, "rev-parse", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    )
    git_head = git_head_proc.stdout.strip()
    print(f"Git branch: {git_branch}")
    print(f"Git HEAD: {git_head}")

    if git_branch != EXPECTED_BRANCH:
        raise RuntimeError(
            f"Expected branch '{EXPECTED_BRANCH}', but current branch is '{git_branch}'"
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

    # 4. Read selected IDs from normalization experiment
    if not os.path.isfile(args.experiment_file):
        raise FileNotFoundError(
            f"Normalization experiment file not found at {args.experiment_file}"
        )
    with open(args.experiment_file) as f:
        exp_data = json.load(f)

    selected_ids = exp_data.get("selected_ids")
    if not isinstance(selected_ids, list) or len(selected_ids) != SAMPLE_SIZE:
        raise RuntimeError(
            f"Expected {SAMPLE_SIZE} selected_ids in {args.experiment_file}, got {selected_ids}"
        )
    print(f"Loaded {len(selected_ids)} selected IDs from {args.experiment_file}: {selected_ids}")

    # 5. Check memory headroom
    mem_avail = get_mem_available_gib()
    print(f"Available memory: {mem_avail:.2f} GiB")
    if mem_avail < MIN_MEMORY_HEADROOM_GIB:
        raise RuntimeError(
            f"Insufficient memory headroom: {mem_avail:.2f} GiB < {MIN_MEMORY_HEADROOM_GIB} GiB required"
        )

    # 6. Check main Solr safety: must NOT be running or touched
    main_solr_info = docker_inspect(MAIN_SOLR_CONTAINER)
    if main_solr_info and main_solr_info["State"].get("Running", False):
        raise RuntimeError(
            f"SAFETY VIOLATION: Main Solr container '{MAIN_SOLR_CONTAINER}' is currently running. "
            "Isolated diagnostic requires main Solr to remain stopped."
        )

    # 7. Check Docker container labels
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
        print("\n[DRY RUN] Safety and provenance checks passed cleanly. Halting before live container start or write.")
        return 0

    # 8. Start profiling Solr if stopped (track for cleanup)
    solr_host_url = state["solr_host_url"].rstrip("/")
    if not is_running(SOLR_CONTAINER):
        print(f"Starting isolated profiling Solr ({SOLR_CONTAINER})...", flush=True)
        subprocess.run(["docker", "start", SOLR_CONTAINER], check=True)
        started_containers.append(SOLR_CONTAINER)

    # Wait for Solr to respond
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

    # 9. Verify native reference core exists and has documents
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

    if native_num_found < 0:
        raise RuntimeError(
            f"Native reference core '{native_core}' select failed to become ready within 60s"
        )
    print(f"Native reference core '{native_core}' verified present with {native_num_found} documents.")
    if native_num_found == 0:
        raise RuntimeError(
            f"Native reference core '{native_core}' contains 0 documents; cannot serve as native baseline"
        )

    # 10. Start profiling backend if stopped (track for cleanup)
    backend_host_url = state["backend_host_url"].rstrip("/")
    if not is_running(BACKEND_CONTAINER):
        print(f"Starting isolated profiling backend ({BACKEND_CONTAINER})...", flush=True)
        subprocess.run(["docker", "start", BACKEND_CONTAINER], check=True)
        started_containers.append(BACKEND_CONTAINER)

    # Wait for backend
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
    if v_mode != "genuine_active_process":
        raise RuntimeError(
            f"Active app arks_enabled is UNVERIFIED (status: '{v_mode}'). Fail-closed refusal."
        )
    if active_arks is None:
        raise RuntimeError("Failed to extract arks_enabled boolean from live /system/info")
    print(f"Live active backend verified: arks_enabled={active_arks} (mode: {v_mode})")

    # 13. Generate unique candidate core name & verify ABSENCE before creation
    run_nonce = f"{time.strftime('%Y%m%d_%H%M%S', time.gmtime())}_{os.getpid()}_{secrets.token_hex(4)}"
    candidate_core = f"stage_staff_{run_nonce}_candidate"
    print(f"\nGenerated candidate core name: {candidate_core}")

    # Verify ABSENCE of candidate core before creation
    cand_status_req = urllib.request.urlopen(
        f"{solr_host_url}/admin/cores?action=STATUS&core={candidate_core}&wt=json",
        timeout=10,
    )
    cand_status_data = json.loads(cand_status_req.read().decode())
    if cand_status_data.get("status", {}).get(candidate_core):
        raise RuntimeError(
            f"Candidate core '{candidate_core}' already exists before creation! No-clobber refusal."
        )
    print(f"Verified absence of '{candidate_core}' before creation.")

    # 14. Create candidate core reusing native harness pattern
    print(f"Creating empty candidate core '{candidate_core}' in isolated Solr...", flush=True)
    create_url = (
        f"{solr_host_url}/admin/cores?"
        + urllib.parse.urlencode({
            "action": "CREATE",
            "name": candidate_core,
            "configSet": "archivesspace",
            "wt": "json",
        })
    )
    create_req = urllib.request.urlopen(create_url, timeout=30)
    create_resp = json.loads(create_req.read().decode())
    if create_resp.get("responseHeader", {}).get("status") != 0:
        raise RuntimeError(f"Solr core CREATE failed: {create_resp}")

    # Verify candidate core is present and has exactly 0 docs
    cand_check_req = urllib.request.urlopen(
        f"{solr_host_url}/{candidate_core}/select?q=*:*&rows=0&wt=json",
        timeout=10,
    )
    cand_check_resp = json.loads(cand_check_req.read().decode())
    cand_init_docs = cand_check_resp.get("response", {}).get("numFound", -1)
    if cand_init_docs != 0:
        raise RuntimeError(
            f"Newly created candidate core '{candidate_core}' contains {cand_init_docs} docs; expected 0"
        )
    print(f"Candidate core '{candidate_core}' successfully created and verified empty (numDocs=0).")

    # 15. Build and write 25 Rust STAFF AO docs via current CLI with IsolatedSolrStaffAoWriter
    print(f"\nBuilding and writing 25 STAFF AO documents via aspace-cli into '{candidate_core}'...", flush=True)
    ids_str = ",".join(str(i) for i in selected_ids)
    arks_str = "true" if active_arks else "false"

    cli_cmd = [
        cli_bin,
        "--url",
        backend_host_url,
        "--user",
        state["admin_user"],
        "--password",
        state["admin_password"],
        "staff-ao-builder",
        "--state",
        args.state,
        "--repo",
        str(DEFAULT_REPO_ID),
        "--ids",
        ids_str,
        "--arks-enabled",
        arks_str,
        "--strip-markers",
        "true",
        "--solr-url",
        solr_host_url,
        "--candidate-core",
        candidate_core,
    ]

    t_write_start = time.perf_counter()
    cli_proc = subprocess.run(cli_cmd, capture_output=True, text=True)
    t_write_elapsed = time.perf_counter() - t_write_start

    if cli_proc.returncode != 0:
        # Generic error to prevent secret leakage
        print(f"aspace-cli write failed with exit code {cli_proc.returncode}", file=sys.stderr)
        raise RuntimeError(f"CLI write failed with exit code {cli_proc.returncode}")
    print(f"CLI write and hard commit completed successfully in {t_write_elapsed:.3f}s.")

    # 16. Verify candidate core document count and absence of markers
    cand_verify_url = f"{solr_host_url}/{candidate_core}/select?q=*:*&rows=50&wt=json"
    with urllib.request.urlopen(cand_verify_url, timeout=15) as resp:
        cand_verify_data = json.loads(resp.read().decode())

    cand_num_found = cand_verify_data.get("response", {}).get("numFound", 0)
    cand_docs = cand_verify_data.get("response", {}).get("docs", [])

    print(f"Candidate core numFound: {cand_num_found}")
    print(f"Candidate core returned docs: {len(cand_docs)}")

    if cand_num_found != SAMPLE_SIZE or len(cand_docs) != SAMPLE_SIZE:
        raise RuntimeError(
            f"Expected exactly {SAMPLE_SIZE} documents in '{candidate_core}', got {cand_num_found} (docs list: {len(cand_docs)})"
        )

    # Check for any private markers in candidate docs
    for doc in cand_docs:
        for k in doc.keys():
            if k.startswith("_archivalist_"):
                raise RuntimeError(
                    f"Found unstripped private marker '{k}' in candidate document {doc.get('id')}"
                )

    # 17. Fetch matching 25 native documents from native reference core
    expected_uris = [f"/repositories/{DEFAULT_REPO_ID}/archival_objects/{i}" for i in selected_ids]
    escaped_uris = [f'"{u}"' for u in expected_uris]
    native_query = f"uri:({' OR '.join(escaped_uris)})"
    native_fetch_url = (
        f"{solr_host_url}/{native_core}/select?"
        + urllib.parse.urlencode({"q": native_query, "rows": 50, "wt": "json"})
    )
    with urllib.request.urlopen(native_fetch_url, timeout=15) as resp:
        native_fetch_data = json.loads(resp.read().decode())

    native_docs = native_fetch_data.get("response", {}).get("docs", [])
    print(f"Native core returned matching docs: {len(native_docs)}")
    if len(native_docs) != SAMPLE_SIZE:
        raise RuntimeError(
            f"Expected {SAMPLE_SIZE} matching native documents for selected IDs, got {len(native_docs)}"
        )

    # Index by uri
    cand_by_uri = {d["uri"]: d for d in cand_docs if "uri" in d}
    native_by_uri = {d["uri"]: d for d in native_docs if "uri" in d}

    if set(cand_by_uri.keys()) != set(expected_uris):
        raise RuntimeError("Candidate docs URIs do not match expected selected URIs")
    if set(native_by_uri.keys()) != set(expected_uris):
        raise RuntimeError("Native docs URIs do not match expected selected URIs")

    # 18. Strict comparison with storage-only _version_ normalization
    print("\n=== Comparing Stored Documents (Candidate vs Native) ===", flush=True)
    all_diffs_by_uri: Dict[str, List[Dict[str, Any]]] = {}
    path_counts: Counter[str] = Counter()
    path_categories: Dict[str, str] = {}
    total_diffs = 0
    records_with_diffs = 0

    for uri in expected_uris:
        c_doc = normalize_solr_doc(cand_by_uri[uri])
        n_doc = normalize_solr_doc(native_by_uri[uri])

        diffs = collect_same_stage_stored_diffs(c_doc, n_doc)
        all_diffs_by_uri[uri] = diffs
        if diffs:
            records_with_diffs += 1
            total_diffs += len(diffs)
            for d in diffs:
                norm_p = normalize_path(d["path"])
                path_counts[norm_p] += 1
                path_categories[norm_p] = d["category"]

    exact_parity = total_diffs == 0
    print(f"Records compared: {SAMPLE_SIZE}")
    print(f"Records with exact equality: {SAMPLE_SIZE - records_with_diffs}/{SAMPLE_SIZE}")
    print(f"Records with differences: {records_with_diffs}/{SAMPLE_SIZE}")
    print(f"Total differences observed: {total_diffs}")
    print(f"Exact parity: {exact_parity}")

    print("\nDiffering stored field paths across 25 records:")
    for rank, (p, cnt) in enumerate(path_counts.most_common(50), start=1):
        cat = path_categories.get(p, "unknown")
        print(f"  {rank:2d}. {p:<45} count={cnt:2d}/{SAMPLE_SIZE}  category={cat}")

    # 19. Generate and store mode 0600 report
    ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
    report_filename = f"gate2_ao_same_stage_diagnostic_{EXPECTED_STACK_ID}_{ts}.json"
    report_path = os.path.join(args.output_dir, report_filename)

    report_payload = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "stack_id": EXPECTED_STACK_ID,
        "git_commit": git_head,
        "git_branch": git_branch,
        "release_binary_sha256": provenance["sha256"],
        "release_binary_path": provenance["path"],
        "native_reference_core": native_core,
        "candidate_stage_core": candidate_core,
        "repo_id": DEFAULT_REPO_ID,
        "sample_size": SAMPLE_SIZE,
        "selected_ids": selected_ids,
        "arks_enabled_evidence": {
            "verification_mode": v_mode,
            "effective_value": active_arks,
            "source": "/system/info",
        },
        "solr_request_response": {
            "candidate_select_num_found": cand_num_found,
            "candidate_select_returned_docs": len(cand_docs),
            "native_select_num_found": len(native_docs),
            "http_status": 200,
        },
        "exact_parity": exact_parity,
        "summary": {
            "records_compared": SAMPLE_SIZE,
            "records_with_exact_equality": SAMPLE_SIZE - records_with_diffs,
            "records_with_differences": records_with_diffs,
            "total_differences": total_diffs,
            "distinct_differing_paths": len(path_counts),
            "top_differing_paths": [
                {"path": p, "count": cnt, "category": path_categories.get(p, "unknown")}
                for p, cnt in path_counts.most_common(50)
            ],
        },
        "per_record_diffs": {
            uri: diffs for uri, diffs in all_diffs_by_uri.items()
        },
        "limitations": [
            "25-record SAME-STAGE stored-Solr diagnostic; NOT a full parity claim across all 24,200 records.",
            "Normalizes ONLY explicitly identified storage-only _version_.",
            "Measures diagnostic stored Solr equality; does NOT infer indexing speedup or throughput.",
            "Does not wire production flags or enable production defaults.",
            "Native reference core and candidate core are preserved; no cores unloaded or deleted.",
        ],
    }

    report_json_str = json.dumps(report_payload, indent=2)
    write_file_mode_0600(report_path, report_json_str)
    print(f"\nDiagnostic report atomically saved (mode 0600): {report_path}")

    return 0


if __name__ == "__main__":
    sys.exit(run_diagnostic())
