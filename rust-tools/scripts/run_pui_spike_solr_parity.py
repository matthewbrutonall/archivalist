#!/usr/bin/env python3
"""Write saved one-tree PUI spike documents to two fresh isolated Solr cores."""

import json
import os
import secrets
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

from profiling_stack import load_state, verify_identity
from run_gate3_combined_candidate import check_for_marker_leaks, run_slice_parity_comparison


ROOT = Path(__file__).resolve().parents[2]
BENCHMARKS = ROOT / "rust-tools" / "benchmarks"
STATE = BENCHMARKS / "profiling_stack_e62875f1.json"
PAIRS = BENCHMARKS / "pui_spike_pairs_20261115_195316.json"


def request_json(url, payload=None, timeout=120):
    headers = {"Content-Type": "application/json; charset=utf-8"} if payload is not None else {}
    data = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8") if payload is not None else None
    req = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(req, timeout=timeout) as response:
        result = json.load(response)
    if result.get("responseHeader", {}).get("status") != 0:
        raise RuntimeError(f"Solr operation failed: {url}")
    return result


def query_all(base, core):
    docs = {}
    cursor = "*"
    while True:
        query = urllib.parse.urlencode({"q": "*:*", "sort": "id asc", "fl": "*", "rows": 500, "cursorMark": cursor, "wt": "json"})
        result = request_json(f"{base}/{core}/select?{query}")
        for doc in result["response"]["docs"]:
            key = doc["id"]
            if key in docs:
                raise RuntimeError("duplicate stored document ID")
            docs[key] = doc
        next_cursor = result["nextCursorMark"]
        if next_cursor == cursor:
            break
        cursor = next_cursor
    return docs


def main():
    state = load_state(str(STATE))
    if state["stack_id"] != "e62875f1":
        raise RuntimeError("wrong isolated stack")
    verify_identity(state, state["backend_host_url"], state["solr_host_url"])
    pairs = json.loads(PAIRS.read_text())
    row = pairs["pairs"][0]
    run_id = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S") + f"_{os.getpid()}_{secrets.token_hex(4)}"
    base = state["solr_host_url"].rstrip("/")
    results = {}
    for role in ("native", "candidate"):
        core = f"stage_pui_{run_id}_{role}"
        source = Path(row[f"{role}_file"])
        if source.parent != BENCHMARKS:
            raise RuntimeError("source report outside isolated benchmarks")
        report = json.loads(source.read_text())
        documents = report["documents"]
        if report["document_count"] != 2843 or len(documents) != 2843:
            raise RuntimeError("unexpected document count")
        if any(check_for_marker_leaks(doc) for doc in documents):
            raise RuntimeError("private marker in candidate input")
        status = request_json(f"{base}/admin/cores?action=STATUS&wt=json")
        if core in status["status"]:
            raise RuntimeError("refusing to use an existing Solr core")
        create = urllib.parse.urlencode({"action": "CREATE", "name": core, "configSet": "archivesspace", "wt": "json"})
        request_json(f"{base}/admin/cores?{create}")
        started = time.monotonic()
        request_json(f"{base}/{core}/update?wt=json", documents)
        request_json(f"{base}/{core}/update?wt=json", {"commit": {}})
        write_seconds = time.monotonic() - started
        stored = query_all(base, core)
        if len(stored) != 2843:
            raise RuntimeError(f"{role} stored only {len(stored)} documents")
        results[role] = {"core": core, "source_report": str(source), "write_commit_seconds": write_seconds, "stored": stored}
        print(role, "core", core, "stored", len(stored), "write_commit_seconds", round(write_seconds, 3), flush=True)
    candidate = results["candidate"].pop("stored")
    native = results["native"].pop("stored")
    leaks = [leak for doc in candidate.values() for leak in check_for_marker_leaks(doc)]
    parity = run_slice_parity_comparison(candidate, native, leaks)
    if not parity["exact_parity"]:
        raise RuntimeError("stored Solr parity failed")
    results["exact_stored_parity"] = True
    results["stored_documents_compared"] = parity["records_compared"]
    results["run_id"] = run_id
    results["stack_id"] = state["stack_id"]
    out = BENCHMARKS / f"pui_spike_solr_{run_id}.json"
    out.write_text(json.dumps(results, indent=2) + "\n")
    print("exact stored parity", parity["records_compared"], "report", out)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"ERROR: {error}", file=sys.stderr)
        raise
