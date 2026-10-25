#!/usr/bin/env python3
"""Offline unit tests for benchmark_native_search_and_tree.py.

Verifies identity failure safety before login, resource/node discovery,
failure isolation across routes, percentile calculations, real response shape
parsing, and zero secret leakage.
"""

import json
import os
import sys
import unittest
import urllib.error
from unittest.mock import MagicMock, patch

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
if SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, SCRIPTS_DIR)

import benchmark_native_search_and_tree as bench


class TestBenchmarkNativeSearchAndTree(unittest.TestCase):
    def setUp(self):
        self.dummy_state = {
            "stack_id": "test_stack_1234",
            "backend_host_url": "http://127.0.0.1:4567",
            "solr_host_url": "http://127.0.0.1:8983/solr",
            "admin_user": "admin",
            "admin_password": "super_secret_password",
        }

    def test_percentiles_calculation(self):
        # 20 samples from 1.0 to 20.0
        durations = [float(i) for i in range(1, 21)]
        res = bench.calculate_percentiles(durations)
        self.assertEqual(res["p50_ms"], 10.0)
        self.assertEqual(res["p95_ms"], 19.0)
        self.assertEqual(res["min_ms"], 1.0)
        self.assertEqual(res["max_ms"], 20.0)
        self.assertEqual(res["mean_ms"], 10.5)

        empty_res = bench.calculate_percentiles([])
        self.assertIsNone(empty_res["p50_ms"])
        self.assertIsNone(empty_res["p95_ms"])

    def test_real_shaped_response_counts(self):
        # Search response fixture
        search_body = {"results": [{"uri": "1"}, {"uri": "2"}], "total_hits": 42}
        hits, count = bench.extract_counts(search_body)
        self.assertEqual(hits, 42)
        self.assertEqual(count, 2)

        # Tree root response fixture
        root_body = {
            "child_count": 8,
            "waypoints": 1,
            "precomputed_waypoints": {"": {"0": [{"uri": "a"}, {"uri": "b"}]}},
        }
        hits, count = bench.extract_counts(root_body)
        self.assertEqual(hits, 8)
        self.assertEqual(count, 2)

        # Tree waypoint response fixture
        wp_body = [{"uri": "a"}, {"uri": "b"}, {"uri": "c"}]
        hits, count = bench.extract_counts(wp_body)
        self.assertIsNone(hits)
        self.assertEqual(count, 3)

        # Tree node response fixture
        node_body = {"title": "Node A", "child_count": 2, "waypoints": 1}
        hits, count = bench.extract_counts(node_body)
        self.assertEqual(hits, 2)
        self.assertIsNone(count)

        # Malformed
        hits, count = bench.extract_counts("not a dict or list")
        self.assertIsNone(hits)
        self.assertIsNone(count)

    def test_node_discovery_from_precomputed_waypoints(self):
        root_fixture = {
            "title": "Resource Root",
            "uri": "/repositories/2/resources/1",
            "child_count": 10,
            "waypoints": 1,
            "precomputed_waypoints": {
                "": {
                    "0": [
                        {"uri": "/repositories/2/archival_objects/101", "title": "Child 1"}
                    ]
                }
            },
        }
        uri, err = bench.discover_node_uri(
            root_fixture, "http://127.0.0.1:4567", 2, 1, {}, timeout=5
        )
        self.assertEqual(uri, "/repositories/2/archival_objects/101")
        self.assertIsNone(err)

    @patch("benchmark_native_search_and_tree.http_request")
    def test_node_discovery_fallback_from_waypoint_0(self, mock_http):
        root_fixture = {
            "title": "Resource Root",
            "uri": "/repositories/2/resources/1",
            "child_count": 10,
            "waypoints": 1,
            "precomputed_waypoints": {},
        }
        wp_fixture = [{"uri": "/repositories/2/archival_objects/202", "title": "WP Child"}]
        mock_http.return_value = (200, wp_fixture, 4.0)

        uri, err = bench.discover_node_uri(
            root_fixture, "http://127.0.0.1:4567", 2, 1, {}, timeout=5
        )
        self.assertEqual(uri, "/repositories/2/archival_objects/202")
        self.assertIsNone(err)

    def test_identity_failure_before_login(self):
        mock_stack = MagicMock()
        mock_stack.load_state.return_value = self.dummy_state
        mock_stack.verify_identity.side_effect = RuntimeError("Stack identity verification failed")

        with patch("benchmark_native_search_and_tree.admin_login") as mock_login:
            with self.assertRaises(RuntimeError) as ctx:
                bench.run_benchmark(
                    state_path="/fake/state.json",
                    stack_mod=mock_stack,
                )
            self.assertIn("identity verification failed", str(ctx.exception).lower())
            mock_login.assert_not_called()

    @patch("benchmark_native_search_and_tree.admin_login")
    @patch("benchmark_native_search_and_tree.http_request")
    def test_failure_isolation_preserves_completed_results(self, mock_http, mock_login):
        mock_login.return_value = "session_token_xyz"
        mock_stack = MagicMock()
        mock_stack.load_state.return_value = self.dummy_state
        mock_stack.verify_identity.return_value = "nonce_123"

        search_resp = (200, {"results": [{"uri": "1"}], "total_hits": 1}, 10.0)

        def mock_request_impl(url, *args, **kwargs):
            if "search" in url:
                return search_resp
            if "tree/root" in url:
                raise urllib.error.URLError("Connection refused")
            return (200, [], 5.0)

        mock_http.side_effect = mock_request_impl

        report = bench.run_benchmark(
            state_path="/fake/state.json",
            repeats=2,
            warmup=1,
            stack_mod=mock_stack,
        )

        routes = report["routes"]
        self.assertEqual(len(routes), 5)

        # Searches succeeded and are preserved
        self.assertEqual(routes[0]["status"], "ok")
        self.assertEqual(len(routes[0]["repeat_durations_ms"]), 2)
        self.assertEqual(routes[1]["status"], "ok")

        # Tree root failed
        self.assertEqual(routes[2]["status"], "blocked")
        self.assertIn("Connection refused", routes[2]["blocker"])

        # Tree node blocked because root failed and node could not be discovered
        self.assertEqual(routes[4]["status"], "blocked")

    @patch("benchmark_native_search_and_tree.http_request")
    def test_no_secret_serialization(self, mock_http):
        secret_password = "very_sensitive_admin_password_99"
        secret_session = "top_secret_session_token_xyz"

        mock_stack = MagicMock()
        state = dict(self.dummy_state, admin_password=secret_password)
        mock_stack.load_state.return_value = state
        mock_stack.verify_identity.return_value = "nonce_123"

        def mock_request(url, *args, **kwargs):
            if "login" in url:
                return (200, {"session": secret_session}, 5.0)
            if "search" in url:
                return (200, {"results": [], "total_hits": 0}, 5.0)
            if "tree/root" in url:
                return (200, {"child_count": 0, "waypoints": 0, "precomputed_waypoints": {}}, 5.0)
            return (200, [], 5.0)

        mock_http.side_effect = mock_request

        report = bench.run_benchmark(
            state_path="/fake/state.json",
            admin_password=secret_password,
            repeats=2,
            warmup=1,
            stack_mod=mock_stack,
        )

        rendered = json.dumps(report)
        self.assertNotIn(secret_password, rendered)
        self.assertNotIn(secret_session, rendered)
        self.assertEqual(report["stack_id"], "test_stack_1234")
        self.assertEqual(report["timing_scope"], "backend_api_proxy_only")
        self.assertIn("excludes client-side JSON decode", report["timing_scope_detail"])
        self.assertIn("excludes client-side JSON decode", report["notes"])

    @patch("urllib.request.urlopen")
    def test_http_request_duration_includes_body_read_and_excludes_json_decode(self, mock_urlopen):
        fake_body_bytes = b'{"status": "ok", "value": 42}'
        mock_clock = [100.0]

        def fake_perf_counter():
            return mock_clock[0]

        class FakeResponse:
            def __init__(self, data):
                self.status = 200
                self.data = data

            def __enter__(self):
                mock_clock[0] += 0.010  # 10ms for connect + headers
                return self

            def __exit__(self, *args):
                pass

            def read(self):
                # Body transfer over wire advances clock by 50ms
                mock_clock[0] += 0.050
                return self.data

        mock_urlopen.return_value = FakeResponse(fake_body_bytes)

        # Mock json.loads to advance clock by 100ms during decode
        original_loads = json.loads

        def delayed_loads(s):
            mock_clock[0] += 0.100
            return original_loads(s)

        with patch("time.perf_counter", side_effect=fake_perf_counter):
            with patch("json.loads", side_effect=delayed_loads):
                status, parsed, dur = bench.http_request("http://127.0.0.1:4567/test")

        self.assertEqual(status, 200)
        self.assertEqual(parsed, {"status": "ok", "value": 42})
        # Timing covers 10ms (headers) + 50ms (body read) = 60ms.
        # It strictly excludes the 100ms JSON decoding phase.
        self.assertAlmostEqual(dur, 60.0, places=2)
        # Total clock advanced by 10ms + 50ms + 100ms = 160ms.
        self.assertAlmostEqual(mock_clock[0], 100.160, places=2)

    @patch("benchmark_native_search_and_tree.admin_login")
    @patch("benchmark_native_search_and_tree.http_request")
    def test_partial_repeat_durations_preserved_on_later_repeat_failure(self, mock_http, mock_login):
        mock_login.return_value = "session_token_xyz"
        mock_stack = MagicMock()
        mock_stack.load_state.return_value = self.dummy_state
        mock_stack.verify_identity.return_value = "nonce_123"

        call_count = {"count": 0}

        def mock_request_impl(url, *args, **kwargs):
            params = kwargs.get("params") or (args[2] if len(args) > 2 else {})
            if "search" in url and params.get("q") == "*":
                call_count["count"] += 1
                # Warmup (1 call): success
                if call_count["count"] == 1:
                    return (200, {"results": [{"uri": "1"}], "total_hits": 1}, 12.0)
                # Repeat 1: success
                if call_count["count"] == 2:
                    return (200, {"results": [{"uri": "1"}], "total_hits": 1}, 15.0)
                # Repeat 2: success
                if call_count["count"] == 3:
                    return (200, {"results": [{"uri": "1"}], "total_hits": 1}, 18.0)
                # Repeat 3: network drop / socket error
                raise urllib.error.URLError("Socket reset by peer")
            if "search" in url:
                return (200, {"results": [], "total_hits": 0}, 10.0)
            if "tree/root" in url:
                return (200, {"child_count": 0, "waypoints": 0, "precomputed_waypoints": {}}, 10.0)
            return (200, [], 10.0)

        mock_http.side_effect = mock_request_impl

        report = bench.run_benchmark(
            state_path="/fake/state.json",
            repeats=3,
            warmup=1,
            stack_mod=mock_stack,
        )

        routes = report["routes"]
        first_route = routes[0]
        self.assertEqual(first_route["name"], "staff_repo_search_wildcard")
        self.assertEqual(first_route["status"], "blocked")
        self.assertIn("Socket reset by peer", first_route["blocker"])

        # Completed repeats 1 and 2 must stay preserved
        self.assertEqual(first_route["repeat_durations_ms"], [15.0, 18.0])
        self.assertEqual(first_route["min_ms"], 15.0)
        self.assertEqual(first_route["max_ms"], 18.0)
        self.assertEqual(first_route["p50_ms"], 15.0)
        self.assertEqual(first_route["mean_ms"], 16.5)
        self.assertEqual(first_route["total_hits"], 1)
        self.assertEqual(first_route["result_count"], 1)

        # Later routes still executed and preserved
        self.assertEqual(routes[1]["status"], "ok")
        self.assertEqual(len(routes[1]["repeat_durations_ms"]), 3)

    @patch("benchmark_native_search_and_tree.admin_login")
    @patch("benchmark_native_search_and_tree.http_request")
    def test_partial_repeat_on_tree_root_still_permits_node_discovery(self, mock_http, mock_login):
        mock_login.return_value = "session_token_xyz"
        mock_stack = MagicMock()
        mock_stack.load_state.return_value = self.dummy_state
        mock_stack.verify_identity.return_value = "nonce_123"

        root_payload = {
            "child_count": 5,
            "waypoints": 1,
            "precomputed_waypoints": {
                "": {"0": [{"uri": "/repositories/2/archival_objects/999"}]}
            },
        }

        call_count = {"root_calls": 0}

        def mock_request_impl(url, *args, **kwargs):
            if "tree/root" in url:
                call_count["root_calls"] += 1
                if call_count["root_calls"] <= 2:  # warmup (1) + repeat 1 (1)
                    return (200, root_payload, 25.0)
                raise urllib.error.URLError("Connection reset")
            if "tree/node" in url:
                return (200, {"title": "Node 999"}, 8.0)
            if "search" in url:
                return (200, {"results": [], "total_hits": 0}, 10.0)
            return (200, [], 10.0)

        mock_http.side_effect = mock_request_impl

        report = bench.run_benchmark(
            state_path="/fake/state.json",
            repeats=2,
            warmup=1,
            stack_mod=mock_stack,
        )

        root_route = report["routes"][2]
        self.assertEqual(root_route["name"], "staff_tree_root")
        self.assertEqual(root_route["status"], "blocked")
        self.assertEqual(root_route["repeat_durations_ms"], [25.0])
        self.assertEqual(root_route["total_hits"], 5)

        # staff_tree_node route was able to discover node URI from partial repeat 1
        node_route = report["routes"][4]
        self.assertEqual(node_route["name"], "staff_tree_node")
        self.assertEqual(node_route["status"], "ok")
        self.assertEqual(node_route["params"]["node_uri"], "/repositories/2/archival_objects/999")


if __name__ == "__main__":
    unittest.main()
