"""Offline tests for profiling_stack.py (docker and HTTP are replaced by fakes).

Run from rust-tools/scripts: python3 -m unittest test_profiling_stack
"""

import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest import mock

import profiling_stack as ps

STATE = {
    "stack_id": "abcd1234",
    "containers": {"db": "archivalist-profiling-db-abcd1234", "solr": "archivalist-profiling-solr-abcd1234",
                   "backend": "archivalist-profiling-backend-abcd1234"},
    "db_volume": "archivalist-profiling-db-abcd1234",
    "db_root_password": "pw",
    "identity_repo_id": 7,
    "identity_repo_code": "ARCHPROF-abcd1234",
    "identity_core": "archivalist_profiling_identity_abcd1234",
    "backend_host_url": "http://127.0.0.1:14567",
    "backend_network_url": "http://archivalist-profiling-backend:4567",
    "solr_host_url": "http://127.0.0.1:18983/solr",
    "solr_network_url": "http://archivalist-profiling-solr:8983/solr",
}


def labelled(role, stack="abcd1234", running=True):
    return {"Config": {"Labels": {ps.LABEL: stack, ps.ROLE_LABEL: role}}, "State": {"Running": running}}


class VerifyIdentityTests(unittest.TestCase):
    def setUp(self):
        self.sql = []
        patches = [
            mock.patch.object(ps, "require_labelled", lambda name, stack_id, role: labelled(role)),
            mock.patch.object(ps, "mysql_exec", self.fake_mysql),
            mock.patch.object(ps.secrets, "token_hex", lambda n=16: "n0nce"),
        ]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)

    def fake_mysql(self, state, sql):
        self.sql.append(sql)
        return "1"

    def http(self, repo, core_ok=True):
        def get(url, timeout=10):
            if "/repositories/" in url:
                return repo
            return {"status": {STATE["identity_core"]: {"name": STATE["identity_core"]} if core_ok else {}}}
        return mock.patch.object(ps, "http_json", get)

    def test_accepts_backend_returning_the_fresh_nonce(self):
        with self.http({"name": ps.IDENTITY_NAME_PREFIX + "n0nce", "repo_code": "ARCHPROF-abcd1234"}):
            self.assertEqual("n0nce", ps.verify_identity(STATE, STATE["backend_host_url"], STATE["solr_host_url"]))
        self.assertIn("WHERE id=7 AND repo_code='ARCHPROF-abcd1234'", self.sql[0])

    def test_rejects_other_database_even_with_colliding_repo_id(self):
        with self.http({"name": "Some other repository", "repo_code": "test_1792756510"}):
            with self.assertRaisesRegex(ps.StackError, "NOT connected"):
                ps.verify_identity(STATE, "http://localhost:4567", STATE["solr_host_url"])

    def test_rejects_wrong_solr(self):
        with self.http({"name": ps.IDENTITY_NAME_PREFIX + "n0nce", "repo_code": "ARCHPROF-abcd1234"}, core_ok=False):
            with self.assertRaisesRegex(ps.StackError, "NOT the profiling stack's Solr"):
                ps.verify_identity(STATE, STATE["backend_host_url"], "http://127.0.0.1:8984/solr")

    def test_rejects_when_identity_record_missing_from_profiling_db(self):
        with mock.patch.object(ps, "mysql_exec", lambda state, sql: "0"):
            with self.assertRaisesRegex(ps.StackError, "identity record not found"):
                ps.verify_identity(STATE, STATE["backend_host_url"], STATE["solr_host_url"])


class TeardownTests(unittest.TestCase):
    def write_state(self):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w") as fh:
            json.dump(STATE, fh)
        self.addCleanup(os.remove, path)
        return path

    def test_refuses_container_with_foreign_labels(self):
        path = self.write_state()
        removed = []
        with mock.patch.object(ps, "inspect", lambda name: labelled("db", stack="someone-else")), \
             mock.patch.object(ps, "docker", lambda *a, **k: removed.append(a)):
            with self.assertRaisesRegex(ps.StackError, "refusing to remove"):
                ps.teardown(mock.Mock(state=path, delete_db_volume=False))
        self.assertEqual([], removed)

    def test_removes_only_labelled_containers_and_keeps_volume_by_default(self):
        path = self.write_state()
        calls = []
        roles = {v: k for k, v in STATE["containers"].items()}
        with mock.patch.object(ps, "inspect", lambda name: labelled(roles[name])), \
             mock.patch.object(ps, "docker", lambda *a, **k: calls.append(a)), redirect_stdout(io.StringIO()) as out:
            ps.teardown(mock.Mock(state=path, delete_db_volume=False))
        self.assertEqual(sorted(("rm", "-f", n) for n in STATE["containers"].values()), sorted(calls))
        self.assertIn("kept database volume", out.getvalue())


if __name__ == "__main__":
    unittest.main()
