#!/usr/bin/env python3
"""Gate 2 25-AO Cross-Stage Observation Probe (Builder vs Stored Solr Diagnostic)

Performs a reproducible, read-only cross-stage diagnostic comparison between:
  1) Raw Rust pre-Solr STAFF AO builder output (with private _archivalist_*
     capability markers stripped)
  2) Stored native AO documents in retained isolated Solr core
     stage_staff_20261025_224741_59214_8a08a5d2_native

IMPORTANT ARCHITECTURAL & METHODOLOGICAL LIMITATION:
This probe compares a pre-Solr document candidate (builder output) against a
post-indexing stored document from a Solr core. In ArchivesSpace, the Solr schema
can omit unstored fields (e.g., stored="false", copyField sources, analysis filters).
Fields observed as "extra_in_builder" (such as fullrecord, extents, subjects,
etc.) likely include unstored native fields that were part of the native IndexBatch
document but were not configured to be stored in Solr. Conversely, fields stored in
Solr reflect post-indexing schema transformations.

Consequently, this comparison is strictly a CROSS-STAGE OBSERVATIONAL DIAGNOSTIC
and CANNOT establish implementation parity or prove missing native construction.
True parity must be established either via a same-stage pre-Solr IndexBatch comparison
or by indexing candidate documents into a separate isolated candidate Solr core and
comparing stored documents stage-to-stage.

Safety guards:
  - Validates actual arks_enabled=false from the active isolated app config before proceeding.
  - Verifies binary provenance (sha256 and mtime freshness against source HEAD).
  - Atomically creates mode 0600 output files without world-readable window.
  - Redacts credentials, tokens, and payloads from stderr on builder failures.
  - Preserves preexisting files, cleans only own temp files.
"""

from __future__ import annotations

import argparse
import atexit
import hashlib
import json
import math
import os
import re
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

REPO_ROOT = "/home/user/Documents/Matthew's work/Archivesspace AI"
BENCHMARKS_DIR = os.path.join(REPO_ROOT, "rust-tools", "benchmarks")
DEFAULT_STATE_FILE = os.path.join(BENCHMARKS_DIR, "profiling_stack_e62875f1.json")
DEFAULT_EXPERIMENT_FILE = os.path.join(
    BENCHMARKS_DIR, "gate2_ao_normalization_experiment_e62875f1_20261026_085620.json"
)
DEFAULT_CORE = "stage_staff_20261025_224741_59214_8a08a5d2_native"
DEFAULT_REPO_ID = 4
EXPECTED_STACK_ID = "e62875f1"
EXPECTED_BRANCH = "codex/native-profile-analysis"
EXPECTED_AO_COUNT = 24200
SAMPLE_SIZE = 25

SOLR_CONTAINER = f"archivalist-profiling-solr-{EXPECTED_STACK_ID}"
BACKEND_CONTAINER = f"archivalist-profiling-backend-{EXPECTED_STACK_ID}"
DB_CONTAINER = f"archivalist-profiling-db-{EXPECTED_STACK_ID}"

# Explicitly documented Solr-added storage fields to normalize
SOLR_ADDED_STORAGE_FIELDS = {"_version_"}

started_containers: List[str] = []


def cleanup():
    global started_containers
    if not started_containers:
        return
    print(f"\n[CLEANUP] Stopping containers started during this run: {started_containers}...", flush=True)
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
    print(f"\n[SIGNAL] Caught signal {signum}, cleaning up...", file=sys.stderr)
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


def strip_capability_markers(doc: Dict[str, Any]) -> Dict[str, Any]:
    """Remove private _archivalist_* capability markers from builder document."""
    return {k: v for k, v in doc.items() if not k.startswith("_archivalist_")}


def normalize_solr_doc(doc: Dict[str, Any]) -> Dict[str, Any]:
    """Normalize ONLY explicitly documented Solr-added fields, preserving all other fields."""
    return {k: v for k, v in doc.items() if k not in SOLR_ADDED_STORAGE_FIELDS}


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


def collect_cross_stage_observations(
    builder_val: Any,
    native_val: Any,
    path: str = "",
) -> List[Dict[str, Any]]:
    """Recursively collects field-level differences between raw pre-Solr builder doc and stored native Solr doc.

    Redacts all payload data: reports only path, category, type, sha256 hash, and size.
    Classification explicitly notes cross-stage nature:
      - cross_stage_extra_in_builder: builder doc has field absent from stored Solr doc (often unstored in Solr schema)
      - cross_stage_missing_in_builder: stored Solr doc has field absent from builder doc
      - cross_stage_value_mismatch: differing values across stages
      - cross_stage_type_mismatch: differing types across stages
      - cross_stage_array_length_mismatch: differing array length
      - cross_stage_array_order_change: identical set of elements in differing sequence
    """
    diffs: List[Dict[str, Any]] = []

    if type(builder_val) is not type(native_val):
        h_b, s_b = value_sha256_and_size(builder_val)
        h_n, s_n = value_sha256_and_size(native_val)
        diffs.append({
            "path": path or "<root>",
            "category": "cross_stage_type_mismatch",
            "builder_type": type(builder_val).__name__,
            "native_type": type(native_val).__name__,
            "builder_hash": h_b,
            "native_hash": h_n,
            "builder_size": s_b,
            "native_size": s_n,
        })
        return diffs

    if isinstance(builder_val, dict):
        b_keys = set(builder_val.keys())
        n_keys = set(native_val.keys())

        # Missing in Rust builder (present in stored native Solr)
        for k in sorted(n_keys - b_keys):
            subpath = f"{path}.{k}" if path else k
            h_n, s_n = value_sha256_and_size(native_val[k])
            diffs.append({
                "path": subpath,
                "category": "cross_stage_missing_in_builder",
                "field": k,
                "native_type": type(native_val[k]).__name__ if native_val[k] is not None else "null",
                "native_hash": h_n,
                "native_size": s_n,
            })

        # Extra in Rust builder (absent in stored native Solr - likely unstored native field)
        for k in sorted(b_keys - n_keys):
            subpath = f"{path}.{k}" if path else k
            h_b, s_b = value_sha256_and_size(builder_val[k])
            diffs.append({
                "path": subpath,
                "category": "cross_stage_extra_in_builder",
                "field": k,
                "builder_type": type(builder_val[k]).__name__ if builder_val[k] is not None else "null",
                "builder_hash": h_b,
                "builder_size": s_b,
            })

        # Common keys
        for k in sorted(b_keys & n_keys):
            subpath = f"{path}.{k}" if path else k
            diffs.extend(collect_cross_stage_observations(builder_val[k], native_val[k], subpath))

    elif isinstance(builder_val, list):
        if len(builder_val) != len(native_val):
            diffs.append({
                "path": path or "<root>",
                "category": "cross_stage_array_length_mismatch",
                "builder_len": len(builder_val),
                "native_len": len(native_val),
            })
        else:
            if is_array_reordered(builder_val, native_val):
                diffs.append({
                    "path": path or "<root>",
                    "category": "cross_stage_array_order_change",
                    "builder_len": len(builder_val),
                    "native_len": len(native_val),
                })
            for idx, (elem_b, elem_n) in enumerate(zip(builder_val, native_val)):
                subpath = f"{path}[{idx}]"
                diffs.extend(collect_cross_stage_observations(elem_b, elem_n, subpath))
    else:
        if builder_val != native_val:
            h_b, s_b = value_sha256_and_size(builder_val)
            h_n, s_n = value_sha256_and_size(native_val)
            diffs.append({
                "path": path or "<root>",
                "category": "cross_stage_value_mismatch",
                "builder_type": type(builder_val).__name__,
                "native_type": type(native_val).__name__,
                "builder_hash": h_b,
                "native_hash": h_n,
                "builder_size": s_b,
                "native_size": s_n,
            })

    return diffs


# Backward-compatible alias for tests
collect_parity_gaps = collect_cross_stage_observations


def normalize_path(p: str) -> str:
    return re.sub(r"\[\d+\]", "[]", p)


def load_selected_ids(experiment_file: str) -> List[int]:
    with open(experiment_file) as f:
        data = json.load(f)
    ids = data.get("selected_ids", [])
    if len(ids) != SAMPLE_SIZE:
        raise RuntimeError(f"Expected {SAMPLE_SIZE} selected IDs in {experiment_file}, got {len(ids)}")
    return ids


def verify_binary_provenance(cli_bin: str) -> Dict[str, Any]:
    """Verifies binary provenance against source tree.

    Checks binary existence, computes sha256, and ensures mtime is newer than or equal
    to any Rust source file in rust-tools/crates. Fails closed if stale.
    """
    if not os.path.isfile(cli_bin):
        raise RuntimeError(f"Binary not found: {cli_bin}. Run cargo build --release first.")

    bin_st = os.stat(cli_bin)
    bin_mtime = bin_st.st_mtime

    h = hashlib.sha256()
    with open(cli_bin, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    bin_sha256 = h.hexdigest()

    crates_dir = os.path.join(REPO_ROOT, "rust-tools", "crates")
    newest_src_mtime = 0.0
    newest_src_file = ""
    for root, _, files in os.walk(crates_dir):
        for fname in files:
            if fname.endswith(".rs") or fname == "Cargo.toml":
                fpath = os.path.join(root, fname)
                fmtime = os.path.getmtime(fpath)
                if fmtime > newest_src_mtime:
                    newest_src_mtime = fmtime
                    newest_src_file = fpath

    if bin_mtime < newest_src_mtime:
        raise RuntimeError(
            f"Binary {cli_bin} is stale (binary mtime {bin_mtime:.2f} < source mtime {newest_src_mtime:.2f} for {newest_src_file}). "
            "Rebuild release binary from current HEAD before running probe."
        )

    return {
        "path": cli_bin,
        "sha256": bin_sha256,
        "mtime": bin_mtime,
        "newest_source_file": newest_src_file,
        "newest_source_mtime": newest_src_mtime,
    }


def verify_active_app_arks_enabled(
    backend_url: str,
    admin_user: str,
    admin_password: str,
    container_name: Optional[str] = None,
) -> Tuple[str, Optional[bool]]:
    """Verifies arks_enabled configuration for the ArchivesSpace backend.

    Attempts genuine active process verification by querying the authenticated
    /system/info endpoint of the live running backend, which returns AppConfig
    directly from JVM memory (via ASUtils.get_diagnostics).

    Returns:
        (verification_status, value)
        where verification_status is one of:
          - "genuine_active_process": verified directly from live backend /system/info
          - "best_effort_config_file": evaluated from config files/env outside the live process
          - "unverified": unable to inspect or verify

    Safety note:
        'best_effort_config_file' does NOT guarantee live process state.
        The caller must fail closed if genuine active verification is required.
    """
    # 1. Genuine active process verification via live authenticated /system/info
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
        # Genuine active process query failed or endpoint unavailable
        pass

    # 2. Best-effort config inspection with local custom config loading and env overrides
    if container_name:
        try:
            ruby_code = (
                "require 'config/config-distribution'; "
                "AppConfig.load_user_config if AppConfig.respond_to?(:load_user_config); "
                "puts AppConfig[:arks_enabled]"
            )
            cmd = [
                "docker", "exec", container_name, "bash", "-c",
                (
                    "JAR=$(ls /archivesspace/build/jruby-complete-*.jar 2>/dev/null | head -1); "
                    "CP=\"$JAR:/archivesspace/common\"; "
                    "export GEM_HOME=/archivesspace/build/gems; "
                    f"java -cp \"$CP\" org.jruby.Main -I/archivesspace/common -e \"{ruby_code}\""
                )
            ]
            res = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
            if res.returncode == 0:
                val = res.stdout.strip().lower()
                if val == "true":
                    return ("best_effort_config_file", True)
                elif val == "false":
                    return ("best_effort_config_file", False)
        except Exception:
            pass

    return ("unverified", None)


def handle_builder_failure(
    returncode: int,
    raw_stderr: str,
    log_dir: Optional[str] = None,
    write_debug_log: bool = False,
) -> Optional[str]:
    """Handles builder process failure safely without emitting raw stderr to stdout or stderr.

    Emits only a generic failure message to stderr to prevent inline credentials or
    record contents from leaking.
    Avoids creating durable raw error files by default.
    Only writes a 0600 ignored log file if write_debug_log is explicitly requested.
    """
    generic_msg = (
        f"ERROR: Rust STAFF AO builder failed with exit code {returncode}. "
        "Process output suppressed to prevent leaking credentials or record contents."
    )
    print(generic_msg, file=sys.stderr)

    if write_debug_log and raw_stderr:
        log_dir = log_dir or BENCHMARKS_DIR
        ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
        err_path = os.path.join(log_dir, f".probe_builder_error_{ts}.log")
        try:
            fd = os.open(err_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(fd, "w") as f:
                f.write(
                    "# WARNING: This file contains raw stderr from the builder process and may contain credentials or record contents.\n"
                )
                f.write(raw_stderr)
            print(
                f"WARNING: Builder error details logged with mode 0600 to {err_path} (may contain secrets).",
                file=sys.stderr,
            )
            return err_path
        except Exception as e:
            print(f"Failed to write error log file: {e}", file=sys.stderr)
    return None


def sanitize_proc_output(raw_stderr: str, password_to_redact: str = "") -> str:
    """Sanitizes process stderr to prevent secret or raw payload leakage.

    Redacts explicit passwords, tokens, and any lines with JSON-like delimiters
    or potential inline record payloads.
    """
    lines = []
    for line in raw_stderr.splitlines():
        if password_to_redact:
            line = line.replace(password_to_redact, "[REDACTED_PASSWORD]")
        clean_stripped = line.strip()
        if "{" in clean_stripped or "}" in clean_stripped or \
           (clean_stripped.startswith("[") and clean_stripped.endswith("]")):
            line = "[PAYLOAD REDACTED]"
        lines.append(line)
    return "\n".join(lines)


def run_preflight_checks(state_file: str, cli_bin: str):
    print("=== Step 1: Pre-flight & Safety Verification ===", flush=True)

    # Git checks
    git_head = (
        subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )
    git_branch = (
        subprocess.run(
            ["git", "rev-parse", "--abbrev-ref", "HEAD"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )
    git_dirty = (
        subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )
    tracked_dirty = [l for l in git_dirty.splitlines() if not l.startswith("??")]

    print(f"Git branch: {git_branch}")
    print(f"Git commit: {git_head}")
    print(f"Tracked dirty files: {len(tracked_dirty)}")

    if git_branch != EXPECTED_BRANCH:
        raise RuntimeError(f"Expected branch {EXPECTED_BRANCH}, got {git_branch}")
    if tracked_dirty:
        raise RuntimeError(f"Tracked files are dirty: {tracked_dirty}")

    # Verify binary provenance
    bin_provenance = verify_binary_provenance(cli_bin)
    print(f"Binary provenance verified: sha256={bin_provenance['sha256'][:16]}... mtime={bin_provenance['mtime']:.2f}")

    # Host RAM check
    mem_avail = get_mem_available_gib()
    print(f"Available RAM: {mem_avail:.2f} GiB (minimum required: 24.0 GiB)")
    if mem_avail < 24.0:
        raise RuntimeError(f"Insufficient RAM: {mem_avail:.2f} GiB < 24.0 GiB")

    # State file check
    if not os.path.isfile(state_file):
        raise RuntimeError(f"State file not found: {state_file}")
    with open(state_file) as f:
        state = json.load(f)
    if state["stack_id"] != EXPECTED_STACK_ID:
        raise RuntimeError(f"Stack ID mismatch: expected {EXPECTED_STACK_ID}, got {state['stack_id']}")

    # Labels verification
    for role in ("db", "solr", "backend"):
        cname = state["containers"][role]
        cinfo = docker_inspect(cname)
        if cinfo is None:
            raise RuntimeError(f"Container {cname} not found")
        lbl = cinfo["Config"].get("Labels", {})
        if (
            lbl.get("archivalist.profiling-stack") != EXPECTED_STACK_ID
            or lbl.get("archivalist.profiling-role") != role
        ):
            raise RuntimeError(f"Label mismatch on {cname}: {lbl}")

    for vrole in ("db", "build", "data"):
        vname = state["volumes"][vrole]
        vres = subprocess.run(
            ["docker", "volume", "inspect", vname],
            capture_output=True,
            text=True,
            check=True,
        )
        vinfo = json.loads(vres.stdout)[0]
        vlbl = vinfo.get("Labels", {})
        if vlbl.get("archivalist.profiling-stack") != EXPECTED_STACK_ID:
            raise RuntimeError(f"Label mismatch on volume {vname}: {vlbl}")

    # Ensure non-profiling Solrs are stopped
    for c_stopped in ["archivesspaceai-solr-1", "archivesspaceai-solr-bench"]:
        if is_running(c_stopped):
            raise RuntimeError(f"Container {c_stopped} is running but must remain stopped!")

    if not is_running(DB_CONTAINER):
        raise RuntimeError(f"DB container {DB_CONTAINER} is not running!")

    # Verify DB content
    pwd = state["db_root_password"]
    db_env = dict(os.environ, MYSQL_PWD=pwd)
    sql_check = (
        "SELECT repo_code, name FROM archivesspace.repository WHERE id=2; "
        "SELECT count(*) FROM archivesspace.archival_object WHERE repo_id=4;"
    )
    db_res = subprocess.run(
        [
            "docker",
            "exec",
            "-e",
            "MYSQL_PWD",
            DB_CONTAINER,
            "mysql",
            "-uroot",
            "-N",
            "-B",
            "-e",
            sql_check,
        ],
        capture_output=True,
        text=True,
        env=db_env,
        check=True,
    )
    db_lines = db_res.stdout.strip().splitlines()
    print(f"DB Identity: {db_lines[0]}")
    print(f"DB Repo 4 AO count: {db_lines[1]}")
    if int(db_lines[1]) != EXPECTED_AO_COUNT:
        raise RuntimeError(f"Repo 4 AO count mismatch: expected {EXPECTED_AO_COUNT}, got {db_lines[1]}")
    if f"ARCHPROF-{EXPECTED_STACK_ID}" not in db_lines[0]:
        raise RuntimeError(f"DB stack identity marker missing from repo 2: {db_lines[0]}")

    print("Pre-flight safety checks passed.", flush=True)
    return state, git_head, git_branch, bin_provenance


def ensure_stack_running(state: Dict[str, Any]):
    print("\n=== Step 2: Lifecycle Management (Solr & Backend) ===", flush=True)

    # Solr container
    if not is_running(SOLR_CONTAINER):
        print(f"Starting {SOLR_CONTAINER}...", flush=True)
        subprocess.run(["docker", "start", SOLR_CONTAINER], check=True)
        started_containers.append(SOLR_CONTAINER)
    else:
        print(f"{SOLR_CONTAINER} is already running.")

    solr_host_url = state["solr_host_url"]
    solr_ready = False
    for _ in range(60):
        try:
            req = urllib.request.urlopen(
                f"{solr_host_url}/admin/cores?action=STATUS&core={state['identity_core']}&wt=json",
                timeout=3,
            )
            if req.status == 200:
                body = json.loads(req.read().decode())
                if state["identity_core"] in body.get("status", {}):
                    solr_ready = True
                    break
        except Exception:
            time.sleep(1)
    if not solr_ready:
        raise RuntimeError("Profiling Solr failed to become ready within 60s")
    print("Profiling Solr is ready.", flush=True)

    # Backend container
    if not is_running(BACKEND_CONTAINER):
        print(f"Starting {BACKEND_CONTAINER}...", flush=True)
        subprocess.run(["docker", "start", BACKEND_CONTAINER], check=True)
        started_containers.append(BACKEND_CONTAINER)
    else:
        print(f"{BACKEND_CONTAINER} is already running.")

    backend_host_url = state["backend_host_url"]
    backend_ready = False
    print("Waiting for profiling backend to initialize...", flush=True)
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
        raise RuntimeError("Profiling backend failed to become ready within 240s")
    print("Profiling backend is ready.", flush=True)

    # Verify identity
    print("\n=== Step 3: Challenge-Response Stack Verification ===", flush=True)
    verify_cmd = [
        "python3",
        os.path.join(REPO_ROOT, "rust-tools", "scripts", "profiling_stack.py"),
        "verify",
        "--state",
        os.path.join(BENCHMARKS_DIR, f"profiling_stack_{EXPECTED_STACK_ID}.json"),
    ]
    verify_res = subprocess.run(verify_cmd, capture_output=True, text=True, check=True)
    print(verify_res.stdout.strip(), flush=True)

    # Verify active arks_enabled from backend
    print("\n=== Step 3b: Validating Active App Configuration ===", flush=True)
    verification_mode, active_arks = verify_active_app_arks_enabled(
        state["backend_host_url"],
        state["admin_user"],
        state["admin_password"],
        container_name=BACKEND_CONTAINER,
    )
    if verification_mode != "genuine_active_process":
        raise RuntimeError(
            f"Active app arks_enabled config is UNVERIFIED (status: '{verification_mode}'). "
            "Best-effort config file evaluation does not guarantee active backend process state. "
            "Stopping live probe before invoking builder: genuine active backend config could not be confirmed."
        )
    if active_arks is None:
        raise RuntimeError(
            "Failed to extract arks_enabled from live backend /system/info. "
            "Stopping live probe before invoking builder."
        )
    if active_arks is not False:
        raise RuntimeError(
            f"Live backend reported arks_enabled={active_arks}; expected false. "
            "Stopping live probe before invoking builder."
        )
    print(
        f"Verified active backend arks_enabled={active_arks} via genuine active process inspection ({verification_mode}).",
        flush=True,
    )


def main():
    parser = argparse.ArgumentParser(description="Gate 2 25-AO Cross-Stage Observation Probe")
    parser.add_argument("--state", default=DEFAULT_STATE_FILE, help="Path to profiling stack state file")
    parser.add_argument("--experiment-file", default=DEFAULT_EXPERIMENT_FILE, help="Experiment JSON file containing selected 25 IDs")
    parser.add_argument("--core", default=DEFAULT_CORE, help="Solr native core to compare against")
    parser.add_argument("--repo-id", type=int, default=DEFAULT_REPO_ID, help="Repository ID")
    parser.add_argument("--cli-bin", default=os.path.join(REPO_ROOT, "rust-tools", "target", "release", "aspace-cli"), help="Path to aspace-cli binary")
    args = parser.parse_args()

    # Pre-flight
    state, git_head, git_branch, bin_provenance = run_preflight_checks(args.state, args.cli_bin)

    # Load 25 IDs
    selected_ids = load_selected_ids(args.experiment_file)
    print(f"\nLoaded {len(selected_ids)} AO IDs from {args.experiment_file}")
    print(f"Selected IDs: {selected_ids}")

    # Ensure stack lifecycle & validate active app config
    ensure_stack_running(state)

    # Step 4: Run Rust STAFF AO Builder
    print("\n=== Step 4: Invoking Rust STAFF AO Builder ===", flush=True)
    ts = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
    builder_out_path = os.path.join(BENCHMARKS_DIR, f"probe_builder_docs_temp_{ts}.json")

    cmd = [
        args.cli_bin,
        "--url",
        state["backend_host_url"],
        "--user",
        state["admin_user"],
        "--password",
        state["admin_password"],
        "staff-ao-builder",
        "--repo",
        str(args.repo_id),
        "--ids",
        ",".join(str(i) for i in selected_ids),
        "--arks-enabled=false",
        "--strip-markers=true",
        "--output",
        builder_out_path,
    ]

    print("Executing Rust STAFF AO Builder via aspace-cli...")
    t0 = time.perf_counter()
    builder_proc = subprocess.run(cmd, capture_output=True, text=True)
    t_builder = time.perf_counter() - t0

    if builder_proc.returncode != 0:
        handle_builder_failure(builder_proc.returncode, builder_proc.stderr)
        raise RuntimeError(f"Rust STAFF AO builder exited {builder_proc.returncode}")

    print(f"Rust STAFF AO builder completed in {t_builder:.3f}s, exit code: {builder_proc.returncode}")

    # Verify temp file permissions are mode 0600
    st = os.stat(builder_out_path)
    file_mode = stat.S_IMODE(st.st_mode)
    if file_mode != 0o600:
        if os.path.exists(builder_out_path):
            os.remove(builder_out_path)
        raise RuntimeError(f"Builder output file created with mode {oct(file_mode)}, expected 0600!")

    try:
        with open(builder_out_path) as f:
            builder_docs_raw = json.load(f)
    finally:
        if os.path.exists(builder_out_path):
            os.remove(builder_out_path)

    if len(builder_docs_raw) != SAMPLE_SIZE:
        raise RuntimeError(f"Expected {SAMPLE_SIZE} builder docs, got {len(builder_docs_raw)}")

    # Index builder docs by URI and ensure stripped markers
    builder_docs_by_uri: Dict[str, Dict[str, Any]] = {}
    for doc in builder_docs_raw:
        uri = doc.get("uri")
        if not uri:
            raise RuntimeError("Builder doc missing uri")
        stripped = strip_capability_markers(doc)
        builder_docs_by_uri[uri] = stripped

    print(f"Prepared {len(builder_docs_by_uri)} Rust builder documents (all private markers stripped).")

    # Step 5: Fetch Stored Native Solr Documents
    print(f"\n=== Step 5: Fetching Stored Native Docs from Core {args.core} ===", flush=True)
    solr_host_url = state["solr_host_url"].rstrip("/")
    native_docs_by_uri: Dict[str, Dict[str, Any]] = {}

    for ao_id in selected_ids:
        expected_uri = f"/repositories/{args.repo_id}/archival_objects/{ao_id}"
        query_url = f"{solr_host_url}/{args.core}/select?q=id:\"{urllib.parse.quote(expected_uri)}\"&wt=json"
        with urllib.request.urlopen(query_url, timeout=10) as resp:
            solr_res = json.loads(resp.read().decode())
        docs = solr_res.get("response", {}).get("docs", [])
        if len(docs) != 1:
            raise RuntimeError(f"Expected 1 Solr doc for {expected_uri}, got {len(docs)}")
        native_doc = normalize_solr_doc(docs[0])
        native_docs_by_uri[expected_uri] = native_doc

    print(f"Fetched {len(native_docs_by_uri)} stored native documents from core {args.core}.")

    # Step 6: Cross-Stage Comparison
    print("\n=== Step 6: Performing Type-Strict Cross-Stage Comparison ===", flush=True)
    total_diffs = 0
    record_diffs: Dict[str, List[Dict[str, Any]]] = {}
    path_counts: Counter[str] = Counter()
    path_categories: Dict[str, str] = {}
    missing_in_builder_fields: Set[str] = set()
    extra_in_builder_fields: Set[str] = set()
    value_mismatch_paths: Set[str] = set()

    for uri in sorted(builder_docs_by_uri.keys()):
        b_doc = builder_docs_by_uri[uri]
        n_doc = native_docs_by_uri[uri]

        diffs = collect_cross_stage_observations(b_doc, n_doc)
        record_diffs[uri] = diffs
        total_diffs += len(diffs)

        seen_normalized_paths_for_record: Set[str] = set()
        for d in diffs:
            cat = d["category"]
            norm_p = normalize_path(d["path"])

            if norm_p not in seen_normalized_paths_for_record:
                path_counts[norm_p] += 1
                seen_normalized_paths_for_record.add(norm_p)
            path_categories[norm_p] = cat

            if cat == "cross_stage_missing_in_builder":
                missing_in_builder_fields.add(d.get("field", norm_p))
            elif cat == "cross_stage_extra_in_builder":
                extra_in_builder_fields.add(d.get("field", norm_p))
            elif cat == "cross_stage_value_mismatch":
                value_mismatch_paths.add(norm_p)

    top_20_paths = path_counts.most_common(20)

    # Step 7: Output Summary to mode-0600 file
    report_filename = f"gate2_ao_parity_gap_probe_e62875f1_{ts}.json"
    report_rel_path = os.path.join("rust-tools", "benchmarks", report_filename)
    report_abs_path = os.path.join(REPO_ROOT, report_rel_path)

    report_payload = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "diagnostic_classification": "cross_stage_observation",
        "diagnostic_limitations": (
            "Compares pre-Solr raw builder documents against post-indexing stored Solr documents. "
            "Fields extra in builder (e.g., fullrecord, extents, subjects) likely include unstored "
            "native fields rather than builder extra construction. Fields missing or transformed may "
            "reflect Solr schema analysis/copyField behavior. This diagnostic cannot establish "
            "implementation parity or infer missing native construction."
        ),
        "stack_id": EXPECTED_STACK_ID,
        "git_commit": git_head,
        "git_branch": git_branch,
        "binary_provenance": bin_provenance,
        "core": args.core,
        "repo_id": args.repo_id,
        "sample_size": SAMPLE_SIZE,
        "selected_ids": selected_ids,
        "builder_input_contract": {
            "record_type": "archival_object",
            "repo_scope": f"/repositories/{args.repo_id}",
            "enrich": True,
            "dynamic_enums_source": "GET /schemas",
            "arks_enabled_verified_from_app_config": False,
        },
        "solr_storage_normalized_fields": sorted(SOLR_ADDED_STORAGE_FIELDS),
        "summary": {
            "records_compared": SAMPLE_SIZE,
            "total_cross_stage_observations": total_diffs,
            "distinct_differing_normalized_paths": len(path_counts),
            "solr_stored_fields_not_in_builder_count": len(missing_in_builder_fields),
            "builder_extra_fields_possibly_unstored_in_solr_count": len(extra_in_builder_fields),
            "solr_stored_fields_not_in_builder": sorted(missing_in_builder_fields),
            "builder_extra_fields_possibly_unstored_in_solr": sorted(extra_in_builder_fields),
            "top_20_differing_paths": [
                {
                    "path": p,
                    "count": cnt,
                    "category": path_categories.get(p, "unknown"),
                }
                for p, cnt in top_20_paths
            ],
        },
        "per_record_observations": record_diffs,
    }

    # Write file mode 0600
    fd = os.open(report_abs_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(report_payload, f, indent=2)

    print("\n=== Cross-Stage Observation Diagnostic Results ===")
    print("NOTE: This diagnostic compares RAW pre-Solr builder documents against STORED native Solr documents.")
    print("      Solr schema can omit unstored native fields (e.g. fullrecord, extents, subjects).")
    print("      These observations reflect cross-stage differences, NOT confirmed implementation gaps.")
    print(f"Command: python3 {' '.join(sys.argv)}")
    print(f"Commit: {git_head} (branch {git_branch})")
    print(f"Binary sha256: {bin_provenance['sha256']}")
    print(f"Core: {args.core}")
    print(f"Selected IDs: {selected_ids}")
    print(f"Records compared: {SAMPLE_SIZE}")
    print(f"Total cross-stage differences observed: {total_diffs}")
    print(f"Distinct differing normalized paths: {len(path_counts)}")
    print(f"Stored Solr fields absent in Rust builder: {len(missing_in_builder_fields)}")
    print(f"Builder fields absent in stored Solr (likely unstored in Solr schema): {len(extra_in_builder_fields)}")

    print("\nTop 20 Differing Paths (by record count):")
    for idx, (p, cnt) in enumerate(top_20_paths, start=1):
        cat = path_categories.get(p, "unknown")
        print(f"  {idx:2d}. {p:<45} count={cnt:2d}/{SAMPLE_SIZE}  category={cat}")

    print(f"\nMode-0600 summary report saved to: {report_rel_path}")
    print("Verification: Exit status 0; all private markers and Solr _version_ normalized.")
    print("Independent review: not yet performed.")


if __name__ == "__main__":
    main()
