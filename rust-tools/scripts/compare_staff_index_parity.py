#!/usr/bin/env python3
"""Staff index parity comparator for ArchivesSpace / Archivalist.

Safely compares stored Solr documents between a native baseline index and a
candidate (Rust-enriched) index for an explicitly scoped repository.

This tool is strictly read-only and deterministic. It performs no writes, triggers
no reindexes, alters no configuration, and stops/starts no services.
"""

from __future__ import annotations

import argparse
import base64
from collections import Counter
import json
import math
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, List, Optional, Set, Tuple

EXIT_PARITY_SUCCESS = 0
EXIT_PARITY_MISMATCH = 1
EXIT_CONFIG_ERROR = 2
EXIT_TRANSPORT_ERROR = 3

DEFAULT_IGNORED_FIELDS: Set[str] = {"_version_", "score"}
MARKER_FIELD = "_archivalist_rust_fields"


class ConfigurationError(Exception):
    """Raised when command-line arguments or environment are invalid."""


class TransportError(Exception):
    """Raised when network, HTTP, or Solr pagination integrity fails."""


def redact_url(url: str) -> str:
    """Redact userinfo credentials and sensitive query parameters from a URL."""
    if not url:
        return ""
    try:
        parsed = urllib.parse.urlsplit(url)
    except Exception:
        return "<invalid-url>"

    netloc = parsed.netloc
    if "@" in netloc:
        userinfo, hostport = netloc.rsplit("@", 1)
        if ":" in userinfo:
            user, _ = userinfo.split(":", 1)
            redacted_userinfo = f"{user}:***"
        else:
            redacted_userinfo = "***"
        netloc = f"{redacted_userinfo}@{hostport}"

    query = parsed.query
    if query:
        pairs = urllib.parse.parse_qsl(query, keep_blank_values=True)
        redacted_pairs = []
        sensitive_keys = {
            "password", "pass", "pwd", "token", "secret", "session",
            "api_key", "key", "auth", "credential", "credentials",
        }
        for k, v in pairs:
            if k.lower() in sensitive_keys:
                redacted_pairs.append((k, "***"))
            else:
                redacted_pairs.append((k, v))
        query = urllib.parse.urlencode(redacted_pairs, safe="*")

    return urllib.parse.urlunsplit((parsed.scheme, netloc, parsed.path, query, parsed.fragment))


def normalize_collection_url(raw_url: str) -> str:
    """Normalize a Solr collection URL for safe comparison.

    Strips trailing slashes, downcases scheme and host, normalizes standard ports,
    collapses duplicate slashes, and ignores user credentials and query strings.
    """
    cleaned = raw_url.strip()
    if not cleaned:
        raise ConfigurationError("Collection URL cannot be empty")

    parsed = urllib.parse.urlsplit(cleaned)
    if not parsed.scheme or not parsed.netloc:
        raise ConfigurationError(f"Collection URL missing scheme or host: {redact_url(cleaned)}")

    scheme = parsed.scheme.lower()
    if scheme not in ("http", "https"):
        raise ConfigurationError(f"Unsupported URL scheme '{scheme}': {redact_url(cleaned)}")

    hostname = (parsed.hostname or "").lower()
    if not hostname:
        raise ConfigurationError(f"Collection URL missing valid hostname: {redact_url(cleaned)}")

    port = parsed.port
    if (scheme == "http" and port == 80) or (scheme == "https" and port == 443) or port is None:
        netloc = hostname
    else:
        netloc = f"{hostname}:{port}"

    path_segments = [seg for seg in parsed.path.split("/") if seg]
    path = "/" + "/".join(path_segments) if path_segments else ""

    return urllib.parse.urlunsplit((scheme, netloc, path, "", ""))


def validate_distinct_collection_urls(baseline_url: str, candidate_url: str) -> Tuple[str, str]:
    """Validate and normalize baseline and candidate URLs, rejecting identical endpoints."""
    norm_base = normalize_collection_url(baseline_url)
    norm_cand = normalize_collection_url(candidate_url)
    if norm_base == norm_cand:
        raise ConfigurationError(
            f"Baseline and candidate URLs resolve to the same normalized Solr endpoint: "
            f"{redact_url(norm_base)}. The comparator cannot compare a collection to itself."
        )
    return norm_base, norm_cand


def resolve_repository_scope(repo_input: str) -> str:
    """Resolve repository input to an exact repository field value."""
    cleaned = repo_input.strip()
    if not cleaned:
        raise ConfigurationError("Repository scope cannot be empty")
    if cleaned.isdigit():
        return f"/repositories/{cleaned}"
    if cleaned.startswith("/repositories/"):
        parts = [p for p in cleaned.split("/") if p]
        if len(parts) == 2 and parts[1].isdigit():
            return f"/repositories/{parts[1]}"
        return cleaned
    return cleaned


def build_repository_filter(scope: str) -> str:
    """Construct an exact Solr filter query for repository scope."""
    escaped = scope.replace("\\", "\\\\").replace('"', '\\"')
    return f'repository:"{escaped}"'


def get_auth_credentials(
    raw_url: str,
    specific_user_env: str,
    specific_pass_env: str,
    generic_user_env: str = "SOLR_USER",
    generic_pass_env: str = "SOLR_PASSWORD",
) -> Optional[Tuple[str, str]]:
    """Retrieve basic-auth credentials from env vars or URL userinfo without logging them."""
    spec_user = os.environ.get(specific_user_env)
    spec_pass = os.environ.get(specific_pass_env)
    if spec_user is not None and spec_pass is not None:
        return (spec_user, spec_pass)

    gen_user = os.environ.get(generic_user_env)
    gen_pass = os.environ.get(generic_pass_env)
    if gen_user is not None and gen_pass is not None:
        return (gen_user, gen_pass)

    parsed = urllib.parse.urlsplit(raw_url)
    if parsed.username is not None and parsed.password is not None:
        return (parsed.username, parsed.password)

    return None


def make_clean_request_url(collection_url: str) -> str:
    """Return a clean collection base URL with userinfo stripped."""
    parsed = urllib.parse.urlsplit(collection_url.rstrip("/"))
    host = parsed.hostname or ""
    if parsed.port:
        host = f"{host}:{parsed.port}"
    return urllib.parse.urlunsplit((parsed.scheme, host, parsed.path, "", ""))


def fetch_solr_documents(
    collection_url: str,
    repository_scope: str,
    auth: Optional[Tuple[str, str]] = None,
    batch_size: int = 1000,
    timeout: float = 30.0,
) -> Tuple[Dict[str, Dict[str, Any]], int, List[str]]:
    """Fetch all matching documents for the scoped repository using cursorMark pagination."""
    if batch_size <= 0:
        raise ConfigurationError(f"batch_size must be positive, got {batch_size}")

    clean_base = make_clean_request_url(collection_url)
    select_url = f"{clean_base}/select"
    fq = build_repository_filter(repository_scope)

    docs_by_id: Dict[str, Dict[str, Any]] = {}
    marker_leaks: List[str] = []
    seen_cursors: Set[str] = set()
    cursor_mark = "*"
    expected_num_found: Optional[int] = None
    page = 0

    headers = {
        "Accept": "application/json",
        "User-Agent": "Archivalist-Staff-Index-Comparator/1.0",
    }
    if auth:
        token = f"{auth[0]}:{auth[1]}"
        encoded = base64.b64encode(token.encode("utf-8")).decode("ascii")
        headers["Authorization"] = f"Basic {encoded}"

    while True:
        if cursor_mark in seen_cursors:
            raise TransportError(
                f"Pagination loop detected in {redact_url(collection_url)}: "
                f"cursorMark '{cursor_mark}' repeated on page {page + 1}"
            )
        seen_cursors.add(cursor_mark)

        params = [
            ("q", "*:*"),
            ("fq", fq),
            ("sort", "id asc"),
            ("fl", "*"),
            ("wt", "json"),
            ("rows", str(batch_size)),
            ("cursorMark", cursor_mark),
        ]
        query_str = urllib.parse.urlencode(params)
        req_url = f"{select_url}?{query_str}"

        request = urllib.request.Request(req_url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            err_msg = ""
            try:
                err_msg = exc.read().decode("utf-8", errors="replace")[:256]
            except Exception:
                pass
            raise TransportError(
                f"Solr HTTP {exc.code} from {redact_url(select_url)}: {err_msg.strip()}"
            ) from exc
        except urllib.error.URLError as exc:
            raise TransportError(
                f"Solr network error connecting to {redact_url(select_url)}: {exc.reason}"
            ) from exc
        except (json.JSONDecodeError, UnicodeDecodeError) as exc:
            raise TransportError(
                f"Invalid JSON returned from Solr endpoint {redact_url(select_url)}: {exc}"
            ) from exc

        if not isinstance(payload, dict) or "response" not in payload:
            raise TransportError(
                f"Malformed Solr response missing 'response' object from {redact_url(select_url)}"
            )

        resp_block = payload["response"]
        if not isinstance(resp_block, dict) or "numFound" not in resp_block:
            raise TransportError(
                f"Malformed Solr response missing 'numFound' from {redact_url(select_url)}"
            )

        num_found = resp_block["numFound"]
        if expected_num_found is None:
            expected_num_found = num_found
        elif num_found != expected_num_found:
            raise TransportError(
                f"numFound changed mid-pagination from {expected_num_found} to {num_found} "
                f"in {redact_url(collection_url)}"
            )

        docs = resp_block.get("docs")
        if not isinstance(docs, list):
            raise TransportError(
                f"Malformed Solr response: 'docs' is not a list in {redact_url(select_url)}"
            )

        for doc in docs:
            if not isinstance(doc, dict):
                raise TransportError(
                    f"Malformed Solr document: not an object in {redact_url(select_url)}"
                )
            doc_id = doc.get("id")
            if doc_id is None:
                raise TransportError(
                    f"Solr document missing required 'id' field in {redact_url(select_url)}"
                )
            if doc_id in docs_by_id:
                raise TransportError(
                    f"Duplicate document ID '{doc_id}' encountered in {redact_url(collection_url)}"
                )
            docs_by_id[doc_id] = doc

            if MARKER_FIELD in doc:
                marker_leaks.append(doc_id)

        next_cursor = payload.get("nextCursorMark")
        if not next_cursor:
            raise TransportError(
                f"Solr response missing 'nextCursorMark' from {redact_url(select_url)}"
            )

        page += 1
        if len(docs_by_id) == expected_num_found or next_cursor == cursor_mark or len(docs) == 0:
            break
        cursor_mark = next_cursor

    total_expected = expected_num_found if expected_num_found is not None else 0
    if len(docs_by_id) != total_expected:
        raise TransportError(
            f"Retrieved document count ({len(docs_by_id)}) does not match Solr numFound "
            f"({total_expected}) in {redact_url(collection_url)}"
        )

    return docs_by_id, total_expected, marker_leaks


def canonical_json_key(val: Any) -> Tuple[Any, ...]:
    """Generate a hashable, type-strict canonical representation of a JSON value.

    Differentiates types strictly (bool vs int vs float) and recursively normalizes
    nested structures (dict keys sorted, lists preserving order).
    """
    if val is None:
        return ("null", None)
    if type(val) is bool:
        return ("bool", val)
    if type(val) is int:
        return ("int", val)
    if type(val) is float:
        if math.isnan(val):
            return ("float", "nan")
        return ("float", val)
    if type(val) is str:
        return ("str", val)
    if isinstance(val, list):
        return ("list", tuple(canonical_json_key(item) for item in val))
    if isinstance(val, dict):
        return ("dict", tuple(sorted((str(k), canonical_json_key(v)) for k, v in val.items())))
    return ("unknown", type(val).__name__, repr(val))


def json_values_strictly_equal(v1: Any, v2: Any) -> bool:
    """Check if two JSON-compatible values are strictly equal in both type and value.

    Guarantees that scalar type differences (e.g. bool vs int, int vs float) are never
    treated as equal, even when Python's loose equality would consider them equal,
    recursing through nested lists and dicts.
    """
    if type(v1) is not type(v2):
        return False
    if isinstance(v1, dict):
        if set(v1.keys()) != set(v2.keys()):
            return False
        return all(json_values_strictly_equal(v1[k], v2[k]) for k in v1)
    if isinstance(v1, list):
        if len(v1) != len(v2):
            return False
        return all(json_values_strictly_equal(x1, x2) for x1, x2 in zip(v1, v2))
    if isinstance(v1, float):
        if math.isnan(v1) and math.isnan(v2):
            return True
    return v1 == v2


def is_array_reordered(list1: List[Any], list2: List[Any]) -> bool:
    """Check if two lists contain identical elements as a multiset but in different order."""
    if len(list1) != len(list2):
        return False
    if json_values_strictly_equal(list1, list2):
        return False
    try:
        keys1 = Counter(canonical_json_key(x) for x in list1)
        keys2 = Counter(canonical_json_key(x) for x in list2)
        return keys1 == keys2
    except Exception:
        return False


def compare_documents(
    baseline_docs: Dict[str, Dict[str, Any]],
    candidate_docs: Dict[str, Dict[str, Any]],
    ignored_fields: Set[str],
    baseline_marker_leaks: List[str],
    candidate_marker_leaks: List[str],
) -> Dict[str, Any]:
    """Perform exhaustive field-by-field parity comparison between two document sets."""
    baseline_ids = set(baseline_docs.keys())
    candidate_ids = set(candidate_docs.keys())

    missing_in_candidate = sorted(baseline_ids - candidate_ids)
    missing_in_baseline = sorted(candidate_ids - baseline_ids)
    common_ids = sorted(baseline_ids & candidate_ids)

    missing_fields_records: List[Dict[str, Any]] = []
    value_diff_records: List[Dict[str, Any]] = []

    missing_fields_count = 0
    value_diffs_count = 0
    array_order_diffs_count = 0

    for doc_id in common_ids:
        b_doc = baseline_docs[doc_id]
        c_doc = candidate_docs[doc_id]

        b_fields = {k for k in b_doc.keys() if k not in ignored_fields}
        c_fields = {k for k in c_doc.keys() if k not in ignored_fields}

        missing_in_cand_fields = sorted(b_fields - c_fields)
        missing_in_base_fields = sorted(c_fields - b_fields)

        if missing_in_cand_fields or missing_in_base_fields:
            missing_fields_records.append({
                "id": doc_id,
                "missing_in_candidate": missing_in_cand_fields,
                "missing_in_baseline": missing_in_base_fields,
            })
            missing_fields_count += len(missing_in_cand_fields) + len(missing_in_base_fields)

        common_fields = sorted(b_fields & c_fields)
        doc_diffs: List[Dict[str, Any]] = []

        for field in common_fields:
            b_val = b_doc[field]
            c_val = c_doc[field]

            if json_values_strictly_equal(b_val, c_val):
                continue

            b_type = type(b_val)
            c_type = type(c_val)

            if b_type is not c_type:
                doc_diffs.append({
                    "field": field,
                    "kind": "type_mismatch",
                    "baseline_type": b_type.__name__,
                    "candidate_type": c_type.__name__,
                    "baseline_value": b_val,
                    "candidate_value": c_val,
                })
                value_diffs_count += 1
            elif isinstance(b_val, list):
                if is_array_reordered(b_val, c_val):
                    doc_diffs.append({
                        "field": field,
                        "kind": "array_order",
                        "baseline_value": b_val,
                        "candidate_value": c_val,
                    })
                    array_order_diffs_count += 1
                else:
                    doc_diffs.append({
                        "field": field,
                        "kind": "value_mismatch",
                        "baseline_value": b_val,
                        "candidate_value": c_val,
                    })
                    value_diffs_count += 1
            else:
                doc_diffs.append({
                    "field": field,
                    "kind": "value_mismatch",
                    "baseline_value": b_val,
                    "candidate_value": c_val,
                })
                value_diffs_count += 1

        if doc_diffs:
            value_diff_records.append({
                "id": doc_id,
                "differences": doc_diffs,
            })

    total_diffs = (
        len(missing_in_candidate)
        + len(missing_in_baseline)
        + missing_fields_count
        + value_diffs_count
        + array_order_diffs_count
        + len(baseline_marker_leaks)
        + len(candidate_marker_leaks)
    )
    parity_passed = (total_diffs == 0)

    return {
        "baseline_count": len(baseline_docs),
        "candidate_count": len(candidate_docs),
        "common_count": len(common_ids),
        "missing_in_candidate": missing_in_candidate,
        "missing_in_baseline": missing_in_baseline,
        "marker_leaks": {
            "baseline": sorted(baseline_marker_leaks),
            "candidate": sorted(candidate_marker_leaks),
        },
        "differences": {
            "missing_fields": missing_fields_records,
            "value_differences": value_diff_records,
        },
        "summary": {
            "missing_in_candidate_count": len(missing_in_candidate),
            "missing_in_baseline_count": len(missing_in_baseline),
            "missing_fields_count": missing_fields_count,
            "value_diffs_count": value_diffs_count,
            "array_order_diffs_count": array_order_diffs_count,
            "baseline_marker_leaks_count": len(baseline_marker_leaks),
            "candidate_marker_leaks_count": len(candidate_marker_leaks),
            "total_differences": total_diffs,
            "parity": parity_passed,
        },
    }


def format_human_report(
    report: Dict[str, Any],
    baseline_url: str,
    candidate_url: str,
    repo_scope: str,
    effective_ignored: List[str],
) -> str:
    """Format comparison results into concise, readable terminal text."""
    lines = []
    lines.append("=== Staff Index Parity Comparison ===")
    lines.append(f"Repository scope:         {repo_scope}")
    lines.append(f"Baseline Solr URL:        {redact_url(baseline_url)}")
    lines.append(f"Candidate Solr URL:       {redact_url(candidate_url)}")
    lines.append(f"Effective ignored fields: {', '.join(effective_ignored) if effective_ignored else '(none)'}")
    lines.append("")
    lines.append(
        f"Document counts: baseline={report['baseline_count']}, "
        f"candidate={report['candidate_count']}, common={report['common_count']}"
    )

    base_leaks = report["marker_leaks"]["baseline"]
    cand_leaks = report["marker_leaks"]["candidate"]
    if base_leaks or cand_leaks:
        lines.append(f"Marker leak check:        FAIL ({MARKER_FIELD} detected)")
        if base_leaks:
            sample = ", ".join(base_leaks[:5])
            lines.append(f"  Baseline leaks ({len(base_leaks)} docs): {sample}{' ...' if len(base_leaks) > 5 else ''}")
        if cand_leaks:
            sample = ", ".join(cand_leaks[:5])
            lines.append(f"  Candidate leaks ({len(cand_leaks)} docs): {sample}{' ...' if len(cand_leaks) > 5 else ''}")
    else:
        lines.append(f"Marker leak check:        PASS (no {MARKER_FIELD} leaked)")

    miss_cand = report["missing_in_candidate"]
    miss_base = report["missing_in_baseline"]
    if miss_cand or miss_base:
        lines.append(
            f"Document ID parity:       FAIL (missing baseline={len(miss_base)}, missing candidate={len(miss_cand)})"
        )
        if miss_cand:
            sample = ", ".join(miss_cand[:5])
            lines.append(f"  Missing in candidate ({len(miss_cand)}): {sample}{' ...' if len(miss_cand) > 5 else ''}")
        if miss_base:
            sample = ", ".join(miss_base[:5])
            lines.append(f"  Missing in baseline ({len(miss_base)}): {sample}{' ...' if len(miss_base) > 5 else ''}")
    else:
        lines.append("Document ID parity:       PASS (exact match)")

    missing_fields_count = report["summary"]["missing_fields_count"]
    if missing_fields_count > 0:
        lines.append(f"Field presence parity:    FAIL ({missing_fields_count} field omissions)")
        for rec in report["differences"]["missing_fields"][:5]:
            if rec["missing_in_candidate"]:
                lines.append(f"  {rec['id']}: missing in candidate: {rec['missing_in_candidate']}")
            if rec["missing_in_baseline"]:
                lines.append(f"  {rec['id']}: missing in baseline: {rec['missing_in_baseline']}")
        if len(report["differences"]["missing_fields"]) > 5:
            lines.append("  ...")
    else:
        lines.append("Field presence parity:    PASS (all fields present on both sides)")

    val_diffs = report["summary"]["value_diffs_count"]
    order_diffs = report["summary"]["array_order_diffs_count"]
    if val_diffs > 0 or order_diffs > 0:
        lines.append(f"Field value parity:       FAIL ({val_diffs} value diffs, {order_diffs} array ordering diffs)")
        count = 0
        for rec in report["differences"]["value_differences"]:
            for d in rec["differences"]:
                if count < 5:
                    if d["kind"] == "array_order":
                        lines.append(f"  {rec['id']}.{d['field']}: array ordering mismatch")
                    elif d["kind"] == "type_mismatch":
                        lines.append(
                            f"  {rec['id']}.{d['field']}: type mismatch ({d['baseline_type']} vs {d['candidate_type']})"
                        )
                    else:
                        lines.append(f"  {rec['id']}.{d['field']}: value mismatch")
                    count += 1
            if count >= 5:
                break
        if val_diffs + order_diffs > 5:
            lines.append("  ...")
    else:
        lines.append("Field value parity:       PASS (exact scalar and array matches)")

    lines.append("")
    if report["summary"]["parity"]:
        lines.append("OVERALL RESULT: PASS (exact parity verified)")
    else:
        lines.append(f"OVERALL RESULT: FAIL ({report['summary']['total_differences']} total parity discrepancies)")

    return "\n".join(lines)


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    """Parse command line arguments."""
    parser = argparse.ArgumentParser(
        description="Safe, read-only Staff Solr parity comparator between native baseline and Rust.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Exit codes:
  0  Exact parity with no marker leakage
  1  Parity failure (discrepancy in counts, IDs, fields, values, ordering, or marker leakage)
  2  Configuration error (invalid parameters, identical collection URLs, invalid scope)
  3  Transport error (network/HTTP failure, Solr response error, pagination loop, duplicate IDs)

Authentication:
  Basic authentication credentials may be supplied via environment variables:
    BASELINE_SOLR_USER, BASELINE_SOLR_PASSWORD
    CANDIDATE_SOLR_USER, CANDIDATE_SOLR_PASSWORD
    SOLR_USER, SOLR_PASSWORD (fallback)
  Credentials must never be passed as CLI arguments.
""",
    )
    parser.add_argument(
        "--baseline-url",
        required=True,
        help="Solr collection URL for native baseline index (e.g. http://solr:8983/solr/native_coll)",
    )
    parser.add_argument(
        "--candidate-url",
        "--rust-url",
        dest="candidate_url",
        required=True,
        help="Solr collection URL for candidate / Rust index (e.g. http://solr:8983/solr/rust_coll)",
    )
    parser.add_argument(
        "--repository",
        "--repo-id",
        "--repo",
        dest="repository",
        required=True,
        help="Explicit repository scope by ID (e.g. 2) or exact URI (e.g. /repositories/2)",
    )
    parser.add_argument(
        "--ignore-field",
        action="append",
        default=[],
        help="Explicitly add a field name to ignore (can be repeated). _version_ and score are ignored by default.",
    )
    parser.add_argument(
        "--ignore-fields",
        help="Comma-separated list of additional fields to ignore.",
    )
    parser.add_argument(
        "--batch-size",
        "--rows",
        type=int,
        default=1000,
        help="Pagination batch size (rows per cursorMark query, default: 1000)",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=30.0,
        help="HTTP timeout in seconds for Solr requests (default: 30.0)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Output deterministic JSON report to stdout instead of human-readable summary",
    )

    return parser.parse_args(argv)


def main(argv: Optional[List[str]] = None) -> int:
    """Main comparator entry point."""
    try:
        raw_args = argv if argv is not None else sys.argv[1:]
        disallowed = [arg for arg in raw_args if arg.startswith(("--user", "--pass", "--auth", "--cred"))]
        if disallowed:
            print(
                f"Configuration error: Basic-auth credentials must be supplied via environment "
                f"variables, never CLI arguments: {', '.join(disallowed)}",
                file=sys.stderr,
            )
            return EXIT_CONFIG_ERROR

        args = parse_args(argv)

        norm_base, norm_cand = validate_distinct_collection_urls(args.baseline_url, args.candidate_url)
        repo_scope = resolve_repository_scope(args.repository)

        ignored = set(DEFAULT_IGNORED_FIELDS)
        for f in args.ignore_field:
            if f:
                ignored.add(f.strip())
        if args.ignore_fields:
            for f in args.ignore_fields.split(","):
                if f.strip():
                    ignored.add(f.strip())
        effective_ignored = sorted(ignored)

        base_auth = get_auth_credentials(args.baseline_url, "BASELINE_SOLR_USER", "BASELINE_SOLR_PASSWORD")
        cand_auth = get_auth_credentials(args.candidate_url, "CANDIDATE_SOLR_USER", "CANDIDATE_SOLR_PASSWORD")

    except ConfigurationError as exc:
        print(f"Configuration error: {exc}", file=sys.stderr)
        return EXIT_CONFIG_ERROR
    except SystemExit as exc:
        return EXIT_CONFIG_ERROR if exc.code != 0 else EXIT_PARITY_SUCCESS
    except Exception as exc:
        print(f"Unexpected configuration error: {exc}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    try:
        base_docs, _, base_leaks = fetch_solr_documents(
            norm_base,
            repo_scope,
            auth=base_auth,
            batch_size=args.batch_size,
            timeout=args.timeout,
        )
        cand_docs, _, cand_leaks = fetch_solr_documents(
            norm_cand,
            repo_scope,
            auth=cand_auth,
            batch_size=args.batch_size,
            timeout=args.timeout,
        )
    except TransportError as exc:
        print(f"Transport/Solr error: {exc}", file=sys.stderr)
        return EXIT_TRANSPORT_ERROR
    except Exception as exc:
        print(f"Unexpected error during document retrieval: {exc}", file=sys.stderr)
        return EXIT_TRANSPORT_ERROR

    report = compare_documents(
        base_docs,
        cand_docs,
        set(effective_ignored),
        base_leaks,
        cand_leaks,
    )

    full_output = {
        "baseline_url": redact_url(norm_base),
        "candidate_url": redact_url(norm_cand),
        "repository": repo_scope,
        "effective_ignored_fields": effective_ignored,
        **report,
    }

    if args.json:
        print(json.dumps(full_output, indent=2, sort_keys=True))
    else:
        print(format_human_report(report, norm_base, norm_cand, repo_scope, effective_ignored))

    return EXIT_PARITY_SUCCESS if report["summary"]["parity"] else EXIT_PARITY_MISMATCH


if __name__ == "__main__":
    sys.exit(main())
