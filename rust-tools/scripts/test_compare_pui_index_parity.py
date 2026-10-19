#!/usr/bin/env python3
"""Offline comprehensive test suite for compare_pui_index_parity.py.

Uses Python standard library only (unittest and a local ephemeral http.server stub).
Requires no live ArchivesSpace, Solr, or external network connectivity.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import sys
import threading
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any, Callable, Dict, List, Optional, Tuple

SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPTS_DIR)

import compare_pui_index_parity as comparator


class StubSolrHandler(BaseHTTPRequestHandler):
    """Configurable HTTP request handler simulating Solr responses."""

    response_dispatcher: Optional[Callable[[str, Dict[str, List[str]]], Tuple[int, Dict[str, Any]]]] = None
    received_requests: List[Dict[str, Any]] = []

    def do_GET(self) -> None:
        parsed = urllib.parse.urlsplit(self.path)
        query_params = urllib.parse.parse_qs(parsed.query)

        auth_header = self.headers.get("Authorization")
        StubSolrHandler.received_requests.append({
            "path": parsed.path,
            "query": query_params,
            "auth": auth_header,
        })

        if StubSolrHandler.response_dispatcher:
            code, payload = StubSolrHandler.response_dispatcher(parsed.path, query_params)
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.end_headers()
            self.wfile.write(json.dumps(payload).encode("utf-8"))
        else:
            self.send_response(500)
            self.end_headers()

    def log_message(self, format: str, *args: Any) -> None:
        pass


class TestPuiParityComparator(unittest.TestCase):
    server: Optional[HTTPServer] = None
    server_thread: Optional[threading.Thread] = None
    server_port: int = 0

    @classmethod
    def setUpClass(cls) -> None:
        cls.server = HTTPServer(("127.0.0.1", 0), StubSolrHandler)
        cls.server_port = cls.server.server_address[1]
        cls.server_thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.server_thread.start()

    @classmethod
    def tearDownClass(cls) -> None:
        if cls.server:
            cls.server.shutdown()
            cls.server.server_close()

    def setUp(self) -> None:
        StubSolrHandler.response_dispatcher = None
        StubSolrHandler.received_requests.clear()

    def test_url_normalization_and_redaction(self) -> None:
        url = "HTTP://Solr.Internal:8983/solr//test_core/?secret=123"
        norm = comparator.normalize_collection_url(url)
        self.assertEqual(norm, "http://solr.internal:8983/solr/test_core")

        redacted = comparator.redact_url("http://user:password@localhost:8983/solr/core?token=xyz&name=abc")
        self.assertIn("user:***", redacted)
        self.assertNotIn("password", redacted)
        self.assertIn("token=***", redacted)
        self.assertIn("name=abc", redacted)

    def test_safety_check_rejects_primary_collections(self) -> None:
        base = "http://localhost:8983/solr/archivesspace"
        cand = "http://localhost:8983/solr/test_canary"
        with self.assertRaises(comparator.ConfigurationError) as ctx:
            comparator.validate_safety_and_distinct_collection_urls(base, cand, allow_primary=False)
        self.assertIn("protected collection name", str(ctx.exception))

        # Rejects identical URLs
        with self.assertRaises(comparator.ConfigurationError) as ctx:
            comparator.validate_safety_and_distinct_collection_urls(cand, cand, allow_primary=True)
        self.assertIn("same normalized Solr endpoint", str(ctx.exception))

    def test_repository_scope_and_pui_filter(self) -> None:
        self.assertEqual(comparator.resolve_repository_scope("2"), "/repositories/2")
        self.assertEqual(comparator.resolve_repository_scope("/repositories/4"), "/repositories/4")
        filter_q = comparator.build_pui_repository_filter("/repositories/2")
        self.assertEqual(filter_q, 'repository:"/repositories/2" AND types:pui_record')

    def test_privacy_leak_detection(self) -> None:
        clean_doc = {
            "id": "/repositories/2/archival_objects/1",
            "title": "Clean Item",
            "fullrecord_published": ["Clean Item"],
            "json": json.dumps({
                "uri": "/repositories/2/archival_objects/1",
                "title": "Clean Item",
                "instances": [
                    {
                        "sub_container": {
                            "top_container": {
                                "_resolved": {
                                    "indicator": "Box 1",
                                    "container_profile": {
                                        "_resolved": {
                                            "name": "Hollinger"
                                        }
                                    }
                                }
                            }
                        }
                    }
                ]
            })
        }
        self.assertEqual(comparator.detect_pui_privacy_leak(clean_doc), [])

        # 1. Leaked ancestors at root
        leaky_root_anc = dict(clean_doc, ancestors=["/repositories/2/resources/1"])
        self.assertTrue(any("root contains 'ancestors'" in msg for msg in comparator.detect_pui_privacy_leak(leaky_root_anc)))

        # 2. Leaked unpublished fullrecord at root
        leaky_fullrecord = dict(clean_doc, fullrecord=["Unpublished text"])
        self.assertTrue(any("root contains unpublished 'fullrecord'" in msg for msg in comparator.detect_pui_privacy_leak(leaky_fullrecord)))

        # 3. Leaked _archivalist_* handoff marker at root
        leaky_handoff = dict(clean_doc, _archivalist_pui_final={"marker": "test"})
        self.assertTrue(any("contains private handoff field" in msg for msg in comparator.detect_pui_privacy_leak(leaky_handoff)))

        # 4. Leaked ancestors in json
        leaky_json_anc = {
            "id": "/repositories/2/archival_objects/1",
            "json": json.dumps({"uri": "/repositories/2/archival_objects/1", "ancestors": [{"ref": "..."}]})
        }
        self.assertTrue(any("parsed 'json' contains 'ancestors'" in msg for msg in comparator.detect_pui_privacy_leak(leaky_json_anc)))

        # 5. Leaked top_container internal_note in json
        leaky_internal_note = {
            "id": "/repositories/2/archival_objects/1",
            "json": json.dumps({
                "uri": "/repositories/2/archival_objects/1",
                "instances": [{"sub_container": {"top_container": {"_resolved": {"internal_note": "SECRET"}}}}]
            })
        }
        self.assertTrue(any("top_container contains 'internal_note'" in msg for msg in comparator.detect_pui_privacy_leak(leaky_internal_note)))

        # 6. Leaked container_profile notes in json
        leaky_cp_notes = {
            "id": "/repositories/2/archival_objects/1",
            "json": json.dumps({
                "uri": "/repositories/2/archival_objects/1",
                "instances": [{
                    "sub_container": {
                        "top_container": {
                            "_resolved": {
                                "container_profile": {"_resolved": {"notes": "CP_SECRET"}}
                            }
                        }
                    }
                }]
            })
        }
        self.assertTrue(any("container_profile contains 'notes'" in msg for msg in comparator.detect_pui_privacy_leak(leaky_cp_notes)))

    def test_deep_type_strict_equal(self) -> None:
        self.assertTrue(comparator.deep_type_strict_equal("abc", "abc")[0])
        self.assertFalse(comparator.deep_type_strict_equal(1, "1")[0])
        self.assertFalse(comparator.deep_type_strict_equal([1, 2], [1])[0])
        self.assertFalse(comparator.deep_type_strict_equal({"a": 1}, {"a": 2})[0])
        self.assertFalse(comparator.deep_type_strict_equal({"a": 1}, {"b": 1})[0])

    def test_end_to_end_parity_success(self) -> None:
        clean_json = json.dumps({
            "uri": "/repositories/2/archival_objects/1",
            "title": "Actas Capitulares: Éire & Dublin 1916",
            "publish": True,
        })
        doc = {
            "id": "/repositories/2/archival_objects/1",
            "uri": "/repositories/2/archival_objects/1",
            "title": "Actas Capitulares: Éire & Dublin 1916",
            "primary_type": "archival_object",
            "types": ["archival_object", "pui_record", "pui"],
            "json": clean_json,
            "fullrecord_published": ["Actas Capitulares: Éire & Dublin 1916"],
            "suppressed": False,
            "publish": True,
            "system_generated": "false",
            "repository": "/repositories/2",
        }

        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            cursor = params.get("cursorMark", ["*"])[0]
            if cursor == "*":
                return 200, {
                    "response": {"numFound": 1, "start": 0, "docs": [doc]},
                    "nextCursorMark": "CUR_END",
                }
            return 200, {
                "response": {"numFound": 1, "start": 1, "docs": []},
                "nextCursorMark": "CUR_END",
            }

        StubSolrHandler.response_dispatcher = dispatcher

        base_url = f"http://127.0.0.1:{self.server_port}/solr/canary_pui_base"
        cand_url = f"http://127.0.0.1:{self.server_port}/solr/canary_pui_cand"

        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            exit_code = comparator.run_pui_parity_comparison(
                baseline_url=base_url,
                candidate_url=cand_url,
                repository="2",
                batch_size=10,
                allow_primary=True,
            )
        self.assertEqual(exit_code, comparator.EXIT_PARITY_SUCCESS)
        output = out.getvalue()
        self.assertIn("RESULT: SUCCESS - Privacy-normalized PUI parity verified.", output)
        self.assertIn("Privacy-normalized matching docs: 1", output)

    def test_end_to_end_privacy_leak_fails(self) -> None:
        clean_doc = {
            "id": "/repositories/2/archival_objects/1",
            "title": "Letters",
            "json": json.dumps({"uri": "/repositories/2/archival_objects/1", "title": "Letters"}),
            "fullrecord_published": ["Letters"],
        }
        leaky_doc = {
            "id": "/repositories/2/archival_objects/1",
            "title": "Letters",
            "fullrecord": ["PRIVATE FULLRECORD LEAK"],
            "json": json.dumps({"uri": "/repositories/2/archival_objects/1", "title": "Letters"}),
            "fullrecord_published": ["Letters"],
        }

        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            is_cand = "cand" in path
            target_doc = leaky_doc if is_cand else clean_doc
            return 200, {
                "response": {"numFound": 1, "start": 0, "docs": [target_doc]},
                "nextCursorMark": params.get("cursorMark", ["*"])[0],
            }

        StubSolrHandler.response_dispatcher = dispatcher

        base_url = f"http://127.0.0.1:{self.server_port}/solr/canary_pui_base"
        cand_url = f"http://127.0.0.1:{self.server_port}/solr/canary_pui_cand"

        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            exit_code = comparator.run_pui_parity_comparison(
                baseline_url=base_url,
                candidate_url=cand_url,
                repository="2",
                batch_size=10,
                allow_primary=True,
            )
        self.assertEqual(exit_code, comparator.EXIT_PARITY_MISMATCH)
        output = out.getvalue()
        self.assertIn("RESULT: FAILURE - Privacy-normalized PUI parity mismatch or privacy invariant violation detected.", output)

    def test_canary_script_sidecar_routes_offline(self) -> None:
        """Static verification: canary script targets valid sidecar routes, not deprecated /ping."""
        canary_path = os.path.join(SCRIPTS_DIR, "canary_pui_document_preparation.sh")
        with open(canary_path, "r", encoding="utf-8") as f:
            script_content = f.read()

        # Must NOT call /ping
        self.assertNotIn("${SIDECAR_URL%/}/ping", script_content)
        self.assertNotIn("${SIDECAR_URL}/ping", script_content)

        # Must call /health and validate served_by
        self.assertIn("${SIDECAR_URL%/}/health", script_content)
        self.assertIn("aspace-shadow-sidecar", script_content)
        self.assertIn("${SIDECAR_URL%/}/index/documents", script_content)

        # Extract all endpoints called against SIDECAR_URL
        import re
        endpoints = re.findall(r"\$\{SIDECAR_URL(?:%/)??\}(/[a-zA-Z0-9_\-/]+)", script_content)
        valid_sidecar_prefixes = ("/health", "/ready", "/metrics", "/index/documents", "/shadow/")
        self.assertTrue(len(endpoints) > 0)
        for ep in endpoints:
            self.assertTrue(
                any(ep.startswith(prefix) for prefix in valid_sidecar_prefixes),
                f"Canary script calls unrecognized sidecar route: {ep}",
            )

    def test_baseline_normalization_permits_documented_differences(self) -> None:
        """Baseline normalization permits ONLY native root ancestors, JSON ancestors, and internal notes."""
        base_json = {
            "uri": "/repositories/2/archival_objects/50",
            "title": "Historical Deed",
            "ancestors": [{"ref": "/repositories/2/resources/1", "title": "Secret Resource"}],
            "instances": [
                {
                    "sub_container": {
                        "top_container": {
                            "_resolved": {
                                "indicator": "Box 10",
                                "internal_note": "SECRET_NATIVE_INTERNAL_NOTE",
                                "container_profile": {
                                    "_resolved": {
                                        "name": "Folio Box",
                                        "notes": "SECRET_CONTAINER_PROFILE_NOTE",
                                    }
                                }
                            }
                        }
                    }
                }
            ],
            "publish": True,
        }

        # Hardened candidate has no ancestors key, no internal notes, and no document-level ancestors
        cand_json = {
            "uri": "/repositories/2/archival_objects/50",
            "title": "Historical Deed",
            "instances": [
                {
                    "sub_container": {
                        "top_container": {
                            "_resolved": {
                                "indicator": "Box 10",
                                "container_profile": {
                                    "_resolved": {
                                        "name": "Folio Box",
                                    }
                                }
                            }
                        }
                    }
                }
            ],
            "publish": True,
        }

        base_doc = {
            "id": "/repositories/2/archival_objects/50",
            "uri": "/repositories/2/archival_objects/50",
            "title": "Historical Deed",
            "primary_type": "archival_object",
            "types": ["archival_object", "pui_record", "pui"],
            "ancestors": ["/repositories/2/resources/1"],  # Root Solr ancestors in baseline
            "json": json.dumps(base_json),
            "fullrecord_published": ["Historical Deed", "Box 10", "Folio Box"],
            "publish": True,
            "suppressed": False,
        }

        cand_doc = {
            "id": "/repositories/2/archival_objects/50",
            "uri": "/repositories/2/archival_objects/50",
            "title": "Historical Deed",
            "primary_type": "archival_object",
            "types": ["archival_object", "pui_record", "pui"],
            # No root ancestors in candidate
            "json": json.dumps(cand_json),
            "fullrecord_published": ["Historical Deed", "Box 10", "Folio Box"],
            "publish": True,
            "suppressed": False,
        }

        is_equal, reasons = comparator.compare_pui_document(base_doc, cand_doc, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertTrue(is_equal, f"Expected parity success after baseline normalization, got: {reasons}")
        self.assertEqual(reasons, [])

    def test_candidate_leaks_fail_without_candidate_normalization(self) -> None:
        """Candidate is NEVER normalized: candidate leaks of root ancestors or internal notes fail immediately."""
        clean_base = {
            "id": "/repositories/2/archival_objects/51",
            "uri": "/repositories/2/archival_objects/51",
            "title": "Clean Base",
            "primary_type": "archival_object",
            "types": ["archival_object", "pui_record"],
            "json": json.dumps({"uri": "/repositories/2/archival_objects/51", "title": "Clean Base"}),
            "publish": True,
        }

        # 1. Candidate leaks root ancestors
        cand_root_anc = dict(clean_base, ancestors=["/repositories/2/resources/1"])
        ok1, reasons1 = comparator.compare_pui_document(clean_base, cand_root_anc, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok1)
        self.assertTrue(any("root contains 'ancestors'" in r for r in reasons1))

        # 2. Candidate leaks JSON ancestors
        cand_json_anc = dict(clean_base, json=json.dumps({"uri": "/repositories/2/archival_objects/51", "ancestors": []}))
        ok2, reasons2 = comparator.compare_pui_document(clean_base, cand_json_anc, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok2)
        self.assertTrue(any("parsed 'json' contains 'ancestors'" in r for r in reasons2))

        # 3. Candidate leaks top_container internal_note
        cand_tc_note = dict(
            clean_base,
            json=json.dumps({
                "uri": "/repositories/2/archival_objects/51",
                "instances": [{"sub_container": {"top_container": {"_resolved": {"internal_note": "LEAK"}}}}]
            })
        )
        ok3, reasons3 = comparator.compare_pui_document(clean_base, cand_tc_note, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok3)
        self.assertTrue(any("top_container contains 'internal_note'" in r for r in reasons3))

        # 4. Candidate leaks container_profile notes
        cand_cp_notes = dict(
            clean_base,
            json=json.dumps({
                "uri": "/repositories/2/archival_objects/51",
                "instances": [{"sub_container": {"top_container": {"_resolved": {"container_profile": {"_resolved": {"notes": "CP_LEAK"}}}}}}]
            })
        )
        ok4, reasons4 = comparator.compare_pui_document(clean_base, cand_cp_notes, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok4)
        self.assertTrue(any("container_profile contains 'notes'" in r for r in reasons4))

        # 5. Candidate leaks unpublished fullrecord
        cand_fullrecord = dict(clean_base, fullrecord=["LEAK"])
        ok5, reasons5 = comparator.compare_pui_document(clean_base, cand_fullrecord, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok5)
        self.assertTrue(any("root contains unpublished 'fullrecord'" in r for r in reasons5))

        # 6. Candidate leaks _archivalist_* handoff metadata
        cand_handoff = dict(clean_base, _archivalist_pui_final={"marker": "test"})
        ok6, reasons6 = comparator.compare_pui_document(clean_base, cand_handoff, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok6)
        self.assertTrue(any("contains private handoff field" in r for r in reasons6))

        # 7. Candidate leaks _archivalist_pui_routing metadata
        cand_routing = dict(clean_base, _archivalist_pui_routing={"marker": "pui_routing_metadata"})
        ok7, reasons7 = comparator.compare_pui_document(clean_base, cand_routing, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok7)
        self.assertTrue(any("contains private handoff field" in r for r in reasons7))

        # 8. Candidate leaks _archivalist_pui_notes metadata
        cand_notes = dict(clean_base, _archivalist_pui_notes={"marker": "pui_notes_metadata"})
        ok8, reasons8 = comparator.compare_pui_document(clean_base, cand_notes, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok8)
        self.assertTrue(any("contains private handoff field" in r for r in reasons8))

    def test_altered_public_field_and_canonical_structural_equality_fails(self) -> None:
        """Any alteration of public metadata or JSON structure fails parity."""
        clean_base = {
            "id": "/repositories/2/archival_objects/52",
            "uri": "/repositories/2/archival_objects/52",
            "title": "Original Title",
            "primary_type": "archival_object",
            "types": ["archival_object", "pui_record"],
            "json": json.dumps({"uri": "/repositories/2/archival_objects/52", "title": "Original Title"}),
            "publish": True,
        }

        # Altered title in doc
        cand_altered_doc_title = dict(clean_base, title="Tampered Title")
        ok1, reasons1 = comparator.compare_pui_document(clean_base, cand_altered_doc_title, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok1)
        self.assertTrue(any("title" in r for r in reasons1))

        # Altered title in JSON
        cand_altered_json = dict(
            clean_base,
            json=json.dumps({"uri": "/repositories/2/archival_objects/52", "title": "Tampered JSON Title"})
        )
        ok2, reasons2 = comparator.compare_pui_document(clean_base, cand_altered_json, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok2)
        self.assertTrue(any("canonical structural JSON mismatch" in r for r in reasons2))

    def test_array_reordering_and_type_mismatch_fails(self) -> None:
        """Array reordering or primitive type mismatch fails parity."""
        clean_base = {
            "id": "/repositories/2/archival_objects/53",
            "uri": "/repositories/2/archival_objects/53",
            "title": "Ordering Test",
            "primary_type": "archival_object",
            "types": ["archival_object", "pui_record", "pui"],
            "json": json.dumps({"uri": "/repositories/2/archival_objects/53", "title": "Ordering Test"}),
            "publish": True,
        }

        # Array reordering in types
        cand_reordered = dict(clean_base, types=["pui", "archival_object", "pui_record"])
        ok1, reasons1 = comparator.compare_pui_document(clean_base, cand_reordered, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok1)
        self.assertTrue(any("types" in r for r in reasons1))

        # Bool vs int type mismatch (publish: 1 vs publish: True)
        cand_type_mismatch = dict(clean_base, publish=1)
        ok2, reasons2 = comparator.compare_pui_document(clean_base, cand_type_mismatch, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok2)
        self.assertTrue(any("publish: type mismatch (bool vs int)" in r for r in reasons2))

    def test_extra_and_missing_non_private_fields_fail(self) -> None:
        """Extra or missing non-private Solr fields in candidate fail parity."""
        clean_base = {
            "id": "/repositories/2/archival_objects/54",
            "uri": "/repositories/2/archival_objects/54",
            "title": "Field Test",
            "primary_type": "archival_object",
            "types": ["archival_object", "pui_record"],
            "json": json.dumps({"uri": "/repositories/2/archival_objects/54", "title": "Field Test"}),
            "publish": True,
            "extra_baseline_field": "some_value",
        }

        # Candidate missing a non-private field
        cand_missing = {k: v for k, v in clean_base.items() if k != "extra_baseline_field"}
        ok1, reasons1 = comparator.compare_pui_document(clean_base, cand_missing, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok1)
        self.assertTrue(any("missing fields in candidate: ['extra_baseline_field']" in r for r in reasons1))

        # Candidate with an unexpected extra field
        cand_extra = dict(clean_base, rogue_extra_field="rogue")
        ok2, reasons2 = comparator.compare_pui_document(clean_base, cand_extra, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok2)
        self.assertTrue(any("extra fields in candidate: ['rogue_extra_field']" in r for r in reasons2))

    def test_get_auth_credentials_precedence(self) -> None:
        """Specific environment credentials take precedence over generic, which take precedence over URL."""
        raw_url = "http://urluser:urlpass@localhost:8983/solr/core"
        # 1. Neither env set -> URL credentials
        creds1 = comparator.get_auth_credentials(raw_url, "TEST_SPEC_USER", "TEST_SPEC_PASS", "TEST_GEN_USER", "TEST_GEN_PASS")
        self.assertEqual(creds1, ("urluser", "urlpass"))

        # 2. Generic env set -> Generic credentials
        os.environ["TEST_GEN_USER"] = "gen_alice"
        os.environ["TEST_GEN_PASS"] = "gen_secret"
        try:
            creds2 = comparator.get_auth_credentials(raw_url, "TEST_SPEC_USER", "TEST_SPEC_PASS", "TEST_GEN_USER", "TEST_GEN_PASS")
            self.assertEqual(creds2, ("gen_alice", "gen_secret"))

            # 3. Specific env set -> Specific credentials
            os.environ["TEST_SPEC_USER"] = "spec_bob"
            os.environ["TEST_SPEC_PASS"] = "spec_secret"
            try:
                creds3 = comparator.get_auth_credentials(raw_url, "TEST_SPEC_USER", "TEST_SPEC_PASS", "TEST_GEN_USER", "TEST_GEN_PASS")
                self.assertEqual(creds3, ("spec_bob", "spec_secret"))
            finally:
                os.environ.pop("TEST_SPEC_USER", None)
                os.environ.pop("TEST_SPEC_PASS", None)
        finally:
            os.environ.pop("TEST_GEN_USER", None)
            os.environ.pop("TEST_GEN_PASS", None)

    def test_pui_auxiliary_document_structure_and_parity(self) -> None:
        """PUI auxiliary documents match exact native shape and pass parity."""
        aux_types = [
            ("resource_ordered_records", "/repositories/2/resources/10/ordered_records", "/repositories/2/resources/10"),
            ("tree_root", "/repositories/2/resources/10/tree/root", "/repositories/2/resources/10"),
            ("tree_waypoint", "/repositories/2/resources/10/tree/waypoint__0", "/repositories/2/resources/10"),
            ("tree_node", "/repositories/2/resources/10/tree/node_/repositories/2/archival_objects/1", "/repositories/2/archival_objects/1"),
            ("tree_node_from_root", "/repositories/2/resources/10/tree/node_from_root_42", "/repositories/2/archival_objects/42"),
        ]
        for ptype, uri, parent_id in aux_types:
            payload = [{"uri": f"{uri}/1", "child_count": 0}] if ptype == "tree_waypoint" else {"test": "value"}
            doc = {
                "id": uri,
                "uri": uri,
                "pui_parent_id": parent_id,
                "publish": "true",
                "primary_type": ptype,
                "types": ["pui"],
                "json": json.dumps(payload),
            }
            cand = dict(doc)
            ok, reasons = comparator.compare_pui_document(doc, cand, comparator.DEFAULT_IGNORED_FIELDS)
            self.assertTrue(ok, f"failed for {ptype}: {reasons}")

    def test_pui_auxiliary_document_marker_leak_fails(self) -> None:
        """Leaked _archivalist marker in candidate auxiliary document fails parity immediately."""
        doc = {
            "id": "/repositories/2/resources/10/tree/root",
            "uri": "/repositories/2/resources/10/tree/root",
            "pui_parent_id": "/repositories/2/resources/10",
            "publish": "true",
            "primary_type": "tree_root",
            "types": ["pui"],
            "json": json.dumps({"title": "Root"}),
        }
        cand_leaked = dict(doc, _archivalist_pui_auxiliary={"marker": "pui_auxiliary_document_metadata"})
        ok, reasons = comparator.compare_pui_document(doc, cand_leaked, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok)
        self.assertTrue(any("candidate privacy leak" in r or "private handoff field" in r for r in reasons))

    def test_pui_auxiliary_document_type_coercion_fails(self) -> None:
        """Boolean publish in candidate auxiliary document fails type-strict check against string 'true'."""
        doc = {
            "id": "/repositories/2/resources/10/tree/root",
            "uri": "/repositories/2/resources/10/tree/root",
            "pui_parent_id": "/repositories/2/resources/10",
            "publish": "true",
            "primary_type": "tree_root",
            "types": ["pui"],
            "json": json.dumps({"title": "Root"}),
        }
        cand_bool = dict(doc, publish=True)
        ok, reasons = comparator.compare_pui_document(doc, cand_bool, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok)
        self.assertTrue(any("publish: type mismatch" in r for r in reasons))

    def test_pui_record_inheritance_candidate_marker_leak_fails(self) -> None:
        """Leaked _archivalist_pui_candidate handoff metadata in candidate fails parity immediately."""
        doc = {
            "id": "/repositories/2/archival_objects/50",
            "uri": "/repositories/2/archival_objects/50",
            "publish": True,
            "primary_type": "archival_object",
            "types": ["archival_object", "pui"],
            "json": json.dumps({"title": "Item 50"}),
            "fullrecord_published": ["Item 50"],
        }
        cand_leaked = dict(
            doc,
            _archivalist_pui_candidate={
                "marker": "pui_record_inheritance_candidate_metadata",
                "record": {"title": "Item 50"},
            },
        )
        ok, reasons = comparator.compare_pui_document(doc, cand_leaked, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok)
        self.assertTrue(any("candidate privacy leak" in r or "private handoff field" in r for r in reasons))

    def test_pui_record_inheritance_candidate_rust_fields_marker_leak_fails(self) -> None:
        """Leaked pui_record_inheritance_candidate_metadata marker in _archivalist_rust_fields fails parity."""
        doc = {
            "id": "/repositories/2/archival_objects/50",
            "uri": "/repositories/2/archival_objects/50",
            "publish": True,
            "primary_type": "archival_object",
            "types": ["archival_object", "pui"],
            "json": json.dumps({"title": "Item 50"}),
            "fullrecord_published": ["Item 50"],
        }
        cand_leaked = dict(
            doc,
            _archivalist_rust_fields=["pui_record_inheritance_candidate_metadata"],
        )
        ok, reasons = comparator.compare_pui_document(doc, cand_leaked, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok)
        self.assertTrue(any("candidate privacy leak" in r or "private handoff field" in r for r in reasons))

    def test_pui_common_metadata_marker_leak_fails(self) -> None:
        """Leaked _archivalist_pui_common handoff metadata in candidate fails parity immediately."""
        doc = {
            "id": "/repositories/2/resources/50",
            "uri": "/repositories/2/resources/50",
            "publish": True,
            "primary_type": "resource",
            "types": ["resource", "pui"],
            "json": json.dumps({"title": "Resource 50"}),
            "fullrecord_published": ["Resource 50"],
        }
        cand_leaked = dict(
            doc,
            _archivalist_pui_common={
                "marker": "pui_common_metadata",
                "uri": "/repositories/2/resources/50",
                "primary_type": "resource",
                "subjects": {},
                "agents": {},
                "audit_info": {},
                "years": {},
                "level": {},
                "summary": {},
                "extents": {},
            },
        )
        ok, reasons = comparator.compare_pui_document(doc, cand_leaked, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok)
        self.assertTrue(any("candidate privacy leak" in r or "private handoff field" in r for r in reasons))

    def test_pui_common_metadata_rust_fields_marker_leak_fails(self) -> None:
        """Leaked pui_common_metadata marker in _archivalist_rust_fields fails parity."""
        doc = {
            "id": "/repositories/2/resources/50",
            "uri": "/repositories/2/resources/50",
            "publish": True,
            "primary_type": "resource",
            "types": ["resource", "pui"],
            "json": json.dumps({"title": "Resource 50"}),
            "fullrecord_published": ["Resource 50"],
        }
        cand_leaked = dict(
            doc,
            _archivalist_rust_fields=["pui_common_metadata"],
        )
        ok, reasons = comparator.compare_pui_document(doc, cand_leaked, comparator.DEFAULT_IGNORED_FIELDS)
        self.assertFalse(ok)
        self.assertTrue(any("candidate privacy leak" in r or "private handoff field" in r for r in reasons))


if __name__ == "__main__":
    unittest.main()
