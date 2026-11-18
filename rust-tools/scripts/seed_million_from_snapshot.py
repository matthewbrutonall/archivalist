#!/usr/bin/env python3
"""Resume-safe million-AO seed for a disposable ArchivesSpace stack.

This tool creates synthetic records through ArchivesSpace /batch_imports. It never
modifies existing records or a primary Solr core. An isolated stack is required.
Snapshot mode additionally requires a separate restore step and permission manifest.

Snapshot mode uses a private owner-permission manifest. Fictional mode uses the
summary from seed_large_repository.py as a verified bootstrap, with no client data.
Run --plan-only before provisioning; use --max-collections-this-run 1 as a canary,
then --resume to continue from the persistent checkpoint.
"""

import argparse
import fcntl
import hashlib
import importlib.util
import json
import math
import os
import random
import re
import secrets
import shutil
import stat
import sys
import time

def load_seed_module():
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "seed_large_repository.py")
    spec = importlib.util.spec_from_file_location("archivalist_seed_large_repository", path)
    module = importlib.util.module_from_spec(spec)
    previous, sys.dont_write_bytecode = sys.dont_write_bytecode, True
    try:
        spec.loader.exec_module(module)
    finally:
        sys.dont_write_bytecode = previous
    return module


seed = load_seed_module()
Backend = seed.Backend
build_tree_batch = seed.build_tree_batch
items_total = seed.items_total
load_profiling_stack = seed.load_profiling_stack


TREE = dict(resources=1, series=10, files=20, items=9, subjects=150, agents=150,
            containers=20, digital_object_fraction=0.02, agent_link_fraction=0.1)
FICTIONAL_TREES = (
    dict(TREE),
    dict(TREE, files=10, items=19, containers=10, digital_object_fraction=0.01),
    dict(TREE, files=25, items=7, containers=40, digital_object_fraction=0.05),
    dict(TREE, series=30, files=6, items=10, containers=30, agent_link_fraction=0.15),
)
MAX_BATCH_RECORDS = 5000
MAX_BATCH_BYTES = 8 * 1024 * 1024
MIN_FREE_GIB = 60
MIN_FREE_DURING_GIB = 30


def plan(existing_ao_count, target_ao_count):
    if not isinstance(existing_ao_count, int) or existing_ao_count < 0:
        raise ValueError("existing AO count must be nonnegative")
    if not isinstance(target_ao_count, int) or target_ao_count < 1:
        raise ValueError("target AO count must be positive")
    per_tree = items_total(TREE)
    if any(items_total(cfg) != per_tree for cfg in FICTIONAL_TREES):
        raise ValueError("fictional tree templates must have equal AO counts")
    trees = max(0, math.ceil((target_ao_count - existing_ao_count) / per_tree))
    samples = [build_tree_batch(random.Random(1), "/repositories/999", i, cfg,
                                [f"/subjects/{j}" for j in range(1, TREE["subjects"] + 1)],
                                [f"/agents/people/{j}" for j in range(1, TREE["agents"] + 1)])[0]
               for i, cfg in enumerate(FICTIONAL_TREES)]
    sample_records = max(len(sample) for sample in samples)
    sample_bytes = max(len(json.dumps(sample, separators=(",", ":")).encode()) for sample in samples)
    if sample_records > MAX_BATCH_RECORDS or sample_bytes > MAX_BATCH_BYTES:
        raise ValueError("a collection batch exceeds the bounded import size")
    return {"existing_ao_count": existing_ao_count, "target_ao_count": target_ao_count,
            "archival_objects_per_collection": per_tree, "collections_to_create": trees,
            "planned_final_ao_count": existing_ao_count + trees * per_tree,
            "sample_batch_records": sample_records, "sample_batch_bytes": sample_bytes}


def load_fictional_bootstrap(path):
    with open(path) as source:
        summary = json.load(source)
    code = summary.get("repo_code", "")
    if not re.fullmatch(r"SEED-[0-9]+", code):
        raise ValueError("fictional bootstrap has an invalid seed repository code")
    if summary.get("repository_uri") != f"/repositories/{summary.get('repo_id')}":
        raise ValueError("fictional bootstrap repository URI mismatch")
    expected = items_total(summary.get("config", {}))
    if summary.get("totals", {}).get("archival_objects") != expected or expected < 1:
        raise ValueError("fictional bootstrap AO count mismatch")
    if summary.get("subjects", 0) < TREE["subjects"] or summary.get("agents", 0) < TREE["agents"]:
        raise ValueError("fictional bootstrap lacks enough authorities")
    digest = hashlib.sha256(json.dumps(summary, sort_keys=True).encode()).hexdigest()
    return {"baseline_ao_count": expected, "source_sha256": digest, "bootstrap_repo_code": code,
            "bootstrap_repo_uri": summary["repository_uri"]}


def fictional_batch(index, repo_uri, seed_value, subjects, agents):
    cfg = FICTIONAL_TREES[index % len(FICTIONAL_TREES)]
    batch, _ = build_tree_batch(random.Random(seed_value + index), repo_uri, index, cfg, subjects, agents)
    for offset, record in enumerate(batch):
        if record["jsonmodel_type"] != "archival_object":
            continue
        if record["level"] == "item":
            if (index + offset) % 17 == 0:
                record["publish"] = False
            if (index + offset) % 11 == 0:
                record["dates"] = dates_for_fictional_item(index, offset)
    return batch


def dates_for_fictional_item(index, offset):
    year = 1850 + ((index * 97 + offset) % 160)
    return [{"date_type": "single", "label": "creation", "begin": str(year), "expression": str(year)}]


def load_snapshot_manifest(path, state):
    if stat.S_IMODE(os.stat(path).st_mode) & 0o077:
        raise ValueError("snapshot manifest must be private (mode 0600)")
    with open(path) as source:
        manifest = json.load(source)
    if manifest.get("stack_id") != state["stack_id"]:
        raise ValueError("snapshot manifest belongs to another stack")
    if manifest.get("db_volume") != state["volumes"]["db"]:
        raise ValueError("snapshot manifest does not identify this isolated DB volume")
    if not re.fullmatch(r"[0-9a-f]{64}", str(manifest.get("source_sha256", ""))):
        raise ValueError("snapshot manifest lacks a source SHA-256")
    if not isinstance(manifest.get("permission_reference"), str) or not manifest["permission_reference"].strip():
        raise ValueError("snapshot manifest lacks owner permission reference")
    source_path = manifest.get("source_path")
    if not isinstance(source_path, str) or not os.path.isfile(source_path):
        raise ValueError("snapshot source file is missing")
    if stat.S_IMODE(os.stat(source_path).st_mode) & 0o077:
        raise ValueError("snapshot source file must be private (mode 0600)")
    digest = hashlib.sha256()
    with open(source_path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != manifest["source_sha256"]:
        raise ValueError("snapshot source SHA-256 mismatch")
    if not isinstance(manifest.get("baseline_ao_count"), int) or manifest["baseline_ao_count"] < 0:
        raise ValueError("snapshot manifest lacks a baseline AO count")
    return manifest


def ao_count(stack, state):
    return int(stack.mysql_exec(state, "SELECT COUNT(*) FROM archivesspace.archival_object;"))


def write_new_checkpoint(path, record):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as out:
        json.dump(record, out, indent=2)
        out.flush()
        os.fsync(out.fileno())


def save_checkpoint(path, record):
    tmp = f"{path}.tmp.{os.getpid()}"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as out:
        json.dump(record, out, indent=2)
        out.flush()
        os.fsync(out.fileno())
    os.replace(tmp, path)


def reconcile_inflight(progress, current_count, per_tree):
    completed = progress["next_collection"]
    expected = progress["baseline_ao_count"] + completed * per_tree
    inflight = progress["inflight_collection"]
    if inflight is None:
        if current_count != expected:
            raise ValueError("AO count changed outside the completed seed batches")
        return "ready"
    if inflight != completed:
        raise ValueError("invalid in-flight collection checkpoint")
    if current_count == expected:
        return "retry"
    if current_count == expected + per_tree:
        progress["next_collection"] = completed + 1
        progress["inflight_collection"] = None
        return "committed"
    raise ValueError("in-flight batch has an unexpected AO count; manual review required")


def find_repository(backend, repo_code):
    if not re.fullmatch(r"SEED-MILLION-[0-9]+-[0-9a-f]{8}", repo_code):
        raise ValueError("invalid seed repository code")
    repos = backend.call("GET", "/repositories")
    matches = [repo for repo in repos if repo.get("repo_code") == repo_code]
    if len(matches) > 1:
        raise ValueError("duplicate seed repository code")
    return matches[0] if matches else None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan-only", action="store_true")
    parser.add_argument("--existing-ao-count", type=int)
    parser.add_argument("--target-ao-count", type=int, default=1_000_000)
    parser.add_argument("--stack-state")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--snapshot-manifest")
    source.add_argument("--fictional-bootstrap-summary")
    parser.add_argument("--checkpoint")
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--max-collections-this-run", type=int)
    parser.add_argument("--seed", type=int, default=20261118)
    args = parser.parse_args(argv)

    if args.target_ao_count < 1 or (args.max_collections_this_run is not None and args.max_collections_this_run < 1):
        parser.error("target and collection limit must be positive")
    if args.plan_only:
        if args.existing_ao_count is None:
            parser.error("--plan-only requires --existing-ao-count")
        print(json.dumps(plan(args.existing_ao_count, args.target_ao_count), indent=2))
        return 0
    if not all((args.stack_state, args.checkpoint)) or not (args.snapshot_manifest or args.fictional_bootstrap_summary):
        parser.error("live seed requires --stack-state, --checkpoint and one source mode")
    if args.existing_ao_count is not None:
        parser.error("live seed reads the AO count from the verified isolated database")
    if shutil.disk_usage("/var/lib/docker").free < MIN_FREE_GIB * 1024**3:
        raise RuntimeError(f"less than {MIN_FREE_GIB} GiB free for a large isolated seed")

    lock_path = args.checkpoint + ".lock"
    with open(lock_path, "a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        stack = load_profiling_stack()
        state = stack.load_state(args.stack_state)
        fictional = bool(args.fictional_bootstrap_summary)
        manifest = (load_fictional_bootstrap(args.fictional_bootstrap_summary) if fictional
                    else load_snapshot_manifest(args.snapshot_manifest, state))
        stack.verify_identity(state, state["backend_host_url"], state["solr_host_url"])
        baseline = manifest["baseline_ao_count"]
        sizing = plan(baseline, args.target_ao_count)
        if sizing["collections_to_create"] == 0:
            print(json.dumps(sizing, indent=2))
            return 0
        backend = Backend(state["backend_host_url"])
        backend.login(state["admin_user"], state["admin_password"])
        if fictional:
            repositories = backend.call("GET", "/repositories")
            allowed = {state["identity_repo_code"], manifest["bootstrap_repo_code"]}
            if args.resume:
                with open(args.checkpoint) as source_file:
                    seed_code = json.load(source_file)["repo_code"]
                possible_inventories = (allowed, allowed | {seed_code})
            else:
                possible_inventories = (allowed,)
            if {repo["repo_code"] for repo in repositories} not in possible_inventories:
                raise ValueError("fictional stack repository inventory changed")
            bootstrap_repo = next(repo for repo in repositories if repo["repo_code"] == manifest["bootstrap_repo_code"])
            if bootstrap_repo["uri"] != manifest["bootstrap_repo_uri"]:
                raise ValueError("fictional bootstrap repository identity changed")
            if not args.resume and ao_count(stack, state) != baseline:
                raise ValueError("fictional bootstrap AO count changed")

        if args.resume:
            if stat.S_IMODE(os.stat(args.checkpoint).st_mode) & 0o077:
                raise ValueError("checkpoint must be private (mode 0600)")
            with open(args.checkpoint) as source:
                progress = json.load(source)
            if (progress.get("stack_id"), progress.get("source_sha256"), progress.get("baseline_ao_count"),
                progress.get("target_ao_count"), progress.get("planned_collections"), progress.get("seed"),
                progress.get("fictional", False)) != (
                state["stack_id"], manifest["source_sha256"], baseline, args.target_ao_count,
                sizing["collections_to_create"], args.seed, fictional):
                raise ValueError("checkpoint does not match the source and plan")
        else:
            if os.path.exists(args.checkpoint):
                raise FileExistsError("checkpoint already exists; use --resume")
            repo_code = f"SEED-MILLION-{int(time.time())}-{secrets.token_hex(4)}"
            progress = {"stack_id": state["stack_id"], "source_sha256": manifest["source_sha256"],
                        "baseline_ao_count": baseline, "target_ao_count": args.target_ao_count,
                        "planned_collections": sizing["collections_to_create"], "seed": args.seed,
                        "fictional": fictional,
                        "repo_code": repo_code, "repo_uri": None, "next_collection": 0,
                        "inflight_collection": None, "global_refs": None}
            write_new_checkpoint(args.checkpoint, progress)

        repo = find_repository(backend, progress["repo_code"])
        if repo is None:
            if progress["repo_uri"] is not None:
                raise ValueError("checkpoint repository is missing")
            created = backend.call("POST", "/repositories", {
                "repo_code": progress["repo_code"], "name": f"Synthetic million-object test {progress['repo_code']}",
                "publish": True})
            progress["repo_uri"] = f"/repositories/{created['id']}"
            save_checkpoint(args.checkpoint, progress)
        else:
            if progress["repo_uri"] not in (None, repo["uri"]):
                raise ValueError("checkpoint repository identity mismatch")
            progress["repo_uri"] = repo["uri"]
            save_checkpoint(args.checkpoint, progress)

        subject_ids = backend.call("GET", "/subjects", params={"all_ids": "true"})
        agent_ids = backend.call("GET", "/agents/people", params={"all_ids": "true"})
        if not subject_ids or not agent_ids:
            raise ValueError("real snapshot lacks subjects or agents for linked seed records")
        subjects = [f"/subjects/{i}" for i in sorted(subject_ids)[:TREE["subjects"]]]
        agents = [f"/agents/people/{i}" for i in sorted(agent_ids)[:TREE["agents"]]]
        global_refs = {"subjects": subjects, "agents": agents}
        if progress["global_refs"] is None:
            progress["global_refs"] = global_refs
            save_checkpoint(args.checkpoint, progress)
        elif progress["global_refs"] != global_refs:
            raise ValueError("global authority selection changed during seed")

        status = reconcile_inflight(progress, ao_count(stack, state), sizing["archival_objects_per_collection"])
        if status == "committed":
            save_checkpoint(args.checkpoint, progress)
        source_label = "fictional bootstrap" if fictional else "isolated snapshot"
        print(f"Verified {source_label}; {status}; {progress['next_collection']}/{progress['planned_collections']} collections complete", flush=True)

        limit = args.max_collections_this_run or progress["planned_collections"]
        completed_this_run = 0
        for index in range(progress["next_collection"], progress["planned_collections"]):
            if completed_this_run >= limit:
                break
            if shutil.disk_usage("/var/lib/docker").free < MIN_FREE_DURING_GIB * 1024**3:
                raise RuntimeError("free disk fell below 30 GiB; checkpoint retained")
            batch = (fictional_batch(index, progress["repo_uri"], args.seed, subjects, agents)
                     if fictional else build_tree_batch(random.Random(args.seed + index), progress["repo_uri"],
                                                        index, TREE, subjects, agents)[0])
            if len(batch) > MAX_BATCH_RECORDS or len(json.dumps(batch).encode()) > MAX_BATCH_BYTES:
                raise ValueError("collection batch exceeded size limit")
            progress["inflight_collection"] = index
            save_checkpoint(args.checkpoint, progress)
            result = backend.call("POST", f"{progress['repo_uri']}/batch_imports", batch, timeout=3600)
            responses = result if isinstance(result, list) else [result]
            if any(isinstance(row, dict) and row.get("errors") for row in responses):
                raise RuntimeError(f"batch import {index} returned errors; checkpoint retained")
            expected = baseline + (index + 1) * sizing["archival_objects_per_collection"]
            if ao_count(stack, state) != expected:
                raise RuntimeError(f"batch import {index} did not reach its exact AO count; checkpoint retained")
            progress["next_collection"] = index + 1
            progress["inflight_collection"] = None
            save_checkpoint(args.checkpoint, progress)
            completed_this_run += 1
            print(f"collection {index + 1}/{progress['planned_collections']}: {expected} total AOs", flush=True)

        print(json.dumps({"checkpoint": args.checkpoint, "completed_collections": progress["next_collection"],
                          "planned_collections": progress["planned_collections"],
                          "expected_total_ao_count": baseline + progress["next_collection"] * sizing["archival_objects_per_collection"]}, indent=2))
        return 0


if __name__ == "__main__":
    sys.exit(main())
