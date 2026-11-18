#!/usr/bin/env python3
"""Read-only paired native/PUI-tree spike on the disposable profiling backend."""

import hashlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

from run_gate3_combined_candidate import check_for_marker_leaks, run_slice_parity_comparison


ROOT = Path(__file__).resolve().parents[2]
BENCHMARKS = ROOT / "rust-tools" / "benchmarks"
STATE = BENCHMARKS / "profiling_stack_e62875f1.json"
RESOURCE_URI = "/repositories/4/resources/6"
REFERENCE = BENCHMARKS / "pui_tree_reference_20261115_1915.json"
BINARY = ROOT / "rust-tools" / "target" / "release" / "aspace-cli"


def run_pass(state, run_id, pair, mode):
    prefix = f"{run_id}_{pair}_{mode}"
    report_name = f"pui_full_reference_{prefix}.json"
    rust_name = f"pui_tree_rust_{prefix}.json"
    env = os.environ.copy()
    env.update(
        BACKEND_URL=state["backend_network_url"],
        RESOURCE_URI=RESOURCE_URI,
        AUXILIARY_REFERENCE_FILE=f"/archivesspace/rust-tools/benchmarks/{REFERENCE.name}",
        AUXILIARY_MODE=mode,
        OUTPUT_FILE=f"/archivesspace/rust-tools/benchmarks/{report_name}",
    )
    args = [
        "docker", "exec", "-w", "/archivesspace",
        "-e", "BACKEND_URL", "-e", "RESOURCE_URI",
        "-e", "AUXILIARY_REFERENCE_FILE", "-e", "AUXILIARY_MODE",
        "-e", "OUTPUT_FILE",
    ]
    if mode != "native":
        env.update(
            ASPACE_ADMIN_USER=state["admin_user"],
            ASPACE_ADMIN_PASSWORD=state["admin_password"],
            RUST_AUXILIARY_OUTPUT_FILE=f"/archivesspace/rust-tools/benchmarks/{rust_name}",
        )
        args += ["-e", "ASPACE_ADMIN_USER", "-e", "ASPACE_ADMIN_PASSWORD", "-e", "RUST_AUXILIARY_OUTPUT_FILE"]
    args += [
        state["app_container"], "/archivesspace/scripts/jruby",
        "/archivesspace/rust-tools/scripts/pui_ordinary_reference.rb",
    ]
    proc = subprocess.run(args, env=env, text=True, capture_output=True, timeout=180, check=False)
    if proc.returncode:
        raise RuntimeError(f"{mode} pass failed ({proc.returncode}): {proc.stderr[-600:]}")
    names = [report_name] + ([rust_name] if mode != "native" else [])
    subprocess.run(
        ["docker", "exec", state["app_container"], "chown", "1000:1000"]
        + [f"/archivesspace/rust-tools/benchmarks/{name}" for name in names],
        check=True, capture_output=True, text=True, timeout=10,
    )
    report = json.loads((BENCHMARKS / report_name).read_text())
    if report["mode"] != mode or report["document_count"] != 2843:
        raise RuntimeError("unexpected PUI report mode or document count")
    if mode != "native" and report.get("fallback_used") is not False:
        raise RuntimeError("candidate speed pass used native fallback")
    return report_name, report


def main():
    state = json.loads(STATE.read_text())
    candidate_mode = os.environ.get("PUI_SPIKE_CANDIDATE_MODE", "candidate")
    if candidate_mode not in ("candidate", "candidate_parallel"):
        raise RuntimeError("invalid PUI spike candidate mode")
    if state["stack_id"] != "e62875f1" or not BINARY.is_file():
        raise RuntimeError("expected isolated stack or release binary missing")
    run_id = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    summary = {
        "resource_uri": RESOURCE_URI,
        "stack_id": state["stack_id"],
        "git_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "git_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT)),
        "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        "timed_scope": "read-only ordinary PUI fetch/build plus auxiliary fetch/build; excludes Solr writes",
        "candidate_mode": candidate_mode,
        "pairs": [],
    }
    path = BENCHMARKS / f"pui_spike_pairs_{run_id}.json"
    for pair in range(1, 6):
        order = ["native", candidate_mode] if pair % 2 else [candidate_mode, "native"]
        results = {}
        for mode in order:
            name, report = run_pass(state, run_id, pair, mode)
            results[mode] = (name, report)
        native_name, native = results["native"]
        candidate_name, candidate = results[candidate_mode]
        leaks = [leak for doc in candidate["documents"] for leak in check_for_marker_leaks(doc)]
        if any(doc.get("publish") is not True for doc in candidate["documents"][: candidate["ordinary_document_count"]]):
            raise RuntimeError(f"pair {pair} contains an unpublished ordinary PUI document")
        for doc in candidate["documents"][: candidate["ordinary_document_count"]]:
            if json.loads(doc["json"]).get("ancestors"):
                raise RuntimeError(f"pair {pair} embeds nonempty ancestors in a PUI document")
        parity = run_slice_parity_comparison(
            {doc["id"]: doc for doc in candidate["documents"]},
            {doc["id"]: doc for doc in native["documents"]},
            leaks,
        )
        deletes_equal = set(candidate["delete_candidates"]) == set(native["delete_candidates"])
        if not parity["exact_parity"] or not deletes_equal:
            raise RuntimeError(f"pair {pair} failed strict parity")
        row = {
            "pair": pair,
            "order": order,
            "native_file": str(BENCHMARKS / native_name),
            "candidate_file": str(BENCHMARKS / candidate_name),
            "native_seconds": native["elapsed_seconds"],
            "candidate_seconds": candidate["elapsed_seconds"],
            "native_ordinary_seconds": native["ordinary_seconds"],
            "candidate_ordinary_seconds": candidate["ordinary_seconds"],
            "native_auxiliary_seconds": native["auxiliary_seconds"],
            "candidate_auxiliary_seconds": candidate["auxiliary_seconds"],
            "candidate_rust_observed_until_collection_seconds": candidate["rust_observed_until_collection_seconds"],
            "candidate_rust_internal_seconds": candidate["rust_internal_seconds"],
            "exact_parity": parity["exact_parity"],
            "documents_compared": parity["records_compared"],
            "delete_candidates_equal": deletes_equal,
        }
        summary["pairs"].append(row)
        path.write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps({"pair": pair, "native_seconds": row["native_seconds"], "candidate_seconds": row["candidate_seconds"], "exact_parity": True}), flush=True)
    print(path)


if __name__ == "__main__":
    main()
