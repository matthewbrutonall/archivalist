#!/usr/bin/env python3
"""Gate 4 bounded create/update/delete-during-Rust-pass and Ruby handoff harness.

Executes a bounded, reproducible create/update/delete-during-pass experiment and
one-shot Ruby PeriodicIndexer handoff round against isolated fictional profiling
stack 844ef09c ONLY:

1. Read-only preflight:
   - Verifies stack 844ef09c identity strictly read-only (container statuses and
     labels, state URLs, GET /repositories/2 code ARCHPROF-844ef09c and 32-hex
     nonce, Solr identity core status) without calling `profiling_stack.py verify`.
   - Verifies main/bench Solr containers are stopped, free disk >= 60 GiB,
     available memory >= 24 GiB, and zero concurrent benchmark or indexer jobs.
   - Verifies all Solr core mappings and document counts (including empty
     `archivesspace`, retained accidental empty candidate core, and fixed native
     reference core `stage_staff_20261124_010605_9807_36038e70_native`).
   - Verifies repository 3 resources ([1]), AO total count (2,010), parent AO
     (/repositories/3/archival_objects/2001), leaf update target AO
     (/repositories/3/archival_objects/2009, children=[], lock_version=0), and
     leaf delete target AO (/repositories/3/archival_objects/2010, children=[],
     lock_version=0, position=8).
   - Samples the fixed native reference core (`sort=id asc`, `rows=25`) to
     select and single-document verify a valid repo-4 archival_object progress
     sentinel URI (e.g. `/repositories/4/archival_objects/10000`).

2. Fixed-reference candidate run + guarded delayed mutator:
   - Loads the 1,036,427 reference documents from fixed read-only native core
     `stage_staff_20261124_010605_9807_36038e70_native` and validates that the
     exact repo-4 AO progress sentinel URI sampled in preflight is present and
     valid (type/repository/id) in the complete native reference mapping.
   - Creates a fresh unique candidate core `stage_staff_<run_nonce>_candidate`.
   - Launches `aspace-cli staff-full-candidate` via `peak_rss.run_with_peak_rss`
     (which uses a clean exec helper so parent memory does not inflate child
     `ru_maxrss`).
   - Triggers the guarded delayed mutator relative to actual `aspace-cli` child
     launch (after the ~155s native reference snapshot load).
   - The mutator waits until `aspace-cli` is alive, candidate core `/select`
     `numFound == 0` (uncommitted), Solr RealTime Get (`/get`) on the candidate
     core confirms repo-3 AOs 2009 and 2010 are already in the candidate core's
     uncommitted transaction log, AND the verified fixed-native repo-4 AO
     sentinel sampled in preflight (e.g. `/repositories/4/archival_objects/10000`)
     is in the transaction log (proving repo 3 is complete and the long repo-4
     AO pass is underway).
   - Saves mode-0600 before/after records and timestamps while applying exactly
     three repo-3 AO mutations via the API with `lock_version` checks:
       a) Update title on `/repositories/3/archival_objects/2009`
       b) Delete leaf `/repositories/3/archival_objects/2010`
       c) Create one new leaf AO beneath `/repositories/3/resources/1` and
          parent `/repositories/3/archival_objects/2001`
     Preserves net repo-3 AO count (2,010) and total document count (1,036,427).

3. Post-pass comparison & isolated Ruby handoff round:
   - Verifies the committed Rust candidate core missed the three repo-3
     mutations (matches the pre-mutation native reference core across all
     1,036,427 documents except the two challenge-nonce identity records
     `/repositories/2` and `/agents/corporate_entities/1`).
   - Performs reversible CoreAdmin SWAP of the candidate core into
     `archivesspace` inside stack 844ef09c, with guaranteed rollback in `finally`
     (and fail-closed refusal to issue a blind second swap if state is ever
     ambiguous).
   - Checkpoints all 59 File `IndexState` keys at `full_reindex_started_at_epoch_sec`
     in a unique state directory (`indexer_state_gate4_mutation_handoff_20261124_<nonce>`),
     keeping PUI state separate and untouched.
   - Runs one actual Ruby `PeriodicIndexer#run_index_round`, verifies discovery
     and processing of the create, update, delete (plus the touched repo-3
     resource), verifies exact Solr documents and total count (1,036,427) after
     the round, verifies checkpoint advancement only for processed feeds, and
     restores original core mappings in `finally`.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import threading
import time
from typing import Any, Callable, Dict, List, Mapping, Optional, Tuple
import urllib.error
import urllib.parse
import urllib.request

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", ".."))
BENCHMARKS_DIR = os.path.join(REPO_ROOT, "rust-tools", "benchmarks")
sys.path.insert(0, SCRIPT_DIR)

from peak_rss import PeakRssResult, compare_peak_memory, run_with_peak_rss  # noqa: E402
import run_gate3_combined_candidate as gate3_harness  # noqa: E402
import run_gate4_no_change_handoff as gate4_base  # noqa: E402

EXPECTED_STACK_ID = "844ef09c"
EXPECTED_BRANCH = "codex/million-snapshot-seed"
DEFAULT_STATE_PATH = os.path.join(BENCHMARKS_DIR, "profiling_stack_844ef09c.json")
FIXED_NATIVE_CORE = "stage_staff_20261124_010605_9807_36038e70_native"
PRIOR_WATERMARKED_CANDIDATE_CORE = "stage_staff_20261124_055901_3358917_575e2fa7_candidate"
PRIOR_UNWATERMARKED_CANDIDATE_CORE = "stage_staff_20261123_224555_3217466_09a8939e_candidate"
ACCIDENTAL_EMPTY_CANDIDATE_CORE = "stage_staff_20261124_054050_3352672_b76bd497_candidate"
DEFAULT_CORE = "archivesspace"
EXPECTED_IDENTITY_CORE = "archivalist_profiling_identity_844ef09c"

EXPECTED_TOTAL_DOCS = 1036427
EXPECTED_EXACT_RECORDS_VS_NATIVE = 1036425
EXPECTED_REPO_IDS = [2, 3, 4]
TARGET_REPO_ID = 3
TARGET_RESOURCE_ID = 1
TARGET_PARENT_AO_ID = 2001
TARGET_UPDATE_AO_ID = 2009
TARGET_DELETE_AO_ID = 2010
EXPECTED_REPO3_AO_COUNT = 2010
WINDOW_SECONDS = 30

MIN_FREE_DISK_GIB = 60.0
MIN_AVAILABLE_MEM_GIB = 24.0
DEFAULT_CONCURRENCY = 12
DEFAULT_MIN_MUTATION_DELAY_SEC = 45.0
DEFAULT_MAX_MUTATION_WAIT_SEC = 240.0
DEFAULT_POLL_INTERVAL_SEC = 3.0

EXPECTED_TYPE_COUNTS = {
    "agent_corporate_entity": 3,
    "agent_person": 151,
    "agent_software": 1,
    "archival_object": 1000980,
    "digital_object": 22201,
    "repository": 3,
    "resource": 498,
    "subject": 150,
    "top_container": 12440,
}


class Gate4MutationHandoffError(RuntimeError):
    """Fail-closed error during Gate 4 mutation-during-pass or handoff validation."""


def redact_secrets(text: Any) -> str:
    return gate4_base.redact_secrets(gate3_harness.redact_secrets(text))


def write_mode_0600_json(path: str, payload: Mapping[str, Any]) -> None:
    parent = os.path.dirname(os.path.abspath(path))
    os.makedirs(parent, exist_ok=True)
    if os.path.exists(path):
        raise Gate4MutationHandoffError(f"Refusing to overwrite existing evidence file: {path}")
    rendered = redact_secrets(json.dumps(payload, indent=2, sort_keys=False)) + "\n"
    tmp_path = os.path.join(
        parent,
        f".tmp_{os.getpid()}_{time.time_ns()}_{os.path.basename(path)}",
    )
    fd = os.open(tmp_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(rendered)
            fh.flush()
            os.fsync(fh.fileno())
        os.chmod(tmp_path, 0o600)
        os.replace(tmp_path, path)
    finally:
        if os.path.exists(tmp_path):
            try:
                os.unlink(tmp_path)
            except OSError:
                pass


def get_free_disk_gib(path: str) -> float:
    usage = shutil.disk_usage(path)
    return usage.free / (1024.0 ** 3)


def verify_read_only_stack_identity_any_nonce(
    state: Mapping[str, Any],
    docker_inspect_fn: Callable[[str], Optional[Dict[str, Any]]],
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
) -> Dict[str, Any]:
    """Proves stack 844ef09c identity strictly read-only without mutating repository 2."""
    if state.get("stack_id") != EXPECTED_STACK_ID:
        raise Gate4MutationHandoffError(
            f"State stack_id={state.get('stack_id')!r} != expected {EXPECTED_STACK_ID!r}"
        )
    if state.get("backend_host_url") != "http://127.0.0.1:24567":
        raise Gate4MutationHandoffError(f"Unexpected backend_host_url: {state.get('backend_host_url')!r}")
    if state.get("solr_host_url") != "http://127.0.0.1:28983/solr":
        raise Gate4MutationHandoffError(f"Unexpected solr_host_url: {state.get('solr_host_url')!r}")
    if state.get("backend_network_url") != "http://archivalist-profiling-backend-844ef09c:4567":
        raise Gate4MutationHandoffError(f"Unexpected backend_network_url: {state.get('backend_network_url')!r}")
    if state.get("solr_network_url") != "http://archivalist-profiling-solr-844ef09c:8983/solr":
        raise Gate4MutationHandoffError(f"Unexpected solr_network_url: {state.get('solr_network_url')!r}")
    if state.get("identity_repo_id") != gate4_base.EXPECTED_IDENTITY_REPO_ID:
        raise Gate4MutationHandoffError(f"Unexpected identity_repo_id: {state.get('identity_repo_id')!r}")
    if state.get("identity_repo_code") != gate4_base.EXPECTED_IDENTITY_REPO_CODE:
        raise Gate4MutationHandoffError(f"Unexpected identity_repo_code: {state.get('identity_repo_code')!r}")
    if state.get("identity_core") != EXPECTED_IDENTITY_CORE:
        raise Gate4MutationHandoffError(f"Unexpected identity_core: {state.get('identity_core')!r}")

    for forbidden in (gate3_harness.MAIN_SOLR_CONTAINER, gate3_harness.BENCH_SOLR_CONTAINER):
        f_info = docker_inspect_fn(forbidden)
        if f_info and bool((f_info.get("State") or {}).get("Running")):
            raise Gate4MutationHandoffError(
                f"Safety violation: non-isolated Solr container {forbidden} is running"
            )

    containers = state.get("containers") or {}
    expected_containers = {
        "db": f"archivalist-profiling-db-{EXPECTED_STACK_ID}",
        "solr": f"archivalist-profiling-solr-{EXPECTED_STACK_ID}",
        "backend": f"archivalist-profiling-backend-{EXPECTED_STACK_ID}",
    }
    if containers != expected_containers:
        raise Gate4MutationHandoffError(
            f"State containers={containers!r} != expected {expected_containers!r}"
        )

    container_evidence: Dict[str, Any] = {}
    for role, expected_name in expected_containers.items():
        info = docker_inspect_fn(expected_name)
        if not info:
            raise Gate4MutationHandoffError(f"Stack container {expected_name} not found")
        labels = (info.get("Config") or {}).get("Labels") or {}
        running = bool((info.get("State") or {}).get("Running"))
        status_str = (info.get("State") or {}).get("Status")
        if not running or status_str != "running":
            raise Gate4MutationHandoffError(
                f"Stack container {expected_name} is not running (status={status_str!r})"
            )
        if labels.get(gate4_base.LABEL_STACK) != EXPECTED_STACK_ID or labels.get(gate4_base.LABEL_ROLE) != role:
            raise Gate4MutationHandoffError(
                f"Container {expected_name} has unexpected labels: {labels!r}"
            )
        container_evidence[expected_name] = {
            "role": role,
            "status": status_str,
            "running": running,
            "labels": {
                gate4_base.LABEL_STACK: labels.get(gate4_base.LABEL_STACK),
                gate4_base.LABEL_ROLE: labels.get(gate4_base.LABEL_ROLE),
            },
        }

    repo2_url = f"{state['backend_host_url'].rstrip('/')}/repositories/{gate4_base.EXPECTED_IDENTITY_REPO_ID}"
    repo2 = http_get_json(repo2_url)
    name = repo2.get("name") or ""
    if repo2.get("repo_code") != gate4_base.EXPECTED_IDENTITY_REPO_CODE or not name.startswith(
        gate4_base.IDENTITY_NAME_PREFIX
    ):
        raise Gate4MutationHandoffError(
            f"Read-only GET {repo2_url} returned unexpected repo_code={repo2.get('repo_code')!r}, name={name!r}"
        )
    nonce = name[len(gate4_base.IDENTITY_NAME_PREFIX) :]
    if not re.fullmatch(r"[0-9a-f]{32}", nonce):
        raise Gate4MutationHandoffError(f"Malformed challenge nonce in /repositories/2 name: {name!r}")

    status_url = (
        f"{state['solr_host_url'].rstrip('/')}/admin/cores?"
        + urllib.parse.urlencode({"action": "STATUS", "core": EXPECTED_IDENTITY_CORE, "wt": "json"})
    )
    solr_status = http_get_json(status_url)
    id_core_info = (solr_status.get("status") or {}).get(EXPECTED_IDENTITY_CORE)
    if not id_core_info:
        raise Gate4MutationHandoffError(
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
        "identity_repo_id": gate4_base.EXPECTED_IDENTITY_REPO_ID,
        "identity_repo_code": repo2.get("repo_code"),
        "identity_repo_name": name,
        "identity_nonce_verified": nonce,
        "identity_repo_system_mtime": repo2.get("system_mtime"),
        "identity_core": EXPECTED_IDENTITY_CORE,
        "containers": container_evidence,
    }


class AspaceApiSession:
    """Thin authenticated ArchivesSpace API client confined to isolated stack 844ef09c."""

    def __init__(self, base_url: str, session_token: str) -> None:
        if base_url.rstrip("/") != "http://127.0.0.1:24567":
            raise Gate4MutationHandoffError(
                f"Safety violation: AspaceApiSession is confined to http://127.0.0.1:24567 (got {base_url!r})"
            )
        self.base_url = base_url.rstrip("/")
        self.session_token = session_token

    @classmethod
    def login(cls, base_url: str, username: str, password: str, timeout: int = 15) -> "AspaceApiSession":
        if base_url.rstrip("/") != "http://127.0.0.1:24567":
            raise Gate4MutationHandoffError(
                f"Safety violation: login is confined to http://127.0.0.1:24567 (got {base_url!r})"
            )
        login_url = f"{base_url.rstrip('/')}/users/{urllib.parse.quote(username)}/login"
        data = urllib.parse.urlencode({"password": password}).encode("utf-8")
        req = urllib.request.Request(
            login_url,
            data=data,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            payload = json.loads(resp.read().decode("utf-8"))
        token = payload.get("session")
        if not token or not isinstance(token, str):
            raise Gate4MutationHandoffError("Backend login did not return a session token")
        return cls(base_url, token)

    def get_json(self, path: str, params: Optional[Mapping[str, Any]] = None, timeout: int = 20) -> Any:
        url = f"{self.base_url}{path}"
        if params:
            url = f"{url}?{urllib.parse.urlencode(params)}"
        req = urllib.request.Request(
            url,
            headers={"X-ArchivesSpace-Session": self.session_token},
            method="GET",
        )
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))

    def get_status_and_json(self, path: str, timeout: int = 20) -> Tuple[int, Any]:
        url = f"{self.base_url}{path}"
        req = urllib.request.Request(
            url,
            headers={"X-ArchivesSpace-Session": self.session_token},
            method="GET",
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.status, json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            try:
                parsed = json.loads(body)
            except Exception:
                parsed = {"raw": body}
            return exc.code, parsed

    def post_json(self, path: str, payload: Mapping[str, Any], timeout: int = 30) -> Dict[str, Any]:
        if not path.startswith("/repositories/3/archival_objects"):
            raise Gate4MutationHandoffError(
                f"Safety violation: POST mutation only permitted on /repositories/3/archival_objects* (got {path!r})"
            )
        url = f"{self.base_url}{path}"
        body = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            url,
            data=body,
            headers={
                "X-ArchivesSpace-Session": self.session_token,
                "Content-Type": "application/json",
            },
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))

    def delete_record(self, path: str, timeout: int = 30) -> Dict[str, Any]:
        expected_path = f"/repositories/{TARGET_REPO_ID}/archival_objects/{TARGET_DELETE_AO_ID}"
        if path != expected_path:
            raise Gate4MutationHandoffError(
                f"Safety violation: DELETE is strictly confined to {expected_path!r} (got {path!r})"
            )
        url = f"{self.base_url}{path}"
        req = urllib.request.Request(
            url,
            headers={"X-ArchivesSpace-Session": self.session_token},
            method="DELETE",
        )
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))


def verify_repo3_mutation_targets_read_only(api: Any) -> Dict[str, Any]:
    """Read-only inspection and verification of repo 3 resource and target AOs."""
    resource_ids = api.get_json(f"/repositories/{TARGET_REPO_ID}/resources", {"all_ids": "true"})
    if resource_ids != [TARGET_RESOURCE_ID]:
        raise Gate4MutationHandoffError(
            f"Expected repo 3 resource IDs {[TARGET_RESOURCE_ID]}, got {resource_ids!r}"
        )

    resource_uri = f"/repositories/{TARGET_REPO_ID}/resources/{TARGET_RESOURCE_ID}"
    resource_rec = api.get_json(resource_uri)
    if (
        resource_rec.get("uri") != resource_uri
        or resource_rec.get("publish") is not True
        or type(resource_rec.get("lock_version")) is not int
    ):
        raise Gate4MutationHandoffError(f"Unexpected repo 3 resource 1 state: {resource_rec!r}")

    ao_ids = sorted(api.get_json(f"/repositories/{TARGET_REPO_ID}/archival_objects", {"all_ids": "true"}))
    if len(ao_ids) != EXPECTED_REPO3_AO_COUNT:
        raise Gate4MutationHandoffError(
            f"Expected {EXPECTED_REPO3_AO_COUNT} AOs in repo 3, got {len(ao_ids)}"
        )
    if TARGET_UPDATE_AO_ID not in ao_ids or TARGET_DELETE_AO_ID not in ao_ids or TARGET_PARENT_AO_ID not in ao_ids:
        raise Gate4MutationHandoffError(
            f"Required target AO IDs ({TARGET_PARENT_AO_ID}, {TARGET_UPDATE_AO_ID}, {TARGET_DELETE_AO_ID}) "
            f"not all present in repo 3 AO list (min={ao_ids[0]}, max={ao_ids[-1]})"
        )

    parent_uri = f"/repositories/{TARGET_REPO_ID}/archival_objects/{TARGET_PARENT_AO_ID}"
    parent_rec = api.get_json(parent_uri)
    parent_children = api.get_json(f"{parent_uri}/children")
    if (
        parent_rec.get("uri") != parent_uri
        or (parent_rec.get("resource") or {}).get("ref") != resource_uri
        or not isinstance(parent_children, list)
        or len(parent_children) != 9
    ):
        raise Gate4MutationHandoffError(
            f"Unexpected parent AO {parent_uri} state or children count: "
            f"children={len(parent_children) if isinstance(parent_children, list) else parent_children!r}"
        )

    update_uri = f"/repositories/{TARGET_REPO_ID}/archival_objects/{TARGET_UPDATE_AO_ID}"
    update_rec = api.get_json(update_uri)
    update_children = api.get_json(f"{update_uri}/children")
    if (
        update_rec.get("uri") != update_uri
        or (update_rec.get("resource") or {}).get("ref") != resource_uri
        or (update_rec.get("parent") or {}).get("ref") != parent_uri
        or update_rec.get("publish") is not True
        or update_rec.get("suppressed") is not False
        or update_rec.get("level") != "item"
        or update_rec.get("lock_version") != 0
        or update_children != []
        or update_rec.get("instances") != []
    ):
        raise Gate4MutationHandoffError(
            f"Update target {update_uri} failed leaf/precondition check: "
            f"lock_version={update_rec.get('lock_version')!r}, children={update_children!r}"
        )

    delete_uri = f"/repositories/{TARGET_REPO_ID}/archival_objects/{TARGET_DELETE_AO_ID}"
    delete_rec = api.get_json(delete_uri)
    delete_children = api.get_json(f"{delete_uri}/children")
    if (
        delete_rec.get("uri") != delete_uri
        or (delete_rec.get("resource") or {}).get("ref") != resource_uri
        or (delete_rec.get("parent") or {}).get("ref") != parent_uri
        or delete_rec.get("publish") is not True
        or delete_rec.get("suppressed") is not False
        or delete_rec.get("level") != "item"
        or delete_rec.get("lock_version") != 0
        or delete_rec.get("position") != 8
        or delete_children != []
        or delete_rec.get("instances") != []
    ):
        raise Gate4MutationHandoffError(
            f"Delete target {delete_uri} failed leaf/precondition check: "
            f"lock_version={delete_rec.get('lock_version')!r}, position={delete_rec.get('position')!r}, "
            f"children={delete_children!r}"
        )

    return {
        "repository_id": TARGET_REPO_ID,
        "resource_ids": resource_ids,
        "resource": {
            "uri": resource_uri,
            "title": resource_rec.get("title"),
            "lock_version": resource_rec.get("lock_version"),
            "publish": resource_rec.get("publish"),
            "system_mtime": resource_rec.get("system_mtime"),
        },
        "repo3_ao_count": len(ao_ids),
        "repo3_ao_min_id": ao_ids[0],
        "repo3_ao_max_id": ao_ids[-1],
        "parent_ao": {
            "uri": parent_uri,
            "title": parent_rec.get("title"),
            "level": parent_rec.get("level"),
            "lock_version": parent_rec.get("lock_version"),
            "children_count": len(parent_children),
            "child_uris": [c.get("uri") for c in parent_children if isinstance(c, Mapping)],
        },
        "update_target_ao": {
            "uri": update_uri,
            "id": TARGET_UPDATE_AO_ID,
            "title": update_rec.get("title"),
            "level": update_rec.get("level"),
            "position": update_rec.get("position"),
            "lock_version": update_rec.get("lock_version"),
            "publish": update_rec.get("publish"),
            "children_count": len(update_children),
            "is_leaf": True,
            "system_mtime": update_rec.get("system_mtime"),
            "record_snapshot": update_rec,
        },
        "delete_target_ao": {
            "uri": delete_uri,
            "id": TARGET_DELETE_AO_ID,
            "title": delete_rec.get("title"),
            "level": delete_rec.get("level"),
            "position": delete_rec.get("position"),
            "lock_version": delete_rec.get("lock_version"),
            "publish": delete_rec.get("publish"),
            "children_count": len(delete_children),
            "is_leaf": True,
            "system_mtime": delete_rec.get("system_mtime"),
            "record_snapshot": delete_rec,
        },
        "create_target_spec": {
            "repository_id": TARGET_REPO_ID,
            "resource_ref": resource_uri,
            "parent_ref": parent_uri,
            "level": "item",
            "publish": True,
            "expected_logical_position_after_delete": 8,
        },
    }


def find_live_aspace_cli_process(candidate_core: str) -> Optional[Dict[str, Any]]:
    """Finds the active `aspace-cli` process targeting `candidate_core` from /proc."""
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        pid = int(entry)
        try:
            with open(f"/proc/{pid}/comm", "r", encoding="utf-8", errors="replace") as fh:
                comm = fh.read().strip()
            with open(f"/proc/{pid}/cmdline", "rb") as fh:
                raw_cmd = fh.read().decode("utf-8", errors="replace").replace("\x00", " ").strip()
            if comm != "aspace-cli" and not raw_cmd.endswith("aspace-cli"):
                if "aspace-cli" not in raw_cmd or "python" in comm:
                    continue
            if "staff-full-candidate" not in raw_cmd or candidate_core not in raw_cmd:
                continue
            vm_rss_kib = None
            vm_hwm_kib = None
            with open(f"/proc/{pid}/status", "r", encoding="utf-8", errors="replace") as fh:
                for line in fh:
                    if line.startswith("VmRSS:"):
                        vm_rss_kib = int(line.split()[1])
                    elif line.startswith("VmHWM:"):
                        vm_hwm_kib = int(line.split()[1])
            return {
                "pid": pid,
                "comm": comm,
                "cmdline_redacted": redact_secrets(raw_cmd),
                "vm_rss_kib": vm_rss_kib,
                "vm_hwm_kib": vm_hwm_kib,
            }
        except OSError:
            continue
    return None


def solr_realtime_get_doc(
    solr_host_url: str,
    core_name: str,
    doc_id: str,
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
) -> Optional[Dict[str, Any]]:
    clean_base = solr_host_url.rstrip("/")
    q = urllib.parse.urlencode({"id": doc_id, "wt": "json"})
    resp = http_get_json(f"{clean_base}/{core_name}/get?{q}")
    doc = resp.get("doc")
    return dict(doc) if isinstance(doc, Mapping) else None


def solr_select_num_found(
    solr_host_url: str,
    core_name: str,
    q: str = "*:*",
    fq: Optional[str] = None,
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
) -> int:
    clean_base = solr_host_url.rstrip("/")
    params = [("q", q), ("rows", "0"), ("wt", "json")]
    if fq:
        params.append(("fq", fq))
    resp = http_get_json(f"{clean_base}/{core_name}/select?{urllib.parse.urlencode(params)}")
    num_found = (resp.get("response") or {}).get("numFound")
    if type(num_found) is not int:
        raise Gate4MutationHandoffError(f"Invalid numFound for {core_name} (q={q!r}): {num_found!r}")
    return num_found


REPO4_ID = 4
REPO4_AO_URI_RE = re.compile(r"^/repositories/4/archival_objects/([1-9][0-9]*)$")


def validate_repo4_ao_sentinel_uri(
    sentinel_uri: Any,
    native_docs_by_id: Optional[Mapping[str, Any]] = None,
) -> int:
    """Validates a repo-4 archival_object sentinel URI and optionally confirms presence in native_docs_by_id."""
    if not isinstance(sentinel_uri, str):
        raise Gate4MutationHandoffError(
            f"Repo-4 AO sentinel URI must be a string (got {sentinel_uri!r})"
        )
    match = REPO4_AO_URI_RE.fullmatch(sentinel_uri)
    if not match:
        raise Gate4MutationHandoffError(
            f"Invalid repo-4 AO sentinel URI {sentinel_uri!r}; expected /repositories/4/archival_objects/<positive_int>"
        )
    ao_id = int(match.group(1))
    if native_docs_by_id is not None:
        if not isinstance(native_docs_by_id, Mapping):
            raise Gate4MutationHandoffError("native_docs_by_id must be a mapping")
        if sentinel_uri not in native_docs_by_id or native_docs_by_id[sentinel_uri] is None:
            raise Gate4MutationHandoffError(
                f"Repo-4 AO sentinel URI {sentinel_uri!r} is missing from fixed native reference documents"
            )
        doc = native_docs_by_id[sentinel_uri]
        if not isinstance(doc, Mapping):
            raise Gate4MutationHandoffError(
                f"Repo-4 AO sentinel {sentinel_uri!r} entry in native reference is not a mapping"
            )
        ptype = doc.get("primary_type")
        repo = doc.get("repository")
        doc_id = doc.get("id")
        doc_uri = doc.get("uri")
        if ptype != "archival_object":
            raise Gate4MutationHandoffError(
                f"Repo-4 AO sentinel {sentinel_uri!r} has unexpected primary_type={ptype!r} in native reference"
            )
        if repo != f"/repositories/{REPO4_ID}":
            raise Gate4MutationHandoffError(
                f"Repo-4 AO sentinel {sentinel_uri!r} has unexpected repository={repo!r} in native reference"
            )
        if doc_id != sentinel_uri:
            raise Gate4MutationHandoffError(
                f"Repo-4 AO sentinel {sentinel_uri!r} has mismatched doc id={doc_id!r} in native reference"
            )
        if doc_uri is not None and doc_uri != sentinel_uri:
            raise Gate4MutationHandoffError(
                f"Repo-4 AO sentinel {sentinel_uri!r} has mismatched doc uri={doc_uri!r} in native reference"
            )
    return ao_id


def select_repo4_ao_sentinel_uri(native_docs_by_id: Mapping[str, Any]) -> str:
    """Selects the numerically smallest valid `/repositories/4/archival_objects/<id>` URI from a sampled or provided mapping."""
    if not isinstance(native_docs_by_id, Mapping):
        raise Gate4MutationHandoffError("native_docs_by_id must be a mapping")
    candidates: List[Tuple[int, str]] = []
    for uri, doc in native_docs_by_id.items():
        if not isinstance(uri, str) or not isinstance(doc, Mapping):
            continue
        match = REPO4_AO_URI_RE.fullmatch(uri)
        if not match:
            continue
        ptype = doc.get("primary_type")
        repo = doc.get("repository")
        doc_id = doc.get("id")
        doc_uri = doc.get("uri")
        if ptype != "archival_object":
            continue
        if repo != f"/repositories/{REPO4_ID}":
            continue
        if doc_id != uri:
            continue
        if doc_uri is not None and doc_uri != uri:
            continue
        ao_id = int(match.group(1))
        candidates.append((ao_id, uri))

    if not candidates:
        raise Gate4MutationHandoffError(
            "No valid /repositories/4/archival_objects/<id> documents found in fixed native reference"
        )
    _, chosen_uri = min(candidates, key=lambda item: item[0])
    validate_repo4_ao_sentinel_uri(chosen_uri, native_docs_by_id=native_docs_by_id)
    return chosen_uri


def verify_repo4_sentinel_in_fixed_native_core(
    solr_host_url: str,
    native_core: str,
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
) -> Dict[str, Any]:
    """Read-only preflight probe that samples and verifies a repo-4 AO progress sentinel in the fixed native Solr core."""
    clean_base = solr_host_url.rstrip("/")
    params = [
        ("q", "*:*"),
        ("fq", f'repository:"/repositories/{REPO4_ID}" AND primary_type:archival_object'),
        ("fl", "id,uri,title,primary_type,repository"),
        ("sort", "id asc"),
        ("rows", "25"),
        ("wt", "json"),
    ]
    resp = http_get_json(f"{clean_base}/{native_core}/select?{urllib.parse.urlencode(params)}")
    response_obj = resp.get("response") or {}
    num_found = response_obj.get("numFound")
    docs = response_obj.get("docs") or []
    if type(num_found) is not int or num_found <= 0 or not docs:
        raise Gate4MutationHandoffError(
            f"Fixed native core '{native_core}' has no repo-4 archival_object docs (numFound={num_found!r})"
        )
    sample_by_id: Dict[str, Dict[str, Any]] = {}
    for doc in docs:
        if isinstance(doc, Mapping) and isinstance(doc.get("id"), str):
            sample_by_id[doc["id"]] = dict(doc)
    sentinel_uri = select_repo4_ao_sentinel_uri(sample_by_id)
    sentinel_ao_id = validate_repo4_ao_sentinel_uri(sentinel_uri, native_docs_by_id=sample_by_id)
    verified_doc = solr_select_single_doc(
        solr_host_url, native_core, sentinel_uri, http_get_json=http_get_json
    )
    if not verified_doc:
        raise Gate4MutationHandoffError(
            f"Sampled repo-4 sentinel {sentinel_uri!r} could not be fetched from fixed native core '{native_core}'"
        )
    validate_repo4_ao_sentinel_uri(sentinel_uri, native_docs_by_id={sentinel_uri: verified_doc})
    return {
        "native_reference_core": native_core,
        "sentinel_uri": sentinel_uri,
        "sentinel_ao_id": sentinel_ao_id,
        "selection_provenance": "verified_sentinel_sampled_from_fixed_native_core",
        "sample_sort": "id asc",
        "sample_rows": 25,
        "verified_in_fixed_native_core": True,
        "repo4_archival_object_num_found": num_found,
        "sentinel_title": verified_doc.get("title"),
    }


def verify_preflight_repo4_sentinel_in_native_map(
    preflight_report: Mapping[str, Any],
    native_docs_by_id: Mapping[str, Any],
) -> Dict[str, Any]:
    """Validates the preflight-sampled repo-4 AO sentinel URI against the full `native_docs_by_id`
    mapping without reselecting the full-map numeric minimum."""
    if not isinstance(preflight_report, Mapping):
        raise Gate4MutationHandoffError("preflight_report must be a mapping")
    sentinel_info = preflight_report.get("repo4_progress_sentinel")
    if not isinstance(sentinel_info, Mapping):
        raise Gate4MutationHandoffError(
            "Preflight report is missing required 'repo4_progress_sentinel' mapping"
        )
    if sentinel_info.get("verified_in_fixed_native_core") is not True:
        raise Gate4MutationHandoffError(
            "Preflight repo-4 sentinel was not marked verified_in_fixed_native_core=True"
        )
    sentinel_uri = sentinel_info.get("sentinel_uri")
    if not isinstance(sentinel_uri, str) or not sentinel_uri:
        raise Gate4MutationHandoffError(
            f"Preflight report is missing valid repo-4 sentinel_uri (got {sentinel_uri!r})"
        )
    if not isinstance(native_docs_by_id, Mapping):
        raise Gate4MutationHandoffError("native_docs_by_id must be a mapping")
    sentinel_ao_id = validate_repo4_ao_sentinel_uri(
        sentinel_uri,
        native_docs_by_id=native_docs_by_id,
    )
    recorded_ao_id = sentinel_info.get("sentinel_ao_id")
    if recorded_ao_id is not None and recorded_ao_id != sentinel_ao_id:
        raise Gate4MutationHandoffError(
            f"Preflight repo-4 sentinel_ao_id={recorded_ao_id!r} does not match URI {sentinel_uri!r} "
            f"(expected {sentinel_ao_id})"
        )
    native_doc = native_docs_by_id[sentinel_uri]
    return {
        "sentinel_uri": sentinel_uri,
        "sentinel_ao_id": sentinel_ao_id,
        "selection_provenance": "verified_sentinel_sampled_from_fixed_native_core",
        "verified_in_fixed_native_core": True,
        "verified_in_full_native_reference_map": True,
        "primary_type": native_doc.get("primary_type"),
        "repository": native_doc.get("repository"),
        "sentinel_title": native_doc.get("title"),
    }


def extract_identity_nonce_from_repo2_doc(
    docs_by_id: Mapping[str, Mapping[str, Any]],
    label: str = "candidate",
) -> str:
    """Extracts and validates the 32-hex identity challenge nonce from `/repositories/2` stored JSON."""
    repo2_doc = docs_by_id.get("/repositories/2")
    if not isinstance(repo2_doc, Mapping):
        raise Gate4MutationHandoffError(f"Missing /repositories/2 document in {label} core")
    raw_json = repo2_doc.get("json")
    if not isinstance(raw_json, str):
        raise Gate4MutationHandoffError(f"Missing json string on /repositories/2 in {label} core")
    try:
        parsed = json.loads(raw_json)
    except Exception as exc:
        raise Gate4MutationHandoffError(
            f"Malformed json on /repositories/2 in {label} core: {exc}"
        ) from exc
    name = parsed.get("name") or ""
    if not isinstance(name, str) or not name.startswith(gate4_base.IDENTITY_NAME_PREFIX):
        raise Gate4MutationHandoffError(
            f"Unexpected /repositories/2 name in {label} core: {name!r}"
        )
    nonce = name[len(gate4_base.IDENTITY_NAME_PREFIX) :]
    if not re.fullmatch(r"[0-9a-f]{32}", nonce):
        raise Gate4MutationHandoffError(
            f"Malformed 32-hex identity challenge nonce in {label} /repositories/2: {name!r}"
        )
    return nonce


def solr_select_single_doc(
    solr_host_url: str,
    core_name: str,
    doc_id: str,
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
) -> Optional[Dict[str, Any]]:
    clean_base = solr_host_url.rstrip("/")
    q = urllib.parse.urlencode({"q": f'id:"{doc_id}"', "rows": "2", "wt": "json"})
    resp = http_get_json(f"{clean_base}/{core_name}/select?{q}")
    docs = (resp.get("response") or {}).get("docs") or []
    if len(docs) > 1:
        raise Gate4MutationHandoffError(f"Expected at most 1 doc for {doc_id} in {core_name}, got {len(docs)}")
    return dict(docs[0]) if len(docs) == 1 else None


def execute_guarded_mutations_during_pass(
    api: Any,
    solr_host_url: str,
    candidate_core: str,
    mutation_token: str,
    cli_launch_monotonic: float,
    cli_launch_epoch: float,
    repo4_sentinel_uri: str,
    native_docs_by_id: Optional[Mapping[str, Any]] = None,
    min_delay_sec: float = DEFAULT_MIN_MUTATION_DELAY_SEC,
    max_wait_sec: float = DEFAULT_MAX_MUTATION_WAIT_SEC,
    poll_interval_sec: float = DEFAULT_POLL_INTERVAL_SEC,
    find_cli_proc_fn: Callable[[str], Optional[Dict[str, Any]]] = find_live_aspace_cli_process,
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
    sleep_fn: Callable[[float], None] = time.sleep,
    monotonic_fn: Callable[[], float] = time.monotonic,
    time_fn: Callable[[], float] = time.time,
    mutations_evidence_path: Optional[str] = None,
) -> Dict[str, Any]:
    """Waits relative to actual `aspace-cli` child launch, verifies guards, and applies 3 repo-3 AO mutations."""
    repo4_sentinel_ao_id = validate_repo4_ao_sentinel_uri(
        repo4_sentinel_uri, native_docs_by_id=native_docs_by_id
    )
    update_uri = f"/repositories/{TARGET_REPO_ID}/archival_objects/{TARGET_UPDATE_AO_ID}"
    delete_uri = f"/repositories/{TARGET_REPO_ID}/archival_objects/{TARGET_DELETE_AO_ID}"
    parent_uri = f"/repositories/{TARGET_REPO_ID}/archival_objects/{TARGET_PARENT_AO_ID}"
    resource_uri = f"/repositories/{TARGET_REPO_ID}/resources/{TARGET_RESOURCE_ID}"

    polls: List[Dict[str, Any]] = []
    ready_guard_snapshot: Optional[Dict[str, Any]] = None

    while True:
        elapsed = monotonic_fn() - cli_launch_monotonic
        if elapsed > max_wait_sec:
            raise Gate4MutationHandoffError(
                f"Timed out after {elapsed:.2f}s waiting for repo-3 completion / repo-4 AO active guard "
                f"({repo4_sentinel_uri}) on candidate core {candidate_core}; zero mutations applied"
            )
        if elapsed < min_delay_sec:
            sleep_fn(min(poll_interval_sec, max(0.05, min_delay_sec - elapsed)))
            continue

        cli_proc = find_cli_proc_fn(candidate_core)
        if not cli_proc:
            raise Gate4MutationHandoffError(
                f"Target aspace-cli process for {candidate_core} is not alive at +{elapsed:.2f}s; "
                "aborting without mutations"
            )

        committed_num_found = solr_select_num_found(
            solr_host_url, candidate_core, http_get_json=http_get_json
        )
        if committed_num_found != 0:
            raise Gate4MutationHandoffError(
                f"Candidate core {candidate_core} already has committed numFound={committed_num_found} "
                f"at +{elapsed:.2f}s; aborting without mutations"
            )

        rt_ao_2009 = solr_realtime_get_doc(
            solr_host_url, candidate_core, update_uri, http_get_json=http_get_json
        )
        rt_ao_2010 = solr_realtime_get_doc(
            solr_host_url, candidate_core, delete_uri, http_get_json=http_get_json
        )
        rt_repo4_sentinel = solr_realtime_get_doc(
            solr_host_url, candidate_core, repo4_sentinel_uri, http_get_json=http_get_json
        )

        poll_entry = {
            "elapsed_since_cli_launch_sec": round(elapsed, 3),
            "aspace_cli_pid": cli_proc["pid"],
            "aspace_cli_vm_rss_kib": cli_proc.get("vm_rss_kib"),
            "aspace_cli_vm_hwm_kib": cli_proc.get("vm_hwm_kib"),
            "candidate_select_num_found": committed_num_found,
            "repo4_sentinel_uri": repo4_sentinel_uri,
            "repo4_sentinel_ao_id": repo4_sentinel_ao_id,
            "repo3_ao_2009_in_tlog": rt_ao_2009 is not None,
            "repo3_ao_2010_in_tlog": rt_ao_2010 is not None,
            "repo4_sentinel_in_tlog": rt_repo4_sentinel is not None,
        }
        polls.append(poll_entry)

        if rt_ao_2009 is not None and rt_ao_2010 is not None and rt_repo4_sentinel is not None:
            ready_guard_snapshot = {
                **poll_entry,
                "repo3_ao_2009_tlog_title": rt_ao_2009.get("title"),
                "repo3_ao_2010_tlog_title": rt_ao_2010.get("title"),
                "repo4_sentinel_tlog_title": rt_repo4_sentinel.get("title"),
            }
            break

        sleep_fn(poll_interval_sec)

    # Re-verify API preconditions immediately prior to mutation
    before_targets = verify_repo3_mutation_targets_read_only(api)
    before_update_rec = before_targets["update_target_ao"]["record_snapshot"]
    before_delete_rec = before_targets["delete_target_ao"]["record_snapshot"]
    before_resource_rec = api.get_json(resource_uri)
    before_parent_rec = api.get_json(parent_uri)

    mutation_started_epoch = int(time_fn())
    mutation_started_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(mutation_started_epoch))
    mutation_started_elapsed = round(monotonic_fn() - cli_launch_monotonic, 3)

    # 1. Update title on AO 2009 with expected lock_version == 0
    orig_update_title = str(before_update_rec.get("title") or "").strip()
    if not orig_update_title or "[GATE4-MUTATION-" in orig_update_title:
        raise Gate4MutationHandoffError(
            f"Refusing to mutate {update_uri} with unexpected original title: {orig_update_title!r}"
        )
    updated_title = f"{orig_update_title} [GATE4-MUTATION-UPDATE-{mutation_token}]"
    update_payload = copy.deepcopy(before_update_rec)
    if update_payload.get("lock_version") != 0:
        raise Gate4MutationHandoffError(
            f"Aborting before mutation: {update_uri} lock_version={update_payload.get('lock_version')} != 0"
        )
    update_payload["title"] = updated_title
    update_api_resp = api.post_json(update_uri, update_payload)
    if (
        update_api_resp.get("status") != "Updated"
        or update_api_resp.get("id") != TARGET_UPDATE_AO_ID
        or update_api_resp.get("lock_version") != 1
    ):
        raise Gate4MutationHandoffError(
            f"Unexpected API response updating {update_uri}: {update_api_resp!r}"
        )
    after_update_rec = api.get_json(update_uri)
    if (
        after_update_rec.get("lock_version") != 1
        or after_update_rec.get("title") != updated_title
        or after_update_rec.get("display_string") != updated_title
    ):
        raise Gate4MutationHandoffError(
            f"Post-update verification failed for {update_uri}: "
            f"lock_version={after_update_rec.get('lock_version')!r}, title={after_update_rec.get('title')!r}"
        )

    # 2. Delete leaf AO 2010 after re-confirming lock_version == 0 and zero children
    delete_check = api.get_json(delete_uri)
    delete_children_check = api.get_json(f"{delete_uri}/children")
    if delete_check.get("lock_version") != 0 or delete_children_check != []:
        raise Gate4MutationHandoffError(
            f"Refusing to delete {delete_uri}: lock_version={delete_check.get('lock_version')!r}, "
            f"children={delete_children_check!r}"
        )
    delete_api_resp = api.delete_record(delete_uri)
    if delete_api_resp.get("status") != "Deleted" or delete_api_resp.get("id") != TARGET_DELETE_AO_ID:
        raise Gate4MutationHandoffError(
            f"Unexpected API response deleting {delete_uri}: {delete_api_resp!r}"
        )
    del_status_code, del_after_body = api.get_status_and_json(delete_uri)
    if del_status_code != 404:
        raise Gate4MutationHandoffError(
            f"Expected GET {delete_uri} after delete to return 404, got {del_status_code}: {del_after_body!r}"
        )

    # 3. Create one new leaf AO beneath /repositories/3/resources/1 and parent /repositories/3/archival_objects/2001
    created_title = f"Gate 4 synthetic leaf AO [GATE4-MUTATION-CREATE-{mutation_token}]"
    create_payload = {
        "jsonmodel_type": "archival_object",
        "title": created_title,
        "level": "item",
        "publish": True,
        "restrictions_apply": False,
        "suppressed": False,
        "is_slug_auto": True,
        "resource": {"ref": resource_uri},
        "parent": {"ref": parent_uri},
        "extents": [],
        "dates": [],
        "lang_materials": [],
        "subjects": [],
        "linked_agents": [],
        "linked_events": [],
        "external_documents": [],
        "rights_statements": [],
        "instances": [],
        "accession_links": [],
        "external_ids": [],
        "notes": [
            {
                "jsonmodel_type": "note_multipart",
                "type": "scopecontent",
                "publish": True,
                "subnotes": [
                    {
                        "jsonmodel_type": "note_text",
                        "publish": True,
                        "content": f"Synthetic Gate 4 create-during-pass archival object marker {mutation_token}.",
                    }
                ],
            }
        ],
    }
    create_api_resp = api.post_json(f"/repositories/{TARGET_REPO_ID}/archival_objects", create_payload)
    created_id = create_api_resp.get("id")
    created_uri = create_api_resp.get("uri")
    if (
        create_api_resp.get("status") != "Created"
        or type(created_id) is not int
        or created_id <= TARGET_DELETE_AO_ID
        or created_uri != f"/repositories/{TARGET_REPO_ID}/archival_objects/{created_id}"
        or create_api_resp.get("lock_version") != 0
    ):
        raise Gate4MutationHandoffError(
            f"Unexpected API response creating repo-3 AO: {create_api_resp!r}"
        )

    after_create_rec = api.get_json(created_uri)
    after_create_children = api.get_json(f"{created_uri}/children")
    if (
        after_create_rec.get("uri") != created_uri
        or after_create_rec.get("title") != created_title
        or after_create_rec.get("display_string") != created_title
        or after_create_rec.get("lock_version") != 0
        or (after_create_rec.get("resource") or {}).get("ref") != resource_uri
        or (after_create_rec.get("parent") or {}).get("ref") != parent_uri
        or after_create_rec.get("position") != 8
        or after_create_children != []
    ):
        raise Gate4MutationHandoffError(
            f"Post-create verification failed for {created_uri}: {after_create_rec!r}"
        )

    mutation_finished_epoch = int(time_fn())
    mutation_finished_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(mutation_finished_epoch))
    mutation_finished_elapsed = round(monotonic_fn() - cli_launch_monotonic, 3)

    # Verify aspace-cli is STILL running and candidate core is STILL uncommitted right after mutations
    cli_proc_after = find_cli_proc_fn(candidate_core)
    if not cli_proc_after:
        raise Gate4MutationHandoffError(
            f"aspace-cli process for {candidate_core} exited before mutation window completed!"
        )
    committed_after = solr_select_num_found(
        solr_host_url, candidate_core, http_get_json=http_get_json
    )
    if committed_after != 0:
        raise Gate4MutationHandoffError(
            f"Candidate core {candidate_core} committed before mutation window completed (numFound={committed_after})!"
        )

    # Verify repo 3 AO total count is preserved at 2,010
    after_ao_ids = sorted(api.get_json(f"/repositories/{TARGET_REPO_ID}/archival_objects", {"all_ids": "true"}))
    if (
        len(after_ao_ids) != EXPECTED_REPO3_AO_COUNT
        or TARGET_DELETE_AO_ID in after_ao_ids
        or TARGET_UPDATE_AO_ID not in after_ao_ids
        or created_id not in after_ao_ids
    ):
        raise Gate4MutationHandoffError(
            f"Repo 3 AO count/membership mismatch after mutations: count={len(after_ao_ids)}, "
            f"2010_present={TARGET_DELETE_AO_ID in after_ao_ids}, "
            f"2009_present={TARGET_UPDATE_AO_ID in after_ao_ids}, "
            f"{created_id}_present={created_id in after_ao_ids}"
        )

    after_resource_rec = api.get_json(resource_uri)
    after_parent_rec = api.get_json(parent_uri)
    after_parent_children = api.get_json(f"{parent_uri}/children")

    result = {
        "status": "mutations_applied_and_audited",
        "stack_id": EXPECTED_STACK_ID,
        "candidate_core": candidate_core,
        "mutation_token": mutation_token,
        "repo4_sentinel_uri": repo4_sentinel_uri,
        "repo4_sentinel_ao_id": repo4_sentinel_ao_id,
        "repo4_sentinel_provenance": "verified_sentinel_sampled_from_fixed_native_core",
        "cli_launch_epoch_sec": round(cli_launch_epoch, 3),
        "mutation_started_epoch_sec": mutation_started_epoch,
        "mutation_started_iso": mutation_started_iso,
        "mutation_started_elapsed_since_cli_launch_sec": mutation_started_elapsed,
        "mutation_finished_epoch_sec": mutation_finished_epoch,
        "mutation_finished_iso": mutation_finished_iso,
        "mutation_finished_elapsed_since_cli_launch_sec": mutation_finished_elapsed,
        "pre_mutation_guard": ready_guard_snapshot,
        "post_mutation_process_guard": {
            "aspace_cli_pid": cli_proc_after["pid"],
            "aspace_cli_alive": True,
            "aspace_cli_vm_rss_kib": cli_proc_after.get("vm_rss_kib"),
            "aspace_cli_vm_hwm_kib": cli_proc_after.get("vm_hwm_kib"),
            "candidate_select_num_found": committed_after,
        },
        "poll_history": polls,
        "repo3_ao_count_before": EXPECTED_REPO3_AO_COUNT,
        "repo3_ao_count_after": len(after_ao_ids),
        "net_repo3_ao_count_preserved": len(after_ao_ids) == EXPECTED_REPO3_AO_COUNT,
        "mutations": {
            "update": {
                "uri": update_uri,
                "id": TARGET_UPDATE_AO_ID,
                "expected_lock_version_before": 0,
                "lock_version_after": after_update_rec.get("lock_version"),
                "title_before": before_update_rec.get("title"),
                "title_after": after_update_rec.get("title"),
                "system_mtime_before": before_update_rec.get("system_mtime"),
                "system_mtime_after": after_update_rec.get("system_mtime"),
                "api_response": update_api_resp,
                "before_record": before_update_rec,
                "after_record": after_update_rec,
            },
            "delete": {
                "uri": delete_uri,
                "id": TARGET_DELETE_AO_ID,
                "expected_lock_version_before": 0,
                "is_leaf_verified_zero_children": True,
                "title_before": before_delete_rec.get("title"),
                "system_mtime_before": before_delete_rec.get("system_mtime"),
                "api_response": delete_api_resp,
                "post_delete_get_http_status": del_status_code,
                "before_record": before_delete_rec,
            },
            "create": {
                "uri": created_uri,
                "id": created_id,
                "resource_ref": resource_uri,
                "parent_ref": parent_uri,
                "lock_version_after": after_create_rec.get("lock_version"),
                "position_after": after_create_rec.get("position"),
                "title_after": after_create_rec.get("title"),
                "system_mtime_after": after_create_rec.get("system_mtime"),
                "is_leaf_verified_zero_children": after_create_children == [],
                "api_response": create_api_resp,
                "after_record": after_create_rec,
            },
        },
        "touched_ancestors": {
            "resource": {
                "uri": resource_uri,
                "id": TARGET_RESOURCE_ID,
                "lock_version_before": before_resource_rec.get("lock_version"),
                "lock_version_after": after_resource_rec.get("lock_version"),
                "system_mtime_before": before_resource_rec.get("system_mtime"),
                "system_mtime_after": after_resource_rec.get("system_mtime"),
            },
            "parent_ao": {
                "uri": parent_uri,
                "id": TARGET_PARENT_AO_ID,
                "lock_version_before": before_parent_rec.get("lock_version"),
                "lock_version_after": after_parent_rec.get("lock_version"),
                "system_mtime_before": before_parent_rec.get("system_mtime"),
                "system_mtime_after": after_parent_rec.get("system_mtime"),
                "children_count_after": len(after_parent_children),
            },
        },
    }

    if mutations_evidence_path:
        write_mode_0600_json(mutations_evidence_path, result)

    return result


def verify_candidate_missed_repo3_mutations(
    cand_docs_by_id: Mapping[str, Mapping[str, Any]],
    native_docs_by_id: Mapping[str, Mapping[str, Any]],
    cand_marker_leaks: List[str],
    mutation_result: Mapping[str, Any],
    candidate_identity_nonce: Optional[str] = None,
) -> Dict[str, Any]:
    """Verifies the Rust candidate matches the pre-mutation native reference (except the 2 nonce records)
    and therefore missed the three repo-3 mutations that occurred after repo 3 was indexed."""
    doc_candidate_nonce = extract_identity_nonce_from_repo2_doc(cand_docs_by_id, label="candidate")
    if candidate_identity_nonce is None:
        candidate_identity_nonce = doc_candidate_nonce
    elif candidate_identity_nonce != doc_candidate_nonce:
        raise Gate4MutationHandoffError(
            f"Provided candidate_identity_nonce {candidate_identity_nonce!r} does not match "
            f"candidate /repositories/2 document nonce {doc_candidate_nonce!r}"
        )

    parity = gate3_harness.run_slice_parity_comparison(
        dict(cand_docs_by_id),
        dict(native_docs_by_id),
        list(cand_marker_leaks),
    )

    if parity.get("candidate_total_docs") != EXPECTED_TOTAL_DOCS:
        raise Gate4MutationHandoffError(
            f"Candidate total docs {parity.get('candidate_total_docs')} != {EXPECTED_TOTAL_DOCS}"
        )
    if parity.get("native_total_docs") != EXPECTED_TOTAL_DOCS:
        raise Gate4MutationHandoffError(
            f"Native total docs {parity.get('native_total_docs')} != {EXPECTED_TOTAL_DOCS}"
        )
    if (
        parity.get("missing_in_candidate_count") != 0
        or parity.get("extra_in_candidate_count") != 0
        or parity.get("candidate_marker_leaks_count") != 0
    ):
        raise Gate4MutationHandoffError(
            f"Candidate has unexpected missing/extra/marker leaks vs pre-mutation native reference: {parity!r}"
        )
    if (
        parity.get("records_with_exact_equality") != EXPECTED_EXACT_RECORDS_VS_NATIVE
        or parity.get("records_with_differences") != 2
        or sorted(parity.get("embedded_json_mismatches_sample") or []) != gate4_base.EXPECTED_NONCE_DIFF_URIS
    ):
        raise Gate4MutationHandoffError(
            "Candidate vs pre-mutation native reference differed by more than the 2 challenge-nonce records: "
            f"{parity!r}"
        )

    # Verify the 2 differing records differ solely by the challenge nonce
    nonce_records_verified: Dict[str, Any] = {}
    for doc_id in gate4_base.EXPECTED_NONCE_DIFF_URIS:
        c_doc = dict(cand_docs_by_id[doc_id])
        n_doc = dict(native_docs_by_id[doc_id])
        c_doc.pop("_version_", None)
        n_doc.pop("_version_", None)
        c_json = c_doc.get("json")
        n_json = n_doc.get("json")
        if not isinstance(c_json, str) or not isinstance(n_json, str):
            raise Gate4MutationHandoffError(f"Missing json string on {doc_id}")
        if candidate_identity_nonce not in c_json:
            raise Gate4MutationHandoffError(
                f"Candidate json for {doc_id} lacks expected nonce {candidate_identity_nonce}"
            )
        if gate4_base.EXPECTED_NATIVE_NONCE not in n_json:
            raise Gate4MutationHandoffError(
                f"Native json for {doc_id} lacks expected nonce {gate4_base.EXPECTED_NATIVE_NONCE}"
            )
        if c_json.replace(candidate_identity_nonce, gate4_base.EXPECTED_NATIVE_NONCE) != n_json:
            raise Gate4MutationHandoffError(
                f"Stored json for {doc_id} differs by more than challenge nonce"
            )
        c_rest = {k: v for k, v in c_doc.items() if k != "json"}
        n_rest = {k: v for k, v in n_doc.items() if k != "json"}
        if c_rest != n_rest:
            raise Gate4MutationHandoffError(f"Non-json fields for {doc_id} differ")
        nonce_records_verified[doc_id] = {
            "candidate_nonce": candidate_identity_nonce,
            "native_nonce": gate4_base.EXPECTED_NATIVE_NONCE,
            "identical_after_nonce_substitution": True,
        }

    update_info = mutation_result["mutations"]["update"]
    delete_info = mutation_result["mutations"]["delete"]
    create_info = mutation_result["mutations"]["create"]

    update_uri = update_info["uri"]
    delete_uri = delete_info["uri"]
    create_uri = create_info["uri"]

    cand_update_doc = cand_docs_by_id.get(update_uri)
    cand_delete_doc = cand_docs_by_id.get(delete_uri)
    cand_create_doc = cand_docs_by_id.get(create_uri)

    if not cand_update_doc or cand_update_doc.get("title") != update_info["title_before"]:
        raise Gate4MutationHandoffError(
            f"Expected candidate core to have pre-mutation title {update_info['title_before']!r} "
            f"for {update_uri}, got {(cand_update_doc or {}).get('title')!r}"
        )
    if not cand_delete_doc or cand_delete_doc.get("title") != delete_info["title_before"]:
        raise Gate4MutationHandoffError(
            f"Expected candidate core to still contain deleted leaf AO {delete_uri} with title "
            f"{delete_info['title_before']!r}, got {cand_delete_doc!r}"
        )
    if cand_create_doc is not None:
        raise Gate4MutationHandoffError(
            f"Expected candidate core to miss newly created AO {create_uri}, but it was present!"
        )

    return {
        "candidate_missed_all_three_mutations": True,
        "pre_mutation_parity_except_nonce": True,
        "parity_result": parity,
        "nonce_records_verified": nonce_records_verified,
        "candidate_pre_handoff_mutation_state": {
            "update_target_uri": update_uri,
            "update_target_candidate_title": cand_update_doc.get("title"),
            "update_target_has_pre_mutation_title": True,
            "delete_target_uri": delete_uri,
            "delete_target_present_in_candidate": True,
            "create_target_uri": create_uri,
            "create_target_present_in_candidate": False,
        },
    }


def verify_promoted_core_after_mutation_round(
    solr_host_url: str,
    promoted_core: str,
    mutation_result: Mapping[str, Any],
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
) -> Dict[str, Any]:
    """Verifies exact Solr documents, counts, and absence of deleted record in `promoted_core` after the Ruby round."""
    total_num_found = solr_select_num_found(solr_host_url, promoted_core, http_get_json=http_get_json)
    if total_num_found != EXPECTED_TOTAL_DOCS:
        raise Gate4MutationHandoffError(
            f"Promoted core '{promoted_core}' total numFound={total_num_found} after Ruby round; "
            f"expected {EXPECTED_TOTAL_DOCS} (no lost docs)"
        )

    repo3_ao_count = solr_select_num_found(
        solr_host_url,
        promoted_core,
        fq=f'repository:"/repositories/{TARGET_REPO_ID}" AND primary_type:archival_object',
        http_get_json=http_get_json,
    )
    if repo3_ao_count != EXPECTED_REPO3_AO_COUNT:
        raise Gate4MutationHandoffError(
            f"Promoted core '{promoted_core}' repo 3 archival_object count={repo3_ao_count}; "
            f"expected {EXPECTED_REPO3_AO_COUNT}"
        )

    per_type_counts: Dict[str, int] = {}
    for ptype, expected_cnt in EXPECTED_TYPE_COUNTS.items():
        actual_cnt = solr_select_num_found(
            solr_host_url,
            promoted_core,
            fq=f"primary_type:{ptype}",
            http_get_json=http_get_json,
        )
        if actual_cnt != expected_cnt:
            raise Gate4MutationHandoffError(
                f"Promoted core '{promoted_core}' primary_type:{ptype} count={actual_cnt} != expected {expected_cnt}"
            )
        per_type_counts[ptype] = actual_cnt

    update_info = mutation_result["mutations"]["update"]
    delete_info = mutation_result["mutations"]["delete"]
    create_info = mutation_result["mutations"]["create"]
    resource_info = mutation_result["touched_ancestors"]["resource"]

    # 1. Verify updated AO 2009 in Solr
    updated_doc = solr_select_single_doc(
        solr_host_url, promoted_core, update_info["uri"], http_get_json=http_get_json
    )
    if not updated_doc:
        raise Gate4MutationHandoffError(f"Updated AO {update_info['uri']} missing from Solr after Ruby round")
    if updated_doc.get("title") != update_info["title_after"]:
        raise Gate4MutationHandoffError(
            f"Solr title for {update_info['uri']} is {updated_doc.get('title')!r}, "
            f"expected {update_info['title_after']!r}"
        )
    updated_embedded = json.loads(updated_doc.get("json") or "{}")
    if (
        updated_embedded.get("uri") != update_info["uri"]
        or updated_embedded.get("title") != update_info["title_after"]
        or updated_embedded.get("display_string") != update_info["title_after"]
        or updated_embedded.get("lock_version") != 1
    ):
        raise Gate4MutationHandoffError(
            f"Embedded JSON mismatch on updated AO {update_info['uri']} in Solr: "
            f"title={updated_embedded.get('title')!r}, lock_version={updated_embedded.get('lock_version')!r}"
        )
    if gate3_harness.check_for_marker_leaks(updated_doc):
        raise Gate4MutationHandoffError(f"Marker leak detected in updated AO {update_info['uri']}")

    # 2. Verify deleted leaf AO 2010 is absent from Solr
    deleted_doc = solr_select_single_doc(
        solr_host_url, promoted_core, delete_info["uri"], http_get_json=http_get_json
    )
    if deleted_doc is not None:
        raise Gate4MutationHandoffError(
            f"Deleted leaf AO {delete_info['uri']} is still present in Solr after Ruby round!"
        )

    # 3. Verify created leaf AO in Solr
    created_doc = solr_select_single_doc(
        solr_host_url, promoted_core, create_info["uri"], http_get_json=http_get_json
    )
    if not created_doc:
        raise Gate4MutationHandoffError(f"Created AO {create_info['uri']} missing from Solr after Ruby round")
    if (
        created_doc.get("title") != create_info["title_after"]
        or created_doc.get("primary_type") != "archival_object"
        or created_doc.get("repository") != f"/repositories/{TARGET_REPO_ID}"
        or created_doc.get("publish") is not True
    ):
        raise Gate4MutationHandoffError(
            f"Created AO {create_info['uri']} Solr fields mismatch: {created_doc!r}"
        )
    created_embedded = json.loads(created_doc.get("json") or "{}")
    if (
        created_embedded.get("uri") != create_info["uri"]
        or created_embedded.get("title") != create_info["title_after"]
        or created_embedded.get("display_string") != create_info["title_after"]
        or created_embedded.get("lock_version") != 0
        or (created_embedded.get("resource") or {}).get("ref") != create_info["resource_ref"]
        or (created_embedded.get("parent") or {}).get("ref") != create_info["parent_ref"]
    ):
        raise Gate4MutationHandoffError(
            f"Embedded JSON mismatch on created AO {create_info['uri']} in Solr"
        )
    if gate3_harness.check_for_marker_leaks(created_doc):
        raise Gate4MutationHandoffError(f"Marker leak detected in created AO {create_info['uri']}")

    # 4. Verify touched resource /repositories/3/resources/1 in Solr
    resource_doc = solr_select_single_doc(
        solr_host_url, promoted_core, resource_info["uri"], http_get_json=http_get_json
    )
    if not resource_doc:
        raise Gate4MutationHandoffError(f"Resource {resource_info['uri']} missing from Solr after Ruby round")
    resource_embedded = json.loads(resource_doc.get("json") or "{}")
    if resource_embedded.get("system_mtime") != resource_info["system_mtime_after"]:
        raise Gate4MutationHandoffError(
            f"Resource {resource_info['uri']} embedded system_mtime={resource_embedded.get('system_mtime')!r} "
            f"!= expected {resource_info['system_mtime_after']!r}"
        )

    return {
        "promoted_core": promoted_core,
        "total_num_found": total_num_found,
        "expected_total_docs": EXPECTED_TOTAL_DOCS,
        "no_lost_docs": total_num_found == EXPECTED_TOTAL_DOCS,
        "repo3_archival_object_count": repo3_ao_count,
        "per_type_counts": per_type_counts,
        "updated_ao_verified": {
            "uri": update_info["uri"],
            "solr_title": updated_doc.get("title"),
            "embedded_lock_version": updated_embedded.get("lock_version"),
            "embedded_system_mtime": updated_embedded.get("system_mtime"),
            "marker_leaks": 0,
        },
        "deleted_ao_verified": {
            "uri": delete_info["uri"],
            "present_in_solr": False,
        },
        "created_ao_verified": {
            "uri": create_info["uri"],
            "solr_title": created_doc.get("title"),
            "embedded_lock_version": created_embedded.get("lock_version"),
            "embedded_position": created_embedded.get("position"),
            "embedded_system_mtime": created_embedded.get("system_mtime"),
            "marker_leaks": 0,
        },
        "touched_resource_verified": {
            "uri": resource_info["uri"],
            "embedded_system_mtime": resource_embedded.get("system_mtime"),
        },
    }


def perform_core_admin_swap_for_candidate(
    solr_host_url: str,
    candidate_core: str,
    core: str,
    other: str,
    http_get_json: Callable[[str], Dict[str, Any]] = gate4_base.default_http_get_json,
) -> Dict[str, Any]:
    if solr_host_url.rstrip("/") != "http://127.0.0.1:28983/solr":
        raise Gate4MutationHandoffError(
            f"Safety violation: CoreAdmin SWAP is only permitted on http://127.0.0.1:28983/solr (got {solr_host_url!r})"
        )
    if not re.fullmatch(r"stage_staff_[0-9]{8}_[0-9]{6}_[0-9]+_[0-9a-f]{8}_candidate", candidate_core):
        raise Gate4MutationHandoffError(f"Invalid candidate_core name for SWAP: {candidate_core!r}")
    if {core, other} != {DEFAULT_CORE, candidate_core}:
        raise Gate4MutationHandoffError(
            f"Safety violation: CoreAdmin SWAP only permitted between {DEFAULT_CORE!r} and {candidate_core!r}"
        )
    swap_url = (
        f"{solr_host_url.rstrip('/')}/admin/cores?"
        + urllib.parse.urlencode({"action": "SWAP", "core": core, "other": other, "wt": "json"})
    )
    resp = http_get_json(swap_url)
    if (resp.get("responseHeader") or {}).get("status") != 0:
        raise Gate4MutationHandoffError(f"CoreAdmin SWAP failed: {resp!r}")
    return resp


def verify_swapped_candidate_cores(
    candidate_core: str,
    pre_snapshot: Mapping[str, Mapping[str, Any]],
    post_snapshot: Mapping[str, Mapping[str, Any]],
) -> None:
    pre_default = pre_snapshot[DEFAULT_CORE]
    pre_cand = pre_snapshot[candidate_core]
    post_default = post_snapshot[DEFAULT_CORE]
    post_cand = post_snapshot[candidate_core]

    if (
        post_default.get("numFound") != EXPECTED_TOTAL_DOCS
        or post_default.get("instanceDir") != pre_cand.get("instanceDir")
        or post_default.get("dataDir") != pre_cand.get("dataDir")
        or post_default.get("version") != pre_cand.get("version")
    ):
        raise Gate4MutationHandoffError(
            f"Ambiguous post-SWAP state for '{DEFAULT_CORE}': got {post_default!r}, "
            f"expected instanceDir/dataDir/version/numFound from {pre_cand!r}"
        )
    if (
        post_cand.get("numFound") != 0
        or post_cand.get("instanceDir") != pre_default.get("instanceDir")
        or post_cand.get("dataDir") != pre_default.get("dataDir")
        or post_cand.get("version") != pre_default.get("version")
    ):
        raise Gate4MutationHandoffError(
            f"Ambiguous post-SWAP state for '{candidate_core}': got {post_cand!r}, "
            f"expected instanceDir/dataDir/version/numFound from {pre_default!r}"
        )


def verify_restored_candidate_cores(
    candidate_core: str,
    pre_snapshot: Mapping[str, Mapping[str, Any]],
    after_round_snapshot: Mapping[str, Mapping[str, Any]],
    restored_snapshot: Mapping[str, Mapping[str, Any]],
) -> None:
    # Default core `archivesspace` must be back to its exact original empty instance
    pre_def = pre_snapshot[DEFAULT_CORE]
    rest_def = restored_snapshot[DEFAULT_CORE]
    for field in ("numFound", "numDocs", "maxDoc", "deletedDocs", "version", "instanceDir", "dataDir", "commitTimeMSec"):
        if pre_def.get(field) != rest_def.get(field):
            raise Gate4MutationHandoffError(
                f"Post-rollback verification failed for core '{DEFAULT_CORE}' field {field}: "
                f"original={pre_def.get(field)!r}, restored={rest_def.get(field)!r}"
            )

    # Candidate core must point back to its own instanceDir/dataDir and hold the post-round index state
    pre_cand = pre_snapshot[candidate_core]
    round_promoted = after_round_snapshot[DEFAULT_CORE]
    rest_cand = restored_snapshot[candidate_core]
    if (
        rest_cand.get("instanceDir") != pre_cand.get("instanceDir")
        or rest_cand.get("dataDir") != pre_cand.get("dataDir")
        or rest_cand.get("numFound") != EXPECTED_TOTAL_DOCS
        or rest_cand.get("version") != round_promoted.get("version")
        or rest_cand.get("commitTimeMSec") != round_promoted.get("commitTimeMSec")
    ):
        raise Gate4MutationHandoffError(
            f"Post-rollback verification failed for candidate core '{candidate_core}': "
            f"restored={rest_cand!r}, expected instanceDir={pre_cand.get('instanceDir')!r} and "
            f"post-round version/commit from {round_promoted!r}"
        )

    # All other retained cores must be completely untouched
    for untouched_core in (
        EXPECTED_IDENTITY_CORE,
        FIXED_NATIVE_CORE,
        PRIOR_WATERMARKED_CANDIDATE_CORE,
        PRIOR_UNWATERMARKED_CANDIDATE_CORE,
        ACCIDENTAL_EMPTY_CANDIDATE_CORE,
    ):
        pre_c = pre_snapshot[untouched_core]
        post_c = restored_snapshot[untouched_core]
        for field in ("numFound", "numDocs", "maxDoc", "deletedDocs", "version", "instanceDir", "dataDir", "commitTimeMSec"):
            if pre_c.get(field) != post_c.get(field):
                raise Gate4MutationHandoffError(
                    f"Untouched core '{untouched_core}' changed on field {field}: "
                    f"before={pre_c.get(field)!r}, after={post_c.get(field)!r}"
                )


def execute_post_pass_ruby_handoff_orchestration(
    solr_url: str,
    candidate_core: str,
    watermark_epoch: int,
    mutation_result: Mapping[str, Any],
    state_dir_name: str,
    http_get_json: Callable[[str], Dict[str, Any]],
    run_jruby_preflight_mutations_fn: Callable[[int, List[int], List[int], List[str]], Dict[str, Any]],
    run_jruby_mutation_round_fn: Callable[[str, int, List[int], List[int], List[str], int], Dict[str, Any]],
    swap_fn: Optional[Callable[[str, str, str, str], Dict[str, Any]]] = None,
    timeout_sec: int = 120,
) -> Dict[str, Any]:
    """Executes feed preflight, reversible CoreAdmin SWAP, one Ruby PeriodicIndexer round,
    Solr post-round verification, and guaranteed CoreAdmin SWAP rollback."""
    if swap_fn is None:
        swap_fn = lambda url, cand, c1, c2: perform_core_admin_swap_for_candidate(
            url, cand, c1, c2, http_get_json=http_get_json
        )

    modified_since = max(watermark_epoch - WINDOW_SECONDS, 0)
    updated_ao_id = int(mutation_result["mutations"]["update"]["id"])
    created_ao_id = int(mutation_result["mutations"]["create"]["id"])
    deleted_uri = str(mutation_result["mutations"]["delete"]["uri"])
    expected_ao_ids = sorted([updated_ao_id, created_ao_id])
    expected_resource_ids = [TARGET_RESOURCE_ID]
    expected_deleted_uris = [deleted_uri]

    jruby_preflight = run_jruby_preflight_mutations_fn(
        modified_since,
        expected_ao_ids,
        expected_resource_ids,
        expected_deleted_uris,
    )
    if jruby_preflight.get("status") != "ok":
        raise Gate4MutationHandoffError(f"JRuby mutation feed preflight failed: {jruby_preflight!r}")

    pre_swap_cores = gate4_base.fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
    if candidate_core not in pre_swap_cores:
        raise Gate4MutationHandoffError(f"Candidate core {candidate_core} missing from pre-swap snapshot")
    if pre_swap_cores[DEFAULT_CORE].get("numFound") != 0:
        raise Gate4MutationHandoffError(
            f"Pre-swap default core '{DEFAULT_CORE}' is not empty: {pre_swap_cores[DEFAULT_CORE]!r}"
        )
    if pre_swap_cores[candidate_core].get("numFound") != EXPECTED_TOTAL_DOCS:
        raise Gate4MutationHandoffError(
            f"Pre-swap candidate core '{candidate_core}' count mismatch: {pre_swap_cores[candidate_core]!r}"
        )

    swapped_verified = False
    ambiguous_state = False
    post_swap_cores: Optional[Dict[str, Dict[str, Any]]] = None
    after_round_cores: Optional[Dict[str, Dict[str, Any]]] = None
    restored_cores: Optional[Dict[str, Dict[str, Any]]] = None
    jruby_round_result: Optional[Dict[str, Any]] = None
    solr_post_round_verification: Optional[Dict[str, Any]] = None
    swap_forward_resp: Optional[Dict[str, Any]] = None
    swap_rollback_resp: Optional[Dict[str, Any]] = None
    primary_error: Optional[Exception] = None
    rollback_error: Optional[Exception] = None

    try:
        try:
            swap_forward_resp = swap_fn(solr_url, candidate_core, DEFAULT_CORE, candidate_core)
        except Exception as exc:
            ambiguous_state = True
            raise Gate4MutationHandoffError(
                f"CoreAdmin SWAP call failed ({exc}); stopping without blind second swap"
            ) from exc

        post_swap_cores = gate4_base.fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
        try:
            verify_swapped_candidate_cores(candidate_core, pre_swap_cores, post_swap_cores)
            swapped_verified = True
        except Exception:
            ambiguous_state = True
            raise

        jruby_round_result = run_jruby_mutation_round_fn(
            state_dir_name,
            watermark_epoch,
            expected_ao_ids,
            expected_resource_ids,
            expected_deleted_uris,
            timeout_sec,
        )
        if jruby_round_result.get("status") != "ok":
            raise Gate4MutationHandoffError(
                f"JRuby mutation handoff round returned non-ok: {jruby_round_result!r}"
            )

        solr_post_round_verification = verify_promoted_core_after_mutation_round(
            solr_url,
            DEFAULT_CORE,
            mutation_result,
            http_get_json=http_get_json,
        )
        after_round_cores = gate4_base.fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
    except Exception as exc:
        primary_error = exc
    finally:
        if swapped_verified and not ambiguous_state:
            try:
                swap_rollback_resp = swap_fn(solr_url, candidate_core, DEFAULT_CORE, candidate_core)
                restored_cores = gate4_base.fetch_all_cores_snapshot(solr_url, http_get_json=http_get_json)
                ref_after_round = after_round_cores if after_round_cores is not None else post_swap_cores
                assert ref_after_round is not None
                verify_restored_candidate_cores(
                    candidate_core, pre_swap_cores, ref_after_round, restored_cores
                )
            except Exception as rb_exc:
                rollback_error = rb_exc

    if primary_error is not None and rollback_error is not None:
        raise Gate4MutationHandoffError(
            f"Primary error: {primary_error}; Rollback ALSO failed: {rollback_error}"
        ) from primary_error
    if primary_error is not None:
        raise primary_error
    if rollback_error is not None:
        raise Gate4MutationHandoffError(f"CoreAdmin SWAP rollback failed: {rollback_error}") from rollback_error

    return {
        "status": "mutation_handoff_round_passed_and_rolled_back",
        "candidate_core": candidate_core,
        "watermark_epoch": watermark_epoch,
        "modified_since_with_overlap": modified_since,
        "state_dir_name": state_dir_name,
        "swap_executed": True,
        "rollback_executed": True,
        "swap_forward_response": swap_forward_resp,
        "swap_rollback_response": swap_rollback_resp,
        "jruby_preflight": jruby_preflight,
        "jruby_handoff_and_round": jruby_round_result,
        "solr_post_round_verification": solr_post_round_verification,
        "core_snapshots": {
            "before_swap": pre_swap_cores,
            "after_swap": post_swap_cores,
            "after_round_before_rollback": after_round_cores,
            "after_rollback": restored_cores,
        },
    }


def run_jruby_helper_in_backend_container(
    backend_container: str,
    args: List[str],
    timeout_sec: int = 180,
) -> Dict[str, Any]:
    if backend_container != f"archivalist-profiling-backend-{EXPECTED_STACK_ID}":
        raise Gate4MutationHandoffError(f"Unexpected backend_container: {backend_container!r}")
    cmd = [
        "docker",
        "exec",
        "-w",
        "/archivesspace",
        backend_container,
        "/archivesspace/scripts/jruby",
        "/archivesspace/rust-tools/scripts/gate4_isolated_handoff_round.rb",
        *args,
    ]
    proc = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=timeout_sec,
        check=False,
    )
    if proc.returncode != 0:
        raise Gate4MutationHandoffError(
            f"JRuby helper exited {proc.returncode}: "
            f"stderr={redact_secrets(proc.stderr[-3000:])!r}, stdout={redact_secrets(proc.stdout[-3000:])!r}"
        )
    lines = [line.strip() for line in proc.stdout.splitlines() if line.strip()]
    if not lines:
        raise Gate4MutationHandoffError("JRuby helper produced empty stdout")
    return json.loads(lines[-1])


def collect_process_snapshots(state: Mapping[str, Any]) -> Tuple[str, str, str]:
    host_ps = subprocess.run(
        ["ps", "-eo", "pid=,comm=,args="],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    backend_ps = subprocess.run(
        ["docker", "exec", state["containers"]["backend"], "ps", "aux"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    app_ps = ""
    app_info = gate3_harness.docker_inspect(state.get("app_container", "archivesspaceai-app-1"))
    if app_info and bool((app_info.get("State") or {}).get("Running")):
        app_ps = subprocess.run(
            ["docker", "exec", state.get("app_container", "archivesspaceai-app-1"), "ps", "aux"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    return host_ps, backend_ps, app_ps


def run_read_only_preflight(
    state_path: str,
    native_core: str = FIXED_NATIVE_CORE,
) -> Tuple[Dict[str, Any], Dict[str, Any], AspaceApiSession]:
    git_branch = subprocess.run(
        ["git", "-C", REPO_ROOT, "rev-parse", "--abbrev-ref", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    git_head = subprocess.run(
        ["git", "-C", REPO_ROOT, "rev-parse", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    git_status = subprocess.run(
        ["git", "-C", REPO_ROOT, "status", "--porcelain", "-uno"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    git_dirty = bool(git_status)

    if git_branch != EXPECTED_BRANCH:
        raise Gate4MutationHandoffError(
            f"Expected branch {EXPECTED_BRANCH!r}, got {git_branch!r}"
        )

    cli_bin = os.path.join(REPO_ROOT, "rust-tools", "target", "release", "aspace-cli")
    binary_provenance = gate3_harness.verify_binary_provenance(cli_bin)

    if not os.path.isfile(state_path):
        raise Gate4MutationHandoffError(f"State file not found: {state_path}")
    with open(state_path, "r", encoding="utf-8") as fh:
        state = json.load(fh)
    if "admin_password" in state:
        gate3_harness.register_secret(state["admin_password"])
    if "db_root_password" in state:
        gate3_harness.register_secret(state["db_root_password"])

    stack_identity = verify_read_only_stack_identity_any_nonce(
        state,
        docker_inspect_fn=gate3_harness.docker_inspect,
        http_get_json=gate4_base.default_http_get_json,
    )

    repo_free_gib = get_free_disk_gib(REPO_ROOT)
    docker_free_gib = get_free_disk_gib("/var/lib/docker")
    mem_avail_gib = gate3_harness.get_mem_available_gib()
    if repo_free_gib < MIN_FREE_DISK_GIB or docker_free_gib < MIN_FREE_DISK_GIB:
        raise Gate4MutationHandoffError(
            f"Insufficient free disk space: repo={repo_free_gib:.2f} GiB, "
            f"docker={docker_free_gib:.2f} GiB (min {MIN_FREE_DISK_GIB} GiB)"
        )
    if mem_avail_gib < MIN_AVAILABLE_MEM_GIB:
        raise Gate4MutationHandoffError(
            f"Insufficient available memory: {mem_avail_gib:.2f} GiB (min {MIN_AVAILABLE_MEM_GIB} GiB)"
        )

    host_ps, backend_ps, app_ps = collect_process_snapshots(state)
    process_check = gate4_base.check_no_active_benchmark_or_indexer_processes(
        host_ps, backend_ps, app_ps
    )

    solr_url = state["solr_host_url"]
    cores_snapshot = gate4_base.fetch_all_cores_snapshot(solr_url)
    gate4_base.verify_pre_swap_cores(cores_snapshot)

    v_mode, active_arks = gate3_harness.verify_active_app_arks_enabled(
        state["backend_host_url"],
        state["admin_user"],
        state["admin_password"],
    )
    if v_mode != "genuine_active_process" or active_arks is not False:
        raise Gate4MutationHandoffError(
            f"Unexpected active app arks_enabled verification: mode={v_mode!r}, arks_enabled={active_arks!r}"
        )

    tz_out = subprocess.run(
        ["docker", "exec", state["containers"]["backend"], "date", "+%Z"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    if tz_out != "UTC":
        raise Gate4MutationHandoffError(f"Expected container timezone UTC, got {tz_out!r}")

    api = AspaceApiSession.login(
        state["backend_host_url"],
        state["admin_user"],
        state["admin_password"],
    )
    repo3_targets = verify_repo3_mutation_targets_read_only(api)
    repo4_sentinel_preflight = verify_repo4_sentinel_in_fixed_native_core(
        solr_url,
        native_core,
        http_get_json=gate4_base.default_http_get_json,
    )

    preflight_report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "preflight_passed",
        "gate": "Gate 4",
        "gate_passed": False,
        "git_branch": git_branch,
        "git_head": git_head,
        "git_dirty": git_dirty,
        "binary_provenance": binary_provenance,
        "stack_identity": stack_identity,
        "system_resources": {
            "repo_free_disk_gib": round(repo_free_gib, 2),
            "docker_free_disk_gib": round(docker_free_gib, 2),
            "min_free_disk_gib": MIN_FREE_DISK_GIB,
            "mem_available_gib": round(mem_avail_gib, 2),
            "min_available_mem_gib": MIN_AVAILABLE_MEM_GIB,
        },
        "process_check": process_check,
        "active_arks_enabled": active_arks,
        "backend_time_zone": tz_out,
        "solr_cores_before": cores_snapshot,
        "repo3_mutation_targets": repo3_targets,
        "repo4_progress_sentinel": repo4_sentinel_preflight,
    }
    return state, preflight_report, api


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Gate 4 bounded create/update/delete-during-Rust-pass and Ruby handoff harness (stack 844ef09c)."
    )
    parser.add_argument(
        "--state",
        default=DEFAULT_STATE_PATH,
        help="Path to profiling_stack_844ef09c.json",
    )
    parser.add_argument(
        "--native-core",
        default=FIXED_NATIVE_CORE,
        help="Fixed read-only native reference Solr core name",
    )
    parser.add_argument(
        "--concurrency",
        type=int,
        default=DEFAULT_CONCURRENCY,
        choices=range(1, 33),
        help="Concurrent Rust fetch/build batches (default: 12)",
    )
    parser.add_argument(
        "--min-mutation-delay-sec",
        type=float,
        default=DEFAULT_MIN_MUTATION_DELAY_SEC,
        help="Minimum seconds after aspace-cli launch before polling/applying mutations (default: 45.0)",
    )
    parser.add_argument(
        "--max-mutation-wait-sec",
        type=float,
        default=DEFAULT_MAX_MUTATION_WAIT_SEC,
        help="Maximum seconds after aspace-cli launch to wait for repo-3 done & repo-4 active guard (default: 240.0)",
    )
    parser.add_argument(
        "--output-dir",
        default=BENCHMARKS_DIR,
        help="Directory for mode-0600 evidence JSON reports",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Run read-only preflight only and write mode-0600 preflight report without starting candidate or mutating DB/Solr",
    )
    return parser.parse_args(argv)


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_args(argv)
    run_nonce = f"{time.strftime('%Y%m%d_%H%M%S', time.gmtime())}_{os.getpid()}_{secrets.token_hex(4)}"
    mutation_token = f"20261124_{secrets.token_hex(4)}"

    print(f"=== Gate 4 Mutation-During-Pass & Ruby Handoff Harness (run_nonce={run_nonce}) ===", flush=True)
    state, preflight_report, api = run_read_only_preflight(args.state, native_core=args.native_core)
    preflight_path = os.path.join(
        args.output_dir,
        f"gate4_mutation_during_pass_preflight_{run_nonce}.json",
    )
    write_mode_0600_json(preflight_path, preflight_report)
    print(f"[PREFLIGHT] Read-only preflight passed. Evidence: {preflight_path}", flush=True)

    if args.dry_run:
        print("[DRY RUN] Stopping cleanly after read-only preflight with zero DB or Solr mutations.", flush=True)
        return 0

    solr_host_url = state["solr_host_url"].rstrip("/")
    backend_host_url = state["backend_host_url"].rstrip("/")

    # 1. Stream fixed read-only native reference documents (~155s, ~9 GiB in parent Python process)
    print(f"\n[STEP 1] Streaming fixed native reference core '{args.native_core}'...", flush=True)
    t_nat_start = time.perf_counter()
    native_docs_by_id, native_marker_leaks = gate3_harness.stream_fetch_solr_core_documents(
        solr_host_url,
        args.native_core,
        fq=None,
        batch_size=1000,
    )
    t_nat_elapsed = time.perf_counter() - t_nat_start
    if len(native_docs_by_id) != EXPECTED_TOTAL_DOCS or native_marker_leaks:
        raise Gate4MutationHandoffError(
            f"Native reference core '{args.native_core}' returned {len(native_docs_by_id)} docs "
            f"(expected {EXPECTED_TOTAL_DOCS}), marker_leaks={len(native_marker_leaks)}"
        )
    repo4_sentinel_verification = verify_preflight_repo4_sentinel_in_native_map(
        preflight_report,
        native_docs_by_id,
    )
    repo4_sentinel_uri = repo4_sentinel_verification["sentinel_uri"]
    print(
        f"[STEP 1] Loaded {len(native_docs_by_id)} native reference docs in {t_nat_elapsed:.2f}s; "
        f"validated preflight-sampled repo-4 AO sentinel '{repo4_sentinel_uri}' against full native reference map.",
        flush=True,
    )

    # 2. Create fresh empty candidate core
    cand_core = f"stage_staff_{run_nonce}_candidate"
    print(f"\n[STEP 2] Creating fresh isolated candidate core '{cand_core}'...", flush=True)
    status_check = gate4_base.default_http_get_json(
        f"{solr_host_url}/admin/cores?" + urllib.parse.urlencode({"action": "STATUS", "core": cand_core, "wt": "json"})
    )
    if (status_check.get("status") or {}).get(cand_core):
        raise Gate4MutationHandoffError(f"Candidate core {cand_core} already exists!")
    create_resp = gate4_base.default_http_get_json(
        f"{solr_host_url}/admin/cores?"
        + urllib.parse.urlencode({"action": "CREATE", "name": cand_core, "configSet": "archivesspace", "wt": "json"})
    )
    if (create_resp.get("responseHeader") or {}).get("status") != 0:
        raise Gate4MutationHandoffError(f"Solr core CREATE failed for {cand_core}: {create_resp!r}")
    if solr_select_num_found(solr_host_url, cand_core) != 0:
        raise Gate4MutationHandoffError(f"Newly created candidate core {cand_core} is not empty!")
    print(f"[STEP 2] Candidate core '{cand_core}' created and verified empty (numFound=0).", flush=True)

    # 3. Launch aspace-cli via clean-exec-helper run_with_peak_rss and start delayed mutator relative to CLI launch
    cli_bin = preflight_report["binary_provenance"]["path"]
    job_locale_file = os.path.join(REPO_ROOT, "common", "locales", "en.yml")
    with open(job_locale_file, "rb") as fh:
        job_locale_sha256 = hashlib.sha256(fh.read()).hexdigest()

    cli_cmd = [
        cli_bin,
        "--url",
        backend_host_url,
        "--user",
        state["admin_user"],
        "staff-full-candidate",
        "--state",
        args.state,
        "--batch-size",
        "25",
        "--concurrency",
        str(args.concurrency),
        "--solr-url",
        solr_host_url,
        "--candidate-core",
        cand_core,
        "--arks-enabled",
        "false",
        "--strip-markers",
        "true",
        "--job-locale-file",
        job_locale_file,
        "--job-time-zone",
        "UTC",
        "--json",
    ]
    cli_env = gate3_harness.build_candidate_cli_env(admin_password=state.get("admin_password"))

    mutations_evidence_path = os.path.join(
        args.output_dir,
        f"gate4_mutation_during_pass_mutations_{run_nonce}.json",
    )
    mutator_box: Dict[str, Any] = {"result": None, "error": None}
    cli_launch_monotonic = time.monotonic()
    cli_launch_epoch = time.time()

    def _mutator_worker() -> None:
        try:
            mutator_api = AspaceApiSession.login(
                backend_host_url,
                state["admin_user"],
                state["admin_password"],
            )
            res = execute_guarded_mutations_during_pass(
                api=mutator_api,
                solr_host_url=solr_host_url,
                candidate_core=cand_core,
                mutation_token=mutation_token,
                cli_launch_monotonic=cli_launch_monotonic,
                cli_launch_epoch=cli_launch_epoch,
                repo4_sentinel_uri=repo4_sentinel_uri,
                native_docs_by_id=native_docs_by_id,
                min_delay_sec=args.min_mutation_delay_sec,
                max_wait_sec=args.max_mutation_wait_sec,
                mutations_evidence_path=mutations_evidence_path,
            )
            mutator_box["result"] = res
            print(
                f"[MUTATOR] Successfully applied and audited 3 repo-3 AO mutations at "
                f"+{res['mutation_started_elapsed_since_cli_launch_sec']}s..+{res['mutation_finished_elapsed_since_cli_launch_sec']}s "
                f"after aspace-cli launch (aspace-cli PID={res['post_mutation_process_guard']['aspace_cli_pid']}, "
                f"VmHWM={res['post_mutation_process_guard']['aspace_cli_vm_hwm_kib']} KiB). "
                f"Evidence: {mutations_evidence_path}",
                flush=True,
            )
        except Exception as exc:
            mutator_box["error"] = exc
            print(f"[MUTATOR ERROR] {redact_secrets(exc)}", file=sys.stderr, flush=True)

    print(
        f"\n[STEP 3] Launching aspace-cli on '{cand_core}' and starting guarded delayed mutator "
        f"(min_delay={args.min_mutation_delay_sec}s relative to CLI launch)...",
        flush=True,
    )
    mutator_thread = threading.Thread(target=_mutator_worker, name="gate4-delayed-mutator", daemon=True)
    mutator_thread.start()

    t_cli_start = time.perf_counter()
    cli_proc: PeakRssResult = run_with_peak_rss(cli_cmd, env=cli_env)
    t_cli_elapsed = time.perf_counter() - t_cli_start
    mutator_thread.join(timeout=30.0)

    print(
        f"[STEP 3] aspace-cli exited with code {cli_proc.returncode} in {t_cli_elapsed:.3f}s; "
        f"clean peak RSS = {cli_proc.peak_rss_kib} KiB ({cli_proc.peak_rss_kib / 1048576:.3f} GiB).",
        flush=True,
    )

    if cli_proc.returncode != 0:
        raise Gate4MutationHandoffError(
            f"aspace-cli failed with exit code {cli_proc.returncode}: {redact_secrets(cli_proc.stderr[-3000:])}"
        )
    if mutator_box["error"] is not None:
        raise Gate4MutationHandoffError(
            f"Guarded delayed mutator failed: {redact_secrets(mutator_box['error'])}"
        ) from mutator_box["error"]
    mutation_result = mutator_box["result"]
    if not isinstance(mutation_result, Mapping) or mutation_result.get("status") != "mutations_applied_and_audited":
        raise Gate4MutationHandoffError(f"Mutator did not complete cleanly: {mutation_result!r}")

    cli_summary = json.loads(cli_proc.stdout)
    watermark_epoch = cli_summary.get("full_reindex_started_at_epoch_sec")
    if type(watermark_epoch) is not int or watermark_epoch <= 0:
        raise Gate4MutationHandoffError(f"Invalid full_reindex_started_at_epoch_sec: {watermark_epoch!r}")
    if cli_summary.get("committed") is not True:
        raise Gate4MutationHandoffError(f"Candidate CLI summary committed != true: {cli_summary!r}")
    if watermark_epoch > mutation_result["mutation_started_epoch_sec"]:
        raise Gate4MutationHandoffError(
            f"Watermark epoch {watermark_epoch} is after mutation_started_epoch_sec "
            f"{mutation_result['mutation_started_epoch_sec']}!"
        )

    # 4. Stream candidate documents, extract the rotated candidate identity nonce from candidate
    #    /repositories/2 document (cross-checked against current GET /repositories/2), and verify
    #    candidate missed the 3 repo-3 mutations.
    print(f"\n[STEP 4] Streaming candidate core '{cand_core}' and comparing with native reference...", flush=True)
    cand_docs_by_id, cand_marker_leaks = gate3_harness.stream_fetch_solr_core_documents(
        solr_host_url,
        cand_core,
        fq=None,
        batch_size=1000,
    )
    candidate_identity_nonce = extract_identity_nonce_from_repo2_doc(cand_docs_by_id, label="candidate")
    repo2_now = gate4_base.default_http_get_json(f"{backend_host_url}/repositories/2")
    repo2_name_now = repo2_now.get("name") or ""
    if not repo2_name_now.startswith(gate4_base.IDENTITY_NAME_PREFIX):
        raise Gate4MutationHandoffError(f"Unexpected /repositories/2 name after candidate pass: {repo2_name_now!r}")
    api_identity_nonce_now = repo2_name_now[len(gate4_base.IDENTITY_NAME_PREFIX) :]
    if api_identity_nonce_now != candidate_identity_nonce:
        raise Gate4MutationHandoffError(
            f"Candidate /repositories/2 doc nonce {candidate_identity_nonce!r} != post-run API nonce {api_identity_nonce_now!r}"
        )
    pre_handoff_comparison = verify_candidate_missed_repo3_mutations(
        cand_docs_by_id=cand_docs_by_id,
        native_docs_by_id=native_docs_by_id,
        cand_marker_leaks=cand_marker_leaks,
        mutation_result=mutation_result,
        candidate_identity_nonce=candidate_identity_nonce,
    )
    print(
        "[STEP 4] Confirmed: Rust candidate has 1,036,427 docs, 1,036,425 exact matches with pre-mutation "
        "native reference (2 challenge-nonce diffs verified), and missed all 3 repo-3 mutations.",
        flush=True,
    )

    # 5. Execute reversible CoreAdmin SWAP, File IndexState checkpointing, and one actual Ruby PeriodicIndexer round
    state_dir_name = f"indexer_state_gate4_mutation_handoff_20261124_{secrets.token_hex(4)}"
    backend_container = state["containers"]["backend"]
    print(
        f"\n[STEP 5] Running reversible CoreAdmin SWAP and one-shot Ruby PeriodicIndexer round "
        f"(state_dir={state_dir_name}, watermark={watermark_epoch})...",
        flush=True,
    )

    def _run_jruby_preflight_mutations(
        mod_since: int,
        ao_ids: List[int],
        res_ids: List[int],
        del_uris: List[str],
    ) -> Dict[str, Any]:
        return run_jruby_helper_in_backend_container(
            backend_container,
            [
                "--mode",
                "preflight-expected-mutations",
                "--modified-since",
                str(mod_since),
                "--repo-ids",
                "2,3,4",
                "--target-repo-id",
                str(TARGET_REPO_ID),
                "--expected-updated-ao-ids",
                ",".join(str(x) for x in ao_ids),
                "--expected-updated-resource-ids",
                ",".join(str(x) for x in res_ids),
                "--expected-deleted-uris",
                ",".join(del_uris),
            ],
        )

    def _run_jruby_mutation_round(
        dir_name: str,
        epoch: int,
        ao_ids: List[int],
        res_ids: List[int],
        del_uris: List[str],
        timeout_s: int,
    ) -> Dict[str, Any]:
        return run_jruby_helper_in_backend_container(
            backend_container,
            [
                "--mode",
                "apply-and-run-expected-mutations",
                "--state-dir-name",
                dir_name,
                "--epoch",
                str(epoch),
                "--repo-ids",
                "2,3,4",
                "--target-repo-id",
                str(TARGET_REPO_ID),
                "--expected-updated-ao-ids",
                ",".join(str(x) for x in ao_ids),
                "--expected-updated-resource-ids",
                ",".join(str(x) for x in res_ids),
                "--expected-deleted-uris",
                ",".join(del_uris),
                "--promoted",
                "true",
                "--timeout-sec",
                str(timeout_s),
            ],
            timeout_sec=timeout_s + 60,
        )

    handoff_result = execute_post_pass_ruby_handoff_orchestration(
        solr_url=solr_host_url,
        candidate_core=cand_core,
        watermark_epoch=watermark_epoch,
        mutation_result=mutation_result,
        state_dir_name=state_dir_name,
        http_get_json=gate4_base.default_http_get_json,
        run_jruby_preflight_mutations_fn=_run_jruby_preflight_mutations,
        run_jruby_mutation_round_fn=_run_jruby_mutation_round,
    )

    final_report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "gate4_mutation_during_pass_and_ruby_handoff_verified",
        "gate": "Gate 4",
        "gate_passed": False,
        "run_nonce": run_nonce,
        "mutation_token": mutation_token,
        "git_branch": preflight_report["git_branch"],
        "git_head": preflight_report["git_head"],
        "git_dirty": preflight_report["git_dirty"],
        "binary_provenance": preflight_report["binary_provenance"],
        "job_locale_sha256": job_locale_sha256,
        "stack_id": EXPECTED_STACK_ID,
        "native_reference_core": args.native_core,
        "candidate_core": cand_core,
        "repo4_sentinel_uri": repo4_sentinel_uri,
        "repo4_progress_sentinel": {
            **(preflight_report.get("repo4_progress_sentinel") or {}),
            **repo4_sentinel_verification,
        },
        "preflight_report_path": preflight_path,
        "mutations_report_path": mutations_evidence_path,
        "cli_summary": cli_summary,
        "candidate_cli_wall_sec": round(t_cli_elapsed, 3),
        "memory": compare_peak_memory(cli_proc.peak_rss_kib, None, None),
        "mutation_during_pass": mutation_result,
        "pre_handoff_candidate_vs_native_comparison": pre_handoff_comparison,
        "ruby_handoff_and_incremental_round": handoff_result,
    }
    final_report_path = os.path.join(
        args.output_dir,
        f"gate4_mutation_during_pass_handoff_{run_nonce}.json",
    )
    write_mode_0600_json(final_report_path, final_report)
    print(f"\n[COMPLETE] Retained mode-0600 final report: {final_report_path}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"\n[FATAL ERROR] {redact_secrets(exc)}", file=sys.stderr, flush=True)
        sys.exit(1)
