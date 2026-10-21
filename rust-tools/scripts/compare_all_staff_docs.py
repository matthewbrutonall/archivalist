#!/usr/bin/env python3
"""Read-only type-strict equality of ALL docs in three stage STAFF cores (native/candidate/fallback).

Unlike compare_staff_index_parity.py this does not exclude profile docs. Only stage-owned cores
(stage_staff_<RUN_ID>_<role>) on an allowlisted host are read. Usage:
  compare_all_staff_docs.py <solr_base_url> <run_id> [--expect N]
Ignores only _version_ and score. Exits 0 only if the sets, key sets, values and types all match.
"""
import json
import re
import sys
import urllib.parse
import urllib.request

IGNORED = {"_version_", "score"}


def fetch(base, core):
    url = f"{base}/{core}/select?" + urllib.parse.urlencode({"q": "*:*", "rows": 100000, "sort": "id asc", "wt": "json"})
    with urllib.request.urlopen(url, timeout=120) as r:
        d = json.load(r)["response"]
    assert d["numFound"] == len(d["docs"]), "truncated fetch"
    return {doc["id"]: {k: v for k, v in doc.items() if k not in IGNORED} for doc in d["docs"]}


def strict(a, b, path, out):
    if type(a) is not type(b):
        out.append(f"{path}: type {type(a).__name__} vs {type(b).__name__}")
    elif isinstance(a, dict):
        if list(a.keys()) != list(b.keys()) and set(a) == set(b):
            pass  # Solr returns stored fields; key order is not a stored-doc contract
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f"{path}.{k}: missing on one side")
            else:
                strict(a[k], b[k], f"{path}.{k}", out)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append(f"{path}: length {len(a)} vs {len(b)}")
        else:
            for i, (x, y) in enumerate(zip(a, b)):
                strict(x, y, f"{path}[{i}]", out)
    elif a != b:
        out.append(f"{path}: value differs")


def main():
    args = sys.argv[1:]
    base, run_id = args[0].rstrip("/"), args[1]
    expect = int(args[args.index("--expect") + 1]) if "--expect" in args else None
    if not re.fullmatch(r"[0-9A-Za-z_]+", run_id):
        sys.exit("bad run id")
    if not re.fullmatch(r"http://(solr-bench|solr-test|solr|localhost|127\.0\.0\.1|172\.\d+\.\d+\.\d+):\d+/solr", base):
        sys.exit("unapproved solr url")
    docs = {r: fetch(base, f"stage_staff_{run_id}_{r}") for r in ("native", "candidate", "fallback")}
    ok = True
    print({r: len(d) for r, d in docs.items()})
    if expect is not None and any(len(d) != expect for d in docs.values()):
        ok = False
        print(f"FAIL: expected {expect} docs in every core")
    for r in ("candidate", "fallback"):
        diffs = []
        if set(docs["native"]) != set(docs[r]):
            diffs.append("id sets differ")
        for i in sorted(set(docs["native"]) & set(docs[r])):
            strict(docs["native"][i], docs[r][i], i, diffs)
        print(f"native vs {r}: {'PASS' if not diffs else 'FAIL'} ({len(diffs)} diffs)")
        for d in diffs[:10]:
            print("  ", d)
        ok = ok and not diffs
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
