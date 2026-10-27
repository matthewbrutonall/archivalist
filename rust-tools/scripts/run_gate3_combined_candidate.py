#!/usr/bin/env python3
"""Gate 3 STAFF Combined (Archival Objects + Resources) Slice Live Parity Harness

Executes end-to-end Rust STAFF combined candidate indexing for isolated stack e62875f1/repo 4:
  - Pre-flight safety checks (branch, release binary SHA256, memory >= 24GiB,
    main Solr stopped, DB identity challenge-response, active arks_enabled).
  - Starts profiling Solr & backend if stopped (tracks for finally/atexit cleanup).
  - Verifies native reference core stage_staff_20261025_204522_55378_7bdd9fd1_native
    exists and has exactly 28,201 docs.
  - Verifies repo 4 native combined slice (archival_objects + resources + digital_objects + collection_management children).
  - Creates a fresh, empty candidate core (stage_staff_<nonce>_candidate).
  - Runs CLI staff-combined-candidate using --state and exact isolated backend/Solr URLs.
  - Fetches every stored candidate document and compares against corresponding
    native reference document type-strict recursively after normalizing only
    _version_ and score.
  - Verifies candidate ID set equals native subset for AO + resources + collection_management children.
  - Compares embedded json string byte-for-byte, verifying zero private _archivalist_* markers.
  - All candidate cores are retained; no deletion or unloading.
  - Mode 0600 report retained in rust-tools/benchmarks/; summary only to stdout.
  - Stops ONLY profiling backend/Solr that were started during this run in cleanup; DB stays.
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
import stat
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
DEFAULT_NATIVE_CORE = "stage_staff_20261025_204522_55378_7bdd9fd1_native"
DEFAULT_REPO_ID = 4
EXPECTED_STACK_ID = "e62875f1"
EXPECTED_BRANCH = "codex/gate3-full-staff"
EXPECTED_TOTAL_NATIVE_DOCS = 28201
MIN_MEMORY_HEADROOM_GIB = 24.0

SOLR_CONTAINER = f"archivalist-profiling-solr-{EXPECTED_STACK_ID}"
BACKEND_CONTAINER = f"archivalist-profiling-backend-{EXPECTED_STACK_ID}"
DB_CONTAINER = f"archivalist-profiling-db-{EXPECTED_STACK_ID}"
MAIN_SOLR_CONTAINER = "archivesspaceai-solr-1"
BENCH_SOLR_CONTAINER = "archivesspaceai-solr-bench"

SOLR_STORAGE_ONLY_FIELDS = {"_version_", "score"}

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
        ["docker", "inspect", name],
        capture_output=True,
        text=True,
        check=False,
    )
    if res.returncode != 0:
        return None
    try:
        data = json.loads(res.stdout)
        return data[0] if data else None
    except Exception:
        return None


def is_running(name: str) -> bool:
    info = docker_inspect(name)
    if not info:
        return False
    return bool(info.get("State", {}).get("Running", False))


def get_mem_available_gib() -> float:
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemAvailable:"):
                    parts = line.split()
                    kb = float(parts[1])
                    return kb / (1024.0 * 1024.0)
    except Exception:
        pass
    return 0.0


def canonical_json_key(val: Any) -> str:
    if isinstance(val, (int, float, str, bool)) or val is None:
        return json.dumps(val, sort_keys=True)
    return json.dumps(val, sort_keys=True, separators=(",", ":"))


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


def value_sha256_and_size(val: Any) -> Tuple[str, int]:
    try:
        encoded = json.dumps(
            val, sort_keys=True, ensure_ascii=True, separators=(",", ":")
        ).encode("utf-8")
        h = hashlib.sha256(encoded).hexdigest()
        return (h, len(encoded))
    except Exception:
        fallback = repr(val).encode("utf-8")
        return (hashlib.sha256(fallback).hexdigest(), len(fallback))


def normalize_solr_doc(doc: Dict[str, Any]) -> Dict[str, Any]:
    return {k: v for k, v in doc.items() if k not in SOLR_STORAGE_ONLY_FIELDS}


def check_for_marker_leaks(doc: Dict[str, Any]) -> List[str]:
    leaks = []
    uri = doc.get("uri", doc.get("id", "<unknown>"))
    for k in doc.keys():
        if k.startswith("_archivalist_"):
            leaks.append(f"{uri}:key:{k}")
    json_str = doc.get("json")
    if isinstance(json_str, str) and "_archivalist_" in json_str:
        leaks.append(f"{uri}:embedded_json:_archivalist_marker_found")
    return leaks


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
                "native_type": (
                    type(native_val[k]).__name__
                    if native_val[k] is not None
                    else "null"
                ),
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
                "candidate_type": (
                    type(candidate_val[k]).__name__
                    if candidate_val[k] is not None
                    else "null"
                ),
                "candidate_hash": h_c,
                "candidate_size": s_c,
            })

        for k in sorted(c_keys & n_keys):
            subpath = f"{path}.{k}" if path else k
            diffs.extend(
                collect_same_stage_stored_diffs(
                    candidate_val[k], native_val[k], subpath
                )
            )
        return diffs

    if isinstance(candidate_val, list):
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
        login_url = (
            f"{backend_url.rstrip('/')}/users/{urllib.parse.quote(admin_user)}/login"
        )
        post_data = urllib.parse.urlencode({"password": admin_password}).encode(
            "utf-8"
        )
        req = urllib.request.Request(
            login_url,
            data=post_data,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            method="POST",
        )
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


_KNOWN_SECRETS: Set[str] = set()


def register_secret(secret: Optional[str]) -> None:
    if secret and isinstance(secret, str):
        _KNOWN_SECRETS.add(secret)


def redact_secrets(text: Any, extra_secrets: Optional[List[str]] = None) -> str:
    if text is None:
        return ""
    str_val = str(text)
    active_secrets: Set[str] = set(_KNOWN_SECRETS)
    if extra_secrets:
        for s in extra_secrets:
            if s and isinstance(s, str):
                active_secrets.add(s)
    result = str_val
    for s in active_secrets:
        if not s:
            continue
        result = result.replace(s, "<REDACTED>")
        quoted = urllib.parse.quote(s)
        if quoted and quoted != s:
            result = result.replace(quoted, "<REDACTED>")
    return result


def build_candidate_cli_command(
    cli_bin: str,
    backend_host_url: str,
    admin_user: str,
    state_path: str,
    repo_id: int,
    solr_host_url: str,
    cand_core: str,
    active_arks: bool,
) -> List[str]:
    arks_flag = "true" if active_arks else "false"
    return [
        cli_bin,
        "--url",
        backend_host_url,
        "--user",
        admin_user,
        "staff-combined-candidate",
        "--state",
        state_path,
        "--repo",
        str(repo_id),
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


def build_candidate_cli_env(
    admin_password: Optional[str] = None,
    base_env: Optional[Dict[str, str]] = None,
) -> Dict[str, str]:
    env = dict(os.environ if base_env is None else base_env)
    if admin_password is not None:
        env["ASPACE_PASSWORD"] = admin_password
    return env


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
    fq: Optional[str] = None,
    batch_size: int = 1000,
    timeout: float = 30.0,
) -> Tuple[Dict[str, Dict[str, Any]], List[str]]:
    """Fetches documents from a Solr core using cursorMark streaming pagination.

    Returns (docs_by_id, marker_leaks).
    """
    clean_base = solr_host_url.rstrip("/")
    select_url = f"{clean_base}/{core_name}/select"

    docs_by_id: Dict[str, Dict[str, Any]] = {}
    marker_leaks: List[str] = []
    seen_cursors: Set[str] = set()
    cursor_mark = "*"
    expected_num_found: Optional[int] = None

    while True:
        if cursor_mark in seen_cursors:
            raise RuntimeError(
                f"Pagination loop detected in {core_name}: cursorMark '{cursor_mark}' repeated"
            )
        seen_cursors.add(cursor_mark)

        params = [
            ("q", "*:*"),
            ("sort", "id asc"),
            ("fl", "*"),
            ("wt", "json"),
            ("rows", str(batch_size)),
            ("cursorMark", cursor_mark),
        ]
        if fq:
            params.append(("fq", fq))
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
            if not doc_id:
                raise RuntimeError(f"Solr doc in {core_name} missing 'id'")
            if doc_id in docs_by_id:
                raise RuntimeError(
                    f"Duplicate document id '{doc_id}' encountered in {core_name}"
                )
            docs_by_id[doc_id] = doc

            leaks = check_for_marker_leaks(doc)
            marker_leaks.extend(leaks)

        next_cursor = payload.get("nextCursorMark")
        if not next_cursor or next_cursor == cursor_mark or len(docs) == 0:
            break
        cursor_mark = next_cursor

    return docs_by_id, marker_leaks


def run_slice_parity_comparison(
    cand_docs_by_id: Dict[str, Dict[str, Any]],
    native_docs_by_id: Dict[str, Dict[str, Any]],
    cand_marker_leaks: List[str],
) -> Dict[str, Any]:
    cand_ids = set(cand_docs_by_id.keys())
    native_ids = set(native_docs_by_id.keys())

    missing_in_candidate = sorted(native_ids - cand_ids)
    extra_in_candidate = sorted(cand_ids - native_ids)
    common_ids = sorted(cand_ids & native_ids)

    path_counts: Counter[str] = Counter()
    path_categories: Dict[str, str] = {}
    records_with_diffs = 0
    total_diffs = 0
    sample_diffs: Dict[str, List[Dict[str, Any]]] = {}
    embedded_json_mismatches: List[str] = []

    for doc_id in common_ids:
        c_raw = cand_docs_by_id[doc_id]
        n_raw = native_docs_by_id[doc_id]

        # Byte-for-byte embedded json string comparison
        c_json = c_raw.get("json")
        n_json = n_raw.get("json")
        if c_json != n_json:
            embedded_json_mismatches.append(doc_id)

        c_doc = normalize_solr_doc(c_raw)
        n_doc = normalize_solr_doc(n_raw)

        diffs = collect_same_stage_stored_diffs(c_doc, n_doc)
        if diffs:
            records_with_diffs += 1
            total_diffs += len(diffs)
            for d in diffs:
                norm_p = d["path"]
                path_counts[norm_p] += 1
                path_categories[norm_p] = d["category"]
            if len(sample_diffs) < 20:
                sample_diffs[doc_id] = diffs

    exact_parity = (
        len(missing_in_candidate) == 0
        and len(extra_in_candidate) == 0
        and len(cand_marker_leaks) == 0
        and total_diffs == 0
        and len(embedded_json_mismatches) == 0
    )

    return {
        "exact_parity": exact_parity,
        "records_compared": len(common_ids),
        "candidate_total_docs": len(cand_docs_by_id),
        "native_total_docs": len(native_docs_by_id),
        "missing_in_candidate_count": len(missing_in_candidate),
        "missing_in_candidate_sample": missing_in_candidate[:10],
        "extra_in_candidate_count": len(extra_in_candidate),
        "extra_in_candidate_sample": extra_in_candidate[:10],
        "candidate_marker_leaks_count": len(cand_marker_leaks),
        "candidate_marker_leaks_sample": cand_marker_leaks[:10],
        "embedded_json_mismatches_count": len(embedded_json_mismatches),
        "embedded_json_mismatches_sample": embedded_json_mismatches[:10],
        "records_with_exact_equality": len(common_ids) - records_with_diffs,
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
        description="Gate 3 STAFF Combined Slice Live Parity Harness"
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
        "--repo",
        type=int,
        default=DEFAULT_REPO_ID,
        help="Repository ID (default: 4)",
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


def run_harness() -> int:
    args = parse_args()
    print("=== Gate 3 STAFF Combined Slice Live Parity Harness ===", flush=True)

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
    git_diff_proc = subprocess.run(
        ["git", "-C", REPO_ROOT, "status", "--porcelain", "-uno"],
        capture_output=True,
        text=True,
        check=True,
    )
    is_dirty = bool(git_diff_proc.stdout.strip())

    print(f"Git branch: {git_branch}")
    print(f"Git HEAD: {git_head} (tracked files dirty: {is_dirty})")

    if git_branch != EXPECTED_BRANCH:
        raise RuntimeError(
            f"Expected branch '{EXPECTED_BRANCH}', but current branch is '{git_branch}'"
        )

    # 2. Check binary provenance
    cli_bin = os.path.join(REPO_ROOT, "rust-tools", "target", "release", "aspace-cli")
    provenance = verify_binary_provenance(cli_bin)
    print(f"Verified release binary: {cli_bin} (sha256: {provenance['sha256']})")

    # 3. Read and verify state file
    if not os.path.isfile(args.state):
        raise FileNotFoundError(f"State file not found at {args.state}")
    with open(args.state) as f:
        state = json.load(f)

    if "admin_password" in state:
        register_secret(state["admin_password"])

    if state.get("stack_id") != EXPECTED_STACK_ID:
        raise RuntimeError(
            f"State file stack_id '{state.get('stack_id')}' != expected '{EXPECTED_STACK_ID}'"
        )

    # 4. Check memory headroom
    mem_avail = get_mem_available_gib()
    print(
        f"Available memory: {mem_avail:.2f} GiB (minimum required: {MIN_MEMORY_HEADROOM_GIB} GiB)"
    )
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
        raise RuntimeError(
            f"Dedicated profiling DB container '{DB_CONTAINER}' is not running!"
        )
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
        print(
            "\n[DRY RUN] Safety and provenance checks passed cleanly. Halting before live container start or write."
        )
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

    # 9. Verify native reference core exists and has exactly EXPECTED_TOTAL_NATIVE_DOCS (28,201) documents
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

    if native_num_found != EXPECTED_TOTAL_NATIVE_DOCS:
        raise RuntimeError(
            f"Native reference core '{native_core}' contains {native_num_found} docs; expected exactly {EXPECTED_TOTAL_NATIVE_DOCS}"
        )
    print(
        f"Native reference core '{native_core}' verified present with exactly {native_num_found} documents."
    )

    # 10. Start profiling backend if stopped (track for cleanup)
    backend_host_url = state["backend_host_url"].rstrip("/")
    if not is_running(BACKEND_CONTAINER):
        print(
            f"Starting isolated profiling backend ({BACKEND_CONTAINER})...", flush=True
        )
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
    print(redact_secrets(verify_res.stdout.strip()), flush=True)

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
        raise RuntimeError(
            "Failed to extract arks_enabled boolean from live /system/info"
        )
    print(f"Live active backend verified: arks_enabled={active_arks} (mode: {v_mode})")

    # 13. Pre-fetch native reference documents for Repo 4 combined slice
    repo_scope = f"/repositories/{args.repo}"
    native_fq = f'repository:"{repo_scope}" AND (primary_type:archival_object OR primary_type:resource OR primary_type:digital_object OR (primary_type:collection_management AND (parent_type:resource OR parent_type:digital_object)))'
    print(
        f"\nStreaming native reference combined slice docs from '{native_core}' (fq: {native_fq})...",
        flush=True,
    )
    t_native_fetch_start = time.perf_counter()
    native_slice_docs_by_id, native_marker_leaks = stream_fetch_solr_core_documents(
        solr_host_url,
        native_core,
        fq=native_fq,
        batch_size=1000,
    )
    t_native_fetch_elapsed = time.perf_counter() - t_native_fetch_start
    print(
        f"Fetched {len(native_slice_docs_by_id)} native combined slice documents in {t_native_fetch_elapsed:.2f}s "
        f"(native marker leaks: {len(native_marker_leaks)})."
    )

    ao_native_ids = [
        k
        for k, v in native_slice_docs_by_id.items()
        if v.get("primary_type") == "archival_object"
    ]
    resource_native_ids = [
        k
        for k, v in native_slice_docs_by_id.items()
        if v.get("primary_type") == "resource"
    ]
    do_native_ids = [
        k
        for k, v in native_slice_docs_by_id.items()
        if v.get("primary_type") == "digital_object"
    ]
    cm_native_ids = [
        k
        for k, v in native_slice_docs_by_id.items()
        if v.get("primary_type") == "collection_management"
    ]
    print(
        f"Native breakdown: {len(ao_native_ids)} archival objects, {len(resource_native_ids)} resources, {len(do_native_ids)} digital objects, {len(cm_native_ids)} collection_management children."
    )

    # 14. Create empty candidate core
    run_nonce = f"{time.strftime('%Y%m%d_%H%M%S', time.gmtime())}_{os.getpid()}_{secrets.token_hex(4)}"
    cand_core = f"stage_staff_{run_nonce}_candidate"
    print(f"\n--- Starting candidate pass on core '{cand_core}' ---", flush=True)

    # A. Verify ABSENCE of candidate core before creation
    c_status_req = urllib.request.urlopen(
        f"{solr_host_url}/admin/cores?action=STATUS&core={cand_core}&wt=json",
        timeout=10,
    )
    c_status_data = json.loads(c_status_req.read().decode())
    if c_status_data.get("status", {}).get(cand_core):
        raise RuntimeError(
            f"Candidate core '{cand_core}' already exists before creation! No-clobber refusal."
        )

    # B. Create empty candidate core
    create_url = f"{solr_host_url}/admin/cores?" + urllib.parse.urlencode({
        "action": "CREATE",
        "name": cand_core,
        "configSet": "archivesspace",
        "wt": "json",
    })
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
        raise RuntimeError(
            f"Candidate core '{cand_core}' contains {c_init_docs} docs; expected 0"
        )

    # 15. Execute aspace-cli staff-combined-candidate
    cli_cmd = build_candidate_cli_command(
        cli_bin=cli_bin,
        backend_host_url=backend_host_url,
        admin_user=state["admin_user"],
        state_path=args.state,
        repo_id=args.repo,
        solr_host_url=solr_host_url,
        cand_core=cand_core,
        active_arks=bool(active_arks),
    )
    cli_env = build_candidate_cli_env(admin_password=state.get("admin_password"))

    # Never print entire env or secret
    rendered_cmd = " ".join(cli_cmd)
    print(f"Running CLI command: {redact_secrets(rendered_cmd)}", flush=True)
    t_cli_start = time.perf_counter()
    cli_proc = subprocess.run(cli_cmd, capture_output=True, text=True, env=cli_env)
    t_cli_elapsed = time.perf_counter() - t_cli_start

    print(
        f"CLI process finished with exit code {cli_proc.returncode} in {t_cli_elapsed:.3f}s"
    )
    if cli_proc.returncode != 0:
        redacted_err = redact_secrets(cli_proc.stderr)
        print(f"CLI stderr: {redacted_err}", file=sys.stderr)
        raise RuntimeError(
            f"Candidate CLI failed with exit code {cli_proc.returncode}"
        )

    try:
        cand_summary = json.loads(cli_proc.stdout)
    except Exception as e:
        redacted_out = redact_secrets(cli_proc.stdout)
        print(f"Raw CLI stdout:\n{redacted_out}", file=sys.stderr)
        raise RuntimeError(
            f"Failed to parse candidate JSON summary: {redact_secrets(e)}"
        )

    print("CLI summary:")
    print(f"  total_ids_discovered: {cand_summary.get('total_ids_discovered')}")
    print(f"  exact_ids_indexed: {cand_summary.get('exact_ids_indexed')}")
    print(f"  parent_docs: {cand_summary.get('parent_docs')}")
    print(f"  child_docs: {cand_summary.get('child_docs')}")
    print(f"  total_docs_indexed: {cand_summary.get('total_docs_indexed')}")
    print(f"  archival_objects: {cand_summary.get('archival_objects')}")
    print(f"  resources: {cand_summary.get('resources')}")
    print(f"  digital_objects: {cand_summary.get('digital_objects')}")
    print(f"  committed: {cand_summary.get('committed')}")
    print(f"  success: {cand_summary.get('success')}")

    if not cand_summary.get("success", False):
        err_msg = redact_secrets(cand_summary.get("error_message"))
        raise RuntimeError(f"Candidate CLI reported success=false: {err_msg}")

    # 16. Verify candidate core document count directly from Solr
    cand_select_url = f"{solr_host_url}/{cand_core}/select?q=*:*&rows=0&wt=json"
    with urllib.request.urlopen(cand_select_url, timeout=10) as resp:
        data = json.loads(resp.read().decode())
        solr_num_found = data.get("response", {}).get("numFound", -1)
    print(f"Solr candidate core numFound: {solr_num_found}")

    # 17. Fetch all stored documents from candidate core
    print(f"Streaming candidate documents from '{cand_core}'...", flush=True)
    cand_docs_by_id, cand_marker_leaks = stream_fetch_solr_core_documents(
        solr_host_url,
        cand_core,
        fq=None,
        batch_size=1000,
    )
    print(
        f"Fetched {len(cand_docs_by_id)} candidate documents (marker leaks: {len(cand_marker_leaks)})."
    )

    # 18. Parity comparison against native reference documents
    print(
        "\nComparing stored candidate documents against native reference documents...",
        flush=True,
    )
    t_comp_start = time.perf_counter()
    parity_result = run_slice_parity_comparison(
        cand_docs_by_id,
        native_slice_docs_by_id,
        cand_marker_leaks,
    )
    t_comp_elapsed = time.perf_counter() - t_comp_start

    print(f"Parity comparison finished in {t_comp_elapsed:.3f}s:")
    print(f"  exact_parity: {parity_result['exact_parity']}")
    print(f"  records_compared: {parity_result['records_compared']}")
    print(f"  candidate_total_docs: {parity_result['candidate_total_docs']}")
    print(f"  native_total_docs: {parity_result['native_total_docs']}")
    print(f"  missing_in_candidate_count: {parity_result['missing_in_candidate_count']}")
    print(f"  extra_in_candidate_count: {parity_result['extra_in_candidate_count']}")
    print(
        f"  candidate_marker_leaks_count: {parity_result['candidate_marker_leaks_count']}"
    )
    print(
        f"  embedded_json_mismatches_count: {parity_result['embedded_json_mismatches_count']}"
    )
    print(
        f"  records_with_exact_equality: {parity_result['records_with_exact_equality']}"
    )
    print(f"  records_with_differences: {parity_result['records_with_differences']}")
    print(f"  total_differences: {parity_result['total_differences']}")

    if parity_result["top_differing_paths"]:
        print("  Top differing paths:")
        for entry in parity_result["top_differing_paths"]:
            print(f"    - {entry['path']}: {entry['count']} ({entry['category']})")

    # 19. Retain evidence report with mode 0600
    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "git_branch": git_branch,
        "git_head": git_head,
        "git_dirty": is_dirty,
        "binary_provenance": provenance,
        "stack_id": EXPECTED_STACK_ID,
        "repo_id": args.repo,
        "native_core": native_core,
        "candidate_core": cand_core,
        "active_arks_enabled": active_arks,
        "cli_summary": cand_summary,
        "solr_verified_docs": solr_num_found,
        "parity_result": parity_result,
    }
    report_filename = f"gate3_combined_slice_parity_{run_nonce}.json"
    report_path = os.path.join(args.output_dir, report_filename)
    write_file_mode_0600(report_path, json.dumps(report, indent=2))
    print(f"\nRetained mode 0600 evidence report at: {report_path}")

    if not parity_result["exact_parity"]:
        print(
            "\n[PARITY FAILURE] Stored candidate documents do not exactly match native reference documents.",
            file=sys.stderr,
        )
        return 1

    print(
        "\n[PARITY SUCCESS] Exact stored-document parity verified for combined slice!",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(run_harness())
    except Exception as e:
        print(f"\n[FATAL ERROR] {redact_secrets(e)}", file=sys.stderr)
        sys.exit(1)
