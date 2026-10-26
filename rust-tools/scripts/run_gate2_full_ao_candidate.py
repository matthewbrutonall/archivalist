#!/usr/bin/env python3
"""Gate 2 Full Archival Object (AO) Candidate Vertical Slice Harness

Executes end-to-end Rust STAFF AO indexing for isolated stack e62875f1/repo 4:
  - Pre-flight safety checks (branch, release binary SHA256, memory >= 24GiB, main Solr stopped, DB identity challenge-response, active arks_enabled).
  - Starts profiling Solr & backend if stopped (tracks for finally/atexit cleanup).
  - Verifies native reference core stage_staff_20261025_224741_59214_8a08a5d2_native has 24,200 docs.
  - Warmup pass on a new unique candidate core (stage_staff_<nonce>_candidate).
  - Streaming cursorMark pagination comparison of all 24,200 documents against native reference core:
    * Type-strict parsed JSON after ONLY _version_ normalization.
    * Exact URI set match, no duplicates, matching count, zero private _archivalist_* markers.
  - If warmup parity passes with 0 diffs: executes 3 measured repeats on fresh candidate cores.
  - All candidate cores are retained; no deletion or unloading.
  - Sensitive record content kept in memory or mode 0600 report; stdout contains summary/counts/hashes only.
"""

from __future__ import annotations

import argparse
import atexit
from collections import Counter
import hashlib
import json
import math
import os
import re
import secrets
import signal
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional, Set, Tuple
import urllib.error
import urllib.parse
import urllib.request

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BENCHMARKS_DIR = os.path.join(REPO_ROOT, "rust-tools", "benchmarks")
DEFAULT_STATE_FILE = os.path.join(BENCHMARKS_DIR, "profiling_stack_e62875f1.json")
DEFAULT_NATIVE_CORE = "stage_staff_20261025_224741_59214_8a08a5d2_native"
DEFAULT_REPO_ID = 4
EXPECTED_STACK_ID = "e62875f1"
EXPECTED_BRANCH = "codex/native-profile-analysis"
EXPECTED_AO_COUNT = 24200
MIN_MEMORY_HEADROOM_GIB = 24.0

# Performance baseline & Gate 2 target
NATIVE_AO_MEDIAN_WALL_SEC = 56.62456
GATE2_TARGET_WALL_SEC = 39.63719  # 30% faster than native median (<= 39.637s)

SOLR_CONTAINER = f"archivalist-profiling-solr-{EXPECTED_STACK_ID}"
BACKEND_CONTAINER = f"archivalist-profiling-backend-{EXPECTED_STACK_ID}"
DB_CONTAINER = f"archivalist-profiling-db-{EXPECTED_STACK_ID}"
MAIN_SOLR_CONTAINER = "archivesspaceai-solr-1"
BENCH_SOLR_CONTAINER = "archivesspaceai-solr-bench"

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
    res = subprocess.run(
        ["docker", "container", "inspect", name],
        capture_output=True,
        text=True,
    )
    if res.returncode != 0:
        return None
    try:
        return json.loads(res.stdout)[0]
    except Exception:
        return None


def is_running(name: str) -> bool:
    info = docker_inspect(name)
    return info is not None and bool(info.get("State", {}).get("Running", False))


def get_mem_available_gib() -> float:
    with open("/proc/meminfo") as f:
        for line in f:
            if line.startswith("MemAvailable:"):
                kb = int(line.split()[1])
                return kb / (1024 * 1024)
    return 0.0


def canonical_json_key(val: Any) -> Any:
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


def value_sha256_and_size(val: Any) -> Tuple[str, int]:
    try:
        encoded = json.dumps(val, sort_keys=True, ensure_ascii=True, separators=(",", ":")).encode("utf-8")
        h = hashlib.sha256(encoded).hexdigest()
        return (h, len(encoded))
    except Exception:
        fallback = repr(val).encode("utf-8")
        return (hashlib.sha256(fallback).hexdigest(), len(fallback))


def normalize_solr_doc(doc: Dict[str, Any]) -> Dict[str, Any]:
    return {k: v for k, v in doc.items() if k not in SOLR_STORAGE_ONLY_FIELDS and k != "score"}


def collect_same_stage_stored_diffs(
    candidate_val: Any,
    native_val: Any,
    path: str = "",
) -> List[Dict[str, Any]]:
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
            "Rebuild release binary from current HEAD before running harness."
        )

    return {
        "path": cli_bin,
        "sha256": bin_sha256,
        "mtime": bin_mtime,
        "newest_source_file": newest_src_file,
        "newest_source_mtime": newest_src_mtime,
    }


def stream_fetch_solr_core_documents(
    solr_host_url: str,
    core_name: str,
    repo_id: int,
    batch_size: int = 1000,
    timeout: float = 30.0,
) -> Tuple[Dict[str, Dict[str, Any]], List[str]]:
    """Fetches all documents from a Solr core using cursorMark streaming pagination.

    Returns (docs_by_uri, marker_leaks).
    """
    clean_base = solr_host_url.rstrip("/")
    select_url = f"{clean_base}/{core_name}/select"
    repo_scope = f"/repositories/{repo_id}"
    fq = f'repository:"{repo_scope}"'

    docs_by_uri: Dict[str, Dict[str, Any]] = {}
    marker_leaks: List[str] = []
    seen_cursors: Set[str] = set()
    cursor_mark = "*"
    expected_num_found: Optional[int] = None

    while True:
        if cursor_mark in seen_cursors:
            raise RuntimeError(f"Pagination loop detected in {core_name}: cursorMark '{cursor_mark}' repeated")
        seen_cursors.add(cursor_mark)

        params = [
            ("q", "*:*"),
            ("fq", fq),
            ("sort", "id asc"),
            ("fl", "*"),
            ("wt", "json"),
            ("rows", str(batch_size)),
            ("cursorMark", cursor_mark),
        ]
        query_str = urllib.parse.urlencode(params)
        req_url = f"{select_url}?{query_str}"

        req = urllib.request.Request(req_url)
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            payload = json.loads(resp.read().decode("utf-8"))

        resp_block = payload.get("response", {})
        num_found = resp_block.get("numFound", 0)
        if expected_num_found is None:
            expected_num_found = num_found
        elif num_found != expected_num_found:
            raise RuntimeError(
                f"numFound changed mid-pagination from {expected_num_found} to {num_found} in {core_name}"
            )

        docs = resp_block.get("docs", [])
        for doc in docs:
            doc_id = doc.get("id")
            uri = doc.get("uri")
            if not uri:
                raise RuntimeError(f"Solr doc {doc_id} in {core_name} missing 'uri'")
            if uri in docs_by_uri:
                raise RuntimeError(f"Duplicate document URI '{uri}' encountered in {core_name}")
            docs_by_uri[uri] = doc

            for k in doc.keys():
                if k.startswith("_archivalist_"):
                    marker_leaks.append(f"{uri}:{k}")

        next_cursor = payload.get("nextCursorMark")
        if not next_cursor or next_cursor == cursor_mark or len(docs) == 0:
            break
        cursor_mark = next_cursor

    return docs_by_uri, marker_leaks


def run_full_parity_comparison(
    cand_docs_by_uri: Dict[str, Dict[str, Any]],
    native_docs_by_uri: Dict[str, Dict[str, Any]],
    cand_marker_leaks: List[str],
) -> Dict[str, Any]:
    cand_uris = set(cand_docs_by_uri.keys())
    native_uris = set(native_docs_by_uri.keys())

    missing_in_candidate = sorted(native_uris - cand_uris)
    extra_in_candidate = sorted(cand_uris - native_uris)
    common_uris = sorted(cand_uris & native_uris)

    path_counts: Counter[str] = Counter()
    path_categories: Dict[str, str] = {}
    records_with_diffs = 0
    total_diffs = 0
    sample_diffs: Dict[str, List[Dict[str, Any]]] = {}

    for uri in common_uris:
        c_doc = normalize_solr_doc(cand_docs_by_uri[uri])
        n_doc = normalize_solr_doc(native_docs_by_uri[uri])

        diffs = collect_same_stage_stored_diffs(c_doc, n_doc)
        if diffs:
            records_with_diffs += 1
            total_diffs += len(diffs)
            for d in diffs:
                norm_p = normalize_path(d["path"])
                path_counts[norm_p] += 1
                path_categories[norm_p] = d["category"]
            if len(sample_diffs) < 10:
                sample_diffs[uri] = diffs

    exact_parity = (
        len(missing_in_candidate) == 0
        and len(extra_in_candidate) == 0
        and len(cand_marker_leaks) == 0
        and total_diffs == 0
    )

    return {
        "exact_parity": exact_parity,
        "records_compared": len(common_uris),
        "candidate_total_docs": len(cand_docs_by_uri),
        "native_total_docs": len(native_docs_by_uri),
        "missing_in_candidate_count": len(missing_in_candidate),
        "extra_in_candidate_count": len(extra_in_candidate),
        "candidate_marker_leaks_count": len(cand_marker_leaks),
        "candidate_marker_leaks_sample": cand_marker_leaks[:10],
        "records_with_exact_equality": len(common_uris) - records_with_diffs,
        "records_with_differences": records_with_diffs,
        "total_differences": total_diffs,
        "distinct_differing_paths": len(path_counts),
        "top_differing_paths": [
            {"path": p, "count": cnt, "category": path_categories.get(p, "unknown")}
            for p, cnt in path_counts.most_common(20)
        ],
        "sample_per_record_diffs": sample_diffs,
    }


def parse_args():
    parser = argparse.ArgumentParser(
        description="Gate 2 Full Archival Object Candidate Vertical Slice Harness"
    )
    parser.add_argument(
        "--state",
        default=DEFAULT_STATE_FILE,
        help="Path to profiling stack state JSON",
    )
    parser.add_argument(
        "--native-core",
        default=DEFAULT_NATIVE_CORE,
        help="Retained native Solr core name",
    )
    parser.add_argument(
        "--output-dir",
        default=BENCHMARKS_DIR,
        help="Directory to save output reports",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Run safety and static checks without starting containers or writing Solr",
    )
    return parser.parse_args()


def run_harness():
    args = parse_args()
    print("=== Gate 2 Full AO Candidate Vertical Slice Harness ===", flush=True)

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
        print("\n[DRY RUN] Safety and provenance checks passed cleanly. Halting before live container start or write.")
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
    if v_mode != "genuine_active_process":
        raise RuntimeError(
            f"Active app arks_enabled is UNVERIFIED (status: '{v_mode}'). Fail-closed refusal."
        )
    if active_arks is None:
        raise RuntimeError("Failed to extract arks_enabled boolean from live /system/info")
    print(f"Live active backend verified: arks_enabled={active_arks} (mode: {v_mode})")

    # 13. Pre-fetch native reference documents once for streaming comparison
    print(f"\nStreaming native reference documents from '{native_core}' for baseline parity...", flush=True)
    t_native_fetch_start = time.perf_counter()
    native_docs_by_uri, native_marker_leaks = stream_fetch_solr_core_documents(
        solr_host_url,
        native_core,
        DEFAULT_REPO_ID,
        batch_size=1000,
    )
    t_native_fetch_elapsed = time.perf_counter() - t_native_fetch_start
    print(
        f"Fetched {len(native_docs_by_uri)} native documents in {t_native_fetch_elapsed:.2f}s "
        f"(native marker leaks: {len(native_marker_leaks)})."
    )
    if len(native_docs_by_uri) != EXPECTED_AO_COUNT:
        raise RuntimeError(
            f"Fetched {len(native_docs_by_uri)} native documents; expected exactly {EXPECTED_AO_COUNT}"
        )

    # Helper function to create and run one candidate indexing pass
    def execute_candidate_pass(pass_name: str, pass_index: int) -> Dict[str, Any]:
        run_nonce = f"{time.strftime('%Y%m%d_%H%M%S', time.gmtime())}_{os.getpid()}_{pass_index}_{secrets.token_hex(4)}"
        cand_core = f"stage_staff_{run_nonce}_candidate"
        print(f"\n--- Starting {pass_name}: candidate core '{cand_core}' ---", flush=True)

        # A. Verify ABSENCE of candidate core before creation
        c_status_req = urllib.request.urlopen(
            f"{solr_host_url}/admin/cores?action=STATUS&core={cand_core}&wt=json",
            timeout=10,
        )
        c_status_data = json.loads(c_status_req.read().decode())
        if c_status_data.get("status", {}).get(cand_core):
            raise RuntimeError(f"Candidate core '{cand_core}' already exists before creation! No-clobber refusal.")

        # B. Create empty candidate core
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

        # C. Verify candidate core is present and has 0 docs
        c_check_req = urllib.request.urlopen(
            f"{solr_host_url}/{cand_core}/select?q=*:*&rows=0&wt=json",
            timeout=10,
        )
        c_check_resp = json.loads(c_check_req.read().decode())
        c_init_docs = c_check_resp.get("response", {}).get("numFound", -1)
        if c_init_docs != 0:
            raise RuntimeError(f"Candidate core '{cand_core}' contains {c_init_docs} docs; expected 0")

        # D. Run aspace-cli staff-ao-candidate
        arks_flag = "true" if active_arks else "false"
        cli_cmd = [
            cli_bin,
            "--url",
            backend_host_url,
            "--user",
            state["admin_user"],
            "--password",
            state["admin_password"],
            "staff-ao-candidate",
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
            "--json",
        ]

        t_harness_wall_start = time.perf_counter()
        cli_proc = subprocess.run(cli_cmd, capture_output=True, text=True)
        t_harness_wall_elapsed = time.perf_counter() - t_harness_wall_start

        if cli_proc.returncode != 0:
            raise RuntimeError(
                f"Candidate pass '{pass_name}' CLI failed with exit code {cli_proc.returncode}. "
                f"Stderr summary: {cli_proc.stderr.strip()[:256]}"
            )

        try:
            cand_summary = json.loads(cli_proc.stdout)
        except Exception as e:
            raise RuntimeError(f"Failed to parse candidate JSON summary: {e}")

        if not cand_summary.get("success", False):
            raise RuntimeError(f"Candidate pass '{pass_name}' reported success=false: {cand_summary.get('error_message')}")

        if cand_summary.get("exact_ids_indexed") != EXPECTED_AO_COUNT:
            raise RuntimeError(
                f"Candidate pass '{pass_name}' indexed {cand_summary.get('exact_ids_indexed')} IDs; expected {EXPECTED_AO_COUNT}"
            )

        # Verify candidate core doc count directly from Solr
        cand_select_url = f"{solr_host_url}/{cand_core}/select?q=*:*&rows=0&wt=json"
        with urllib.request.urlopen(cand_select_url, timeout=10) as resp:
            data = json.loads(resp.read().decode())
            solr_num_found = data.get("response", {}).get("numFound", -1)
        if solr_num_found != EXPECTED_AO_COUNT:
            raise RuntimeError(
                f"Solr candidate core '{cand_core}' numFound is {solr_num_found}; expected {EXPECTED_AO_COUNT}"
            )

        print(
            f"Pass '{pass_name}' candidate indexing complete in {cand_summary['total_wall_elapsed_sec']:.3f}s "
            f"({cand_summary['records_per_second']:.2f} docs/sec)."
        )

        # E. Streaming comparison against native reference documents (outside timed region)
        print(f"Comparing stored Solr documents ({cand_core} vs {native_core})...", flush=True)
        t_comp_start = time.perf_counter()
        cand_docs_by_uri, cand_marker_leaks = stream_fetch_solr_core_documents(
            solr_host_url,
            cand_core,
            DEFAULT_REPO_ID,
            batch_size=1000,
        )
        parity_result = run_full_parity_comparison(
            cand_docs_by_uri,
            native_docs_by_uri,
            cand_marker_leaks,
        )
        t_comp_elapsed = time.perf_counter() - t_comp_start
        print(
            f"Parity comparison finished in {t_comp_elapsed:.2f}s: exact_parity={parity_result['exact_parity']}, "
            f"diffs={parity_result['total_differences']}, marker_leaks={parity_result['candidate_marker_leaks_count']}."
        )

        return {
            "pass_name": pass_name,
            "pass_index": pass_index,
            "candidate_core": cand_core,
            "cli_summary": cand_summary,
            "solr_verified_docs": solr_num_found,
            "parity_result": parity_result,
            "timings": {
                "id_list_elapsed_sec": cand_summary["id_list_elapsed_sec"],
                "batch_fetch_build_elapsed_sec": cand_summary["batch_fetch_build_elapsed_sec"],
                "solr_write_elapsed_sec": cand_summary["solr_write_elapsed_sec"],
                "solr_commit_elapsed_sec": cand_summary["solr_commit_elapsed_sec"],
                "total_wall_elapsed_sec": cand_summary["total_wall_elapsed_sec"],
                "records_per_second": cand_summary["records_per_second"],
                "comparison_elapsed_sec": t_comp_elapsed,
                "harness_wall_elapsed_sec": t_harness_wall_elapsed,
            },
        }

    # === Step 14: Execute Warmup Pass ===
    warmup_res = execute_candidate_pass("Warmup Pass", 0)
    warmup_parity = warmup_res["parity_result"]

    if not warmup_parity["exact_parity"]:
        print(
            f"\n[PARITY FAILURE] Warmup pass found {warmup_parity['total_differences']} differences "
            f"and {warmup_parity['candidate_marker_leaks_count']} marker leaks across {EXPECTED_AO_COUNT} records!",
            file=sys.stderr,
        )
        print("\nTop differing paths:", file=sys.stderr)
        for p_info in warmup_parity["top_differing_paths"]:
            print(f"  - {p_info['path']}: count={p_info['count']} ({p_info['category']})", file=sys.stderr)

        # Save failure report and STOP
        ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
        report_path = os.path.join(args.output_dir, f"gate2_full_ao_candidate_failure_{EXPECTED_STACK_ID}_{ts}.json")
        fail_report = {
            "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "status": "parity_failure",
            "git_commit": git_head,
            "git_branch": git_branch,
            "release_binary_sha256": provenance["sha256"],
            "warmup_pass": warmup_res,
        }
        write_file_mode_0600(report_path, json.dumps(fail_report, indent=2))
        print(f"Saved failure diagnostic report: {report_path}")
        return 1

    print("\n[WARMUP PARITY SUCCESS] 100% exact parity achieved across all 24,200 documents! Zero marker leaks.", flush=True)

    # === Step 15: Execute 3 Measured Repeats ===
    measured_passes: List[Dict[str, Any]] = []
    for repeat_idx in range(1, 4):
        pass_name = f"Measured Pass {repeat_idx}"
        pass_res = execute_candidate_pass(pass_name, repeat_idx)
        pass_parity = pass_res["parity_result"]

        if not pass_parity["exact_parity"]:
            print(
                f"\n[PARITY FAILURE] {pass_name} found {pass_parity['total_differences']} differences! Halting.",
                file=sys.stderr,
            )
            measured_passes.append(pass_res)
            break

        measured_passes.append(pass_res)

    # === Step 16: Compute Aggregates & Performance Targets ===
    measured_wall_times = [p["timings"]["total_wall_elapsed_sec"] for p in measured_passes]
    measured_median_wall = sorted(measured_wall_times)[len(measured_wall_times) // 2] if measured_wall_times else 0.0
    measured_min_wall = min(measured_wall_times) if measured_wall_times else 0.0
    measured_max_wall = max(measured_wall_times) if measured_wall_times else 0.0

    target_met = measured_median_wall <= GATE2_TARGET_WALL_SEC
    speedup_ratio = (
        NATIVE_AO_MEDIAN_WALL_SEC / measured_median_wall if measured_median_wall > 0 else 0.0
    )
    percent_faster = (
        ((NATIVE_AO_MEDIAN_WALL_SEC - measured_median_wall) / NATIVE_AO_MEDIAN_WALL_SEC) * 100.0
        if NATIVE_AO_MEDIAN_WALL_SEC > 0
        else 0.0
    )

    print("\n=======================================================", flush=True)
    print("=== GATE 2 CANDIDATE AO INDEXING PERFORMANCE SUMMARY ===", flush=True)
    print("=======================================================", flush=True)
    print(f"Records indexed per pass: {EXPECTED_AO_COUNT}")
    print(f"Warmup pass wall time: {warmup_res['timings']['total_wall_elapsed_sec']:.3f}s")
    for p in measured_passes:
        t = p["timings"]
        print(
            f"{p['pass_name']}: total_wall={t['total_wall_elapsed_sec']:.3f}s "
            f"(id_list={t['id_list_elapsed_sec']:.3f}s, batch_fetch_build={t['batch_fetch_build_elapsed_sec']:.3f}s, "
            f"solr_writes={t['solr_write_elapsed_sec']:.3f}s, solr_commit={t['solr_commit_elapsed_sec']:.3f}s) "
            f"throughput={t['records_per_second']:.2f} docs/sec"
        )
    print(f"\nMeasured passes wall times: {[round(w, 3) for w in measured_wall_times]}s")
    print(f"Measured Warmed Median: {measured_median_wall:.3f}s (min: {measured_min_wall:.3f}s, max: {measured_max_wall:.3f}s)")
    print(f"Native AO-only Reference Median: {NATIVE_AO_MEDIAN_WALL_SEC:.3f}s")
    print(f"Gate 2 Target (<= 30% faster): <= {GATE2_TARGET_WALL_SEC:.3f}s")
    print(f"Observed Speedup: {speedup_ratio:.2f}x ({percent_faster:+.1f}%)")
    print(f"Gate 2 30%-Faster Target Met: {target_met}")

    # === Step 17: Save Full Atomic Mode 0600 Report ===
    ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
    report_filename = f"gate2_full_ao_candidate_{EXPECTED_STACK_ID}_{ts}.json"
    report_path = os.path.join(args.output_dir, report_filename)

    final_report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "stack_id": EXPECTED_STACK_ID,
        "git_commit": git_head,
        "git_branch": git_branch,
        "release_binary_sha256": provenance["sha256"],
        "release_binary_path": provenance["path"],
        "repo_id": DEFAULT_REPO_ID,
        "expected_ao_count": EXPECTED_AO_COUNT,
        "native_reference_core": native_core,
        "native_reference_median_wall_sec": NATIVE_AO_MEDIAN_WALL_SEC,
        "gate2_target_wall_sec": GATE2_TARGET_WALL_SEC,
        "warmup_pass": warmup_res,
        "measured_passes": measured_passes,
        "summary": {
            "all_passes_exact_parity": all(
                p["parity_result"]["exact_parity"]
                for p in [warmup_res] + measured_passes
            ),
            "measured_passes_count": len(measured_passes),
            "measured_wall_times_sec": measured_wall_times,
            "measured_median_wall_sec": measured_median_wall,
            "measured_min_wall_sec": measured_min_wall,
            "measured_max_wall_sec": measured_max_wall,
            "speedup_ratio": speedup_ratio,
            "percent_faster": percent_faster,
            "gate2_target_met": target_met,
        },
        "baseline_caveats": [
            "Native reference is 56.625s median from native_ao_profile_20261025_224440_59214_55c63af5.json on isolated stack e62875f1.",
            "Candidate wall time includes: authenticated ID list modified_since(0), 4-way concurrent 25-ID batch fetch & build, Solr writes via IsolatedSolrStaffAoWriter, and final hard commit.",
            "Candidate setup excludes: initial API login, schema dynamic enum discovery, and Solr STATUS core readiness verification.",
            "Solr streaming parity comparison is measured separately outside the timed candidate indexing region.",
            "All candidate cores and native reference cores are preserved in isolated Solr; no cores were deleted or unloaded.",
        ],
    }

    write_file_mode_0600(report_path, json.dumps(final_report, indent=2))
    print(f"\nFinal Gate 2 candidate report atomically saved (mode 0600): {report_path}")

    return 0 if (target_met and final_report["summary"]["all_passes_exact_parity"]) else 1


if __name__ == "__main__":
    sys.exit(run_harness())
