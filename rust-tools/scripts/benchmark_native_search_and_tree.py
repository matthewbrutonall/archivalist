#!/usr/bin/env python3
"""Native search and resource-tree backend API proxy latency baseline harness.

Measures Gate 1 baseline latency for backend API proxy routes against the isolated
profiling stack provisioned by profiling_stack.py.

Timing Scope & Measurement:
- Measures connection setup, request dispatch, response headers, and full response
  body transfer over the wire via resp.read().
- Client-side JSON decoding and parsing into Python objects are explicitly excluded
  from recorded durations to isolate proxy/backend latency from client CPU parsing.
- Partial route durations from completed repeats are preserved if a later repeat fails.

Safety & Mutation Disclosure:
- Preflight setup mutation: verify_identity (via profiling_stack.py) executes an
  isolated DB write of a random challenge nonce into the profiling stack's repository
  record (archivesspace.repository) to prove backend database connectivity.
- All measurement calls are strictly read-only HTTP GET requests (preceded by a single
  POST /users/admin/login to acquire an administrative session token).
- No application records are mutated, created, or deleted.

Architecture & Scope Note:
- The profiling stack provisions database, Solr, and backend containers only. It lacks
  Rails staff and public UI application tiers. Measured routes are backend API proxies,
  not user-facing UI routes. Full Rails staff and public UI routes remain unmeasured.
"""

import argparse
import importlib.util
import json
import math
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def load_profiling_stack(script_dir=None):
    if script_dir is None:
        script_dir = os.path.dirname(os.path.abspath(__file__))
    path = os.path.join(script_dir, "profiling_stack.py")
    if not os.path.isfile(path):
        raise FileNotFoundError(f"profiling_stack.py not found at {path}")
    spec = importlib.util.spec_from_file_location("profiling_stack", path)
    mod = importlib.util.module_from_spec(spec)
    prev = sys.dont_write_bytecode
    sys.dont_write_bytecode = True
    try:
        spec.loader.exec_module(mod)
    finally:
        sys.dont_write_bytecode = prev
    return mod


def git_provenance(repo_dir=REPO_ROOT):
    commit, dirty = "unknown", None
    try:
        commit = subprocess.check_output(
            ["git", "-C", repo_dir, "rev-parse", "HEAD"],
            stderr=subprocess.DEVNULL, text=True,
        ).strip()
        status = subprocess.check_output(
            ["git", "-C", repo_dir, "status", "--porcelain"],
            stderr=subprocess.DEVNULL, text=True,
        ).strip()
        dirty = bool(status)
    except Exception:
        pass
    return commit, dirty


def calculate_percentiles(durations_ms):
    if not durations_ms:
        return {"p50_ms": None, "p95_ms": None, "min_ms": None, "max_ms": None, "mean_ms": None}
    s = sorted(durations_ms)
    n = len(s)
    def pct(p):
        idx = max(0, min(n - 1, math.ceil(p * n) - 1))
        return round(s[idx], 3)
    return {
        "p50_ms": pct(0.50),
        "p95_ms": pct(0.95),
        "min_ms": round(s[0], 3),
        "max_ms": round(s[-1], 3),
        "mean_ms": round(sum(s) / n, 3),
    }


def http_request(url, method="GET", headers=None, params=None, body=None, timeout=30):
    if params:
        qs = urllib.parse.urlencode(params)
        url = f"{url}?{qs}" if "?" not in url else f"{url}&{qs}"
    data = body.encode("utf-8") if isinstance(body, str) else body
    req = urllib.request.Request(url, data=data, headers=headers or {}, method=method)
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw_bytes = resp.read()
        dur = (time.perf_counter() - t0) * 1000.0
        raw = raw_bytes.decode("utf-8") if isinstance(raw_bytes, (bytes, bytearray)) else raw_bytes
        try:
            parsed = json.loads(raw)
        except Exception:
            parsed = raw
        return resp.status, parsed, dur


def admin_login(backend_url, username, password, timeout=30):
    url = f"{backend_url.rstrip('/')}/users/{urllib.parse.quote(username)}/login"
    status, body, _ = http_request(url, method="POST", params={"password": password}, timeout=timeout)
    if status != 200 or not isinstance(body, dict) or "session" not in body:
        raise RuntimeError(f"Admin login failed (status {status})")
    return body["session"]


def extract_counts(body):
    if isinstance(body, dict):
        if "results" in body and isinstance(body["results"], list):
            return body.get("total_hits"), len(body["results"])
        if "child_count" in body:
            total_hits = body.get("child_count")
            result_count = None
            pre = body.get("precomputed_waypoints")
            if isinstance(pre, dict):
                first_slice = (pre.get("") or {}).get("0") or (pre.get("") or {}).get(0)
                if isinstance(first_slice, list):
                    result_count = len(first_slice)
            return total_hits, result_count
        if "waypoints" in body:
            return body.get("child_count"), body.get("waypoints")
        return None, None
    elif isinstance(body, list):
        return None, len(body)
    return None, None


def discover_node_uri(root_body, backend_url, repo_id, resource_id, headers, timeout=30):
    if isinstance(root_body, dict):
        pre = root_body.get("precomputed_waypoints")
        if isinstance(pre, dict):
            first_slice = (pre.get("") or {}).get("0") or (pre.get("") or {}).get(0)
            if isinstance(first_slice, list) and len(first_slice) > 0:
                uri = first_slice[0].get("uri")
                if uri:
                    return uri, None
    try:
        wp_url = f"{backend_url.rstrip('/')}/repositories/{repo_id}/resources/{resource_id}/tree/waypoint"
        status, body, _ = http_request(wp_url, headers=headers, params={"offset": 0}, timeout=timeout)
        if status == 200 and isinstance(body, list) and len(body) > 0:
            uri = body[0].get("uri")
            if uri:
                return uri, None
    except Exception as err:
        return None, f"Node discovery error: {err}"
    return None, "No child archival object found in tree root or waypoint 0"


def run_benchmark(state_path, repo_id=2, resource_id=1, query="papers", repeats=20, warmup=2,
                  admin_user=None, admin_password=None, stack_mod=None, timeout=30):
    if stack_mod is None:
        stack_mod = load_profiling_stack()
    state = stack_mod.load_state(state_path)
    backend_url = state["backend_host_url"]
    solr_url = state["solr_host_url"]

    # Preflight identity verification writes challenge nonce into isolated DB
    stack_mod.verify_identity(state, backend_url, solr_url)

    user = admin_user or state.get("admin_user", "admin")
    password = admin_password or state.get("admin_password", "admin")
    session_token = admin_login(backend_url, user, password, timeout=timeout)
    headers = {"X-ArchivesSpace-Session": session_token}

    commit, dirty = git_provenance()
    host_load = list(os.getloadavg()) if hasattr(os, "getloadavg") else None

    route_specs = [
        {
            "name": "staff_repo_search_wildcard",
            "route": "/repositories/:repo_id/search",
            "url": f"{backend_url.rstrip('/')}/repositories/{repo_id}/search",
            "params": {"q": "*", "page": 1, "page_size": 25},
        },
        {
            "name": "staff_repo_search_keyword",
            "route": "/repositories/:repo_id/search",
            "url": f"{backend_url.rstrip('/')}/repositories/{repo_id}/search",
            "params": {"q": query, "page": 1, "page_size": 25},
        },
        {
            "name": "staff_tree_root",
            "route": "/repositories/:repo_id/resources/:id/tree/root",
            "url": f"{backend_url.rstrip('/')}/repositories/{repo_id}/resources/{resource_id}/tree/root",
            "params": {},
        },
        {
            "name": "staff_tree_waypoint",
            "route": "/repositories/:repo_id/resources/:id/tree/waypoint",
            "url": f"{backend_url.rstrip('/')}/repositories/{repo_id}/resources/{resource_id}/tree/waypoint",
            "params": {"offset": 0},
        },
        {
            "name": "staff_tree_node",
            "route": "/repositories/:repo_id/resources/:id/tree/node",
            "url": f"{backend_url.rstrip('/')}/repositories/{repo_id}/resources/{resource_id}/tree/node",
            "params": {},
        },
    ]

    results = []
    discovered_node_uri = None
    node_blocker = None

    for spec in route_specs:
        route_entry = {
            "name": spec["name"],
            "timing_scope": "backend_api_proxy",
            "timing_scope_detail": "Headers and full body transfer; excludes client-side JSON decode",
            "route": spec["route"],
            "params": dict(spec["params"]),
            "status": "ok",
            "blocker": None,
            "total_hits": None,
            "result_count": None,
            "warmup_durations_ms": [],
            "repeat_durations_ms": [],
            "p50_ms": None,
            "p95_ms": None,
            "min_ms": None,
            "max_ms": None,
            "mean_ms": None,
        }

        if spec["name"] == "staff_tree_node":
            if not discovered_node_uri:
                route_entry["status"] = "blocked"
                route_entry["blocker"] = node_blocker or "Node URI not discovered"
                results.append(route_entry)
                continue
            route_entry["params"]["node_uri"] = discovered_node_uri
            spec["params"]["node_uri"] = discovered_node_uri

        warmup_err = None
        for _ in range(warmup):
            try:
                status, body, dur = http_request(spec["url"], headers=headers, params=spec["params"], timeout=timeout)
                if status != 200:
                    warmup_err = f"HTTP {status}"
                    break
                route_entry["warmup_durations_ms"].append(round(dur, 3))
            except Exception as err:
                warmup_err = str(err)
                break

        if warmup_err:
            route_entry["status"] = "blocked"
            route_entry["blocker"] = f"Warmup error: {warmup_err}"
            results.append(route_entry)
            continue

        repeat_err = None
        last_body = None
        for _ in range(repeats):
            try:
                status, body, dur = http_request(spec["url"], headers=headers, params=spec["params"], timeout=timeout)
                if status != 200:
                    repeat_err = f"HTTP {status}"
                    break
                route_entry["repeat_durations_ms"].append(round(dur, 3))
                last_body = body
            except Exception as err:
                repeat_err = str(err)
                break

        # Preserve partial route durations and statistics if any repeats completed
        if route_entry["repeat_durations_ms"]:
            route_entry.update(calculate_percentiles(route_entry["repeat_durations_ms"]))
        if last_body is not None:
            total_hits, result_count = extract_counts(last_body)
            route_entry["total_hits"] = total_hits
            route_entry["result_count"] = result_count

            if spec["name"] == "staff_tree_root" and not discovered_node_uri:
                discovered_node_uri, node_blocker = discover_node_uri(
                    last_body, backend_url, repo_id, resource_id, headers, timeout=timeout
                )

        if repeat_err:
            route_entry["status"] = "blocked"
            route_entry["blocker"] = f"Measurement error: {repeat_err}"

        results.append(route_entry)

    return {
        "stack_id": state.get("stack_id"),
        "git_commit": commit,
        "git_dirty": dirty,
        "host_load": host_load,
        "timing_scope": "backend_api_proxy_only",
        "timing_scope_detail": "Measures connection, request dispatch, response headers, and full body read over the wire (resp.read()); excludes client-side JSON decode.",
        "notes": "Backend API proxy timings only (includes full response body read, excludes client-side JSON decode); Rails staff and public UI end-to-end unmeasured.",
        "safety_disclosure": "Preflight identity verification executes an isolated DB write of a random "
                             "challenge nonce via profiling_stack.verify_identity. All benchmark requests "
                             "are read-only HTTP GETs.",
        "samples_per_route": repeats,
        "warmup_samples": warmup,
        "routes": results,
    }


def main():
    parser = argparse.ArgumentParser(
        description="Gate 1 read-only backend API latency probe against profiling stack."
    )
    parser.add_argument("--state", required=True, help="Path to profiling stack state JSON")
    parser.add_argument("--repo-id", type=int, default=2, help="Repository ID (default: 2)")
    parser.add_argument("--resource-id", type=int, default=1, help="Resource ID (default: 1)")
    parser.add_argument("--query", default="papers", help="Search keyword (default: 'papers')")
    parser.add_argument("--repeats", type=int, default=20, help="Timed repeat samples (default: 20)")
    parser.add_argument("--warmup", type=int, default=2, help="Warmup iterations (default: 2)")
    parser.add_argument("--timeout", type=int, default=30, help="HTTP request timeout seconds")
    parser.add_argument("--output", help="Optional output JSON file path")
    args = parser.parse_args()

    report = run_benchmark(
        state_path=args.state,
        repo_id=args.repo_id,
        resource_id=args.resource_id,
        query=args.query,
        repeats=args.repeats,
        warmup=args.warmup,
        timeout=args.timeout,
    )
    rendered = json.dumps(report, indent=2)
    if args.output:
        os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
        with open(args.output, "w") as fh:
            fh.write(rendered + "\n")
    print(rendered)


if __name__ == "__main__":
    main()
