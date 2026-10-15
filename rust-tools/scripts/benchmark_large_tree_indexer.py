#!/usr/bin/env python3

import argparse
import hashlib
import json
import statistics
import time
import urllib.error
import urllib.parse
import urllib.request


def request_json(base_url, path, session=None, params=None, form=None):
    url = f"{base_url.rstrip('/')}{path}"
    data = None
    if params:
        url = f"{url}?{urllib.parse.urlencode(params, doseq=True)}"
    if form is not None:
        data = urllib.parse.urlencode(form).encode()
    request = urllib.request.Request(url, data=data)
    if session:
        request.add_header("X-ArchivesSpace-Session", session)
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"{error.code} {url}: {error.read().decode()}") from error


def login(base_url, username, password):
    response = request_json(
        base_url,
        f"/users/{urllib.parse.quote(username)}/login",
        form={"password": password, "expiring": "false"},
    )
    return response["session"]


def tree_document(root_uri, kind, suffix, parent_id, value):
    uri = f"{root_uri}/tree/{suffix}"
    return {
        "id": uri,
        "uri": uri,
        "pui_parent_id": parent_id,
        "publish": "true",
        "primary_type": kind,
        "types": ["pui"],
        "json": json.dumps(value, separators=(",", ":")),
    }


def run_indexer_contract(base_url, root_uri, session):
    documents = []
    node_uris = []
    request_counts = {"root": 0, "waypoint": 0, "node": 0, "node_from_root": 0}
    phase_seconds = {"tree": 0.0, "node_from_root": 0.0}

    started = time.perf_counter()
    root = request_json(
        base_url, root_uri + "/tree/root", session, {"published_only": "true"}
    )
    request_counts["root"] += 1
    documents.append(tree_document(root_uri, "tree_root", "root", root_uri, root))

    def add_waypoints(tree, parent_uri):
        for offset in range(tree["waypoints"]):
            params = {"offset": offset, "published_only": "true"}
            if parent_uri is not None:
                params["parent_node"] = parent_uri
            waypoint = request_json(
                base_url,
                root_uri + "/tree/waypoint",
                session,
                params,
            )
            request_counts["waypoint"] += 1
            suffix = f"waypoint_{parent_uri or ''}_{offset}"
            documents.append(
                tree_document(
                    root_uri,
                    "tree_waypoint",
                    suffix,
                    parent_uri or root_uri,
                    waypoint,
                )
            )
            for waypoint_record in waypoint:
                record_uri = waypoint_record["uri"]
                node_uris.append(record_uri)
                if waypoint_record["child_count"] > 0:
                    node = request_json(
                        base_url,
                        root_uri + "/tree/node",
                        session,
                        {"node_uri": record_uri, "published_only": "true"},
                    )
                    request_counts["node"] += 1
                    documents.append(
                        tree_document(
                            root_uri,
                            "tree_node",
                            f"node_{node['uri']}",
                            node["uri"],
                            node,
                        )
                    )
                    add_waypoints(node, node["uri"])

    add_waypoints(root, None)
    phase_seconds["tree"] = time.perf_counter() - started

    paths_started = time.perf_counter()
    for index in range(0, len(node_uris), 128):
        batch = node_uris[index : index + 128]
        node_id_to_uri = {int(uri.rsplit("/", 1)[1]): uri for uri in batch}
        paths = request_json(
            base_url,
            root_uri + "/tree/node_from_root",
            session,
            {"node_ids[]": list(node_id_to_uri), "published_only": "true"},
        )
        request_counts["node_from_root"] += 1
        for node_id, path in paths.items():
            documents.append(
                tree_document(
                    root_uri,
                    "tree_node_from_root",
                    f"node_from_root_{node_id}",
                    node_id_to_uri[int(node_id)],
                    {node_id: path},
                )
            )
    phase_seconds["node_from_root"] = time.perf_counter() - paths_started

    semantic_documents = sorted(
        ({**document, "json": json.loads(document["json"])} for document in documents),
        key=lambda document: document["id"],
    )
    canonical = json.dumps(
        semantic_documents, sort_keys=True, separators=(",", ":")
    ).encode()
    return {
        "document_count": len(documents),
        "node_count": len(node_uris),
        "request_counts": request_counts,
        "phase_seconds": phase_seconds,
        "elapsed_seconds": time.perf_counter() - started,
        "sha256": hashlib.sha256(canonical).hexdigest(),
    }


def millisecond_stats(values):
    return {
        "min": round(min(values) * 1000, 2),
        "mean": round(statistics.mean(values) * 1000, 2),
        "max": round(max(values) * 1000, 2),
    }


def main():
    parser = argparse.ArgumentParser(
        description="Benchmark the API request contract used by LargeTreeDocIndexer"
    )
    parser.add_argument("--url", default="http://127.0.0.1:4567")
    parser.add_argument("--resource-uri", default="/repositories/2/resources/16")
    parser.add_argument("--username", default="admin")
    parser.add_argument("--password", default="admin")
    parser.add_argument("--warmup", type=int, default=1)
    parser.add_argument("--repeat", type=int, default=5)
    args = parser.parse_args()
    if args.warmup < 0 or args.repeat < 1:
        parser.error("--warmup must be non-negative and --repeat must be positive")

    session = login(args.url, args.username, args.password)
    for _ in range(args.warmup):
        run_indexer_contract(args.url, args.resource_uri, session)

    results = [
        run_indexer_contract(args.url, args.resource_uri, session)
        for _ in range(args.repeat)
    ]
    hashes = {result["sha256"] for result in results}
    if len(hashes) != 1:
        raise RuntimeError(f"non-deterministic document hashes: {sorted(hashes)}")

    output = {
        "resource_uri": args.resource_uri,
        "repeat": args.repeat,
        "document_count": results[0]["document_count"],
        "node_count": results[0]["node_count"],
        "request_counts": results[0]["request_counts"],
        "sha256": results[0]["sha256"],
        "total_ms": millisecond_stats(
            [result["elapsed_seconds"] for result in results]
        ),
        "tree_ms": millisecond_stats(
            [result["phase_seconds"]["tree"] for result in results]
        ),
        "node_from_root_ms": millisecond_stats(
            [result["phase_seconds"]["node_from_root"] for result in results]
        ),
    }
    print(json.dumps(output, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
