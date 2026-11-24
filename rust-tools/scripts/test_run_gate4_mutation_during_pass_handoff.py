#!/usr/bin/env python3
"""Offline unit tests for Gate 4 create/update/delete-during-pass and Ruby handoff harness."""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
import urllib.parse

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import run_gate4_mutation_during_pass_handoff as harness  # noqa: E402


class FakeAspaceRepo3Api:
    def __init__(self) -> None:
        self.resource = {
            "uri": "/repositories/3/resources/1",
            "title": "Fixture Resource 1",
            "lock_version": 0,
            "publish": True,
            "system_mtime": "2026-11-18T17:57:46Z",
        }
        self.parent_ao = {
            "uri": "/repositories/3/archival_objects/2001",
            "title": "Fixture Parent File",
            "level": "file",
            "lock_version": 0,
            "publish": True,
            "resource": {"ref": "/repositories/3/resources/1"},
            "system_mtime": "2026-11-18T17:57:46Z",
        }
        self.ao_2009 = {
            "uri": "/repositories/3/archival_objects/2009",
            "title": "Fixture Leaf A",
            "display_string": "Fixture Leaf A",
            "level": "item",
            "position": 7,
            "lock_version": 0,
            "publish": True,
            "suppressed": False,
            "resource": {"ref": "/repositories/3/resources/1"},
            "parent": {"ref": "/repositories/3/archival_objects/2001"},
            "instances": [],
            "system_mtime": "2026-11-18T17:57:46Z",
        }
        self.ao_2010 = {
            "uri": "/repositories/3/archival_objects/2010",
            "title": "Fixture Leaf B",
            "display_string": "Fixture Leaf B",
            "level": "item",
            "position": 8,
            "lock_version": 0,
            "publish": True,
            "suppressed": False,
            "resource": {"ref": "/repositories/3/resources/1"},
            "parent": {"ref": "/repositories/3/archival_objects/2001"},
            "instances": [],
            "system_mtime": "2026-11-18T17:57:46Z",
        }
        self.created_records: dict[int, dict] = {}
        self.deleted_ids: set[int] = set()
        self.ao_ids = list(range(1, 2011))
        self.children_by_id: dict[int, list] = {
            2001: [{"uri": f"/repositories/3/archival_objects/{i}"} for i in range(2002, 2011)],
            2009: [],
            2010: [],
        }
        self.next_id = 2011
        self.post_calls: list[tuple[str, dict]] = []
        self.delete_calls: list[str] = []

    def get_json(self, path: str, params=None):
        if path == "/repositories/3/resources" and params == {"all_ids": "true"}:
            return [1]
        if path == "/repositories/3/resources/1":
            return copy.deepcopy(self.resource)
        if path == "/repositories/3/archival_objects" and params == {"all_ids": "true"}:
            return list(self.ao_ids)
        if path == "/repositories/3/archival_objects/2001":
            return copy.deepcopy(self.parent_ao)
        if path == "/repositories/3/archival_objects/2009":
            return copy.deepcopy(self.ao_2009)
        if path == "/repositories/3/archival_objects/2010":
            if 2010 in self.deleted_ids:
                raise AssertionError("2010 was deleted")
            return copy.deepcopy(self.ao_2010)
        if path.endswith("/children"):
            ao_id = int(path.split("/")[-2])
            return copy.deepcopy(self.children_by_id.get(ao_id, []))
        for cid, crec in self.created_records.items():
            if path == f"/repositories/3/archival_objects/{cid}":
                return copy.deepcopy(crec)
        raise AssertionError(f"Unhandled GET {path} params={params}")

    def get_status_and_json(self, path: str):
        if path == "/repositories/3/archival_objects/2010" and 2010 in self.deleted_ids:
            return 404, {"error": "ArchivalObject not found"}
        return 200, self.get_json(path)

    def post_json(self, path: str, payload: dict):
        self.post_calls.append((path, copy.deepcopy(payload)))
        if path == "/repositories/3/archival_objects/2009":
            if payload.get("lock_version") != self.ao_2009["lock_version"]:
                raise AssertionError("Stale lock_version")
            self.ao_2009["title"] = payload["title"]
            self.ao_2009["display_string"] = payload["title"]
            self.ao_2009["lock_version"] += 1
            self.ao_2009["system_mtime"] = "2026-11-24T13:20:00Z"
            self.resource["system_mtime"] = "2026-11-24T13:20:00Z"
            return {
                "status": "Updated",
                "id": 2009,
                "lock_version": self.ao_2009["lock_version"],
                "uri": "/repositories/3/archival_objects/2009",
            }
        if path == "/repositories/3/archival_objects":
            cid = self.next_id
            self.next_id += 1
            uri = f"/repositories/3/archival_objects/{cid}"
            rec = {
                "uri": uri,
                "title": payload["title"],
                "display_string": payload["title"],
                "level": payload["level"],
                "publish": payload["publish"],
                "lock_version": 0,
                "position": 8,
                "resource": copy.deepcopy(payload["resource"]),
                "parent": copy.deepcopy(payload["parent"]),
                "instances": [],
                "system_mtime": "2026-11-24T13:20:02Z",
            }
            self.created_records[cid] = rec
            self.children_by_id[cid] = []
            self.ao_ids.append(cid)
            self.children_by_id[2001].append({"uri": uri})
            self.resource["system_mtime"] = "2026-11-24T13:20:02Z"
            return {"status": "Created", "id": cid, "lock_version": 0, "uri": uri}
        raise AssertionError(f"Unhandled POST {path}")

    def delete_record(self, path: str):
        self.delete_calls.append(path)
        if path != "/repositories/3/archival_objects/2010":
            raise AssertionError(f"Unexpected DELETE {path}")
        self.deleted_ids.add(2010)
        self.ao_ids.remove(2010)
        self.children_by_id[2001] = [
            c for c in self.children_by_id[2001] if c["uri"] != "/repositories/3/archival_objects/2010"
        ]
        self.resource["system_mtime"] = "2026-11-24T13:20:01Z"
        return {"status": "Deleted", "id": 2010}


class FakeMutationHandoffSolrEnv:
    def __init__(self, candidate_core: str, mutation_result: dict) -> None:
        self.candidate_core = candidate_core
        self.mutation_result = mutation_result
        self.swapped = False
        self.swap_calls: list[tuple[str, str]] = []
        self.round_applied = False
        self.corrupt_post_swap = False

    def swap(self, solr_url: str, cand_core: str, c1: str, c2: str) -> dict:
        self.swap_calls.append((c1, c2))
        if not self.corrupt_post_swap:
            self.swapped = not self.swapped
        return {"responseHeader": {"status": 0, "QTime": 1}}

    def http_get_json(self, url: str) -> dict:
        if "action=STATUS" in url:
            default_docs = harness.EXPECTED_TOTAL_DOCS if self.swapped else 0
            cand_docs = 0 if self.swapped else harness.EXPECTED_TOTAL_DOCS
            default_dir = (
                f"/var/solr/data/{self.candidate_core}"
                if self.swapped
                else "/var/solr/data/archivesspace"
            )
            cand_dir = (
                "/var/solr/data/archivesspace"
                if self.swapped
                else f"/var/solr/data/{self.candidate_core}"
            )
            cand_ver = 6610 if self.round_applied else 6600
            default_ver = cand_ver if self.swapped else 2
            other_ver = 2 if self.swapped else cand_ver
            cand_commit = "1795520005000" if self.round_applied else "1795520000000"
            return {
                "responseHeader": {"status": 0},
                "status": {
                    "archivesspace": {
                        "name": "archivesspace",
                        "instanceDir": default_dir,
                        "dataDir": f"{default_dir}/data/",
                        "index": {
                            "numDocs": default_docs,
                            "maxDoc": default_docs,
                            "deletedDocs": 0,
                            "version": default_ver,
                            "segmentCount": 25 if default_docs else 0,
                            "userData": {"commitTimeMSec": cand_commit} if default_docs else {},
                        },
                    },
                    harness.EXPECTED_IDENTITY_CORE: {
                        "name": harness.EXPECTED_IDENTITY_CORE,
                        "instanceDir": f"/var/solr/data/{harness.EXPECTED_IDENTITY_CORE}",
                        "dataDir": f"/var/solr/data/{harness.EXPECTED_IDENTITY_CORE}/data/",
                        "index": {"numDocs": 0, "maxDoc": 0, "deletedDocs": 0, "version": 2, "segmentCount": 0, "userData": {}},
                    },
                    harness.ACCIDENTAL_EMPTY_CANDIDATE_CORE: {
                        "name": harness.ACCIDENTAL_EMPTY_CANDIDATE_CORE,
                        "instanceDir": f"/var/solr/data/{harness.ACCIDENTAL_EMPTY_CANDIDATE_CORE}",
                        "dataDir": f"/var/solr/data/{harness.ACCIDENTAL_EMPTY_CANDIDATE_CORE}/data/",
                        "index": {"numDocs": 0, "maxDoc": 0, "deletedDocs": 0, "version": 2, "segmentCount": 0, "userData": {}},
                    },
                    harness.PRIOR_UNWATERMARKED_CANDIDATE_CORE: {
                        "name": harness.PRIOR_UNWATERMARKED_CANDIDATE_CORE,
                        "instanceDir": f"/var/solr/data/{harness.PRIOR_UNWATERMARKED_CANDIDATE_CORE}",
                        "dataDir": f"/var/solr/data/{harness.PRIOR_UNWATERMARKED_CANDIDATE_CORE}/data/",
                        "index": {
                            "numDocs": harness.EXPECTED_TOTAL_DOCS,
                            "maxDoc": harness.EXPECTED_TOTAL_DOCS,
                            "deletedDocs": 0,
                            "version": 6546,
                            "segmentCount": 24,
                            "userData": {"commitTimeMSec": "1795482353266"},
                        },
                    },
                    harness.PRIOR_WATERMARKED_CANDIDATE_CORE: {
                        "name": harness.PRIOR_WATERMARKED_CANDIDATE_CORE,
                        "instanceDir": f"/var/solr/data/{harness.PRIOR_WATERMARKED_CANDIDATE_CORE}",
                        "dataDir": f"/var/solr/data/{harness.PRIOR_WATERMARKED_CANDIDATE_CORE}/data/",
                        "index": {
                            "numDocs": harness.EXPECTED_TOTAL_DOCS,
                            "maxDoc": harness.EXPECTED_TOTAL_DOCS,
                            "deletedDocs": 0,
                            "version": 6600,
                            "segmentCount": 25,
                            "userData": {"commitTimeMSec": "1795506740019"},
                        },
                    },
                    harness.FIXED_NATIVE_CORE: {
                        "name": harness.FIXED_NATIVE_CORE,
                        "instanceDir": f"/var/solr/data/{harness.FIXED_NATIVE_CORE}",
                        "dataDir": f"/var/solr/data/{harness.FIXED_NATIVE_CORE}/data/",
                        "index": {
                            "numDocs": harness.EXPECTED_TOTAL_DOCS,
                            "maxDoc": harness.EXPECTED_TOTAL_DOCS,
                            "deletedDocs": 0,
                            "version": 8848,
                            "segmentCount": 18,
                            "userData": {"commitTimeMSec": "1795493721386"},
                        },
                    },
                    self.candidate_core: {
                        "name": self.candidate_core,
                        "instanceDir": cand_dir,
                        "dataDir": f"{cand_dir}/data/",
                        "index": {
                            "numDocs": cand_docs,
                            "maxDoc": cand_docs,
                            "deletedDocs": 0,
                            "version": other_ver,
                            "segmentCount": 25 if cand_docs else 0,
                            "userData": {"commitTimeMSec": cand_commit} if cand_docs else {},
                        },
                    },
                },
            }

        parsed = urllib.parse.urlparse(url)
        qs = urllib.parse.parse_qs(parsed.query)
        q = (qs.get("q") or [""])[0]
        fq = (qs.get("fq") or [None])[0]

        if q == "*:*" and fq is None:
            if f"/{harness.DEFAULT_CORE}/select" in parsed.path:
                return {"response": {"numFound": harness.EXPECTED_TOTAL_DOCS if self.swapped else 0}}
            if f"/{self.candidate_core}/select" in parsed.path:
                return {"response": {"numFound": 0 if self.swapped else harness.EXPECTED_TOTAL_DOCS}}
            if (
                f"/{harness.EXPECTED_IDENTITY_CORE}/select" in parsed.path
                or f"/{harness.ACCIDENTAL_EMPTY_CANDIDATE_CORE}/select" in parsed.path
            ):
                return {"response": {"numFound": 0}}
            return {"response": {"numFound": harness.EXPECTED_TOTAL_DOCS}}

        if q == "*:*" and fq == 'repository:"/repositories/3" AND primary_type:archival_object':
            return {"response": {"numFound": harness.EXPECTED_REPO3_AO_COUNT}}

        if q == "*:*" and fq and fq.startswith("primary_type:"):
            ptype = fq.split(":", 1)[1]
            return {"response": {"numFound": harness.EXPECTED_TYPE_COUNTS[ptype]}}

        upd = self.mutation_result["mutations"]["update"]
        cre = self.mutation_result["mutations"]["create"]
        dele = self.mutation_result["mutations"]["delete"]
        res = self.mutation_result["touched_ancestors"]["resource"]

        if q == f'id:"{upd["uri"]}"':
            return {
                "response": {
                    "numFound": 1,
                    "docs": [
                        {
                            "id": upd["uri"],
                            "uri": upd["uri"],
                            "title": upd["title_after"],
                            "primary_type": "archival_object",
                            "repository": "/repositories/3",
                            "publish": True,
                            "json": json.dumps(
                                {
                                    "uri": upd["uri"],
                                    "title": upd["title_after"],
                                    "display_string": upd["title_after"],
                                    "lock_version": 1,
                                    "system_mtime": upd["system_mtime_after"],
                                }
                            ),
                        }
                    ],
                }
            }
        if q == f'id:"{dele["uri"]}"':
            return {"response": {"numFound": 0, "docs": []}}
        if q == f'id:"{cre["uri"]}"':
            return {
                "response": {
                    "numFound": 1,
                    "docs": [
                        {
                            "id": cre["uri"],
                            "uri": cre["uri"],
                            "title": cre["title_after"],
                            "primary_type": "archival_object",
                            "repository": "/repositories/3",
                            "publish": True,
                            "json": json.dumps(
                                {
                                    "uri": cre["uri"],
                                    "title": cre["title_after"],
                                    "display_string": cre["title_after"],
                                    "lock_version": 0,
                                    "position": 8,
                                    "resource": {"ref": cre["resource_ref"]},
                                    "parent": {"ref": cre["parent_ref"]},
                                    "system_mtime": cre["system_mtime_after"],
                                }
                            ),
                        }
                    ],
                }
            }
        if q == f'id:"{res["uri"]}"':
            return {
                "response": {
                    "numFound": 1,
                    "docs": [
                        {
                            "id": res["uri"],
                            "uri": res["uri"],
                            "title": "Fixture Resource 1",
                            "primary_type": "resource",
                            "repository": "/repositories/3",
                            "publish": True,
                            "json": json.dumps(
                                {
                                    "uri": res["uri"],
                                    "title": "Fixture Resource 1",
                                    "system_mtime": res["system_mtime_after"],
                                }
                            ),
                        }
                    ],
                }
            }
        raise AssertionError(f"Unhandled fake Solr query: {url}")


class TestGate4MutationDuringPassHandoff(unittest.TestCase):
    def test_verify_repo3_mutation_targets_read_only_passes_and_rejects_bad_state(self) -> None:
        api = FakeAspaceRepo3Api()
        res = harness.verify_repo3_mutation_targets_read_only(api)
        self.assertEqual(res["repo3_ao_count"], 2010)
        self.assertTrue(res["update_target_ao"]["is_leaf"])
        self.assertTrue(res["delete_target_ao"]["is_leaf"])
        self.assertEqual(res["delete_target_ao"]["position"], 8)

        # Reject if delete target has children (not a leaf)
        bad_api = FakeAspaceRepo3Api()
        bad_api.children_by_id[2010] = [{"uri": "/repositories/3/archival_objects/9999"}]
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.verify_repo3_mutation_targets_read_only(bad_api)

        # Reject if update target lock_version != 0
        bad_api2 = FakeAspaceRepo3Api()
        bad_api2.ao_2009["lock_version"] = 2
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.verify_repo3_mutation_targets_read_only(bad_api2)

    def test_select_repo4_ao_sentinel_uri_numeric_order_and_filtering(self) -> None:
        # Must use numeric ordering (10000 < 20000 < 100000) and ignore other repos, types, and malformed URIs
        native_docs = {
            "/repositories/2": {"id": "/repositories/2", "primary_type": "repository"},
            "/repositories/3/archival_objects/1": {
                "id": "/repositories/3/archival_objects/1",
                "primary_type": "archival_object",
                "repository": "/repositories/3",
            },
            "/repositories/3/archival_objects/2010": {
                "id": "/repositories/3/archival_objects/2010",
                "primary_type": "archival_object",
                "repository": "/repositories/3",
            },
            "/repositories/4/resources/1": {
                "id": "/repositories/4/resources/1",
                "primary_type": "resource",
                "repository": "/repositories/4",
            },
            "/repositories/4/top_containers/1": {
                "id": "/repositories/4/top_containers/1",
                "primary_type": "top_container",
                "repository": "/repositories/4",
            },
            "/repositories/4/archival_objects/0": {"id": "/repositories/4/archival_objects/0"},
            "/repositories/4/archival_objects/01": {"id": "/repositories/4/archival_objects/01"},
            "/repositories/4/archival_objects/-5": {"id": "/repositories/4/archival_objects/-5"},
            "/repositories/4/archival_objects/abc": {"id": "/repositories/4/archival_objects/abc"},
            "/repositories/4/archival_objects/1/children": {"id": "/repositories/4/archival_objects/1/children"},
            "/repositories/4/archival_objects/": {"id": "/repositories/4/archival_objects/"},
            "/repositories/4/archival_objects/2": {
                "id": "/repositories/4/archival_objects/2",
                "primary_type": "resource",
                "repository": "/repositories/4",
            },
            "/repositories/4/archival_objects/3": {
                "id": "/repositories/4/archival_objects/3",
                "primary_type": "archival_object",
                "repository": "/repositories/3",
            },
            "/repositories/4/archival_objects/100000": {
                "id": "/repositories/4/archival_objects/100000",
                "primary_type": "archival_object",
                "repository": "/repositories/4",
            },
            "/repositories/4/archival_objects/20000": {
                "id": "/repositories/4/archival_objects/20000",
                "primary_type": "archival_object",
                "repository": "/repositories/4",
            },
            "/repositories/4/archival_objects/10000": {
                "id": "/repositories/4/archival_objects/10000",
                "primary_type": "archival_object",
                "repository": "/repositories/4",
            },
            "/repositories/4/archival_objects/10001": {
                "id": "/repositories/4/archival_objects/10001",
                "primary_type": "archival_object",
                "repository": "/repositories/4",
            },
        }
        chosen = harness.select_repo4_ao_sentinel_uri(native_docs)
        self.assertEqual(chosen, "/repositories/4/archival_objects/10000")
        self.assertEqual(
            harness.validate_repo4_ao_sentinel_uri(chosen, native_docs_by_id=native_docs),
            10000,
        )

        # Lexicographically "10000" < "9999", but numerically 9999 < 10000
        with_9999 = dict(native_docs)
        with_9999["/repositories/4/archival_objects/9999"] = {
            "id": "/repositories/4/archival_objects/9999",
            "primary_type": "archival_object",
            "repository": "/repositories/4",
        }
        self.assertEqual(
            harness.select_repo4_ao_sentinel_uri(with_9999),
            "/repositories/4/archival_objects/9999",
        )

    def test_select_repo4_ao_sentinel_uri_refuses_empty_or_missing_repo4_ao_set(self) -> None:
        only_other_records = {
            "/repositories/3/archival_objects/1": {
                "id": "/repositories/3/archival_objects/1",
                "primary_type": "archival_object",
                "repository": "/repositories/3",
            },
            "/repositories/4/resources/2": {
                "id": "/repositories/4/resources/2",
                "primary_type": "resource",
                "repository": "/repositories/4",
            },
            "/repositories/4/archival_objects/0": {"id": "/repositories/4/archival_objects/0"},
            "/repositories/4/archival_objects/bad": {"id": "/repositories/4/archival_objects/bad"},
        }
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.select_repo4_ao_sentinel_uri({})
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.select_repo4_ao_sentinel_uri(only_other_records)
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.validate_repo4_ao_sentinel_uri(
                "/repositories/4/archival_objects/10000",
                native_docs_by_id=only_other_records,
            )

    def test_verify_repo4_sentinel_in_fixed_native_core_probes_and_verifies_doc(self) -> None:
        def fake_http_get_json(url: str) -> dict:
            if "sort=id+asc" in url:
                return {
                    "response": {
                        "numFound": 998970,
                        "docs": [
                            {
                                "id": "/repositories/4/archival_objects/10000",
                                "uri": "/repositories/4/archival_objects/10000",
                                "title": "Repo 4 First AO 10000",
                                "primary_type": "archival_object",
                                "repository": "/repositories/4",
                            },
                            {
                                "id": "/repositories/4/archival_objects/100000",
                                "uri": "/repositories/4/archival_objects/100000",
                                "title": "Repo 4 AO 100000",
                                "primary_type": "archival_object",
                                "repository": "/repositories/4",
                            },
                        ],
                    }
                }
            if "id%3A%22%2Frepositories%2F4%2Farchival_objects%2F10000%22" in url:
                return {
                    "response": {
                        "numFound": 1,
                        "docs": [
                            {
                                "id": "/repositories/4/archival_objects/10000",
                                "uri": "/repositories/4/archival_objects/10000",
                                "title": "Repo 4 First AO 10000",
                                "primary_type": "archival_object",
                                "repository": "/repositories/4",
                            }
                        ],
                    }
                }
            raise AssertionError(f"Unexpected URL: {url}")

        info = harness.verify_repo4_sentinel_in_fixed_native_core(
            "http://127.0.0.1:28983/solr",
            harness.FIXED_NATIVE_CORE,
            http_get_json=fake_http_get_json,
        )
        self.assertEqual(info["sentinel_uri"], "/repositories/4/archival_objects/10000")
        self.assertEqual(info["sentinel_ao_id"], 10000)
        self.assertTrue(info["verified_in_fixed_native_core"])

    def test_extract_identity_nonce_from_repo2_doc_reads_candidate_doc_nonce(self) -> None:
        rotated_nonce = "852f279a5f0ba2945f5d0f2a3910550b"
        cand_docs = {
            "/repositories/2": {
                "id": "/repositories/2",
                "json": json.dumps(
                    {
                        "uri": "/repositories/2",
                        "repo_code": "ARCHPROF-844ef09c",
                        "name": f"archivalist profiling identity {rotated_nonce}",
                    }
                ),
            }
        }
        self.assertEqual(
            harness.extract_identity_nonce_from_repo2_doc(cand_docs, label="candidate"),
            rotated_nonce,
        )
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.extract_identity_nonce_from_repo2_doc({}, label="candidate")

    def test_guarded_mutations_wait_for_tlog_and_preserve_net_count(self) -> None:
        api = FakeAspaceRepo3Api()
        cand_core = "stage_staff_20261124_132000_12345_cafebabe_candidate"
        sentinel_uri = "/repositories/4/archival_objects/10000"
        native_docs = {
            sentinel_uri: {
                "id": sentinel_uri,
                "primary_type": "archival_object",
                "repository": "/repositories/4",
                "title": "Repo 4 First AO 10000",
            }
        }
        clock = {"mono": 100.0, "epoch": 1795520000.0, "polls": 0}
        get_urls: list[str] = []

        def monotonic_fn() -> float:
            return clock["mono"]

        def time_fn() -> float:
            return clock["epoch"]

        def sleep_fn(dt: float) -> None:
            clock["mono"] += dt
            clock["epoch"] += dt

        def find_cli_proc_fn(core: str):
            self.assertEqual(core, cand_core)
            return {"pid": 54321, "comm": "aspace-cli", "vm_rss_kib": 182400, "vm_hwm_kib": 198100}

        def http_get_json(url: str) -> dict:
            if "/select?" in url:
                return {"response": {"numFound": 0}}
            if "/get?" in url:
                get_urls.append(url)
                self.assertNotIn("repositories%2F4%2Farchival_objects%2F1&", url)
                self.assertNotIn("repositories%2F4%2Farchival_objects%2F1$", url)
                clock["polls"] += 1
                # First poll (3 requests): repo 3 is in tlog, but repo 4 sentinel 10000 not yet in tlog
                if "archival_objects%2F2009" in url:
                    return {"doc": {"id": "/repositories/3/archival_objects/2009", "title": "Fixture Leaf A"}}
                if "archival_objects%2F2010" in url:
                    return {"doc": {"id": "/repositories/3/archival_objects/2010", "title": "Fixture Leaf B"}}
                if "repositories%2F4%2Farchival_objects%2F10000" in url:
                    if clock["polls"] <= 3:
                        return {"doc": None}
                    return {"doc": {"id": sentinel_uri, "title": "Repo 4 First AO 10000"}}
            raise AssertionError(f"Unexpected URL {url}")

        with tempfile.TemporaryDirectory() as tmpdir:
            ev_path = os.path.join(tmpdir, "mutations.json")
            res = harness.execute_guarded_mutations_during_pass(
                api=api,
                solr_host_url="http://127.0.0.1:28983/solr",
                candidate_core=cand_core,
                mutation_token="20261124_deadbeef",
                cli_launch_monotonic=100.0,
                cli_launch_epoch=1795520000.0,
                repo4_sentinel_uri=sentinel_uri,
                native_docs_by_id=native_docs,
                min_delay_sec=45.0,
                max_wait_sec=180.0,
                poll_interval_sec=3.0,
                find_cli_proc_fn=find_cli_proc_fn,
                http_get_json=http_get_json,
                sleep_fn=sleep_fn,
                monotonic_fn=monotonic_fn,
                time_fn=time_fn,
                mutations_evidence_path=ev_path,
            )
            self.assertEqual(res["status"], "mutations_applied_and_audited")
            self.assertEqual(res["repo4_sentinel_uri"], sentinel_uri)
            self.assertEqual(res["repo4_sentinel_ao_id"], 10000)
            self.assertEqual(res["pre_mutation_guard"]["repo4_sentinel_uri"], sentinel_uri)
            self.assertTrue(res["pre_mutation_guard"]["repo4_sentinel_in_tlog"])
            self.assertEqual(
                res["pre_mutation_guard"]["repo4_sentinel_tlog_title"],
                "Repo 4 First AO 10000",
            )
            self.assertTrue(any("repositories%2F4%2Farchival_objects%2F10000" in u for u in get_urls))
            self.assertGreaterEqual(res["mutation_started_elapsed_since_cli_launch_sec"], 48.0)
            self.assertEqual(res["repo3_ao_count_before"], 2010)
            self.assertEqual(res["repo3_ao_count_after"], 2010)
            self.assertTrue(res["net_repo3_ao_count_preserved"])
            self.assertEqual(res["mutations"]["update"]["lock_version_after"], 1)
            self.assertEqual(res["mutations"]["delete"]["post_delete_get_http_status"], 404)
            self.assertEqual(res["mutations"]["create"]["id"], 2011)
            self.assertEqual(res["mutations"]["create"]["position_after"], 8)
            self.assertEqual(os.stat(ev_path).st_mode & 0o777, 0o600)

    def test_guarded_mutations_abort_without_changes_if_process_dead_or_core_committed_or_bad_sentinel(self) -> None:
        api = FakeAspaceRepo3Api()
        cand_core = "stage_staff_20261124_132000_12345_cafebabe_candidate"
        sentinel_uri = "/repositories/4/archival_objects/10000"

        # Case 0: invalid or unverified sentinel URI
        for bad_sentinel in ("", "/repositories/3/archival_objects/10000", "/repositories/4/archival_objects/0"):
            with self.assertRaises(harness.Gate4MutationHandoffError):
                harness.execute_guarded_mutations_during_pass(
                    api=api,
                    solr_host_url="http://127.0.0.1:28983/solr",
                    candidate_core=cand_core,
                    mutation_token="tok",
                    cli_launch_monotonic=0.0,
                    cli_launch_epoch=1795520000.0,
                    repo4_sentinel_uri=bad_sentinel,
                    min_delay_sec=0.0,
                    max_wait_sec=10.0,
                )
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.execute_guarded_mutations_during_pass(
                api=api,
                solr_host_url="http://127.0.0.1:28983/solr",
                candidate_core=cand_core,
                mutation_token="tok",
                cli_launch_monotonic=0.0,
                cli_launch_epoch=1795520000.0,
                repo4_sentinel_uri=sentinel_uri,
                native_docs_by_id={},
                min_delay_sec=0.0,
                max_wait_sec=10.0,
            )
        self.assertEqual(api.post_calls, [])
        self.assertEqual(api.delete_calls, [])

        # Case 1: aspace-cli not alive
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.execute_guarded_mutations_during_pass(
                api=api,
                solr_host_url="http://127.0.0.1:28983/solr",
                candidate_core=cand_core,
                mutation_token="tok",
                cli_launch_monotonic=0.0,
                cli_launch_epoch=1795520000.0,
                repo4_sentinel_uri=sentinel_uri,
                min_delay_sec=0.0,
                max_wait_sec=10.0,
                find_cli_proc_fn=lambda _c: None,
                http_get_json=lambda _u: {"response": {"numFound": 0}},
                sleep_fn=lambda _d: None,
                monotonic_fn=lambda: 1.0,
            )
        self.assertEqual(api.post_calls, [])
        self.assertEqual(api.delete_calls, [])

        # Case 2: candidate core already committed (numFound > 0)
        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.execute_guarded_mutations_during_pass(
                api=api,
                solr_host_url="http://127.0.0.1:28983/solr",
                candidate_core=cand_core,
                mutation_token="tok",
                cli_launch_monotonic=0.0,
                cli_launch_epoch=1795520000.0,
                repo4_sentinel_uri=sentinel_uri,
                min_delay_sec=0.0,
                max_wait_sec=10.0,
                find_cli_proc_fn=lambda _c: {"pid": 123, "comm": "aspace-cli"},
                http_get_json=lambda _u: {"response": {"numFound": 1036427}},
                sleep_fn=lambda _d: None,
                monotonic_fn=lambda: 1.0,
            )
        self.assertEqual(api.post_calls, [])
        self.assertEqual(api.delete_calls, [])

        # Case 3: sentinel never enters candidate TLog before timeout -> zero mutations
        clock = {"mono": 0.0}

        def advance_sleep(dt: float) -> None:
            clock["mono"] += dt

        def tlog_missing_sentinel(url: str) -> dict:
            if "/select?" in url:
                return {"response": {"numFound": 0}}
            if "archival_objects%2F2009" in url or "archival_objects%2F2010" in url:
                return {"doc": {"id": "ok"}}
            return {"doc": None}

        with self.assertRaises(harness.Gate4MutationHandoffError):
            harness.execute_guarded_mutations_during_pass(
                api=api,
                solr_host_url="http://127.0.0.1:28983/solr",
                candidate_core=cand_core,
                mutation_token="tok",
                cli_launch_monotonic=0.0,
                cli_launch_epoch=1795520000.0,
                repo4_sentinel_uri=sentinel_uri,
                min_delay_sec=0.0,
                max_wait_sec=6.0,
                poll_interval_sec=3.0,
                find_cli_proc_fn=lambda _c: {"pid": 123, "comm": "aspace-cli"},
                http_get_json=tlog_missing_sentinel,
                sleep_fn=advance_sleep,
                monotonic_fn=lambda: clock["mono"],
            )
        self.assertEqual(api.post_calls, [])
        self.assertEqual(api.delete_calls, [])

    def test_post_pass_ruby_handoff_orchestration_swaps_verifies_and_rolls_back(self) -> None:
        cand_core = "stage_staff_20261124_132000_12345_cafebabe_candidate"
        mutation_result = {
            "mutations": {
                "update": {
                    "uri": "/repositories/3/archival_objects/2009",
                    "id": 2009,
                    "title_before": "Fixture Leaf A",
                    "title_after": "Fixture Leaf A [GATE4-MUTATION-UPDATE-tok]",
                    "system_mtime_after": "2026-11-24T13:20:00Z",
                },
                "delete": {
                    "uri": "/repositories/3/archival_objects/2010",
                    "id": 2010,
                    "title_before": "Fixture Leaf B",
                },
                "create": {
                    "uri": "/repositories/3/archival_objects/2011",
                    "id": 2011,
                    "resource_ref": "/repositories/3/resources/1",
                    "parent_ref": "/repositories/3/archival_objects/2001",
                    "title_after": "Gate 4 synthetic leaf AO [GATE4-MUTATION-CREATE-tok]",
                    "system_mtime_after": "2026-11-24T13:20:02Z",
                },
            },
            "touched_ancestors": {
                "resource": {
                    "uri": "/repositories/3/resources/1",
                    "id": 1,
                    "system_mtime_after": "2026-11-24T13:20:02Z",
                }
            },
        }
        env = FakeMutationHandoffSolrEnv(cand_core, mutation_result)

        def fake_round(dir_name, epoch, ao_ids, res_ids, del_uris, timeout_s):
            self.assertEqual(ao_ids, [2009, 2011])
            self.assertEqual(res_ids, [1])
            self.assertEqual(del_uris, ["/repositories/3/archival_objects/2010"])
            env.round_applied = True
            return {"status": "ok", "state_dir_name": dir_name, "epoch": epoch}

        res = harness.execute_post_pass_ruby_handoff_orchestration(
            solr_url="http://127.0.0.1:28983/solr",
            candidate_core=cand_core,
            watermark_epoch=1795520000,
            mutation_result=mutation_result,
            state_dir_name="indexer_state_gate4_mutation_handoff_20261124_cafebabe",
            http_get_json=env.http_get_json,
            run_jruby_preflight_mutations_fn=lambda ms, a, r, d: {"status": "ok"},
            run_jruby_mutation_round_fn=fake_round,
            swap_fn=env.swap,
        )
        self.assertEqual(res["status"], "mutation_handoff_round_passed_and_rolled_back")
        self.assertEqual(len(env.swap_calls), 2)
        self.assertFalse(env.swapped)
        self.assertTrue(res["solr_post_round_verification"]["no_lost_docs"])
        self.assertEqual(res["core_snapshots"]["after_rollback"]["archivesspace"]["numFound"], 0)
        self.assertEqual(
            res["core_snapshots"]["after_rollback"][cand_core]["numFound"],
            harness.EXPECTED_TOTAL_DOCS,
        )
        self.assertEqual(res["core_snapshots"]["after_rollback"][cand_core]["version"], 6610)

    def test_ruby_helper_mutation_guard_and_selective_checkpoint_advancement(self) -> None:
        ruby_code = r"""
        require 'tmpdir'
        require_relative 'rust-tools/scripts/gate4_isolated_handoff_round'

        Dir.mktmpdir('gate4-ruby-mutation-guard-') do |dir|
          Object.const_set(:AppConfig, {}) unless defined?(AppConfig)
          AppConfig[:data_directory] = dir
          state_name = 'indexer_state_gate4_mutation_handoff_20261124_cafebabe'
          state = IndexState.new(state_name)
          baseline_epoch = 1795520000
          round_epoch = 1795526800
          summary = {
            'committed' => true,
            'full_reindex_started_at_epoch_sec' => baseline_epoch,
            'repository_ids' => [2, 3, 4]
          }
          StaffFullHandoffSpike.apply(summary, state: state, promoted: true)

          # Simulate PeriodicIndexer advancing ONLY 3_archival_object, 3_resource, and _deletes_deletes
          state.set_last_mtime(3, :archival_object, round_epoch)
          state.set_last_mtime(3, :resource, round_epoch + 1)
          state.set_last_mtime('_deletes', 'deletes', round_epoch + 2)

          check = Gate4IsolatedHandoffRound.verify_state_checkpoints_after_mutation_round!(
            state,
            state_dir_path: File.join(dir, state_name),
            repo_ids: [2, 3, 4],
            expected_epoch: baseline_epoch,
            expected_advanced_keys: [['_deletes', 'deletes'], [3, :archival_object], [3, :resource]],
            min_advanced_epoch: round_epoch
          )
          raise 'expected 3 advanced files' unless check['advanced_file_count'] == 3
          raise 'expected 56 unchanged files' unless check['unchanged_file_count'] == 56

          # Advancing an unexpected checkpoint must fail
          state.set_last_mtime(4, :archival_object, round_epoch)
          begin
            Gate4IsolatedHandoffRound.verify_state_checkpoints_after_mutation_round!(
              state,
              state_dir_path: File.join(dir, state_name),
              repo_ids: [2, 3, 4],
              expected_epoch: baseline_epoch,
              expected_advanced_keys: [['_deletes', 'deletes'], [3, :archival_object], [3, :resource]],
              min_advanced_epoch: round_epoch
            )
            raise 'unexpected checkpoint advancement did not fail'
          rescue ArgumentError
          end

          # Verify expected-mutation guard allowlist
          fake_indexer_cls = Class.new do
            def index_records(records, *args); :indexed_ok; end
            def index_batch(batch, *args); :batch_ok; end
            def delete_records(records, *args); :deleted_ok; end
            def send_commit(*args); :commit_ok; end
            def do_http_request(url, req); :http_ok; end
          end
          idx = fake_indexer_cls.new
          Gate4IsolatedHandoffRound.install_expected_mutation_guard!(
            idx,
            allowed_index_uris: [
              '/repositories/3/archival_objects/2009',
              '/repositories/3/archival_objects/2011',
              '/repositories/3/resources/1'
            ],
            expected_deleted_uris: ['/repositories/3/archival_objects/2010']
          )

          idx.index_records([
            {'uri' => '/repositories/3/archival_objects/2009'},
            {'uri' => '/repositories/3/archival_objects/2011'}
          ])
          idx.index_records([{'uri' => '/repositories/3/resources/1'}])
          idx.delete_records([])
          idx.delete_records(['/repositories/3/archival_objects/2010'])

          begin
            idx.index_records([{'uri' => '/repositories/4/archival_objects/1'}])
            raise 'unallowed index_records URI did not abort'
          rescue Gate4IsolatedHandoffRound::Gate4MutationViolationError
          end

          begin
            idx.delete_records(['/repositories/3/archival_objects/2009'])
            raise 'unallowed delete_records URI did not abort'
          rescue Gate4IsolatedHandoffRound::Gate4MutationViolationError
          end
        end
        puts 'RUBY_MUTATION_GUARD_OK'
        """
        proc = subprocess.run(
            ["ruby", "-e", ruby_code],
            cwd=harness.REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("RUBY_MUTATION_GUARD_OK", proc.stdout)


if __name__ == "__main__":
    unittest.main()
