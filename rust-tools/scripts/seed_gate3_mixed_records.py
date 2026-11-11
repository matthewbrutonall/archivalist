#!/usr/bin/env python3
"""Add bounded mixed STAFF fixtures to an already isolated profiling stack.

Creates records only, through the isolated ArchivesSpace API. No existing record is
updated or deleted. The verifier proves backend and Solr stack identity before login.
"""

import argparse
import json
import os
import re
import secrets
import sys

from seed_large_repository import Backend, load_profiling_stack


def first_uri(backend, path):
    ids = backend.call("GET", path, params={"all_ids": "true"})
    if not isinstance(ids, list) or not ids or not isinstance(ids[0], int):
        raise RuntimeError(f"no usable IDs at {path}")
    return f"{path}/{ids[0]}"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stack-state", required=True)
    parser.add_argument("--repo-id", type=int, required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--resume", action="store_true", help="reuse only verified URIs in the existing manifest")
    args = parser.parse_args(argv)
    if args.repo_id < 1 or os.path.exists(args.output) != args.resume:
        parser.error("positive --repo-id and an existing output only with --resume")

    stack = load_profiling_stack()
    state = stack.load_state(args.stack_state)
    stack.verify_identity(state, state["backend_host_url"], state["solr_host_url"])
    backend = Backend(state["backend_host_url"])
    backend.login(state["admin_user"], state["admin_password"])
    repo = f"/repositories/{args.repo_id}"
    repo_record = backend.call("GET", repo)
    if not repo_record.get("repo_code", "").startswith("SEED-"):
        raise RuntimeError("target repository is not a profiling seed repository")
    if args.resume:
        with open(args.output) as source:
            evidence = json.load(source)
        if evidence.get("stack_id") != state["stack_id"] or evidence.get("repo") != repo:
            raise RuntimeError("fixture manifest does not belong to this stack and repository")
        token = evidence.get("token")
        if not isinstance(token, str) or not re.fullmatch(r"[0-9a-f]{16}", token):
            raise RuntimeError("invalid fixture token")
        created = evidence.get("created")
        if not isinstance(created, dict):
            raise RuntimeError("invalid fixture URI map")
        for kind, uri in created.items():
            if not isinstance(uri, str) or backend.call("GET", uri).get("jsonmodel_type") != kind:
                raise RuntimeError(f"fixture record no longer matches manifest: {kind}")
    else:
        token = secrets.token_hex(8)
        created = {}
        evidence = {"stack_id": state["stack_id"], "repo": repo, "token": token, "created": created}
        fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.close(fd)

    def save_evidence():
        with open(args.output, "w") as output:
            json.dump(evidence, output, indent=2)

    def create(kind, path, payload):
        if kind in created:
            return created[kind]
        result = backend.call("POST", path, {"jsonmodel_type": kind, **payload})
        uri = result.get("uri")
        if not isinstance(uri, str) or not uri.startswith(path + "/"):
            raise RuntimeError(f"unexpected {kind} create response: {result}")
        created[kind] = uri
        save_evidence()
        print(f"{kind}: {uri}", flush=True)
        return uri

    def cm(label):
        return {"jsonmodel_type": "collection_management", "processing_plan": f"Gate 3 {token} {label}",
                "rights_determined": True}

    person = first_uri(backend, "/agents/people")
    create("agent_family", "/agents/families", {
        "names": [{"jsonmodel_type": "name_family", "family_name": f"Gate3-{token}",
                   "sort_name_auto_generate": True, "source": "local"}], "publish": True})
    create("container_profile", "/container_profiles", {
        "name": f"Gate3 container {token}", "dimension_units": "inches",
        "extent_dimension": "height", "height": "12", "width": "10", "depth": "15"})
    create("location_profile", "/location_profiles", {
        "name": f"Gate3 location {token}", "dimension_units": "inches",
        "height": "100", "width": "100", "depth": "100"})
    create("location", "/locations", {
        "building": f"Gate3 isolated building {token}",
        "barcode": f"G3L-{token}",
        "location_profile": {"ref": created["location_profile"]},
        "owner_repo": {"ref": repo}})
    resource = create("resource", f"{repo}/resources", {
        "title": f"Gate3 mixed resource {token}", "id_0": f"G3-{token}",
        "level": "collection", "publish": True, "finding_aid_language": "eng",
        "finding_aid_script": "Latn",
        "lang_materials": [{"language_and_script": {"language": "eng", "script": "Latn"}}],
        "extents": [{"portion": "whole", "number": "1", "extent_type": "linear_feet"}],
        "dates": [{"date_type": "inclusive", "label": "creation", "begin": "2026"}],
        "collection_management": cm("resource")})
    digital = create("digital_object", f"{repo}/digital_objects", {
        "title": f"Gate3 mixed digital {token}", "digital_object_id": f"G3D-{token}",
        "publish": True, "collection_management": cm("digital")})
    create("accession", f"{repo}/accessions", {
        "title": f"Gate3 mixed accession {token}", "id_0": f"G3A-{token}",
        "accession_date": "2026-01-01", "publish": True,
        "collection_management": cm("accession")})
    create("classification", f"{repo}/classifications", {
        "identifier": f"G3C-{token}", "title": f"Gate3 classification {token}"})
    create("classification_term", f"{repo}/classification_terms", {
        "identifier": f"G3T-{token}", "title": f"Gate3 term {token}",
        "classification": {"ref": created["classification"]}})
    create("digital_object_component", f"{repo}/digital_object_components", {
        "title": f"Gate3 component {token}", "component_id": f"G3DC-{token}",
        "digital_object": {"ref": digital}})
    create("assessment", f"{repo}/assessments", {
        "records": [{"ref": resource}], "surveyed_by": [{"ref": person}],
        "survey_begin": "2026-01-01", "purpose": f"Gate3 {token}"})
    create("event", f"{repo}/events", {
        "event_type": "custody_transfer", "timestamp": "2026-01-01T12:00:00Z",
        "linked_agents": [{"ref": person, "role": "recipient"}],
        "linked_records": [{"ref": resource, "role": "source"}]})
    create("job", f"{repo}/jobs", {
        "job": {"jsonmodel_type": "print_to_pdf_job", "source": resource}})

    print(f"evidence: {args.output}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
