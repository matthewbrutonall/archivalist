#!/usr/bin/env python3
"""Offline unit tests for Gate 4 one-shot no-change Ruby STAFF handoff runner."""

from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
import unittest

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import run_gate4_no_change_handoff as runner  # noqa: E402


def make_valid_report() -> dict:
    return {
        "stack_id": "844ef09c",
        "native_core": runner.EXPECTED_NATIVE_CORE,
        "candidate_core": runner.EXPECTED_CANDIDATE_CORE,
        "binary_provenance": {
            "sha256": runner.EXPECTED_BINARY_SHA256,
        },
        "cli_summary": {
            "full_reindex_started_at_epoch_sec": runner.EXPECTED_WATERMARK_EPOCH,
            "repository_ids": [2, 3, 4],
            "total_ids": runner.EXPECTED_TOTAL_DOCS,
            "total_documents": runner.EXPECTED_TOTAL_DOCS,
            "unique_uris": runner.EXPECTED_TOTAL_DOCS,
            "committed": True,
            "candidate_core": runner.EXPECTED_CANDIDATE_CORE,
        },
        "candidate_cli_wall_sec": 6799.744,
        "memory": {
            "candidate_peak_rss_kib": 9329056,
        },
        "solr_verified_docs": runner.EXPECTED_TOTAL_DOCS,
        "parity_result": {
            "exact_parity": False,
            "records_compared": runner.EXPECTED_TOTAL_DOCS,
            "candidate_total_docs": runner.EXPECTED_TOTAL_DOCS,
            "native_total_docs": runner.EXPECTED_TOTAL_DOCS,
            "missing_in_candidate_count": 0,
            "extra_in_candidate_count": 0,
            "candidate_marker_leaks_count": 0,
            "embedded_json_mismatches_count": 2,
            "embedded_json_mismatches_sample": [
                "/agents/corporate_entities/1",
                "/repositories/2",
            ],
            "records_with_exact_equality": runner.EXPECTED_EXACT_RECORDS,
            "records_with_differences": 2,
            "total_differences": 2,
            "sample_per_record_diffs": {
                "/agents/corporate_entities/1": [
                    {
                        "path": "json",
                        "category": "stored_value_mismatch",
                    }
                ],
                "/repositories/2": [
                    {
                        "path": "json",
                        "category": "stored_value_mismatch",
                    }
                ],
            },
        },
    }


def make_valid_state() -> dict:
    return {
        "stack_id": "844ef09c",
        "containers": {
            "db": "archivalist-profiling-db-844ef09c",
            "solr": "archivalist-profiling-solr-844ef09c",
            "backend": "archivalist-profiling-backend-844ef09c",
        },
        "backend_host_url": "http://127.0.0.1:24567",
        "backend_network_url": "http://archivalist-profiling-backend-844ef09c:4567",
        "solr_host_url": "http://127.0.0.1:28983/solr",
        "solr_network_url": "http://archivalist-profiling-solr-844ef09c:8983/solr",
        "identity_core": "archivalist_profiling_identity_844ef09c",
        "identity_repo_id": 2,
        "identity_repo_code": "ARCHPROF-844ef09c",
    }


class FakeSolrAndBackendEnv:
    def __init__(self) -> None:
        self.swapped = False
        self.swap_calls: list[tuple[str, str]] = []
        self.round_mutated_default_core = False
        self.corrupt_post_swap_state = False

    def docker_inspect(self, name: str) -> dict:
        role = name.split("-")[2]
        return {
            "State": {"Running": True, "Status": "running"},
            "Config": {
                "Labels": {
                    "archivalist.profiling-stack": "844ef09c",
                    "archivalist.profiling-role": role,
                }
            },
        }

    def swap(self, solr_url: str, c1: str, c2: str) -> dict:
        self.swap_calls.append((c1, c2))
        if not self.corrupt_post_swap_state:
            self.swapped = not self.swapped
        return {"responseHeader": {"status": 0, "QTime": 1}}

    def http_get_json(self, url: str) -> dict:
        if url.endswith("/repositories/2"):
            return {
                "uri": "/repositories/2",
                "repo_code": "ARCHPROF-844ef09c",
                "name": f"archivalist profiling identity {runner.EXPECTED_CANDIDATE_NONCE}",
                "system_mtime": "2026-11-18T17:56:43Z",
            }
        if "id%3A%22%2Frepositories%2F2%22" in url:
            nonce = (
                runner.EXPECTED_CANDIDATE_NONCE
                if runner.EXPECTED_CANDIDATE_CORE in url
                else runner.EXPECTED_NATIVE_NONCE
            )
            return {
                "response": {
                    "numFound": 1,
                    "docs": [
                        {
                            "id": "/repositories/2",
                            "title": "ARCHPROF-844ef09c",
                            "json": json.dumps({"uri": "/repositories/2", "name": f"id {nonce}"}),
                        }
                    ],
                }
            }
        if "id%3A%22%2Fagents%2Fcorporate_entities%2F1%22" in url:
            nonce = (
                runner.EXPECTED_CANDIDATE_NONCE
                if runner.EXPECTED_CANDIDATE_CORE in url
                else runner.EXPECTED_NATIVE_NONCE
            )
            return {
                "response": {
                    "numFound": 1,
                    "docs": [
                        {
                            "id": "/agents/corporate_entities/1",
                            "title": "ARCHPROF-844ef09c",
                            "json": json.dumps({"uri": "/agents/corporate_entities/1", "title": f"id {nonce}"}),
                        }
                    ],
                }
            }
        if "action=STATUS" in url:
            default_docs = runner.EXPECTED_TOTAL_DOCS if self.swapped else 0
            if self.swapped and self.round_mutated_default_core:
                default_docs += 1
            cand_docs = 0 if self.swapped else runner.EXPECTED_TOTAL_DOCS
            default_dir = (
                f"/var/solr/data/{runner.EXPECTED_CANDIDATE_CORE}"
                if self.swapped
                else "/var/solr/data/archivesspace"
            )
            cand_dir = (
                "/var/solr/data/archivesspace"
                if self.swapped
                else f"/var/solr/data/{runner.EXPECTED_CANDIDATE_CORE}"
            )
            default_ver = 6600 if self.swapped else 2
            cand_ver = 2 if self.swapped else 6600
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
                            "userData": {"commitTimeMSec": "1795506740019"} if default_docs else {},
                        },
                    },
                    runner.EXPECTED_IDENTITY_CORE: {
                        "name": runner.EXPECTED_IDENTITY_CORE,
                        "instanceDir": f"/var/solr/data/{runner.EXPECTED_IDENTITY_CORE}",
                        "dataDir": f"/var/solr/data/{runner.EXPECTED_IDENTITY_CORE}/data/",
                        "index": {"numDocs": 0, "maxDoc": 0, "deletedDocs": 0, "version": 2, "segmentCount": 0, "userData": {}},
                    },
                    runner.EXPECTED_EMPTY_CANDIDATE_CORE: {
                        "name": runner.EXPECTED_EMPTY_CANDIDATE_CORE,
                        "instanceDir": f"/var/solr/data/{runner.EXPECTED_EMPTY_CANDIDATE_CORE}",
                        "dataDir": f"/var/solr/data/{runner.EXPECTED_EMPTY_CANDIDATE_CORE}/data/",
                        "index": {"numDocs": 0, "maxDoc": 0, "deletedDocs": 0, "version": 2, "segmentCount": 0, "userData": {}},
                    },
                    runner.EXPECTED_PREVIOUS_CANDIDATE_CORE: {
                        "name": runner.EXPECTED_PREVIOUS_CANDIDATE_CORE,
                        "instanceDir": f"/var/solr/data/{runner.EXPECTED_PREVIOUS_CANDIDATE_CORE}",
                        "dataDir": f"/var/solr/data/{runner.EXPECTED_PREVIOUS_CANDIDATE_CORE}/data/",
                        "index": {
                            "numDocs": runner.EXPECTED_TOTAL_DOCS,
                            "maxDoc": runner.EXPECTED_TOTAL_DOCS,
                            "deletedDocs": 0,
                            "version": 6546,
                            "segmentCount": 24,
                            "userData": {"commitTimeMSec": "1795482353266"},
                        },
                    },
                    runner.EXPECTED_NATIVE_CORE: {
                        "name": runner.EXPECTED_NATIVE_CORE,
                        "instanceDir": f"/var/solr/data/{runner.EXPECTED_NATIVE_CORE}",
                        "dataDir": f"/var/solr/data/{runner.EXPECTED_NATIVE_CORE}/data/",
                        "index": {
                            "numDocs": runner.EXPECTED_TOTAL_DOCS,
                            "maxDoc": runner.EXPECTED_TOTAL_DOCS,
                            "deletedDocs": 0,
                            "version": 8848,
                            "segmentCount": 18,
                            "userData": {"commitTimeMSec": "1795493721386"},
                        },
                    },
                    runner.EXPECTED_CANDIDATE_CORE: {
                        "name": runner.EXPECTED_CANDIDATE_CORE,
                        "instanceDir": cand_dir,
                        "dataDir": f"{cand_dir}/data/",
                        "index": {
                            "numDocs": cand_docs,
                            "maxDoc": cand_docs,
                            "deletedDocs": 0,
                            "version": cand_ver,
                            "segmentCount": 25 if cand_docs else 0,
                            "userData": {"commitTimeMSec": "1795506740019"} if cand_docs else {},
                        },
                    },
                },
            }
        if "/select?q=*:*&rows=0&wt=json" in url:
            if "/archivesspace/select" in url:
                cnt = runner.EXPECTED_TOTAL_DOCS if self.swapped else 0
                if self.swapped and self.round_mutated_default_core:
                    cnt += 1
                return {"response": {"numFound": cnt}}
            if f"/{runner.EXPECTED_CANDIDATE_CORE}/select" in url:
                cnt = 0 if self.swapped else runner.EXPECTED_TOTAL_DOCS
                return {"response": {"numFound": cnt}}
            if f"/{runner.EXPECTED_IDENTITY_CORE}/select" in url or f"/{runner.EXPECTED_EMPTY_CANDIDATE_CORE}/select" in url:
                return {"response": {"numFound": 0}}
            return {"response": {"numFound": runner.EXPECTED_TOTAL_DOCS}}
        raise AssertionError(f"Unhandled URL in test fake: {url}")


class TestGate4NoChangeHandoff(unittest.TestCase):
    def test_validate_candidate_report_accepts_exact_nonce_exception_report(self) -> None:
        report = make_valid_report()
        res = runner.validate_candidate_report(report, report_path="test.json")
        self.assertEqual(res["stack_id"], "844ef09c")
        self.assertEqual(res["full_reindex_started_at_epoch_sec"], 1795499941)
        self.assertEqual(res["modified_since_with_overlap"], 1795499911)
        self.assertIs(res["exact_parity"], False)

    def test_validate_candidate_report_refuses_invalid_fields(self) -> None:
        mutations = [
            lambda r: r.update(stack_id="e62875f1"),
            lambda r: r.update(candidate_core="stage_staff_wrong_candidate"),
            lambda r: r["cli_summary"].update(committed=False),
            lambda r: r["cli_summary"].update(full_reindex_started_at_epoch_sec="1795499941"),
            lambda r: r["cli_summary"].update(full_reindex_started_at_epoch_sec=0),
            lambda r: r["cli_summary"].update(total_documents=1036426),
            lambda r: r["parity_result"].update(missing_in_candidate_count=1),
            lambda r: r["parity_result"].update(extra_in_candidate_count=1),
            lambda r: r["parity_result"].update(candidate_marker_leaks_count=1),
            lambda r: r["parity_result"].update(exact_parity=True),
            lambda r: r["parity_result"].update(records_with_differences=3),
            lambda r: r["parity_result"]["sample_per_record_diffs"].update(
                {"/repositories/4/archival_objects/1": [{"path": "json", "category": "stored_value_mismatch"}]}
            ),
        ]
        for mutate in mutations:
            rep = make_valid_report()
            mutate(rep)
            with self.assertRaises(runner.Gate4HandoffError):
                runner.validate_candidate_report(rep)

    def test_check_no_active_processes_refuses_conflicts(self) -> None:
        with self.assertRaises(runner.Gate4HandoffError):
            runner.check_no_active_benchmark_or_indexer_processes(
                host_ps_output="99999 aspace-cli /path/to/aspace-cli staff-full-candidate\n",
                backend_ps_output="",
                app_ps_output="",
                current_pid=100,
                parent_pid=99,
            )
        with self.assertRaises(runner.Gate4HandoffError):
            runner.check_no_active_benchmark_or_indexer_processes(
                host_ps_output="",
                backend_ps_output="root 42 1.0 1.0 100 100 ? Ssl 12:00 0:01 java -Daspace.service=indexer indexer/app/main.rb\n",
                app_ps_output="",
                current_pid=100,
                parent_pid=99,
            )

    def test_preflight_failure_stops_before_core_swap(self) -> None:
        env = FakeSolrAndBackendEnv()
        with self.assertRaises(runner.Gate4HandoffError):
            runner.execute_handoff_orchestration(
                state=make_valid_state(),
                report=make_valid_report(),
                report_path="report.json",
                state_dir_name="indexer_state_gate4_handoff_20261124_deadbeef",
                docker_inspect_fn=env.docker_inspect,
                http_get_json=env.http_get_json,
                process_snapshots_fn=lambda: ("", "", ""),
                run_jruby_preflight_fn=lambda ms: {"status": "ok", "nonempty_feeds_count": 1},
                run_jruby_round_fn=lambda *a: {"status": "ok"},
                swap_fn=env.swap,
            )
        self.assertEqual(env.swap_calls, [])
        self.assertFalse(env.swapped)

    def test_successful_round_swaps_and_rolls_back_cores(self) -> None:
        env = FakeSolrAndBackendEnv()
        res = runner.execute_handoff_orchestration(
            state=make_valid_state(),
            report=make_valid_report(),
            report_path="report.json",
            state_dir_name="indexer_state_gate4_handoff_20261124_deadbeef",
            docker_inspect_fn=env.docker_inspect,
            http_get_json=env.http_get_json,
            process_snapshots_fn=lambda: ("", "", ""),
            run_jruby_preflight_fn=lambda ms: {"status": "ok", "nonempty_feeds_count": 0},
            run_jruby_round_fn=lambda dir_name, epoch, promoted, timeout_s: {
                "status": "ok",
                "state_dir_name": dir_name,
                "epoch": epoch,
                "promoted": promoted,
            },
            swap_fn=env.swap,
        )
        self.assertEqual(len(env.swap_calls), 2)
        self.assertFalse(env.swapped)
        self.assertTrue(res["swap_executed"])
        self.assertTrue(res["rollback_executed"])
        self.assertEqual(res["core_snapshots"]["before_swap"]["archivesspace"]["numFound"], 0)
        self.assertEqual(
            res["core_snapshots"]["after_swap"]["archivesspace"]["numFound"],
            runner.EXPECTED_TOTAL_DOCS,
        )
        self.assertEqual(res["core_snapshots"]["after_rollback"]["archivesspace"]["numFound"], 0)
        self.assertEqual(
            res["core_snapshots"]["after_rollback"][runner.EXPECTED_CANDIDATE_CORE]["numFound"],
            runner.EXPECTED_TOTAL_DOCS,
        )

    def test_round_exception_still_rolls_back_verified_swap(self) -> None:
        env = FakeSolrAndBackendEnv()

        def failing_round(*_args):
            raise runner.Gate4HandoffError("Simulated round failure before Solr write")

        with self.assertRaises(runner.Gate4HandoffError):
            runner.execute_handoff_orchestration(
                state=make_valid_state(),
                report=make_valid_report(),
                report_path="report.json",
                state_dir_name="indexer_state_gate4_handoff_20261124_deadbeef",
                docker_inspect_fn=env.docker_inspect,
                http_get_json=env.http_get_json,
                process_snapshots_fn=lambda: ("", "", ""),
                run_jruby_preflight_fn=lambda ms: {"status": "ok", "nonempty_feeds_count": 0},
                run_jruby_round_fn=failing_round,
                swap_fn=env.swap,
            )
        self.assertEqual(len(env.swap_calls), 2)
        self.assertFalse(env.swapped)

    def test_ambiguous_post_swap_state_stops_without_second_swap(self) -> None:
        env = FakeSolrAndBackendEnv()
        env.corrupt_post_swap_state = True
        with self.assertRaises(runner.Gate4HandoffError):
            runner.execute_handoff_orchestration(
                state=make_valid_state(),
                report=make_valid_report(),
                report_path="report.json",
                state_dir_name="indexer_state_gate4_handoff_20261124_deadbeef",
                docker_inspect_fn=env.docker_inspect,
                http_get_json=env.http_get_json,
                process_snapshots_fn=lambda: ("", "", ""),
                run_jruby_preflight_fn=lambda ms: {"status": "ok", "nonempty_feeds_count": 0},
                run_jruby_round_fn=lambda *a: {"status": "ok"},
                swap_fn=env.swap,
            )
        # Must NOT issue a second swap when post-swap state is ambiguous
        self.assertEqual(len(env.swap_calls), 1)

    def test_write_mode_0600_json_redacts_secrets_and_sets_permissions(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            out_path = os.path.join(tmpdir, "evidence.json")
            runner.write_mode_0600_json(
                out_path,
                {
                    "cmd": "curl http://127.0.0.1:24567/login?password=supersecret",
                    "db_root_password": "secretdbpass",
                },
            )
            mode = os.stat(out_path).st_mode & 0o777
            self.assertEqual(mode, 0o600)
            with open(out_path, encoding="utf-8") as fh:
                content = fh.read()
            self.assertNotIn("supersecret", content)
            self.assertNotIn("secretdbpass", content)

    def test_ruby_helper_offline_checkpoints_and_no_change_guard(self) -> None:
        import subprocess

        ruby_code = r"""
        require 'tmpdir'
        require_relative 'rust-tools/scripts/gate4_isolated_handoff_round'

        Dir.mktmpdir('gate4-ruby-guard-') do |dir|
          Object.const_set(:AppConfig, {}) unless defined?(AppConfig)
          AppConfig[:data_directory] = dir
          state_name = 'indexer_state_gate4_handoff_20261124_cafebabe'
          state = IndexState.new(state_name)
          summary = {
            'committed' => true,
            'full_reindex_started_at_epoch_sec' => 1795499941,
            'repository_ids' => [2, 3, 4]
          }
          StaffFullHandoffSpike.apply(summary, state: state, promoted: true)
          res = Gate4IsolatedHandoffRound.verify_state_checkpoints!(
            state,
            state_dir_path: File.join(dir, state_name),
            repo_ids: [2, 3, 4],
            expected_epoch: 1795499941
          )
          raise 'expected 59 files' unless res['file_count'] == 59

          fake_indexer_cls = Class.new do
            def index_records(records, *args); :indexed; end
            def index_batch(batch, *args); :batched; end
            def delete_records(records, *args); :deleted_ok; end
            def send_commit(*args); :committed; end
            def do_http_request(url, req); :http_ok; end
          end
          idx = fake_indexer_cls.new
          Gate4IsolatedHandoffRound.install_no_change_guard!(idx)

          raise 'empty delete_records should succeed' unless idx.delete_records([]) == :deleted_ok

          begin
            idx.index_records([{'uri' => '/repositories/2/resources/1'}])
            raise 'non-empty index_records did not abort'
          rescue Gate4IsolatedHandoffRound::Gate4NoChangeViolationError
          end

          begin
            idx.delete_records(['/repositories/2/resources/1'])
            raise 'non-empty delete_records did not abort'
          rescue Gate4IsolatedHandoffRound::Gate4NoChangeViolationError
          end

          begin
            idx.send_commit
            raise 'send_commit did not abort'
          rescue Gate4IsolatedHandoffRound::Gate4NoChangeViolationError
          end

          req = Struct.new(:method, :path).new('POST', '/solr/archivesspace/update')
          begin
            idx.do_http_request('http://archivalist-profiling-solr-844ef09c:8983/solr/archivesspace', req)
            raise 'Solr update HTTP request did not abort'
          rescue Gate4IsolatedHandoffRound::Gate4NoChangeViolationError
          end
        end
        puts 'RUBY_HELPER_GUARD_OK'
        """
        proc = subprocess.run(
            ["ruby", "-e", ruby_code],
            cwd=runner.REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("RUBY_HELPER_GUARD_OK", proc.stdout)


if __name__ == "__main__":
    unittest.main()
