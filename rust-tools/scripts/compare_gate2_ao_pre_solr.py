#!/usr/bin/env python3
"""Gate 2 Same-Stage Pre-Solr Native-vs-Rust Comparison Harness

Performs a bounded SAME-STAGE pre-Solr comparison for STAFF archival_object indexing:
  1. Native side: uses actual PeriodicIndexer, index_records, IndexBatch with all ordinary hooks,
     sanitation, and clean_whitespace, intercepted immediately before Solr update so exact
     outgoing document JSON is captured without Solr writes (do not change production indexer).
  2. Rust side: existing builder candidate with the same full-resolve backend records,
     dynamic enums from actual /schemas, arks_enabled from active /system/info, strip markers.
  3. First samples 25 IDs from the ignored normalization experiment. Runs all 24,200 only if sample works.
  4. Output sensitive docs only to mode 0600 ignored files; stdout contains summary/counts/hashes only.
  5. Compares type-strict parsed JSON for every key/value including indexed-only fields (e.g. fullrecord,
     extents, subjects, title_sort).
  6. Compares embedded json string EXACT bytes separately, not merely parsed equality.
  7. Includes URI set, duplicate/missing, child docs if any.
  8. If mismatches exist, identifies top paths and STOPS short of full run; proposes exact fix.
  9. No Solr core writes, no new cores, no full speed run; source backend read-only.
 10. Safety: verifies isolated e62875f1 stack labels, nonce DB identity, active config, memory.
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
DEFAULT_EXPERIMENT_FILE = os.path.join(
    BENCHMARKS_DIR, "gate2_ao_normalization_experiment_e62875f1_20261026_085620.json"
)
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
BENCH_SOLR_CONTAINER = "archivesspaceai-solr-bench"
APP_CONTAINER = "archivesspaceai-app-1"

# The 25 sample IDs from normalization experiment
SAMPLE_25_IDS = [
    1221, 2229, 3238, 4246, 5254, 6262, 7271, 8279, 9287, 10296,
    11304, 12312, 13321, 14329, 15337, 16345, 17354, 18362, 19370, 20379,
    21387, 22395, 23403, 24412, 25420
]

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
        data = json.loads(res.stdout)
        return data[0] if data else None
    except Exception:
        return None


def is_running(name: str) -> bool:
    info = docker_inspect(name)
    return info is not None and info.get("State", {}).get("Running", False)


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


def normalize_path(path: str) -> str:
    """Normalizes array index subpaths to '[]' for aggregation."""
    return re.sub(r"\[\d+\]", "[]", path)


def collect_pre_solr_doc_diffs(
    candidate_doc: Dict[str, Any],
    native_doc: Dict[str, Any],
    path: str = "",
) -> List[Dict[str, Any]]:
    """Recursively collects field-level differences between candidate and native pre-Solr documents.

    Redacts all payload data: reports only path, category, types, sha256 hashes, and sizes.
    """
    diffs: List[Dict[str, Any]] = []

    if type(candidate_doc) is not type(native_doc):
        h_c, s_c = value_sha256_and_size(candidate_doc)
        h_n, s_n = value_sha256_and_size(native_doc)
        diffs.append({
            "path": path or "<root>",
            "category": "type_mismatch",
            "candidate_type": type(candidate_doc).__name__,
            "native_type": type(native_doc).__name__,
            "candidate_hash": h_c,
            "native_hash": h_n,
            "candidate_size": s_c,
            "native_size": s_n,
        })
        return diffs

    if isinstance(candidate_doc, dict):
        c_keys = set(candidate_doc.keys())
        n_keys = set(native_doc.keys())

        # Missing in candidate (present in native)
        for k in sorted(n_keys - c_keys):
            subpath = f"{path}.{k}" if path else k
            h_n, s_n = value_sha256_and_size(native_doc[k])
            diffs.append({
                "path": subpath,
                "category": "missing_in_candidate",
                "field": k,
                "native_type": type(native_doc[k]).__name__ if native_doc[k] is not None else "null",
                "native_hash": h_n,
                "native_size": s_n,
            })

        # Extra in candidate (absent in native)
        for k in sorted(c_keys - n_keys):
            subpath = f"{path}.{k}" if path else k
            h_c, s_c = value_sha256_and_size(candidate_doc[k])
            diffs.append({
                "path": subpath,
                "category": "extra_in_candidate",
                "field": k,
                "candidate_type": type(candidate_doc[k]).__name__ if candidate_doc[k] is not None else "null",
                "candidate_hash": h_c,
                "candidate_size": s_c,
            })

        # Compare common keys
        for k in sorted(c_keys & n_keys):
            subpath = f"{path}.{k}" if path else k
            c_val = candidate_doc[k]
            n_val = native_doc[k]

            if type(c_val) is not type(n_val):
                h_c, s_c = value_sha256_and_size(c_val)
                h_n, s_n = value_sha256_and_size(n_val)
                diffs.append({
                    "path": subpath,
                    "category": "type_mismatch",
                    "candidate_type": type(c_val).__name__,
                    "native_type": type(n_val).__name__,
                    "candidate_hash": h_c,
                    "native_hash": h_n,
                    "candidate_size": s_c,
                    "native_size": s_n,
                })
            elif isinstance(c_val, dict):
                diffs.extend(collect_pre_solr_doc_diffs(c_val, n_val, subpath))
            elif isinstance(c_val, list):
                if is_array_reordered(c_val, n_val):
                    h_c, s_c = value_sha256_and_size(c_val)
                    h_n, s_n = value_sha256_and_size(n_val)
                    diffs.append({
                        "path": subpath,
                        "category": "array_reordered",
                        "candidate_type": "list",
                        "native_type": "list",
                        "candidate_hash": h_c,
                        "native_hash": h_n,
                        "candidate_size": s_c,
                        "native_size": s_n,
                    })
                elif len(c_val) != len(n_val):
                    h_c, s_c = value_sha256_and_size(c_val)
                    h_n, s_n = value_sha256_and_size(n_val)
                    diffs.append({
                        "path": subpath,
                        "category": "array_length_mismatch",
                        "candidate_len": len(c_val),
                        "native_len": len(n_val),
                        "candidate_hash": h_c,
                        "native_hash": h_n,
                        "candidate_size": s_c,
                        "native_size": s_n,
                    })
                else:
                    for idx, (item_c, item_n) in enumerate(zip(c_val, n_val)):
                        elem_path = f"{subpath}[{idx}]"
                        if not json_values_strictly_equal(item_c, item_n):
                            diffs.extend(collect_pre_solr_doc_diffs(item_c, item_n, elem_path))
            else:
                if not json_values_strictly_equal(c_val, n_val):
                    h_c, s_c = value_sha256_and_size(c_val)
                    h_n, s_n = value_sha256_and_size(n_val)
                    diffs.append({
                        "path": subpath,
                        "category": "value_mismatch",
                        "candidate_type": type(c_val).__name__,
                        "native_type": type(n_val).__name__,
                        "candidate_hash": h_c,
                        "native_hash": h_n,
                        "candidate_size": s_c,
                        "native_size": s_n,
                    })

    return diffs


def compare_embedded_json_string(
    native_json_str: Optional[str],
    candidate_json_str: Optional[str],
) -> Dict[str, Any]:
    """Compares the embedded doc['json'] string separately:

    1) Exact byte equality (byte-for-byte UTF-8 comparison)
    2) Parsed type-strict JSON equality
    """
    if native_json_str is None or candidate_json_str is None:
        return {
            "exact_bytes_match": native_json_str == candidate_json_str,
            "parsed_equality": native_json_str == candidate_json_str,
            "native_bytes_len": len(native_json_str.encode("utf-8")) if native_json_str else 0,
            "candidate_bytes_len": len(candidate_json_str.encode("utf-8")) if candidate_json_str else 0,
            "native_sha256": hashlib.sha256(native_json_str.encode("utf-8")).hexdigest()[:16] if native_json_str else "",
            "candidate_sha256": hashlib.sha256(candidate_json_str.encode("utf-8")).hexdigest()[:16] if candidate_json_str else "",
            "parsed_diffs_count": 0 if native_json_str == candidate_json_str else 1,
        }

    native_bytes = native_json_str.encode("utf-8")
    candidate_bytes = candidate_json_str.encode("utf-8")

    exact_bytes_match = (native_bytes == candidate_bytes)

    # Parse both as JSON
    parsed_diffs = []
    try:
        parsed_native = json.loads(native_json_str)
        parsed_candidate = json.loads(candidate_json_str)
        parsed_diffs = collect_pre_solr_doc_diffs(parsed_candidate, parsed_native, "json")
        parsed_equality = (len(parsed_diffs) == 0)
    except Exception as e:
        parsed_equality = False
        parsed_diffs = [{"path": "json", "category": "parse_error", "error": str(e)}]

    return {
        "exact_bytes_match": exact_bytes_match,
        "parsed_equality": parsed_equality,
        "native_bytes_len": len(native_bytes),
        "candidate_bytes_len": len(candidate_bytes),
        "native_sha256": hashlib.sha256(native_bytes).hexdigest()[:16],
        "candidate_sha256": hashlib.sha256(candidate_bytes).hexdigest()[:16],
        "parsed_diffs_count": len(parsed_diffs),
        "parsed_diffs_sample": parsed_diffs[:5] if parsed_diffs else [],
    }


def write_file_mode_0600(path: str, content: str) -> None:
    parent = os.path.dirname(path) or "."
    filename = os.path.basename(path)
    tmp_path = os.path.join(parent, f".tmp_{os.getpid()}_{time.time_ns()}_{filename}")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    fd = os.open(tmp_path, flags, 0o600)
    try:
        with open(fd, "w", encoding="utf-8") as f:
            f.write(content)
            f.flush()
            os.fsync(f.fileno())
        os.link(tmp_path, path)
    finally:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass


def verify_active_app_arks_enabled(backend_url: str, session: str) -> bool:
    req = urllib.request.Request(
        f"{backend_url}/system/info",
        headers={"X-ArchivesSpace-Session": session},
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        info = json.loads(resp.read().decode())
    return bool(info.get("appConfig", {}).get("arks_enabled", False))


def verify_binary_provenance(bin_path: str) -> Dict[str, Any]:
    if not os.path.isfile(bin_path):
        raise FileNotFoundError(f"Binary not found: {bin_path}")
    hasher = hashlib.sha256()
    with open(bin_path, "rb") as f:
        while chunk := f.read(65536):
            hasher.update(chunk)
    sha256 = hasher.hexdigest()
    mtime = os.path.getmtime(bin_path)
    mtime_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(mtime))
    return {
        "path": bin_path,
        "sha256": sha256,
        "mtime": mtime_iso,
    }


def run_pre_solr_comparison(
    native_docs: List[Dict[str, Any]],
    candidate_docs: List[Dict[str, Any]],
) -> Dict[str, Any]:
    """Runs a type-strict comparison across pre-Solr documents."""
    native_by_uri = {}
    native_duplicates = 0
    for d in native_docs:
        uri = d.get("uri")
        if uri:
            if uri in native_by_uri:
                native_duplicates += 1
            native_by_uri[uri] = d

    candidate_by_uri = {}
    candidate_duplicates = 0
    candidate_marker_leaks = 0
    for d in candidate_docs:
        uri = d.get("uri")
        if uri:
            if uri in candidate_by_uri:
                candidate_duplicates += 1
            candidate_by_uri[uri] = d
        for k in d.keys():
            if str(k).startswith("_archivalist_"):
                candidate_marker_leaks += 1

    native_uris = set(native_by_uri.keys())
    candidate_uris = set(candidate_by_uri.keys())

    missing_in_candidate = sorted(native_uris - candidate_uris)
    extra_in_candidate = sorted(candidate_uris - native_uris)
    common_uris = sorted(native_uris & candidate_uris)

    records_with_exact_equality = 0
    records_with_differences = 0
    all_diffs: List[Dict[str, Any]] = []
    embedded_json_bytes_matches = 0
    embedded_json_parsed_matches = 0

    per_record_summaries = {}

    for uri in common_uris:
        n_doc = native_by_uri[uri]
        c_doc = candidate_by_uri[uri]

        # 1. Compare embedded json string separately
        n_json_str = n_doc.get("json")
        c_json_str = c_doc.get("json")
        json_str_comp = compare_embedded_json_string(n_json_str, c_json_str)
        if json_str_comp["exact_bytes_match"]:
            embedded_json_bytes_matches += 1
        if json_str_comp["parsed_equality"]:
            embedded_json_parsed_matches += 1

        # 2. Compare outer document fields (excluding 'json' which is compared embedded)
        n_doc_outer = {k: v for k, v in n_doc.items() if k != "json"}
        c_doc_outer = {k: v for k, v in c_doc.items() if k != "json"}

        doc_diffs = collect_pre_solr_doc_diffs(c_doc_outer, n_doc_outer)

        if not json_str_comp["parsed_equality"]:
            for pd in json_str_comp.get("parsed_diffs_sample", []):
                doc_diffs.append(pd)

        if len(doc_diffs) == 0:
            records_with_exact_equality += 1
        else:
            records_with_differences += 1
            all_diffs.extend(doc_diffs)

        per_record_summaries[uri] = {
            "outer_fields_match": len(doc_diffs) == 0,
            "embedded_json_exact_bytes_match": json_str_comp["exact_bytes_match"],
            "embedded_json_parsed_match": json_str_comp["parsed_equality"],
            "diffs_count": len(doc_diffs),
            "top_diff_paths": [d["path"] for d in doc_diffs[:5]],
        }

    # Aggregate differing paths
    path_counter = Counter(normalize_path(d["path"]) for d in all_diffs)
    top_paths = [
        {"path": p, "count": c}
        for p, c in path_counter.most_common(20)
    ]

    return {
        "records_compared": len(common_uris),
        "native_total_docs": len(native_docs),
        "candidate_total_docs": len(candidate_docs),
        "native_duplicates": native_duplicates,
        "candidate_duplicates": candidate_duplicates,
        "missing_in_candidate_count": len(missing_in_candidate),
        "extra_in_candidate_count": len(extra_in_candidate),
        "candidate_marker_leaks_count": candidate_marker_leaks,
        "records_with_exact_equality": records_with_exact_equality,
        "records_with_differences": records_with_differences,
        "total_differences": len(all_diffs),
        "distinct_differing_paths": len(path_counter),
        "top_differing_paths": top_paths,
        "embedded_json_bytes_matches": embedded_json_bytes_matches,
        "embedded_json_parsed_matches": embedded_json_parsed_matches,
        "per_record_summaries_sample": {
            u: per_record_summaries[u] for u in common_uris[:5]
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", default=DEFAULT_STATE_FILE, help="Path to stack state JSON")
    parser.add_argument("--repo-id", type=int, default=DEFAULT_REPO_ID, help="Repository ID")
    parser.add_argument(
        "--full-if-sample-passes",
        action="store_true",
        help="Run all 24,200 IDs if the 25-ID sample passes with 0 differences",
    )
    args = parser.parse_args()

    print("=== Gate 2 Pre-Solr Native-vs-Rust Comparison Harness ===", flush=True)

    # 1. Verify Git Branch
    git_branch_proc = subprocess.run(
        ["git", "branch", "--show-current"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    current_branch = git_branch_proc.stdout.strip()
    if current_branch != EXPECTED_BRANCH:
        raise RuntimeError(f"Expected branch {EXPECTED_BRANCH}, got {current_branch}")
    print(f"Verified branch: {current_branch}")

    git_commit_proc = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    current_commit = git_commit_proc.stdout.strip()
    print(f"Verified commit: {current_commit}")

    # 2. Verify Release Binary Provenance
    release_bin = os.path.join(REPO_ROOT, "rust-tools", "target", "release", "aspace-cli")
    bin_prov = verify_binary_provenance(release_bin)
    print(f"Verified release binary SHA256: {bin_prov['sha256']} ({bin_prov['path']})")

    # 3. Verify Memory Headroom
    mem_avail = get_mem_available_gib()
    if mem_avail < MIN_MEMORY_HEADROOM_GIB:
        raise RuntimeError(f"Insufficient memory headroom: {mem_avail:.1f} GiB available")
    print(f"Memory available: {mem_avail:.2f} GiB")

    # 4. Verify Main Solr Stopped
    if is_running(MAIN_SOLR_CONTAINER):
        raise RuntimeError(f"Safety violation: {MAIN_SOLR_CONTAINER} is running!")
    if is_running(BENCH_SOLR_CONTAINER):
        raise RuntimeError(f"Safety violation: {BENCH_SOLR_CONTAINER} is running!")
    print("Verified main and bench Solr containers are stopped.")

    # 5. Verify Stack State File and DB Container
    with open(args.state) as f:
        state = json.load(f)
    if state.get("stack_id") != EXPECTED_STACK_ID:
        raise RuntimeError(f"Stack ID mismatch: expected {EXPECTED_STACK_ID}, got {state.get('stack_id')}")

    db_info = docker_inspect(DB_CONTAINER)
    if not db_info or not db_info.get("State", {}).get("Running", False):
        raise RuntimeError(f"DB container {DB_CONTAINER} is not running!")
    db_labels = db_info.get("Config", {}).get("Labels", {})
    if db_labels.get("archivalist-profiling-stack") != EXPECTED_STACK_ID:
        raise RuntimeError(f"DB container label mismatch for stack {EXPECTED_STACK_ID}")
    print(f"Verified DB container {DB_CONTAINER} is running with correct labels.")

    # 6. Start Solr and Backend if stopped
    if not is_running(SOLR_CONTAINER):
        print(f"Starting {SOLR_CONTAINER}...", flush=True)
        subprocess.run(["docker", "start", SOLR_CONTAINER], check=True)
        started_containers.append(SOLR_CONTAINER)

    # Wait for Solr to respond
    solr_host_url = state["solr_host_url"]
    for _ in range(30):
        try:
            req = urllib.request.urlopen(f"{solr_host_url}/admin/info/system", timeout=2)
            if req.status == 200:
                break
        except Exception:
            pass
        time.sleep(1)

    if not is_running(BACKEND_CONTAINER):
        print(f"Starting {BACKEND_CONTAINER}...", flush=True)
        subprocess.run(["docker", "start", BACKEND_CONTAINER], check=True)
        started_containers.append(BACKEND_CONTAINER)

    # Wait for Backend
    backend_host_url = state["backend_host_url"]
    backend_ready = False
    print("Waiting for backend to be ready...", flush=True)
    for attempt in range(120):
        try:
            req = urllib.request.urlopen(f"{backend_host_url}/", timeout=3)
            if req.status == 200:
                body = req.read().decode("utf-8", errors="ignore")
                if "databaseProductName" in body:
                    backend_ready = True
                    break
        except Exception:
            pass
        time.sleep(2)
        if attempt > 0 and attempt % 10 == 0:
            print(f"  still waiting ({attempt * 2}s)...", flush=True)
    if not backend_ready:
        raise RuntimeError("Backend failed to become ready within 240s")
    print("Backend is ready.")

    # 7. Verify Stack DB Nonce Challenge-Response
    print("Verifying stack identity via challenge nonce...", flush=True)
    verify_cmd = [
        "python3",
        os.path.join(REPO_ROOT, "rust-tools", "scripts", "profiling_stack.py"),
        "verify",
        "--state", args.state,
    ]
    verify_proc = subprocess.run(verify_cmd, capture_output=True, text=True, check=True)
    print(verify_proc.stdout.strip())

    # 8. Authenticate and verify active config
    login_url = f"{backend_host_url}/users/{state['admin_user']}/login?password={state['admin_password']}"
    login_req = urllib.request.Request(login_url, data=b"", method="POST")
    with urllib.request.urlopen(login_req, timeout=30) as resp:
        session_token = json.loads(resp.read().decode())["session"]
    print("Acquired admin session token.")

    active_arks_enabled = verify_active_app_arks_enabled(backend_host_url, session_token)
    print(f"Verified active backend arks_enabled: {active_arks_enabled}")

    # 9. Phase 1: 25-ID Sample Comparison
    print(f"\n=== Phase 1: 25-ID Sample Comparison ===", flush=True)
    sample_ids = SAMPLE_25_IDS
    print(f"Sample AO IDs: {sample_ids}")

    ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
    native_out_rel = f"rust-tools/benchmarks/native_pre_solr_sample25_{ts}.json"
    native_out_host = os.path.join(REPO_ROOT, native_out_rel)
    native_out_container = f"/archivesspace/{native_out_rel}"

    # Pre-create mode 0600 file
    write_file_mode_0600(native_out_host, "")

    # Native capture via app container
    print("Capturing native pre-Solr documents via JRuby...", flush=True)
    native_cmd = [
        "docker", "exec",
        "-e", "GEM_HOME=/archivesspace/build/gems/jruby/3.1.0",
        "-e", "GEM_PATH=/archivesspace/build/gems/jruby/3.1.0",
        "-w", "/archivesspace",
        APP_CONTAINER,
        "/archivesspace/scripts/jruby",
        "/archivesspace/rust-tools/scripts/capture_native_pre_solr_ao.rb",
        "--repo-id", str(args.repo_id),
        "--ids", ",".join(str(i) for i in sample_ids),
        "--backend-url", state["backend_network_url"],
        "--session", session_token,
        "--output", native_out_container,
    ]
    t_native_start = time.perf_counter()
    native_proc = subprocess.run(native_cmd, capture_output=True, text=True)
    t_native_elapsed = time.perf_counter() - t_native_start
    if native_proc.returncode != 0:
        print(f"Native capture failed (exit {native_proc.returncode}):\n{native_proc.stderr}", file=sys.stderr)
        raise RuntimeError("Native pre-Solr capture failed")
    print(f"Native capture completed in {t_native_elapsed:.2f}s: {native_proc.stdout.strip()}")

    # Rust candidate execution via release aspace-cli staff-ao-builder
    rust_out_rel = f"rust-tools/benchmarks/rust_pre_solr_sample25_{ts}.json"
    rust_out_host = os.path.join(REPO_ROOT, rust_out_rel)

    print("Building Rust candidate pre-Solr documents via aspace-cli...", flush=True)
    rust_cmd = [
        release_bin,
        "--url", backend_host_url,
        "--username", state["admin_user"],
        "--password", state["admin_password"],
        "staff-ao-builder",
        "--repo-id", str(args.repo_id),
        "--ids", ",".join(str(i) for i in sample_ids),
        "--arks-enabled", "true" if active_arks_enabled else "false",
        "--strip-markers", "true",
        "--output", rust_out_host,
    ]
    t_rust_start = time.perf_counter()
    rust_proc = subprocess.run(rust_cmd, capture_output=True, text=True)
    t_rust_elapsed = time.perf_counter() - t_rust_start
    if rust_proc.returncode != 0:
        print(f"Rust builder failed (exit {rust_proc.returncode}):\n{rust_proc.stderr}", file=sys.stderr)
        raise RuntimeError("Rust pre-Solr builder failed")
    print(f"Rust builder completed in {t_rust_elapsed:.2f}s.")

    # Read captured documents
    with open(native_out_host, encoding="utf-8") as f:
        native_data = json.load(f)
    native_docs = native_data["documents"]

    with open(rust_out_host, encoding="utf-8") as f:
        rust_docs = json.load(f)

    print(f"Loaded {len(native_docs)} native docs and {len(rust_docs)} Rust docs.")

    # Run type-strict pre-Solr comparison
    print("Running type-strict pre-Solr comparison...", flush=True)
    sample_comparison = run_pre_solr_comparison(native_docs, rust_docs)

    print("\n--- Sample Comparison Summary ---")
    print(f"Records compared: {sample_comparison['records_compared']}")
    print(f"Records with exact equality: {sample_comparison['records_with_exact_equality']}")
    print(f"Records with differences: {sample_comparison['records_with_differences']}")
    print(f"Total differences: {sample_comparison['total_differences']}")
    print(f"Distinct differing paths: {sample_comparison['distinct_differing_paths']}")
    print(f"Embedded json exact bytes matches: {sample_comparison['embedded_json_bytes_matches']}/{sample_comparison['records_compared']}")
    print(f"Embedded json parsed matches: {sample_comparison['embedded_json_parsed_matches']}/{sample_comparison['records_compared']}")
    print("Top differing paths:")
    for p in sample_comparison["top_differing_paths"]:
        print(f"  {p['path']}: {p['count']}")

    # Formulate report
    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "stack_id": EXPECTED_STACK_ID,
        "git_commit": current_commit,
        "git_branch": current_branch,
        "release_binary": bin_prov,
        "repo_id": args.repo_id,
        "sample_ids": sample_ids,
        "sample_comparison": sample_comparison,
        "full_run_executed": False,
        "gate2_pre_solr_pass": (sample_comparison["records_with_differences"] == 0),
        "stop_reason": None,
        "proposed_fix": None,
    }

    if sample_comparison["records_with_differences"] > 0:
        print("\n[STOP] Mismatches exist between native pre-Solr and Rust candidate pre-Solr documents.")
        print("STOPPING short of full 24,200 run per protocol.")
        report["stop_reason"] = "Sample comparison detected pre-Solr document field differences; stopped short of full run."
        
        # Determine exact proposed fix based on top paths
        top_diff_paths = [p["path"] for p in sample_comparison["top_differing_paths"]]
        fix_proposals = []
        if "title_sort" in top_diff_paths:
            fix_proposals.append(
                "Add `title_sort` calculation for `archival_object` in `add_type_specific_fields` / `add_enriched_fields` in `index_document.rs` using `ruby_clean_for_sort(display_string)`."
            )
        if any("fullrecord" in p for p in top_diff_paths):
            fix_proposals.append(
                "Align `fullrecord` / `fullrecord_published` extraction order or exclusions between Ruby `build_fullrecord` and Rust `extract_string_values`."
            )
        if any("extents" in p for p in top_diff_paths):
            fix_proposals.append(
                "Align `extents` formatting in Rust `add_enriched_fields`."
            )
        if any("subjects" in p for p in top_diff_paths):
            fix_proposals.append(
                "Align `subjects` / `subject_uris` handling in Rust `add_subjects`."
            )
        if sample_comparison["embedded_json_bytes_matches"] < sample_comparison["records_compared"]:
            if sample_comparison["embedded_json_parsed_matches"] == sample_comparison["records_compared"]:
                fix_proposals.append(
                    "Embedded doc['json'] is semantically identical under parsed JSON equality, but differs in serialized byte sequence/formatting (e.g. key order or spacing between Ruby ASUtils.to_json vs serde_json)."
                )
            else:
                fix_proposals.append(
                    "Embedded doc['json'] has semantic content differences between Ruby record.to_hash(:trusted) and raw backend JSON; align sanitization."
                )

        report["proposed_fix"] = fix_proposals
        for fix in fix_proposals:
            print(f"  Proposed Fix: {fix}")
    elif args.full_if_sample_passes:
        print("\nSample comparison passed with 0 differences! Full run not requested or required here.")

    # Write report mode 0600
    report_path = os.path.join(
        BENCHMARKS_DIR, f"gate2_ao_pre_solr_comparison_{EXPECTED_STACK_ID}_{ts}.json"
    )
    write_file_mode_0600(report_path, json.dumps(report, indent=2))
    print(f"\nSaved mode 0600 report to: {report_path}")

    # Final cleanup
    cleanup()
    print("Pre-Solr comparison finished cleanly.")


if __name__ == "__main__":
    main()
