#!/usr/bin/env python3
"""Offline comprehensive test suite for compare_staff_index_parity.py.

Uses Python standard library only (unittest and a local ephemeral http.server stub).
Requires no live ArchivesSpace, Solr, or external network connectivity.
"""

from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import threading
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any, Callable, Dict, List, Optional

# Add scripts directory to path to import comparator module directly
SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPTS_DIR)

import compare_staff_index_parity as comparator


class StubSolrHandler(BaseHTTPRequestHandler):
    """Configurable HTTP request handler simulating Solr responses."""

    # Handlers can be set per test
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
        # Suppress standard HTTP server logging to keep test output clean
        pass


class TestStaffIndexParityOffline(unittest.TestCase):
    """Test suite for staff index parity comparator."""

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
        StubSolrHandler.received_requests = []
        StubSolrHandler.response_dispatcher = None

    def server_url(self, collection_name: str) -> str:
        return f"http://127.0.0.1:{self.server_port}/solr/{collection_name}"

    # 1. URL Normalization and Identical-URL Rejection
    def test_normalize_collection_url(self) -> None:
        url1 = comparator.normalize_collection_url("http://localhost:8983/solr/c1/")
        url2 = comparator.normalize_collection_url("http://LOCALHOST:8983//solr/c1")
        self.assertEqual(url1, "http://localhost:8983/solr/c1")
        self.assertEqual(url1, url2)

        # Default ports stripped
        self.assertEqual(
            comparator.normalize_collection_url("http://host:80/solr/c1"),
            "http://host/solr/c1",
        )
        self.assertEqual(
            comparator.normalize_collection_url("https://host:443/solr/c1"),
            "https://host/solr/c1",
        )

    def test_identical_url_rejection(self) -> None:
        with self.assertRaises(comparator.ConfigurationError) as ctx:
            comparator.validate_distinct_collection_urls(
                "http://localhost:8983/solr/c1",
                "http://localhost:8983/solr/c1/",
            )
        self.assertIn("resolve to the same normalized Solr endpoint", str(ctx.exception))

        with self.assertRaises(comparator.ConfigurationError):
            comparator.validate_distinct_collection_urls(
                "http://user:secret@localhost:8983/solr/c1",
                "http://localhost:8983/solr/c1",
            )

    # 2. Repository Filter Construction
    def test_repository_filter_construction(self) -> None:
        self.assertEqual(comparator.resolve_repository_scope("2"), "/repositories/2")
        self.assertEqual(comparator.resolve_repository_scope("/repositories/2"), "/repositories/2")
        self.assertEqual(comparator.resolve_repository_scope("special_repo"), "special_repo")

        self.assertEqual(
            comparator.build_repository_filter("/repositories/2"),
            'repository:"/repositories/2"',
        )
        self.assertEqual(
            comparator.build_repository_filter('special"quote'),
            'repository:"special\\"quote"',
        )

    # 3. Redaction
    def test_redact_url(self) -> None:
        url = "http://admin:supersecret@localhost:8983/solr/coll?password=p123&token=my_secret_token&fl=*&q=*:*"
        redacted = comparator.redact_url(url)
        self.assertNotIn("supersecret", redacted)
        self.assertNotIn("p123", redacted)
        self.assertNotIn("my_secret_token", redacted)
        self.assertIn("admin:***", redacted)
        self.assertIn("password=***", redacted)
        self.assertIn("token=***", redacted)
        self.assertIn("fl=*", redacted)

    # 4. Pure Fixture Comparisons: Exact Parity
    def test_exact_parity_pure_fixture(self) -> None:
        docs = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "title": "Resource 1",
                "types": ["resource"],
                "publish": True,
                "_version_": 12345,
                "score": 1.0,
            }
        }
        report = comparator.compare_documents(
            baseline_docs=docs,
            candidate_docs=docs,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertTrue(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 0)
        self.assertEqual(report["baseline_count"], 1)
        self.assertEqual(report["candidate_count"], 1)

    # 5. Missing Documents (Baseline vs Candidate)
    def test_missing_documents(self) -> None:
        doc1 = {"id": "/repositories/2/resources/1", "title": "R1"}
        doc2 = {"id": "/repositories/2/resources/2", "title": "R2"}

        base = {"/repositories/2/resources/1": doc1, "/repositories/2/resources/2": doc2}
        cand = {"/repositories/2/resources/1": doc1}

        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["missing_in_candidate"], ["/repositories/2/resources/2"])
        self.assertEqual(report["summary"]["missing_in_candidate_count"], 1)

    # 6. Extra Documents (Missing in Baseline)
    def test_extra_documents(self) -> None:
        doc1 = {"id": "/repositories/2/resources/1", "title": "R1"}
        doc3 = {"id": "/repositories/2/resources/3", "title": "R3"}

        base = {"/repositories/2/resources/1": doc1}
        cand = {"/repositories/2/resources/1": doc1, "/repositories/2/resources/3": doc3}

        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["missing_in_baseline"], ["/repositories/2/resources/3"])
        self.assertEqual(report["summary"]["missing_in_baseline_count"], 1)

    # 7. Missing Fields
    def test_missing_fields(self) -> None:
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "title": "R1",
                "subjects": ["Archives"],
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "title": "R1",
                "langcode": ["eng"],
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["missing_fields_count"], 2)
        diff_record = report["differences"]["missing_fields"][0]
        self.assertEqual(diff_record["missing_in_candidate"], ["subjects"])
        self.assertEqual(diff_record["missing_in_baseline"], ["langcode"])

    # 8. Scalar & Type Differences
    def test_scalar_and_type_differences(self) -> None:
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "title": "Native Title",
                "year_range": 1974,
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "title": "Rust Title",
                "year_range": "1974",
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        diffs = report["differences"]["value_differences"][0]["differences"]
        kinds = {d["field"]: d["kind"] for d in diffs}
        self.assertEqual(kinds["title"], "value_mismatch")
        self.assertEqual(kinds["year_range"], "type_mismatch")

    # 9. Reordered Arrays (Must Not Be Normalized Away)
    def test_reordered_arrays_not_normalized(self) -> None:
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "subjects": ["Irish archives", "Sánchez"],
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "subjects": ["Sánchez", "Irish archives"],
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["array_order_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "subjects")
        self.assertEqual(diff["kind"], "array_order")

    # 10. Ignored Fields (Default and Custom)
    def test_ignored_fields(self) -> None:
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "title": "R1",
                "_version_": 100,
                "score": 1.5,
                "custom_timestamp": "2026-10-17T12:00:00Z",
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "title": "R1",
                "_version_": 200,
                "score": 3.0,
                "custom_timestamp": "2026-10-17T12:05:00Z",
            }
        }
        ignored = comparator.DEFAULT_IGNORED_FIELDS | {"custom_timestamp"}
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=ignored,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertTrue(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 0)

    # 11. Marker Leakage on Either Side
    def test_marker_leakage_detection(self) -> None:
        # Candidate leak
        report_cand = comparator.compare_documents(
            baseline_docs={"r1": {"id": "r1"}},
            candidate_docs={"r1": {"id": "r1"}},
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=["r1"],
        )
        self.assertFalse(report_cand["summary"]["parity"])
        self.assertEqual(report_cand["summary"]["candidate_marker_leaks_count"], 1)

        # Baseline leak
        report_base = comparator.compare_documents(
            baseline_docs={"r1": {"id": "r1"}},
            candidate_docs={"r1": {"id": "r1"}},
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=["r1"],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report_base["summary"]["parity"])
        self.assertEqual(report_base["summary"]["baseline_marker_leaks_count"], 1)

    # 12. Multiple Cursor Pages & numFound Validation via Local HTTP Stub
    def test_multiple_cursor_pages_success(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            self.assertEqual(params.get("fq"), ['repository:"/repositories/2"'])
            cursor = params.get("cursorMark", ["*"])[0]

            if cursor == "*":
                docs = [{"id": f"/repositories/2/resources/{i}", "title": f"R{i}"} for i in range(1, 3)]
                return 200, {"response": {"numFound": 3, "docs": docs}, "nextCursorMark": "cursor_page2"}
            elif cursor == "cursor_page2":
                docs = [{"id": "/repositories/2/resources/3", "title": "R3"}]
                return 200, {"response": {"numFound": 3, "docs": docs}, "nextCursorMark": "cursor_page3"}
            elif cursor == "cursor_page3":
                return 200, {"response": {"numFound": 3, "docs": []}, "nextCursorMark": "cursor_page3"}
            return 500, {}

        StubSolrHandler.response_dispatcher = dispatcher
        docs, num_found, leaks = comparator.fetch_solr_documents(
            collection_url=self.server_url("native"),
            repository_scope="/repositories/2",
            batch_size=2,
        )
        self.assertEqual(num_found, 3)
        self.assertEqual(len(docs), 3)
        self.assertEqual(len(leaks), 0)

    # 13. Duplicate ID Detection in Pagination
    def test_duplicate_id_detection(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            docs = [
                {"id": "/repositories/2/resources/1", "title": "R1"},
                {"id": "/repositories/2/resources/1", "title": "R1 duplicate"},
            ]
            return 200, {"response": {"numFound": 2, "docs": docs}, "nextCursorMark": "c1"}

        StubSolrHandler.response_dispatcher = dispatcher
        with self.assertRaises(comparator.TransportError) as ctx:
            comparator.fetch_solr_documents(
                collection_url=self.server_url("native"),
                repository_scope="/repositories/2",
            )
        self.assertIn("Duplicate document ID", str(ctx.exception))

    # 14. Pagination Loop Detection
    def test_pagination_loop_detection(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            cursor = params.get("cursorMark", ["*"])[0]
            if cursor == "*":
                docs = [{"id": "/repositories/2/resources/1", "title": "R1"}]
                return 200, {"response": {"numFound": 10, "docs": docs}, "nextCursorMark": "c1"}
            elif cursor == "c1":
                docs = [{"id": "/repositories/2/resources/2", "title": "R2"}]
                return 200, {"response": {"numFound": 10, "docs": docs}, "nextCursorMark": "c2"}
            elif cursor == "c2":
                docs = [{"id": "/repositories/2/resources/3", "title": "R3"}]
                return 200, {"response": {"numFound": 10, "docs": docs}, "nextCursorMark": "c1"}
            return 500, {}

        StubSolrHandler.response_dispatcher = dispatcher
        with self.assertRaises(comparator.TransportError) as ctx:
            comparator.fetch_solr_documents(
                collection_url=self.server_url("native"),
                repository_scope="/repositories/2",
                batch_size=1,
            )
        self.assertIn("Pagination loop detected", str(ctx.exception))

    # 15. numFound Mismatch Detection
    def test_numfound_mismatch_detection(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            cursor = params.get("cursorMark", ["*"])[0]
            if cursor == "*":
                docs = [{"id": "/repositories/2/resources/1", "title": "R1"}]
                return 200, {"response": {"numFound": 5, "docs": docs}, "nextCursorMark": "c_end"}
            else:
                return 200, {"response": {"numFound": 5, "docs": []}, "nextCursorMark": "c_end"}

        StubSolrHandler.response_dispatcher = dispatcher
        with self.assertRaises(comparator.TransportError) as ctx:
            comparator.fetch_solr_documents(
                collection_url=self.server_url("native"),
                repository_scope="/repositories/2",
                batch_size=1,
            )
        self.assertIn("does not match Solr numFound", str(ctx.exception))

    # 16. Marker Leakage Scanning in Fetcher
    def test_marker_leakage_in_fetcher(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            docs = [
                {
                    "id": "/repositories/2/resources/1",
                    "title": "R1",
                    comparator.MARKER_FIELD: ["common_metadata"],
                }
            ]
            return 200, {"response": {"numFound": 1, "docs": docs}, "nextCursorMark": "done"}

        StubSolrHandler.response_dispatcher = dispatcher
        docs, num_found, leaks = comparator.fetch_solr_documents(
            collection_url=self.server_url("candidate"),
            repository_scope="/repositories/2",
        )
        self.assertEqual(leaks, ["/repositories/2/resources/1"])

    # 17. Transport Failure Handling (HTTP 500, Bad JSON, Connection Refused)
    def test_transport_failures(self) -> None:
        # HTTP 500
        def error_dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            return 500, {"error": "Solr core error"}

        StubSolrHandler.response_dispatcher = error_dispatcher
        with self.assertRaises(comparator.TransportError) as ctx:
            comparator.fetch_solr_documents(
                collection_url=self.server_url("broken"),
                repository_scope="/repositories/2",
            )
        self.assertIn("Solr HTTP 500", str(ctx.exception))

        # Connection refused / invalid port
        with self.assertRaises(comparator.TransportError) as ctx:
            comparator.fetch_solr_documents(
                collection_url="http://127.0.0.1:1/solr/nonexistent",
                repository_scope="/repositories/2",
                timeout=1.0,
            )
        self.assertIn("Solr network error", str(ctx.exception))

    # 18. Basic Auth via Environment Variables
    def test_basic_auth_header_sent(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            return 200, {"response": {"numFound": 0, "docs": []}, "nextCursorMark": "done"}

        StubSolrHandler.response_dispatcher = dispatcher
        os.environ["BASELINE_SOLR_USER"] = "alice"
        os.environ["BASELINE_SOLR_PASSWORD"] = "secret_pass"
        try:
            creds = comparator.get_auth_credentials(
                self.server_url("native"),
                "BASELINE_SOLR_USER",
                "BASELINE_SOLR_PASSWORD",
            )
            self.assertEqual(creds, ("alice", "secret_pass"))

            comparator.fetch_solr_documents(
                collection_url=self.server_url("native"),
                repository_scope="/repositories/2",
                auth=creds,
            )
            req = StubSolrHandler.received_requests[-1]
            self.assertTrue(req["auth"].startswith("Basic "))
        finally:
            os.environ.pop("BASELINE_SOLR_USER", None)
            os.environ.pop("BASELINE_SOLR_PASSWORD", None)

    # 19. Disallow CLI Credentials
    def test_disallow_cli_credentials(self) -> None:
        exit_code = comparator.main(["--user", "admin", "--baseline-url", "a", "--candidate-url", "b", "--repo", "2"])
        self.assertEqual(exit_code, comparator.EXIT_CONFIG_ERROR)

    # 20. End-to-End CLI Invocation with JSON Output
    def test_cli_e2e_json_output(self) -> None:
        doc = {
            "id": "/repositories/2/resources/1",
            "title": "E2E Parity",
            "_version_": 999,
        }

        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            return 200, {"response": {"numFound": 1, "docs": [doc]}, "nextCursorMark": "done"}

        StubSolrHandler.response_dispatcher = dispatcher

        base_url = self.server_url("native")
        cand_url = self.server_url("rust")

        # Capture stdout
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            exit_code = comparator.main([
                "--baseline-url", base_url,
                "--candidate-url", cand_url,
                "--repository", "2",
                "--json",
            ])
            output = sys.stdout.getvalue()
        finally:
            sys.stdout = old_stdout

        self.assertEqual(exit_code, comparator.EXIT_PARITY_SUCCESS)
        parsed = json.loads(output)
        self.assertTrue(parsed["summary"]["parity"])
        self.assertEqual(parsed["repository"], "/repositories/2")
        self.assertEqual(parsed["effective_ignored_fields"], ["_version_", "score"])
        self.assertEqual(parsed["baseline_count"], 1)
        self.assertEqual(parsed["candidate_count"], 1)

    # 21. End-to-End CLI Invocation with Parity Failure
    def test_cli_e2e_parity_failure(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            if "/solr/native" in path:
                doc = {"id": "/repositories/2/resources/1", "title": "Native Title"}
            else:
                doc = {"id": "/repositories/2/resources/1", "title": "Rust Title"}
            return 200, {"response": {"numFound": 1, "docs": [doc]}, "nextCursorMark": "done"}

        StubSolrHandler.response_dispatcher = dispatcher

        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            exit_code = comparator.main([
                "--baseline-url", self.server_url("native"),
                "--candidate-url", self.server_url("rust"),
                "--repository", "2",
                "--json",
            ])
            output = sys.stdout.getvalue()
        finally:
            sys.stdout = old_stdout

        self.assertEqual(exit_code, comparator.EXIT_PARITY_MISMATCH)
        parsed = json.loads(output)
        self.assertFalse(parsed["summary"]["parity"])
        self.assertEqual(parsed["summary"]["value_diffs_count"], 1)

    # 22. Marker leakage fails even if explicitly added to --ignore-field
    def test_cli_e2e_marker_leak_fails_even_if_ignored(self) -> None:
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            if "/solr/native" in path:
                doc = {"id": "/repositories/2/resources/1", "title": "Same Title"}
            else:
                doc = {
                    "id": "/repositories/2/resources/1",
                    "title": "Same Title",
                    comparator.MARKER_FIELD: ["common_metadata"],
                }
            return 200, {"response": {"numFound": 1, "docs": [doc]}, "nextCursorMark": "done"}

        StubSolrHandler.response_dispatcher = dispatcher

        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            exit_code = comparator.main([
                "--baseline-url", self.server_url("native"),
                "--candidate-url", self.server_url("rust"),
                "--repository", "2",
                "--ignore-field", comparator.MARKER_FIELD,
                "--json",
            ])
            output = sys.stdout.getvalue()
        finally:
            sys.stdout = old_stdout

        self.assertEqual(exit_code, comparator.EXIT_PARITY_MISMATCH)
        parsed = json.loads(output)
        self.assertFalse(parsed["summary"]["parity"])
        self.assertEqual(parsed["summary"]["candidate_marker_leaks_count"], 1)

    # 23. End-to-End CLI Invocation with Human-Readable Output
    def test_cli_e2e_human_readable_output(self) -> None:
        doc = {"id": "/repositories/2/resources/1", "title": "Same Title"}

        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            return 200, {"response": {"numFound": 1, "docs": [doc]}, "nextCursorMark": "done"}

        StubSolrHandler.response_dispatcher = dispatcher

        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            exit_code = comparator.main([
                "--baseline-url", self.server_url("native"),
                "--candidate-url", self.server_url("rust"),
                "--repository", "2",
            ])
            output = sys.stdout.getvalue()
        finally:
            sys.stdout = old_stdout

        self.assertEqual(exit_code, comparator.EXIT_PARITY_SUCCESS)
        self.assertIn("=== Staff Index Parity Comparison ===", output)
        self.assertIn("OVERALL RESULT: PASS", output)
        self.assertIn("Effective ignored fields: _version_, score", output)

    # 24. Exact JSON Parity & Recursive Type Safety Regression Tests

    def test_top_level_bool_versus_int(self) -> None:
        """Top-level bool vs int must fail parity as a type mismatch."""
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "publish": True,
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "publish": 1,
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["value_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "publish")
        self.assertEqual(diff["kind"], "type_mismatch")
        self.assertEqual(diff["baseline_type"], "bool")
        self.assertEqual(diff["candidate_type"], "int")
        self.assertEqual(diff["baseline_value"], True)
        self.assertEqual(diff["candidate_value"], 1)

        # Also verify False vs 0
        base_f = {"/repositories/2/resources/1": {"id": "/repositories/2/resources/1", "suppressed": False}}
        cand_0 = {"/repositories/2/resources/1": {"id": "/repositories/2/resources/1", "suppressed": 0}}
        report_f = comparator.compare_documents(
            baseline_docs=base_f,
            candidate_docs=cand_0,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report_f["summary"]["parity"])
        diff_f = report_f["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff_f["kind"], "type_mismatch")
        self.assertEqual(diff_f["baseline_type"], "bool")
        self.assertEqual(diff_f["candidate_type"], "int")

    def test_top_level_int_versus_float(self) -> None:
        """Top-level int vs float must fail parity as a type mismatch."""
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "position": 1,
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "position": 1.0,
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["value_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "position")
        self.assertEqual(diff["kind"], "type_mismatch")
        self.assertEqual(diff["baseline_type"], "int")
        self.assertEqual(diff["candidate_type"], "float")
        self.assertEqual(diff["baseline_value"], 1)
        self.assertEqual(diff["candidate_value"], 1.0)

    def test_bool_int_differences_inside_arrays(self) -> None:
        """Bool vs int differences inside arrays must be detected and fail parity."""
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "flags": [True, 1],
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "flags": [1, 1],
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["value_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "flags")
        self.assertEqual(diff["kind"], "value_mismatch")
        self.assertFalse(comparator.is_array_reordered([True], [1]))

    def test_numeric_type_differences_inside_nested_objects(self) -> None:
        """Numeric type differences (int vs float) nested inside objects must fail parity."""
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "metrics": {"total_count": 10, "ratio": 1},
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "metrics": {"total_count": 10, "ratio": 1.0},
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["value_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "metrics")
        self.assertEqual(diff["kind"], "value_mismatch")

    def test_reordered_arrays_containing_duplicate_scalars(self) -> None:
        """Reordered arrays containing duplicate scalar values must report array_order."""
        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "tags": [1, 2, 1, 3],
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "tags": [2, 1, 3, 1],
            }
        }
        self.assertTrue(comparator.is_array_reordered([1, 2, 1, 3], [2, 1, 3, 1]))
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["array_order_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "tags")
        self.assertEqual(diff["kind"], "array_order")

        # Duplicate counts mismatch must not be treated as simple reordering
        self.assertFalse(comparator.is_array_reordered([1, 2, 1, 3], [1, 2, 3, 3]))

    def test_reordered_arrays_containing_nested_objects(self) -> None:
        """Reordered arrays containing unhashable nested objects must report array_order."""
        base_objs = [{"key": "a", "val": 1}, {"key": "b", "val": 2}]
        cand_objs = [{"key": "b", "val": 2}, {"key": "a", "val": 1}]
        self.assertTrue(comparator.is_array_reordered(base_objs, cand_objs))

        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "entries": base_objs,
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "entries": cand_objs,
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["array_order_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "entries")
        self.assertEqual(diff["kind"], "array_order")

        # Nested type mismatch must not be considered reordered
        cand_objs_mismatch = [{"key": "b", "val": 2.0}, {"key": "a", "val": 1}]
        self.assertFalse(comparator.is_array_reordered(base_objs, cand_objs_mismatch))

    def test_arrays_equal_under_python_loose_equality_not_exact_json_parity(self) -> None:
        """Arrays equal under Python loose equality must fail exact JSON parity."""
        list1 = [True, 1.0, 0]
        list2 = [1, 1, False]

        # Verify Python loose equality treats them as equal
        self.assertEqual(list1, list2)
        # Verify strict comparator rejects equality and reordering
        self.assertFalse(comparator.json_values_strictly_equal(list1, list2))
        self.assertFalse(comparator.is_array_reordered(list1, list2))

        base = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "values": list1,
            }
        }
        cand = {
            "/repositories/2/resources/1": {
                "id": "/repositories/2/resources/1",
                "values": list2,
            }
        }
        report = comparator.compare_documents(
            baseline_docs=base,
            candidate_docs=cand,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["value_diffs_count"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "values")
        self.assertEqual(diff["kind"], "value_mismatch")

        # Reordered array case where loose Counter equality treats them as equal
        from collections import Counter as PyCounter
        reordered_loose_1 = [True, 2]
        reordered_loose_2 = [2, 1]
        self.assertEqual(PyCounter(reordered_loose_1), PyCounter(reordered_loose_2))
        self.assertFalse(comparator.is_array_reordered(reordered_loose_1, reordered_loose_2))

    def test_json_values_strictly_equal_unit(self) -> None:
        """Direct unit checks for json_values_strictly_equal."""
        self.assertTrue(comparator.json_values_strictly_equal(None, None))
        self.assertTrue(comparator.json_values_strictly_equal(True, True))
        self.assertTrue(comparator.json_values_strictly_equal(1, 1))
        self.assertTrue(comparator.json_values_strictly_equal(1.5, 1.5))
        self.assertTrue(comparator.json_values_strictly_equal("test", "test"))
        self.assertTrue(comparator.json_values_strictly_equal([1, 2], [1, 2]))
        self.assertTrue(comparator.json_values_strictly_equal({"a": 1, "b": 2}, {"b": 2, "a": 1}))

        # Strict type inequality
        self.assertFalse(comparator.json_values_strictly_equal(True, 1))
        self.assertFalse(comparator.json_values_strictly_equal(False, 0))
        self.assertFalse(comparator.json_values_strictly_equal(1, 1.0))
        self.assertFalse(comparator.json_values_strictly_equal([True], [1]))
        self.assertFalse(comparator.json_values_strictly_equal([1], [1.0]))
        self.assertFalse(comparator.json_values_strictly_equal({"a": True}, {"a": 1}))
        self.assertFalse(comparator.json_values_strictly_equal({"a": 1}, {"a": 1.0}))

    def test_accession_date_ordinary_and_sentinel_parity(self) -> None:
        """Accession documents with ordinary and sentinel dates compare cleanly under exact parity."""
        docs = {
            "/repositories/2/accessions/10": {
                "id": "/repositories/2/accessions/10",
                "types": ["accession"],
                "primary_type": "accession",
                "title": "Accession 10",
                "accession_date": "2026-10-17",
                "accession_date_year": 2026,
                "fullrecord": ["ACC-10 title", "summary content"],
                "_version_": 1001,
            },
            "/repositories/2/accessions/11": {
                "id": "/repositories/2/accessions/11",
                "types": ["accession"],
                "primary_type": "accession",
                "title": "Accession 11",
                "accession_date": "Date Unknown",
                "accession_date_year": 9999,
                "fullrecord": ["ACC-11 title", "Date Unknown "],
                "_version_": 1002,
            },
        }
        report = comparator.compare_documents(
            baseline_docs=docs,
            candidate_docs=docs,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertTrue(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 0)
        self.assertEqual(report["baseline_count"], 2)
        self.assertEqual(report["candidate_count"], 2)

    def test_accession_date_year_type_mismatch_detected(self) -> None:
        """Integer vs String type mismatch on accession_date_year is detected and fails parity."""
        baseline = {
            "/repositories/2/accessions/10": {
                "id": "/repositories/2/accessions/10",
                "accession_date": "2026-10-17",
                "accession_date_year": 2026,
            }
        }
        candidate = {
            "/repositories/2/accessions/10": {
                "id": "/repositories/2/accessions/10",
                "accession_date": "2026-10-17",
                "accession_date_year": "2026",  # Coerced string vs integer
            }
        }
        report = comparator.compare_documents(
            baseline_docs=baseline,
            candidate_docs=candidate,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "accession_date_year")
        self.assertEqual(diff["kind"], "type_mismatch")

    def test_accession_sentinel_fullrecord_mismatch_detected(self) -> None:
        """Missing sentinel label append in fullrecord is detected and fails parity."""
        baseline = {
            "/repositories/2/accessions/11": {
                "id": "/repositories/2/accessions/11",
                "accession_date": "незнаний",
                "accession_date_year": 9999,
                "fullrecord": ["body", "незнаний "],
            }
        }
        candidate = {
            "/repositories/2/accessions/11": {
                "id": "/repositories/2/accessions/11",
                "accession_date": "незнаний",
                "accession_date_year": 9999,
                "fullrecord": ["body"],  # Missing localized unknown append
            }
        }
        report = comparator.compare_documents(
            baseline_docs=baseline,
            candidate_docs=candidate,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "fullrecord")
        self.assertEqual(diff["kind"], "value_mismatch")

    def test_accession_date_non_canonical_schema_valid_parity(self) -> None:
        """Non-canonical schema-valid dates (year 0, non-padded month, whitespace) compare cleanly."""
        docs = {
            "/repositories/2/accessions/20": {
                "id": "/repositories/2/accessions/20",
                "types": ["accession"],
                "accession_date": "0000-01-01",
                "accession_date_year": 0,
                "fullrecord": ["Legacy Ancient Acc"],
            },
            "/repositories/2/accessions/21": {
                "id": "/repositories/2/accessions/21",
                "types": ["accession"],
                "accession_date": "2026-2-03",
                "accession_date_year": 2026,
                "fullrecord": ["Unpadded Month Acc"],
            },
            "/repositories/2/accessions/22": {
                "id": "/repositories/2/accessions/22",
                "types": ["accession"],
                "accession_date": "  2026-10-17  ",
                "accession_date_year": 2026,
                "fullrecord": ["Whitespace Acc"],
            },
        }
        report = comparator.compare_documents(
            baseline_docs=docs,
            candidate_docs=docs,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertTrue(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 0)
        self.assertEqual(report["baseline_count"], 3)
        self.assertEqual(report["candidate_count"], 3)

    def test_accession_ordinary_record_with_unknown_label_in_fullrecord_parity(self) -> None:
        """Ordinary accessions whose legitimate content happens to end with unknown label string have parity."""
        docs = {
            "/repositories/2/accessions/30": {
                "id": "/repositories/2/accessions/30",
                "types": ["accession"],
                "title": "Accession 30",
                "accession_date": "2026-10-17",
                "accession_date_year": 2026,
                "fullrecord": ["Scope and content", "Date Unknown "],
            }
        }
        report = comparator.compare_documents(
            baseline_docs=docs,
            candidate_docs=docs,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertTrue(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 0)

    def test_accession_candidate_wrong_localized_label_parity_failure(self) -> None:
        """Candidate appending the wrong localized label to sentinel fullrecord fails parity."""
        baseline = {
            "/repositories/2/accessions/40": {
                "id": "/repositories/2/accessions/40",
                "accession_date": "незнаний",
                "accession_date_year": 9999,
                "fullrecord": ["content", "незнаний "],
            }
        }
        candidate = {
            "/repositories/2/accessions/40": {
                "id": "/repositories/2/accessions/40",
                "accession_date": "незнаний",
                "accession_date_year": 9999,
                "fullrecord": ["content", "Date Unknown "],  # Wrong localization appended
            }
        }
        report = comparator.compare_documents(
            baseline_docs=baseline,
            candidate_docs=candidate,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 1)
        diff = report["differences"]["value_differences"][0]["differences"][0]
        self.assertEqual(diff["field"], "fullrecord")
        self.assertEqual(diff["kind"], "value_mismatch")

    def test_accession_marker_leakage_detected(self) -> None:
        """Candidate leaking accession_date_metadata marker into Solr fails parity."""
        report = comparator.compare_documents(
            baseline_docs={"/repositories/2/accessions/10": {"id": "/repositories/2/accessions/10"}},
            candidate_docs={"/repositories/2/accessions/10": {"id": "/repositories/2/accessions/10"}},
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=["/repositories/2/accessions/10"],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["candidate_marker_leaks_count"], 1)
        self.assertIn("/repositories/2/accessions/10", report["marker_leaks"]["candidate"])

    # 25. PUI Availability Parity & Marker Leakage Tests
    def test_pui_availability_marker_leakage_in_fetcher(self) -> None:
        """Candidate leaking pui_availability_metadata marker into Solr is caught by fetcher."""
        def dispatcher(path: str, params: Dict[str, List[str]]) -> Tuple[int, Dict[str, Any]]:
            docs = [
                {
                    "id": "/repositories/2/top_containers/1",
                    "title": "Box 1",
                    comparator.MARKER_FIELD: ["pui_availability_metadata"],
                }
            ]
            return 200, {"response": {"numFound": 1, "docs": docs}, "nextCursorMark": "done"}

        StubSolrHandler.response_dispatcher = dispatcher
        docs, num_found, leaks = comparator.fetch_solr_documents(
            collection_url=self.server_url("candidate"),
            repository_scope="/repositories/2",
        )
        self.assertEqual(leaks, ["/repositories/2/top_containers/1"])

    def test_pui_availability_projection_key_leak_fails_parity(self) -> None:
        """Candidate leaking _archivalist_pui_availability into Solr fails parity."""
        baseline = {
            "/repositories/2/top_containers/1": {
                "id": "/repositories/2/top_containers/1",
                "types": ["top_container", "pui_top_container", "pui", "pui_container", "pui"],
                "publish": True,
            }
        }
        candidate = {
            "/repositories/2/top_containers/1": {
                "id": "/repositories/2/top_containers/1",
                "types": ["top_container", "pui_top_container", "pui", "pui_container", "pui"],
                "publish": True,
                "_archivalist_pui_availability": {
                    "marker": "pui_availability_metadata",
                    "uri": "/repositories/2/top_containers/1",
                },
            }
        }
        report = comparator.compare_documents(
            baseline_docs=baseline,
            candidate_docs=candidate,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertFalse(report["summary"]["parity"])
        self.assertEqual(report["summary"]["missing_fields_count"], 1)
        missing_record = report["differences"]["missing_fields"][0]
        self.assertEqual(missing_record["id"], "/repositories/2/top_containers/1")
        self.assertEqual(missing_record["missing_in_baseline"], ["_archivalist_pui_availability"])

    def test_pui_availability_exact_parity_with_native_types(self) -> None:
        """Candidate with clean PUI availability types matches native baseline perfectly."""
        docs = {
            "/repositories/2/top_containers/1": {
                "id": "/repositories/2/top_containers/1",
                "types": ["top_container", "pui_top_container", "pui", "pui_container", "pui"],
                "publish": True,
                "_version_": 100,
            },
            "/repositories/2/resources/5": {
                "id": "/repositories/2/resources/5",
                "types": ["resource", "pui_collection", "pui"],
                "publish": True,
                "_version_": 101,
            },
            "/repositories/2/archival_objects/12": {
                "id": "/repositories/2/archival_objects/12",
                "types": ["archival_object"],
                "publish": False,
                "_version_": 102,
            },
        }
        report = comparator.compare_documents(
            baseline_docs=docs,
            candidate_docs=docs,
            ignored_fields=comparator.DEFAULT_IGNORED_FIELDS,
            baseline_marker_leaks=[],
            candidate_marker_leaks=[],
        )
        self.assertTrue(report["summary"]["parity"])
        self.assertEqual(report["summary"]["total_differences"], 0)
        self.assertEqual(report["baseline_count"], 3)
        self.assertEqual(report["candidate_count"], 3)


if __name__ == "__main__":
    unittest.main()
