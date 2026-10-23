#!/usr/bin/env python3
"""Seed a large, deterministic test repository through the ArchivesSpace API.

For profiling only, against the DISPOSABLE profiling stack from profiling_stack.py: it creates a new published
repository with subjects, agents, and per-collection trees (resource -> series -> files ->
items) carrying notes, dates, extents, subject/agent links and top-container instances, plus a
fraction of digital objects linked from items. Each collection tree is sent as one
/batch_imports request, with placeholder URIs that the backend resolves when it saves them.

Safety: --stack-state (the state file from `profiling_stack.py setup`) is required. Before any
login or write it runs the stack's database-identity proof: a fresh nonce is written directly into
the profiling database and the target backend must return it from an anonymous GET; the stack's
Solr identity core must also be present. A main or shared test backend fails that check and is
never written to. It only creates records (in its new repository and the global subject/agent
lists); it never updates or deletes anything. Uses only the Python standard library.

Example:
  python3 rust-tools/scripts/seed_large_repository.py \\
      --stack-state rust-tools/benchmarks/profiling_stack_<id>.json --preset medium
"""

import argparse
import importlib.util
import json
import os
import random
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

PRESETS = {
    # resources, series/resource, files/series, items/file, subjects, agents, top containers/resource
    "small": dict(resources=5, series=4, files=10, items=5, subjects=40, agents=40, containers=20),
    "medium": dict(resources=20, series=10, files=20, items=5, subjects=150, agents=150, containers=60),
    "large": dict(resources=40, series=12, files=25, items=6, subjects=300, agents=300, containers=120),
}


def load_profiling_stack():
    """profiling_stack.py from this directory, loaded by path (works under python3 -I)."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "profiling_stack.py")
    spec = importlib.util.spec_from_file_location("archivalist_profiling_stack", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
WORDS = ("correspondence photographs minutes ledgers drawings reports diaries maps sermons "
         "pamphlets accounts surveys petitions transcripts notebooks programmes").split()


def items_total(cfg):
    return cfg["resources"] * cfg["series"] * cfg["files"] * (1 + cfg["items"]) + cfg["resources"] * cfg["series"]


class Backend:
    def __init__(self, base_url):
        self.base = base_url.rstrip("/")
        self.session = None

    def call(self, method, path, data=None, params=None, timeout=600):
        url = self.base + path + ("?" + urllib.parse.urlencode(params) if params else "")
        body = json.dumps(data).encode() if data is not None else (b"" if method == "POST" else None)
        req = urllib.request.Request(url, data=body, method=method)
        req.add_header("Content-Type", "application/json")
        if self.session:
            req.add_header("X-ArchivesSpace-Session", self.session)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as err:
            raise SystemExit(f"{method} {path} failed: HTTP {err.code}: {err.read()[:600]!r}")

    def login(self, user, password):
        self.session = self.call("POST", f"/users/{urllib.parse.quote(user)}/login", params={"password": password})["session"]


def note(rng, kind, n_words):
    text = " ".join(rng.choice(WORDS) for _ in range(n_words))
    return {"jsonmodel_type": "note_multipart", "type": kind, "publish": True,
            "subnotes": [{"jsonmodel_type": "note_text", "publish": True, "content": text.capitalize() + "."}]}


def dates(rng):
    begin = rng.randint(1800, 1990)
    end = begin + rng.randint(0, 30)
    return [{"date_type": "inclusive", "label": "creation", "begin": str(begin), "end": str(end),
             "expression": f"{begin}-{end}"}]


def build_tree_batch(rng, repo_uri, r, cfg, subjects, agents):
    """One collection: placeholder URIs are unique within the batch."""
    batch = []
    counter = {"n": 0}

    def placeholder(kind):
        counter["n"] += 1
        return f"{repo_uri}/{kind}/{counter['n']}"

    containers = []
    for c in range(cfg["containers"]):
        uri = placeholder("top_containers")
        containers.append(uri)
        batch.append({"jsonmodel_type": "top_container", "uri": uri, "indicator": f"R{r}-B{c + 1}", "type": "box"})

    resource_uri = placeholder("resources")
    batch.append({
        "jsonmodel_type": "resource", "uri": resource_uri, "title": f"{rng.choice(WORDS).capitalize()} collection {r + 1}",
        "id_0": f"SEED-{r + 1}", "level": "collection", "publish": True, "dates": dates(rng),
        "extents": [{"portion": "whole", "number": str(rng.randint(1, 80)), "extent_type": "linear_feet"}],
        "lang_materials": [{"language_and_script": {"language": "eng", "script": "Latn"}}],
        "finding_aid_language": "eng", "finding_aid_script": "Latn",
        "notes": [note(rng, "scopecontent", 120), note(rng, "bioghist", 200), note(rng, "arrangement", 40)],
        "subjects": [{"ref": s} for s in rng.sample(subjects, min(5, len(subjects)))],
        "linked_agents": [{"ref": a, "role": "creator"} for a in rng.sample(agents, min(2, len(agents)))],
    })

    digital_objects = 0
    for s in range(cfg["series"]):
        series_uri = placeholder("archival_objects")
        batch.append({"jsonmodel_type": "archival_object", "uri": series_uri, "resource": {"ref": resource_uri},
                      "title": f"Series {s + 1}: {rng.choice(WORDS)}", "level": "series", "publish": True,
                      "dates": dates(rng), "notes": [note(rng, "scopecontent", 60)]})
        for f in range(cfg["files"]):
            file_uri = placeholder("archival_objects")
            batch.append({
                "jsonmodel_type": "archival_object", "uri": file_uri, "resource": {"ref": resource_uri},
                "parent": {"ref": series_uri}, "title": f"{rng.choice(WORDS).capitalize()}, file {s + 1}.{f + 1}",
                "level": "file", "publish": True, "dates": dates(rng), "notes": [note(rng, "scopecontent", 30)],
                "subjects": [{"ref": rng.choice(subjects)}],
                "instances": [{"instance_type": "mixed_materials",
                               "sub_container": {"top_container": {"ref": rng.choice(containers)},
                                                 "type_2": "folder", "indicator_2": str(f + 1)}}],
            })
            for i in range(cfg["items"]):
                item = {"jsonmodel_type": "archival_object", "uri": placeholder("archival_objects"),
                        "resource": {"ref": resource_uri}, "parent": {"ref": file_uri},
                        "title": f"{rng.choice(WORDS).capitalize()} {s + 1}.{f + 1}.{i + 1}", "level": "item",
                        "publish": True, "notes": [note(rng, "scopecontent", 15)]}
                if rng.random() < cfg["agent_link_fraction"]:
                    item["linked_agents"] = [{"ref": rng.choice(agents), "role": "subject"}]
                if rng.random() < cfg["digital_object_fraction"]:
                    do_uri = placeholder("digital_objects")
                    digital_objects += 1
                    batch.append({"jsonmodel_type": "digital_object", "uri": do_uri, "publish": True,
                                  "title": f"Digital surrogate {r + 1}.{s + 1}.{f + 1}.{i + 1}",
                                  "digital_object_id": f"SEED-DO-{r + 1}-{s + 1}-{f + 1}-{i + 1}"})
                    item["instances"] = [{"instance_type": "digital_object", "digital_object": {"ref": do_uri}}]
                batch.append(item)
    return batch, digital_objects


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--stack-state", required=True, help="state file from profiling_stack.py setup")
    parser.add_argument("--backend-url", default=None, help="default: the stack's backend URL")
    parser.add_argument("--user", default="admin")
    parser.add_argument("--preset", choices=sorted(PRESETS), default="small")
    for key in ("resources", "series", "files", "items", "subjects", "agents", "containers"):
        parser.add_argument(f"--{key}", type=int, help=f"override the preset's {key}")
    parser.add_argument("--digital-object-fraction", type=float, default=0.05)
    parser.add_argument("--agent-link-fraction", type=float, default=0.1)
    parser.add_argument("--seed", type=int, default=20261023)
    parser.add_argument("--repo-code", default=None, help="default SEED-<timestamp>")
    parser.add_argument("--summary-json", default=None, help="write a JSON summary to this path")
    args = parser.parse_args(argv)

    stack = load_profiling_stack()
    try:
        state = stack.load_state(args.stack_state)
        backend_url = args.backend_url or state["backend_host_url"]
        stack.verify_identity(state, backend_url, state["solr_host_url"])
    except (stack.StackError, OSError, ValueError) as err:
        print(f"ERROR: refusing to seed: {err}", file=sys.stderr)
        return 2
    print(f"Verified {backend_url} belongs to profiling stack {state['stack_id']}", flush=True)

    cfg = dict(PRESETS[args.preset])
    for key in cfg:
        if getattr(args, key) is not None:
            cfg[key] = getattr(args, key)
    cfg["digital_object_fraction"] = args.digital_object_fraction
    cfg["agent_link_fraction"] = args.agent_link_fraction
    rng = random.Random(args.seed)

    backend = Backend(backend_url)
    backend.login(args.user, os.environ.get("ARCHIVALIST_SEED_PASSWORD", state.get("admin_password", "admin")))
    started = time.monotonic()

    repo_code = args.repo_code or f"SEED-{int(time.time())}"
    repo_id = backend.call("POST", "/repositories", {"repo_code": repo_code, "name": f"Seeded profiling repository {repo_code}",
                                                     "publish": True})["id"]
    repo_uri = f"/repositories/{repo_id}"
    print(f"Created {repo_uri} ({repo_code}); target ~{items_total(cfg)} archival objects", flush=True)

    subjects = [backend.call("POST", "/subjects", {
        "terms": [{"term": f"{rng.choice(WORDS).capitalize()} {repo_code} {i}", "term_type": "topical", "vocabulary": "/vocabularies/1"}],
        "vocabulary": "/vocabularies/1", "source": "local"})["uri"] for i in range(cfg["subjects"])]
    agents = [backend.call("POST", "/agents/people", {
        "names": [{"primary_name": f"Person{i}", "rest_of_name": repo_code, "name_order": "inverted",
                   "sort_name_auto_generate": True, "source": "local"}], "publish": True})["uri"] for i in range(cfg["agents"])]
    print(f"Created {len(subjects)} subjects and {len(agents)} agents", flush=True)

    totals = {"resources": 0, "archival_objects": 0, "top_containers": 0, "digital_objects": 0, "batch_records": 0}
    for r in range(cfg["resources"]):
        batch, dos = build_tree_batch(rng, repo_uri, r, cfg, subjects, agents)
        t0 = time.monotonic()
        result = backend.call("POST", f"{repo_uri}/batch_imports", batch, timeout=3600)
        errors = [m for m in (result if isinstance(result, list) else [result]) if isinstance(m, dict) and m.get("errors")]
        if errors:
            raise SystemExit(f"batch import for collection {r + 1} reported errors: {json.dumps(errors)[:800]}")
        totals["resources"] += 1
        totals["archival_objects"] += sum(1 for rec in batch if rec["jsonmodel_type"] == "archival_object")
        totals["top_containers"] += cfg["containers"]
        totals["digital_objects"] += dos
        totals["batch_records"] += len(batch)
        print(f"  collection {r + 1}/{cfg['resources']}: {len(batch)} records in {time.monotonic() - t0:.1f}s", flush=True)

    summary = {"repository_uri": repo_uri, "repo_code": repo_code, "repo_id": repo_id, "preset": args.preset,
               "config": cfg, "seed": args.seed, "subjects": len(subjects), "agents": len(agents),
               "totals": totals, "seconds": round(time.monotonic() - started, 1)}
    print(json.dumps(summary, indent=2))
    if args.summary_json:
        with open(args.summary_json, "w") as fh:
            json.dump(summary, fh, indent=2)
    return 0


if __name__ == "__main__":
    sys.exit(main())
