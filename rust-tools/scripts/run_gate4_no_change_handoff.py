#!/usr/bin/env python3
"""Gate 4 bounded one-shot no-change Ruby STAFF incremental handoff runner.

Executes a single bounded, reversible no-change Ruby STAFF incremental handoff
round on isolated profiling stack 844ef09c using the watermarked million-AO
candidate core stage_staff_20261124_055901_3358917_575e2fa7_candidate.

Safety & Scope Invariants:
- Does NOT call `profiling_stack.py verify` (which would mutate repository 2's
  database nonce and invalidate the candidate's identity record). Instead, proves
  stack identity strictly read-only via container labels/statuses, state file
  URLs, GET /repositories/2 (repo_code ARCHPROF-844ef09c and nonce
  2fc7025e3711177ecb588ef328846dec), and Solr identity core status.
- Refuses unless the saved report names stack 844ef09c, candidate core
  stage_staff_20261124_055901_3358917_575e2fa7_candidate, committed=true,
  positive integer watermark 1795499941, 1,036,427 candidate docs, zero
  missing/extra/markers, exact_parity=false, and the only two differences are
  the documented cross-run verifier-nonce embedded JSON strings (/repositories/2
  and /agents/corporate_entities/1), which are also live-verified read-only in
  Solr. Never rewrites or normalizes the saved parity report.
- Preflights all 19 STAFF feeds across repositories [2, 3, 4], repository
  discovery, and /delete-feed against modified_since=1795499911 (watermark
  minus 30s overlap) read-only before any CoreAdmin SWAP.
- Performs CoreAdmin SWAP strictly inside Solr stack 844ef09c between empty
  `archivesspace` and the watermarked candidate core, with automatic rollback in
  `finally`. If post-swap state is ever ambiguous, stops and reports without
  issuing a blind second swap.
- Writes checkpoints to a unique File state directory
  `indexer_state_gate4_handoff_20261124_<nonce>` inside
  `/archivalist-profiling-data`, leaving `indexer_state` and
  `indexer_pui_state` untouched, and runs exactly one bounded
  `PeriodicIndexer#run_index_round` with a zero-mutation guard.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import secrets
import subprocess
import sys
import time
from typing import Any, Callable, Dict, List, Mapping, Optional, Tuple
import urllib.parse
import urllib.request

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", ".."))
BENCHMARKS_DIR = os.path.join(REPO_ROOT, "rust-tools", "benchmarks")

EXPECTED_STACK_ID = "844ef09c"
EXPECTED_BRANCH = "codex/million-snapshot-seed"
EXPECTED_CANDIDATE_CORE = "stage_staff_20261124_055901_3358917_575e2fa7_candidate"
EXPECTED_NATIVE_CORE = "stage_staff_20261124_010605_9807_36038e70_native"
EXPECTED_PREVIOUS_CANDIDATE_CORE = "stage_staff_20261123_224555_3217466_09a8939e_candidate"
EXPECTED_EMPTY_CANDIDATE_CORE = "stage_staff_20261124_054050_3352672_b76bd497_candidate"
DEFAULT_CORE = "archivesspace"
EXPECTED_IDENTITY_CORE = "archivalist_profiling_identity_844ef09c"

EXPECTED_WATERMARK_EPOCH = 1795499941
WINDOW_SECONDS = 30
EXPECTED_MODIFIED_SINCE = EXPECTED_WATERMARK_EPOCH - WINDOW_SECONDS  # 1795499911
EXPECTED_TOTAL_DOCS = 1036427
EXPECTED_EXACT_RECORDS = 1036425
EXPECTED_REPO_IDS = [2, 3, 4]
EXPECTED_BINARY_SHA256 = "1376f3ca43404f7e9d27a2fc397f47b89479614e6b97dabc6ea945898178f8df"

EXPECTED_IDENTITY_REPO_ID = 2
EXPECTED_IDENTITY_REPO_CODE = "ARCHPROF-844ef09c"
EXPECTED_CANDIDATE_NONCE = "2fc7025e3711177ecb588ef328846dec"
EXPECTED_NATIVE_NONCE = "2e3448f45326ace938c4451015a03339"
IDENTITY_NAME_PREFIX = "archivalist profiling identity "
EXPECTED_NONCE_DIFF_URIS = ["/agents/corporate_entities/1", "/repositories/2"]

DEFAULT_STATE_PATH = os.path.join(BENCHMARKS_DIR, "profiling_stack_844ef09c.json")
DEFAULT_REPORT_PATH = os.path.join(
    BENCHMARKS_DIR, "gate3_full_parity_20261124_055901_3358917_575e2fa7.json"
)
PRIOR_BLOCKER_REPORT_PATH = os.path.join(
    BENCHMARKS_DIR, "gate4_handoff_preflight_blocker_20261124_053600_844ef09c.json"
)

LABEL_STACK = "archivalist.profiling-stack"
LABEL_ROLE = "archivalist.profiling-role"


class Gate4HandoffError(RuntimeError):
    """Fail-closed error during Gate 4 handoff verification or execution."""


def redact_secrets(text: Any) -> str:
    raw = str(text)
    raw = re.sub(r"(password=)[^&\s\"']+", r"\1<REDACTED>", raw, flags=re.IGNORECASE)
    raw = re.sub(r"(MYSQL_PWD=)[^\s\"']+", r"\1<REDACTED>", raw, flags=re.IGNORECASE)
    raw = re.sub(r"(\"db_root_password\"\s*:\s*\")[^\"]+(\")", r"\1<REDACTED>\2", raw)
    raw = re.sub(r"(\"admin_password\"\s*:\s*\")[^\"]+(\")", r"\1<REDACTED>\2", raw)
    return raw


def strict_bool(value: Any) -> bool:
    return type(value) is bool


def strict_positive_int(value: Any) -> bool:
    return type(value) is int and value > 0


def default_http_get_json(url: str, timeout: int = 15) -> Dict[str, Any]:
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        if resp.status != 200:
            raise Gate4HandoffError(f"GET {url} returned HTTP {resp.status}")
        return json.loads(resp.read().decode("utf-8"))


def validate_candidate_report(report: Mapping[str, Any], report_path: str = "") -> Dict[str, Any]:
    if not isinstance(report, Mapping):
        raise Gate4HandoffError("Candidate report is not a JSON object")

    if report.get("stack_id") != EXPECTED_STACK_ID:
        raise Gate4HandoffError(
            f"Report stack_id={report.get('stack_id')!r} != expected {EXPECTED_STACK_ID!r}"
        )
    if report.get("candidate_core") != EXPECTED_CANDIDATE_CORE:
        raise Gate4HandoffError(
            f"Report candidate_core={report.get('candidate_core')!r} != expected {EXPECTED_CANDIDATE_CORE!r}"
        )
    if report.get("native_core") != EXPECTED_NATIVE_CORE:
        raise Gate4HandoffError(
            f"Report native_core={report.get('native_core')!r} != expected {EXPECTED_NATIVE_CORE!r}"
        )

    binary_prov = report.get("binary_provenance")
    if not isinstance(binary_prov, Mapping) or binary_prov.get("sha256") != EXPECTED_BINARY_SHA256:
        raise Gate4HandoffError(
            f"Report binary_provenance.sha256={(binary_prov or {}).get('sha256')!r} "
            f"!= expected {EXPECTED_BINARY_SHA256!r}"
        )

    cli_summary = report.get("cli_summary")
    if not isinstance(cli_summary, Mapping):
        raise Gate4HandoffError("Report is missing cli_summary object")

    committed = cli_summary.get("committed")
    if not strict_bool(committed) or committed is not True:
        raise Gate4HandoffError(f"Report cli_summary.committed must be true (got {committed!r})")

    if cli_summary.get("candidate_core") != EXPECTED_CANDIDATE_CORE:
        raise Gate4HandoffError(
            f"Report cli_summary.candidate_core={cli_summary.get('candidate_core')!r} "
            f"!= expected {EXPECTED_CANDIDATE_CORE!r}"
        )

    epoch = cli_summary.get("full_reindex_started_at_epoch_sec")
    if not strict_positive_int(epoch) or epoch != EXPECTED_WATERMARK_EPOCH:
        raise Gate4HandoffError(
            "Report cli_summary.full_reindex_started_at_epoch_sec must be integer "
            f"{EXPECTED_WATERMARK_EPOCH} (got {epoch!r})"
        )

    repo_ids = cli_summary.get("repository_ids")
    if repo_ids != EXPECTED_REPO_IDS:
        raise Gate4HandoffError(
            f"Report cli_summary.repository_ids={repo_ids!r} != expected {EXPECTED_REPO_IDS!r}"
        )

    for key in ("total_ids", "total_documents", "unique_uris"):
        if cli_summary.get(key) != EXPECTED_TOTAL_DOCS:
            raise Gate4HandoffError(
                f"Report cli_summary.{key}={cli_summary.get(key)!r} != expected {EXPECTED_TOTAL_DOCS}"
            )

    if report.get("solr_verified_docs") != EXPECTED_TOTAL_DOCS:
        raise Gate4HandoffError(
            f"Report solr_verified_docs={report.get('solr_verified_docs')!r} != expected {EXPECTED_TOTAL_DOCS}"
        )

    parity = report.get("parity_result")
    if not isinstance(parity, Mapping):
        raise Gate4HandoffError("Report is missing parity_result object")

    exact_parity = parity.get("exact_parity")
    if not strict_bool(exact_parity) or exact_parity is not False:
        raise Gate4HandoffError(
            "Report parity_result.exact_parity must be strictly false for the cross-run "
            f"verifier-nonce report (got {exact_parity!r})"
        )

    for key in ("records_compared", "candidate_total_docs", "native_total_docs"):
        if parity.get(key) != EXPECTED_TOTAL_DOCS:
            raise Gate4HandoffError(
                f"Report parity_result.{key}={parity.get(key)!r} != expected {EXPECTED_TOTAL_DOCS}"
            )

    for zero_key in (
        "missing_in_candidate_count",
        "extra_in_candidate_count",
        "candidate_marker_leaks_count",
    ):
        if parity.get(zero_key) != 0:
            raise Gate4HandoffError(
                f"Report parity_result.{zero_key}={parity.get(zero_key)!r} != 0"
            )

    if parity.get("records_with_exact_equality") != EXPECTED_EXACT_RECORDS:
        raise Gate4HandoffError(
            f"Report parity_result.records_with_exact_equality={parity.get('records_with_exact_equality')!r} "
            f"!= expected {EXPECTED_EXACT_RECORDS}"
        )
    if parity.get("records_with_differences") != 2 or parity.get("total_differences") != 2:
        raise Gate4HandoffError(
            "Report parity_result must have exactly 2 differing records and 2 total differences "
            f"(got records_with_differences={parity.get('records_with_differences')!r}, "
            f"total_differences={parity.get('total_differences')!r})"
        )
    if parity.get("embedded_json_mismatches_count") != 2:
        raise Gate4HandoffError(
            f"Report parity_result.embedded_json_mismatches_count={parity.get('embedded_json_mismatches_count')!r} != 2"
        )
    if sorted(parity.get("embedded_json_mismatches_sample") or []) != EXPECTED_NONCE_DIFF_URIS:
        raise Gate4HandoffError(
            "Report parity_result.embedded_json_mismatches_sample="
            f"{parity.get('embedded_json_mismatches_sample')!r} != {EXPECTED_NONCE_DIFF_URIS!r}"
        )

    sample_diffs = parity.get("sample_per_record_diffs")
    if not isinstance(sample_diffs, Mapping) or sorted(sample_diffs.keys()) != EXPECTED_NONCE_DIFF_URIS:
        raise Gate4HandoffError(
            "Report parity_result.sample_per_record_diffs keys must be strictly "
            f"{EXPECTED_NONCE_DIFF_URIS!r} (got {list(sample_diffs.keys()) if isinstance(sample_diffs, Mapping) else sample_diffs!r})"
        )

    for uri in EXPECTED_NONCE_DIFF_URIS:
        entries = sample_diffs.get(uri)
        if not isinstance(entries, list) or len(entries) != 1:
            raise Gate4HandoffError(f"Expected exactly 1 diff entry for {uri}, got {entries!r}")
        entry = entries[0]
        if (
            not isinstance(entry, Mapping)
            or entry.get("path") != "json"
            or entry.get("category") != "stored_value_mismatch"
        ):
            raise Gate4HandoffError(f"Unexpected diff entry for {uri}: {entry!r}")

    return {
        "report_path": report_path,
        "stack_id": report["stack_id"],
        "candidate_core": report["candidate_core"],
        "native_core": report["native_core"],
        "binary_sha256": binary_prov["sha256"],
        "committed": True,
        "full_reindex_started_at_epoch_sec": epoch,
        "modified_since_with_overlap": epoch - WINDOW_SECONDS,
        "repository_ids": list(repo_ids),
        "candidate_total_docs": EXPECTED_TOTAL_DOCS,
        "native_total_docs": EXPECTED_TOTAL_DOCS,
        "records_compared": EXPECTED_TOTAL_DOCS,
        "records_with_exact_equality": EXPECTED_EXACT_RECORDS,
        "records_with_differences": 2,
        "total_differences": 2,
        "missing_in_candidate_count": 0,
        "extra_in_candidate_count": 0,
        "candidate_marker_leaks_count": 0,
        "exact_parity": False,
        "parity_limitation_note": (
            "exact_parity=false against fixed prior-run native reference core "
            f"{EXPECTED_NATIVE_CORE}; 1,036,425 of 1,036,427 documents are type-strictly "
            "identical, and the only 2 differing documents (/repositories/2 and "
            "/agents/corporate_entities/1) differ solely in the synthetic profiling-stack "
            f"challenge nonce ({EXPECTED_CANDIDATE_NONCE} in candidate/current API vs "
            f"{EXPECTED_NATIVE_NONCE} in the older native reference core)."
        ),
        "candidate_peak_rss_kib": (report.get("memory") or {}).get("candidate_peak_rss_kib"),
        "candidate_cli_wall_sec": report.get("candidate_cli_wall_sec"),
    }


def verify_live_nonce_only_solr_diffs(
    solr_host_url: str,
    http_get_json: Callable[[str], Dict[str, Any]] = default_http_get_json,
) -> Dict[str, Any]:
    clean_base = solr_host_url.rstrip("/")
    records_checked: Dict[str, Any] = {}
    for doc_id in EXPECTED_NONCE_DIFF_URIS:
        q = urllib.parse.urlencode({"q": f'id:"{doc_id}"', "wt": "json"})
        cand_resp = http_get_json(f"{clean_base}/{EXPECTED_CANDIDATE_CORE}/select?{q}")
        nat_resp = http_get_json(f"{clean_base}/{EXPECTED_NATIVE_CORE}/select?{q}")
        cand_docs = (cand_resp.get("response") or {}).get("docs") or []
        nat_docs = (nat_resp.get("response") or {}).get("docs") or []
        if len(cand_docs) != 1 or len(nat_docs) != 1:
            raise Gate4HandoffError(
                f"Expected 1 Solr doc each for {doc_id} in candidate/native cores "
                f"(got candidate={len(cand_docs)}, native={len(nat_docs)})"
            )
        cand_doc = dict(cand_docs[0])
        nat_doc = dict(nat_docs[0])
        cand_doc.pop("_version_", None)
        nat_doc.pop("_version_", None)

        cand_json = cand_doc.get("json")
        nat_json = nat_doc.get("json")
        if not isinstance(cand_json, str) or not isinstance(nat_json, str):
            raise Gate4HandoffError(f"Missing stored json string for {doc_id}")
        if EXPECTED_CANDIDATE_NONCE not in cand_json:
            raise Gate4HandoffError(
                f"Candidate stored json for {doc_id} does not contain expected nonce {EXPECTED_CANDIDATE_NONCE}"
            )
        if EXPECTED_NATIVE_NONCE not in nat_json:
            raise Gate4HandoffError(
                f"Native stored json for {doc_id} does not contain expected nonce {EXPECTED_NATIVE_NONCE}"
            )

        normalized_cand_json = cand_json.replace(EXPECTED_CANDIDATE_NONCE, EXPECTED_NATIVE_NONCE)
        if normalized_cand_json != nat_json:
            raise Gate4HandoffError(
                f"Stored json for {doc_id} differs by more than the challenge nonce"
            )

        cand_without_json = {k: v for k, v in cand_doc.items() if k != "json"}
        nat_without_json = {k: v for k, v in nat_doc.items() if k != "json"}
        if cand_without_json != nat_without_json:
            raise Gate4HandoffError(
                f"Non-json stored fields for {doc_id} differ between candidate and native"
            )

        records_checked[doc_id] = {
            "candidate_json_sha256": hashlib.sha256(cand_json.encode("utf-8")).hexdigest(),
            "native_json_sha256": hashlib.sha256(nat_json.encode("utf-8")).hexdigest(),
            "candidate_nonce": EXPECTED_CANDIDATE_NONCE,
            "native_nonce": EXPECTED_NATIVE_NONCE,
            "identical_after_nonce_substitution": True,
            "all_non_json_stored_fields_exact": True,
        }

    return {
        "verified": True,
        "records": records_checked,
    }


def verify_read_only_stack_identity(
    state: Mapping[str, Any],
    docker_inspect_fn: Callable[[str], Dict[str, Any]],
    http_get_json: Callable[[str], Dict[str, Any]] = default_http_get_json,
) -> Dict[str, Any]:
    """Proves stack 844ef09c identity strictly read-only without mutating repository 2."""
    if state.get("stack_id") != EXPECTED_STACK_ID:
        raise Gate4HandoffError(
            f"State stack_id={state.get('stack_id')!r} != expected {EXPECTED_STACK_ID!r}"
        )
    if state.get("backend_host_url") != "http://127.0.0.1:24567":
        raise Gate4HandoffError(f"Unexpected backend_host_url: {state.get('backend_host_url')!r}")
    if state.get("solr_host_url") != "http://127.0.0.1:28983/solr":
        raise Gate4HandoffError(f"Unexpected solr_host_url: {state.get('solr_host_url')!r}")
    if state.get("backend_network_url") != "http://archivalist-profiling-backend-844ef09c:4567":
        raise Gate4HandoffError(f"Unexpected backend_network_url: {state.get('backend_network_url')!r}")
    if state.get("solr_network_url") != "http://archivalist-profiling-solr-844ef09c:8983/solr":
        raise Gate4HandoffError(f"Unexpected solr_network_url: {state.get('solr_network_url')!r}")
    if state.get("identity_repo_id") != EXPECTED_IDENTITY_REPO_ID:
        raise Gate4HandoffError(f"Unexpected identity_repo_id: {state.get('identity_repo_id')!r}")
    if state.get("identity_repo_code") != EXPECTED_IDENTITY_REPO_CODE:
        raise Gate4HandoffError(f"Unexpected identity_repo_code: {state.get('identity_repo_code')!r}")
    if state.get("identity_core") != EXPECTED_IDENTITY_CORE:
        raise Gate4HandoffError(f"Unexpected identity_core: {state.get('identity_core')!r}")

    containers = state.get("containers") or {}
    expected_containers = {
        "db": f"archivalist-profiling-db-{EXPECTED_STACK_ID}",
        "solr": f"archivalist-profiling-solr-{EXPECTED_STACK_ID}",
        "backend": f"archivalist-profiling-backend-{EXPECTED_STACK_ID}",
    }
    if containers != expected_containers:
        raise Gate4HandoffError(
            f"State containers={containers!r} != expected {expected_containers!r}"
        )

    container_evidence: Dict[str, Any] = {}
    for role, expected_name in expected_containers.items():
        info = docker_inspect_fn(expected_name)
        labels = (info.get("Config") or {}).get("Labels") or {}
        running = bool((info.get("State") or {}).get("Running"))
        status_str = (info.get("State") or {}).get("Status")
        if not running or status_str != "running":
            raise Gate4HandoffError(
                f"Stack container {expected_name} is not running (status={status_str!r})"
            )
        if labels.get(LABEL_STACK) != EXPECTED_STACK_ID or labels.get(LABEL_ROLE) != role:
            raise Gate4HandoffError(
                f"Container {expected_name} has unexpected labels: {labels!r}"
            )
        container_evidence[expected_name] = {
            "role": role,
            "status": status_str,
            "running": running,
            "labels": {
                LABEL_STACK: labels.get(LABEL_STACK),
                LABEL_ROLE: labels.get(LABEL_ROLE),
            },
        }

    repo2_url = f"{state['backend_host_url'].rstrip('/')}/repositories/{EXPECTED_IDENTITY_REPO_ID}"
    repo2 = http_get_json(repo2_url)
    expected_name = IDENTITY_NAME_PREFIX + EXPECTED_CANDIDATE_NONCE
    if repo2.get("repo_code") != EXPECTED_IDENTITY_REPO_CODE or repo2.get("name") != expected_name:
        raise Gate4HandoffError(
            f"Read-only GET {repo2_url} returned repo_code={repo2.get('repo_code')!r}, "
            f"name={repo2.get('name')!r}; expected repo_code={EXPECTED_IDENTITY_REPO_CODE!r}, "
            f"name={expected_name!r}"
        )

    status_url = (
        f"{state['solr_host_url'].rstrip('/')}/admin/cores?"
        + urllib.parse.urlencode({"action": "STATUS", "core": EXPECTED_IDENTITY_CORE, "wt": "json"})
    )
    solr_status = http_get_json(status_url)
    id_core_info = (solr_status.get("status") or {}).get(EXPECTED_IDENTITY_CORE)
    if not id_core_info:
        raise Gate4HandoffError(
            f"Solr identity core {EXPECTED_IDENTITY_CORE} missing from {state['solr_host_url']}"
        )

    return {
        "verification_mode": "read_only_no_db_mutation",
        "profiling_stack_verify_called": False,
        "stack_id": EXPECTED_STACK_ID,
        "backend_host_url": state["backend_host_url"],
        "backend_network_url": state["backend_network_url"],
        "solr_host_url": state["solr_host_url"],
        "solr_network_url": state["solr_network_url"],
        "identity_repo_id": EXPECTED_IDENTITY_REPO_ID,
        "identity_repo_code": repo2.get("repo_code"),
        "identity_repo_name": repo2.get("name"),
        "identity_nonce_verified": EXPECTED_CANDIDATE_NONCE,
        "identity_repo_system_mtime": repo2.get("system_mtime"),
        "identity_core": EXPECTED_IDENTITY_CORE,
        "containers": container_evidence,
    }


def check_no_active_benchmark_or_indexer_processes(
    host_ps_output: str,
    backend_ps_output: str,
    app_ps_output: str,
    current_pid: Optional[int] = None,
    parent_pid: Optional[int] = None,
) -> Dict[str, Any]:
    my_pid = os.getpid() if current_pid is None else current_pid
    ppid = os.getppid() if parent_pid is None else parent_pid
    conflicts: List[str] = []

    for raw_line in host_ps_output.splitlines():
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
        if pid in (my_pid, ppid):
            continue
        comm, cmd = parts[1], parts[2]
        if comm in ("agy", "cat", "grep", "ps"):
            continue
        if "--print-timeout" in cmd or "cat >" in cmd:
            continue
        is_aspace_cli = comm == "aspace-cli" or (
            "aspace-cli" in cmd
            and ("staff-full-candidate" in cmd or "staff-combined-candidate" in cmd)
        )
        is_benchmark_runner = comm.startswith("python") and any(
            name in cmd
            for name in (
                "run_gate3_combined_candidate.py",
                "run_million_staff_benchmark.py",
                "run_million_staff_fixed_reference.py",
                "run_gate4_no_change_handoff.py",
                "run_gate4_mutation_during_pass_handoff.py",
            )
        )
        is_ruby_indexer = comm in ("java", "jruby", "ruby") and any(
            name in cmd
            for name in (
                "native_indexing_profile.rb",
                "PeriodicIndexer",
                "indexer/app/main.rb",
                "gate4_isolated_handoff_round.rb",
            )
        )
        if is_aspace_cli or is_benchmark_runner or is_ruby_indexer:
            conflicts.append(f"host pid={pid} ({comm}): {cmd}")

    for container_label, ps_text in (
        ("archivalist-profiling-backend-844ef09c", backend_ps_output),
        ("archivesspaceai-app-1", app_ps_output),
    ):
        for raw_line in ps_text.splitlines():
            line = raw_line.strip()
            if not line or line.startswith("USER"):
                continue
            if any(
                token in line
                for token in (
                    "indexer/app/main.rb",
                    "PeriodicIndexer",
                    "native_indexing_profile.rb",
                    "gate4_isolated_handoff_round.rb",
                    "-Daspace.service=indexer",
                )
            ):
                conflicts.append(f"{container_label}: {line}")

    if conflicts:
        raise Gate4HandoffError(
            "Refusing to run while another benchmark or indexer process is active: "
            + "; ".join(conflicts)
        )

    return {
        "active_conflicts": [],
        "periodic_indexer_daemon_running_in_backend": False,
        "periodic_indexer_daemon_running_in_app": False,
    }


def fetch_all_cores_snapshot(
    solr_host_url: str,
    http_get_json: Callable[[str], Dict[str, Any]] = default_http_get_json,
) -> Dict[str, Dict[str, Any]]:
    clean_base = solr_host_url.rstrip("/")
    status_resp = http_get_json(f"{clean_base}/admin/cores?action=STATUS&wt=json")
    if (status_resp.get("responseHeader") or {}).get("status") != 0:
        raise Gate4HandoffError(f"Solr STATUS failed: {status_resp!r}")

    status_map = status_resp.get("status") or {}
    required_cores = (
        DEFAULT_CORE,
        EXPECTED_IDENTITY_CORE,
        EXPECTED_CANDIDATE_CORE,
        EXPECTED_NATIVE_CORE,
        EXPECTED_PREVIOUS_CANDIDATE_CORE,
        EXPECTED_EMPTY_CANDIDATE_CORE,
    )
    for req_core in required_cores:
        if req_core not in status_map:
            raise Gate4HandoffError(f"Required Solr core {req_core!r} missing from STATUS response")

    snapshot: Dict[str, Dict[str, Any]] = {}
    for core_name in sorted(status_map.keys()):
        info = status_map[core_name] or {}
        idx = info.get("index") or {}
        select_resp = http_get_json(
            f"{clean_base}/{core_name}/select?q=*:*&rows=0&wt=json"
        )
        num_found = (select_resp.get("response") or {}).get("numFound")
        if type(num_found) is not int:
            raise Gate4HandoffError(f"Invalid numFound for core {core_name}: {num_found!r}")
        snapshot[core_name] = {
            "name": info.get("name"),
            "instanceDir": info.get("instanceDir"),
            "dataDir": info.get("dataDir"),
            "startTime": info.get("startTime"),
            "numFound": num_found,
            "numDocs": idx.get("numDocs"),
            "maxDoc": idx.get("maxDoc"),
            "deletedDocs": idx.get("deletedDocs"),
            "version": idx.get("version"),
            "segmentCount": idx.get("segmentCount"),
            "lastModified": idx.get("lastModified"),
            "commitTimeMSec": (idx.get("userData") or {}).get("commitTimeMSec"),
        }
    return snapshot


def verify_pre_swap_cores(snapshot: Mapping[str, Mapping[str, Any]]) -> None:
    expected_counts = {
        DEFAULT_CORE: 0,
        EXPECTED_IDENTITY_CORE: 0,
        EXPECTED_EMPTY_CANDIDATE_CORE: 0,
        EXPECTED_PREVIOUS_CANDIDATE_CORE: EXPECTED_TOTAL_DOCS,
        EXPECTED_NATIVE_CORE: EXPECTED_TOTAL_DOCS,
        EXPECTED_CANDIDATE_CORE: EXPECTED_TOTAL_DOCS,
    }
    for core_name, exp_count in expected_counts.items():
        core_info = snapshot.get(core_name)
        if not core_info:
            raise Gate4HandoffError(f"Core {core_name} missing from pre-swap snapshot")
        if core_info.get("numFound") != exp_count or core_info.get("numDocs") != exp_count:
            raise Gate4HandoffError(
                f"Pre-swap core {core_name} count mismatch: numFound={core_info.get('numFound')}, "
                f"numDocs={core_info.get('numDocs')}, expected={exp_count}"
            )
        expected_instance_dir = f"/var/solr/data/{core_name}"
        if core_info.get("instanceDir") != expected_instance_dir:
            raise Gate4HandoffError(
                f"Pre-swap core {core_name} has unexpected instanceDir={core_info.get('instanceDir')!r} "
                f"(expected {expected_instance_dir!r})"
            )


def verify_swapped_cores(
    pre_snapshot: Mapping[str, Mapping[str, Any]],
    post_snapshot: Mapping[str, Mapping[str, Any]],
) -> None:
    pre_default = pre_snapshot[DEFAULT_CORE]
    pre_cand = pre_snapshot[EXPECTED_CANDIDATE_CORE]
    post_default = post_snapshot[DEFAULT_CORE]
    post_cand = post_snapshot[EXPECTED_CANDIDATE_CORE]

    if (
        post_default.get("numFound") != EXPECTED_TOTAL_DOCS
        or post_default.get("instanceDir") != pre_cand.get("instanceDir")
        or post_default.get("dataDir") != pre_cand.get("dataDir")
        or post_default.get("version") != pre_cand.get("version")
    ):
        raise Gate4HandoffError(
            "Ambiguous post-SWAP state for 'archivesspace': "
            f"got {post_default!r}, expected instanceDir/dataDir/version/numFound from {pre_cand!r}"
        )

    if (
        post_cand.get("numFound") != 0
        or post_cand.get("instanceDir") != pre_default.get("instanceDir")
        or post_cand.get("dataDir") != pre_default.get("dataDir")
        or post_cand.get("version") != pre_default.get("version")
    ):
        raise Gate4HandoffError(
            f"Ambiguous post-SWAP state for '{EXPECTED_CANDIDATE_CORE}': "
            f"got {post_cand!r}, expected instanceDir/dataDir/version/numFound from {pre_default!r}"
        )


def verify_promoted_core_unchanged_after_round(
    swapped_snapshot: Mapping[str, Mapping[str, Any]],
    after_round_snapshot: Mapping[str, Mapping[str, Any]],
) -> None:
    before_def = swapped_snapshot[DEFAULT_CORE]
    after_def = after_round_snapshot[DEFAULT_CORE]
    for field in ("numFound", "numDocs", "maxDoc", "deletedDocs", "version", "instanceDir", "dataDir", "commitTimeMSec"):
        if before_def.get(field) != after_def.get(field):
            raise Gate4HandoffError(
                f"Promoted core '{DEFAULT_CORE}' changed during no-change round on field {field}: "
                f"before={before_def.get(field)!r}, after={after_def.get(field)!r}"
            )


def verify_restored_cores(
    pre_snapshot: Mapping[str, Mapping[str, Any]],
    restored_snapshot: Mapping[str, Mapping[str, Any]],
) -> None:
    for core_name in (DEFAULT_CORE, EXPECTED_CANDIDATE_CORE, EXPECTED_NATIVE_CORE, EXPECTED_IDENTITY_CORE):
        pre_c = pre_snapshot[core_name]
        post_c = restored_snapshot[core_name]
        for field in ("numFound", "numDocs", "maxDoc", "deletedDocs", "version", "instanceDir", "dataDir", "commitTimeMSec"):
            if pre_c.get(field) != post_c.get(field):
                raise Gate4HandoffError(
                    f"Post-rollback verification failed for core {core_name} field {field}: "
                    f"original={pre_c.get(field)!r}, restored={post_c.get(field)!r}"
                )


def perform_core_admin_swap(
    solr_host_url: str,
    core: str,
    other: str,
    http_get_json: Callable[[str], Dict[str, Any]] = default_http_get_json,
) -> Dict[str, Any]:
    if solr_host_url.rstrip("/") != "http://127.0.0.1:28983/solr":
        raise Gate4HandoffError(
            f"Safety violation: CoreAdmin SWAP is only permitted on http://127.0.0.1:28983/solr (got {solr_host_url!r})"
        )
    if {core, other} != {DEFAULT_CORE, EXPECTED_CANDIDATE_CORE}:
        raise Gate4HandoffError(
            f"Safety violation: CoreAdmin SWAP only permitted between {DEFAULT_CORE!r} and {EXPECTED_CANDIDATE_CORE!r}"
        )
    swap_url = (
        f"{solr_host_url.rstrip('/')}/admin/cores?"
        + urllib.parse.urlencode({"action": "SWAP", "core": core, "other": other, "wt": "json"})
    )
    resp = http_get_json(swap_url)
    if (resp.get("responseHeader") or {}).get("status") != 0:
        raise Gate4HandoffError(f"CoreAdmin SWAP failed: {resp!r}")
    return resp


def execute_handoff_orchestration(
    state: Mapping[str, Any],
    report: Mapping[str, Any],
    report_path: str,
    state_dir_name: str,
    docker_inspect_fn: Callable[[str], Dict[str, Any]],
    http_get_json: Callable[[str], Dict[str, Any]],
    process_snapshots_fn: Callable[[], Tuple[str, str, str]],
    run_jruby_preflight_fn: Callable[[int], Dict[str, Any]],
    run_jruby_round_fn: Callable[[str, int, bool, int], Dict[str, Any]],
    swap_fn: Optional[Callable[[str, str, str], Dict[str, Any]]] = None,
    timeout_sec: int = 120,
    dry_run: bool = False,
) -> Dict[str, Any]:
    if swap_fn is None:
        swap_fn = lambda url, c1, c2: perform_core_admin_swap(url, c1, c2, http_get_json=http_get_json)

    report_audit = validate_candidate_report(report, report_path=report_path)
    stack_identity = verify_read_only_stack_identity(
        state, docker_inspect_fn=docker_inspect_fn, http_get_json=http_get_json
    )
    host_ps, backend_ps, app_ps = process_snapshots_fn()
    process_check = check_no_active_benchmark_or_indexer_processes(host_ps, backend_ps, app_ps)

    solr_url = state["solr_host_url"]
    nonce_diff_proof = verify_live_nonce_only_solr_diffs(solr_url, http_get_json=http_get_json)
    pre_swap_cores = fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
    verify_pre_swap_cores(pre_swap_cores)

    jruby_preflight = run_jruby_preflight_fn(EXPECTED_MODIFIED_SINCE)
    if jruby_preflight.get("status") != "ok" or jruby_preflight.get("nonempty_feeds_count") != 0:
        raise Gate4HandoffError(f"JRuby feed preflight failed: {jruby_preflight!r}")

    if dry_run:
        return {
            "status": "dry_run_preflight_passed",
            "gate": "Gate 4",
            "gate_passed": False,
            "report_audit": report_audit,
            "stack_identity": stack_identity,
            "process_check": process_check,
            "nonce_only_parity_verification": nonce_diff_proof,
            "core_snapshots": {
                "before_swap": pre_swap_cores,
                "after_swap": None,
                "after_round_before_rollback": None,
                "after_rollback": None,
            },
            "jruby_preflight": jruby_preflight,
            "jruby_handoff_and_round": None,
            "swap_executed": False,
            "rollback_executed": False,
        }

    swapped_verified = False
    ambiguous_state = False
    post_swap_cores: Optional[Dict[str, Dict[str, Any]]] = None
    after_round_cores: Optional[Dict[str, Dict[str, Any]]] = None
    restored_cores: Optional[Dict[str, Dict[str, Any]]] = None
    jruby_round_result: Optional[Dict[str, Any]] = None
    swap_forward_resp: Optional[Dict[str, Any]] = None
    swap_rollback_resp: Optional[Dict[str, Any]] = None
    primary_error: Optional[Exception] = None
    rollback_error: Optional[Exception] = None

    try:
        try:
            swap_forward_resp = swap_fn(solr_url, DEFAULT_CORE, EXPECTED_CANDIDATE_CORE)
        except Exception as exc:
            ambiguous_state = True
            raise Gate4HandoffError(
                f"CoreAdmin SWAP call failed ({exc}); stopping without blind second swap"
            ) from exc

        post_swap_cores = fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
        try:
            verify_swapped_cores(pre_swap_cores, post_swap_cores)
            swapped_verified = True
        except Exception as exc:
            ambiguous_state = True
            raise

        jruby_round_result = run_jruby_round_fn(
            state_dir_name, EXPECTED_WATERMARK_EPOCH, True, timeout_sec
        )
        if jruby_round_result.get("status") != "ok":
            raise Gate4HandoffError(f"JRuby handoff round returned non-ok: {jruby_round_result!r}")

        after_round_cores = fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
        try:
            verify_promoted_core_unchanged_after_round(post_swap_cores, after_round_cores)
        except Exception:
            ambiguous_state = True
            raise
    except Exception as exc:
        primary_error = exc
    finally:
        if swapped_verified and not ambiguous_state:
            try:
                swap_rollback_resp = swap_fn(solr_url, DEFAULT_CORE, EXPECTED_CANDIDATE_CORE)
                restored_cores = fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
                verify_restored_cores(pre_swap_cores, restored_cores)
            except Exception as rb_exc:
                rollback_error = rb_exc

    if primary_error is not None and rollback_error is not None:
        raise Gate4HandoffError(
            f"Primary error: {primary_error}; Rollback ALSO failed: {rollback_error}"
        ) from primary_error
    if primary_error is not None:
        raise primary_error
    if rollback_error is not None:
        raise rollback_error

    return {
        "status": "one_shot_no_change_round_passed_and_rolled_back",
        "gate": "Gate 4",
        "gate_passed": False,
        "report_audit": report_audit,
        "stack_identity": stack_identity,
        "process_check": process_check,
        "nonce_only_parity_verification": nonce_diff_proof,
        "core_snapshots": {
            "before_swap": pre_swap_cores,
            "after_swap": post_swap_cores,
            "after_round_before_rollback": after_round_cores,
            "after_rollback": restored_cores,
        },
        "swap_responses": {
            "forward": swap_forward_resp,
            "rollback": swap_rollback_resp,
        },
        "jruby_preflight": jruby_preflight,
        "jruby_handoff_and_round": jruby_round_result,
        "swap_executed": True,
        "rollback_executed": True,
    }


def write_mode_0600_json(path: str, payload: Mapping[str, Any]) -> None:
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    rendered = redact_secrets(json.dumps(payload, indent=2) + "\n")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(rendered)
    os.chmod(path, 0o600)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run Gate 4 one-shot no-change Ruby STAFF incremental handoff round on stack 844ef09c."
    )
    parser.add_argument("--state", default=DEFAULT_STATE_PATH)
    parser.add_argument("--report", default=DEFAULT_REPORT_PATH)
    parser.add_argument("--timeout-sec", type=int, default=120)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--output", default=None)
    args = parser.parse_args(argv)

    commands_executed: List[Dict[str, Any]] = []

    def run_cmd(cmd: List[str], summary: str, check: bool = True) -> subprocess.CompletedProcess:
        proc = subprocess.run(cmd, capture_output=True, text=True)
        commands_executed.append(
            {
                "command": redact_secrets(" ".join(cmd)),
                "exit_code": proc.returncode,
                "summary": summary,
            }
        )
        if check and proc.returncode != 0:
            raise Gate4HandoffError(
                f"Command {' '.join(cmd)} failed (exit {proc.returncode}): "
                f"{redact_secrets(proc.stderr.strip() or proc.stdout.strip())[:600]}"
            )
        return proc

    with open(args.state, encoding="utf-8") as fh:
        state = json.load(fh)
    with open(args.report, encoding="utf-8") as fh:
        report = json.load(fh)

    report_sha256_before = hashlib.sha256(open(args.report, "rb").read()).hexdigest()
    prior_blocker_sha256_before = (
        hashlib.sha256(open(PRIOR_BLOCKER_REPORT_PATH, "rb").read()).hexdigest()
        if os.path.isfile(PRIOR_BLOCKER_REPORT_PATH)
        else None
    )

    git_branch = run_cmd(
        ["git", "-C", REPO_ROOT, "rev-parse", "--abbrev-ref", "HEAD"],
        "Check current Git branch",
    ).stdout.strip()
    git_head = run_cmd(
        ["git", "-C", REPO_ROOT, "rev-parse", "HEAD"],
        "Check current Git HEAD commit",
    ).stdout.strip()
    git_status = run_cmd(
        ["git", "-C", REPO_ROOT, "status", "--porcelain"],
        "Check working tree status",
    ).stdout.splitlines()

    if git_branch != EXPECTED_BRANCH:
        raise Gate4HandoffError(f"Expected branch {EXPECTED_BRANCH}, got {git_branch}")

    run_nonce = secrets.token_hex(4)
    ts_str = time.strftime("%Y%m%d_%H%M%S", time.gmtime())
    state_dir_name = f"indexer_state_gate4_handoff_20261124_{run_nonce}"
    output_path = args.output or os.path.join(
        BENCHMARKS_DIR,
        f"gate4_no_change_handoff_{ts_str}_{os.getpid()}_{run_nonce}.json",
    )

    def docker_inspect_fn(container_name: str) -> Dict[str, Any]:
        proc = run_cmd(
            ["docker", "container", "inspect", container_name],
            f"Read-only inspect container {container_name}",
        )
        return json.loads(proc.stdout)[0]

    def process_snapshots_fn() -> Tuple[str, str, str]:
        host_p = run_cmd(["ps", "-eo", "pid=,comm=,args="], "Inspect host processes").stdout
        back_p = run_cmd(
            ["docker", "exec", f"archivalist-profiling-backend-{EXPECTED_STACK_ID}", "ps", "aux"],
            "Inspect isolated backend container processes",
        ).stdout
        app_p = run_cmd(
            ["docker", "exec", "archivesspaceai-app-1", "ps", "aux"],
            "Inspect app container processes",
        ).stdout
        return host_p, back_p, app_p

    def run_jruby_preflight_fn(modified_since: int) -> Dict[str, Any]:
        proc = run_cmd(
            [
                "docker",
                "exec",
                "-w",
                "/archivesspace",
                f"archivalist-profiling-backend-{EXPECTED_STACK_ID}",
                "/archivesspace/scripts/jruby",
                "/archivesspace/rust-tools/scripts/gate4_isolated_handoff_round.rb",
                "--mode",
                "preflight",
                "--modified-since",
                str(modified_since),
                "--repo-ids",
                "2,3,4",
            ],
            f"Read-only JRuby preflight of all 19 STAFF feeds, /repositories, and /delete-feed at modified_since={modified_since}",
        )
        lines = [ln.strip() for ln in proc.stdout.splitlines() if ln.strip()]
        return json.loads(lines[-1])

    def run_jruby_round_fn(
        dir_name: str, epoch: int, promoted: bool, timeout_s: int
    ) -> Dict[str, Any]:
        proc = run_cmd(
            [
                "docker",
                "exec",
                "-w",
                "/archivesspace",
                f"archivalist-profiling-backend-{EXPECTED_STACK_ID}",
                "/archivesspace/scripts/jruby",
                "/archivesspace/rust-tools/scripts/gate4_isolated_handoff_round.rb",
                "--mode",
                "apply-and-run",
                "--state-dir-name",
                dir_name,
                "--epoch",
                str(epoch),
                "--repo-ids",
                "2,3,4",
                "--promoted",
                "true" if promoted else "false",
                "--timeout-sec",
                str(timeout_s),
            ],
            f"Apply 59 IndexState checkpoints to {dir_name} and execute one bounded PeriodicIndexer#run_index_round",
        )
        lines = [ln.strip() for ln in proc.stdout.splitlines() if ln.strip()]
        return json.loads(lines[-1])

    def logged_swap_fn(solr_url: str, c1: str, c2: str) -> Dict[str, Any]:
        resp = perform_core_admin_swap(solr_url, c1, c2, http_get_json=default_http_get_json)
        commands_executed.append(
            {
                "command": f"GET {solr_url.rstrip('/')}/admin/cores?action=SWAP&core={c1}&other={c2}&wt=json",
                "exit_code": 0,
                "summary": f"Solr CoreAdmin SWAP between {c1} and {c2}",
            }
        )
        return resp

    orchestration = execute_handoff_orchestration(
        state=state,
        report=report,
        report_path=os.path.relpath(args.report, REPO_ROOT),
        state_dir_name=state_dir_name,
        docker_inspect_fn=docker_inspect_fn,
        http_get_json=default_http_get_json,
        process_snapshots_fn=process_snapshots_fn,
        run_jruby_preflight_fn=run_jruby_preflight_fn,
        run_jruby_round_fn=run_jruby_round_fn,
        swap_fn=logged_swap_fn,
        timeout_sec=args.timeout_sec,
        dry_run=args.dry_run,
    )

    report_sha256_after = hashlib.sha256(open(args.report, "rb").read()).hexdigest()
    if report_sha256_before != report_sha256_after:
        raise Gate4HandoffError("Candidate parity report file was unexpectedly modified!")

    prior_blocker_sha256_after = (
        hashlib.sha256(open(PRIOR_BLOCKER_REPORT_PATH, "rb").read()).hexdigest()
        if os.path.isfile(PRIOR_BLOCKER_REPORT_PATH)
        else None
    )
    if prior_blocker_sha256_before != prior_blocker_sha256_after:
        raise Gate4HandoffError("Prior blocker report file was unexpectedly modified!")

    evidence = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "git_branch": git_branch,
        "git_head": git_head,
        "git_dirty": bool(git_status),
        "git_status_porcelain": git_status,
        "preserved_reports_integrity": {
            "candidate_report_path": os.path.relpath(args.report, REPO_ROOT),
            "candidate_report_sha256_unchanged": report_sha256_before == report_sha256_after,
            "candidate_report_sha256": report_sha256_after,
            "prior_blocker_report_path": os.path.relpath(PRIOR_BLOCKER_REPORT_PATH, REPO_ROOT),
            "prior_blocker_report_sha256_unchanged": prior_blocker_sha256_before == prior_blocker_sha256_after,
            "prior_blocker_report_sha256": prior_blocker_sha256_after,
        },
        **orchestration,
        "limitations_and_remaining_gate4_scope": {
            "gate4_passed": False,
            "exact_full_core_parity_claimed": False,
            "nonce_only_parity_exception": (
                "Candidate report has exact_parity=false against the fixed prior-run native "
                "reference core because of the 2 synthetic stack challenge-nonce records "
                "(/repositories/2 and /agents/corporate_entities/1); 1,036,425/1,036,427 "
                "documents are strict-exact and the 2 differing records match byte-for-byte "
                "after substituting the challenge nonce."
            ),
            "pui_state_limitation": (
                "This test exercises STAFF IndexState + PeriodicIndexer#run_index_round only; "
                "/archivalist-profiling-data/indexer_pui_state remained absent/untouched and "
                "PUIIndexer was not run."
            ),
            "s3_state_limitation": (
                "AppConfig[:index_state_class] is 'IndexState' (local File provider) and "
                "AppConfig[:index_state_s3] is null on stack 844ef09c; IndexStateS3 was not "
                "configured, invented, or exercised."
            ),
            "untested_gate4_matrix": [
                "Live create/update/delete during the Rust full pass picked up by a subsequent Ruby incremental round",
                "Candidate failure before promotion and post-promotion checkpoint interruption/replay",
                "Process restart/outage boundary around core promotion and checkpoint persistence",
                "Configured S3 state provider (IndexStateS3) handoff and interruption/replay",
            ],
        },
        "commands_executed": commands_executed,
    }

    write_mode_0600_json(output_path, evidence)
    print(f"Wrote mode-0600 Gate 4 handoff evidence to: {output_path}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"\n[FATAL ERROR] {redact_secrets(exc)}", file=sys.stderr)
        sys.exit(1)
