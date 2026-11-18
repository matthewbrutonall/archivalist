import contextlib
import hashlib
import importlib.util
import io
import json
import os
import stat
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch


def load_script():
    path = os.path.join(os.path.dirname(__file__), "seed_million_from_snapshot.py")
    spec = importlib.util.spec_from_file_location("seed_million_from_snapshot", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


seed = load_script()


class MillionSeedTest(unittest.TestCase):
    def test_plan_bounds_one_million_in_small_atomic_batches(self):
        result = seed.plan(25_420, 1_000_000)
        self.assertEqual(result["archival_objects_per_collection"], 2010)
        self.assertEqual(result["collections_to_create"], 485)
        self.assertEqual(result["planned_final_ao_count"], 1_000_270)
        self.assertLess(result["sample_batch_records"], seed.MAX_BATCH_RECORDS)
        self.assertLess(result["sample_batch_bytes"], seed.MAX_BATCH_BYTES)

    def test_fictional_trees_keep_equal_counts_but_vary_shape_and_publication(self):
        self.assertEqual({seed.items_total(cfg) for cfg in seed.FICTIONAL_TREES}, {2010})
        subjects = [f"/subjects/{i}" for i in range(1, 151)]
        agents = [f"/agents/people/{i}" for i in range(1, 151)]
        batches = [seed.fictional_batch(i, "/repositories/9", 20261118, subjects, agents)
                   for i in range(4)]
        for batch in batches:
            self.assertEqual(sum(row["jsonmodel_type"] == "archival_object" for row in batch), 2010)
            self.assertTrue(any(row.get("publish") is False for row in batch))
            self.assertTrue(any(row.get("level") == "item" and row.get("dates") for row in batch))
        self.assertEqual([sum(row.get("level") == "series" for row in batch) for batch in batches],
                         [10, 10, 10, 30])

    def test_fictional_bootstrap_summary_must_match_real_seed_counts(self):
        directory = tempfile.mkdtemp(prefix="fictional-seed-summary-")
        path = os.path.join(directory, "summary.json")
        config = dict(seed.TREE)
        summary = {"repo_code": "SEED-123456", "repo_id": 8, "repository_uri": "/repositories/8",
                   "config": config, "totals": {"archival_objects": 2010},
                   "subjects": 150, "agents": 150}
        with open(path, "w") as out:
            json.dump(summary, out)
        self.assertEqual(seed.load_fictional_bootstrap(path)["baseline_ao_count"], 2010)
        summary["totals"]["archival_objects"] = 2009
        with open(path, "w") as out:
            json.dump(summary, out)
        with self.assertRaisesRegex(ValueError, "AO count mismatch"):
            seed.load_fictional_bootstrap(path)

    def test_plan_only_does_not_access_stack_or_database(self):
        output = io.StringIO()
        with patch.object(seed, "load_profiling_stack", side_effect=AssertionError("network used")):
            with contextlib.redirect_stdout(output):
                self.assertEqual(seed.main(["--plan-only", "--existing-ao-count", "25420"]), 0)
        self.assertEqual(json.loads(output.getvalue())["planned_final_ao_count"], 1_000_270)

    def test_inflight_reconciliation_never_replays_an_uncertain_batch(self):
        progress = {"baseline_ao_count": 100, "next_collection": 2, "inflight_collection": 2}
        self.assertEqual(seed.reconcile_inflight(progress, 100 + 2 * 2010, 2010), "retry")
        self.assertEqual(progress["next_collection"], 2)
        self.assertEqual(seed.reconcile_inflight(progress, 100 + 3 * 2010, 2010), "committed")
        self.assertEqual(progress["next_collection"], 3)
        self.assertIsNone(progress["inflight_collection"])
        with self.assertRaisesRegex(ValueError, "changed outside"):
            seed.reconcile_inflight(progress, 100 + 4 * 2010, 2010)

    def test_snapshot_manifest_requires_matching_source_and_permission(self):
        directory = tempfile.mkdtemp(prefix="million-seed-test-")
        source_path = os.path.join(directory, "snapshot.sql")
        with open(source_path, "wb") as source:
            source.write(b"test snapshot bytes")
        os.chmod(source_path, 0o600)
        data = {"stack_id": "test1234", "db_volume": "test-db",
                "source_path": source_path,
                "source_sha256": hashlib.sha256(b"test snapshot bytes").hexdigest(),
                "permission_reference": "owner approved local benchmark copy",
                "baseline_ao_count": 25_420}
        manifest_path = os.path.join(directory, "manifest.json")
        with open(manifest_path, "w") as out:
            json.dump(data, out)
        os.chmod(manifest_path, 0o600)
        state = {"stack_id": "test1234", "volumes": {"db": "test-db"}}
        self.assertEqual(seed.load_snapshot_manifest(manifest_path, state), data)
        with self.assertRaisesRegex(ValueError, "another stack"):
            seed.load_snapshot_manifest(manifest_path, {**state, "stack_id": "wrong"})
        data["permission_reference"] = ""
        with open(manifest_path, "w") as out:
            json.dump(data, out)
        with self.assertRaisesRegex(ValueError, "permission"):
            seed.load_snapshot_manifest(manifest_path, state)
        data["permission_reference"] = "owner approved local benchmark copy"
        data["source_sha256"] = "0" * 64
        with open(manifest_path, "w") as out:
            json.dump(data, out)
        with self.assertRaisesRegex(ValueError, "mismatch"):
            seed.load_snapshot_manifest(manifest_path, state)

    def test_seed_repository_code_cannot_target_a_real_repository(self):
        class Backend:
            def call(self, *_args):
                raise AssertionError("backend called")
        with self.assertRaisesRegex(ValueError, "invalid seed repository"):
            seed.find_repository(Backend(), "REAL-CLIENT")

    def test_checkpoint_is_private_and_atomic(self):
        directory = tempfile.mkdtemp(prefix="million-seed-checkpoint-")
        path = os.path.join(directory, "progress.json")
        seed.write_new_checkpoint(path, {"next_collection": 0})
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o600)
        with self.assertRaises(FileExistsError):
            seed.write_new_checkpoint(path, {"next_collection": 999})
        seed.save_checkpoint(path, {"next_collection": 1})
        with open(path) as source:
            self.assertEqual(json.load(source)["next_collection"], 1)

    def test_two_phase_seed_and_uncertain_commit_resume_without_duplicate_post(self):
        directory = tempfile.mkdtemp(prefix="million-seed-flow-")
        checkpoint = os.path.join(directory, "progress.json")
        state = {"stack_id": "test1234", "volumes": {"db": "test-db"},
                 "backend_host_url": "http://isolated", "solr_host_url": "http://isolated-solr",
                 "admin_user": "admin", "admin_password": "test"}
        manifest = {"baseline_ao_count": 0, "source_sha256": "a" * 64}

        class Stack:
            def load_state(self, _path):
                return state

            def verify_identity(self, *_args):
                return None

        class Backend:
            count = 0
            batches = 0
            repo = None

            def __init__(self, _url):
                pass

            def login(self, *_args):
                return None

            def call(self, method, path, data=None, params=None, timeout=None):
                if (method, path) == ("GET", "/repositories"):
                    return [self.repo] if self.repo else []
                if (method, path) == ("POST", "/repositories"):
                    self.repo = {"uri": "/repositories/9", "repo_code": data["repo_code"]}
                    return {"id": 9}
                if (method, path) == ("GET", "/subjects"):
                    return list(range(1, 151))
                if (method, path) == ("GET", "/agents/people"):
                    return list(range(1, 151))
                if method == "POST" and path.endswith("/batch_imports"):
                    self.batches += 1
                    self.count += sum(row["jsonmodel_type"] == "archival_object" for row in data)
                    return [{"saved": {}}]
                raise AssertionError((method, path))

        backend = Backend("http://isolated")
        args = ["--stack-state", "unused", "--snapshot-manifest", "unused",
                "--checkpoint", checkpoint, "--target-ao-count", "3000",
                "--max-collections-this-run", "1"]
        with patch.object(seed, "load_profiling_stack", return_value=Stack()), \
             patch.object(seed, "load_snapshot_manifest", return_value=manifest), \
             patch.object(seed, "Backend", return_value=backend), \
             patch.object(seed, "ao_count", side_effect=lambda *_: backend.count), \
             patch.object(seed.shutil, "disk_usage", return_value=SimpleNamespace(free=100 * 1024**3)), \
             contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(seed.main(args), 0)
            self.assertEqual((backend.batches, backend.count), (1, 2010))
            with open(checkpoint) as source:
                progress = json.load(source)
            self.assertEqual(progress["next_collection"], 1)

            progress["inflight_collection"] = 1
            seed.save_checkpoint(checkpoint, progress)
            backend.count += 2010
            self.assertEqual(seed.main(args + ["--resume"]), 0)
            self.assertEqual((backend.batches, backend.count), (1, 4020))
            with open(checkpoint) as source:
                final = json.load(source)
            self.assertEqual(final["next_collection"], 2)
            self.assertIsNone(final["inflight_collection"])


if __name__ == "__main__":
    unittest.main()
