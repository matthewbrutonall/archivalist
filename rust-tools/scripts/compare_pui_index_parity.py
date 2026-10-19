#!/usr/bin/env python3
"""Privacy-normalized PUI index parity comparator for ArchivesSpace / Archivalist.

Safely compares stored Solr documents between a native baseline index and a
candidate (Rust PUI final prepared) index for an explicitly scoped repository.

This tool is strictly read-only and deterministic. It performs no writes, triggers
no reindexes, alters no configuration, and stops/starts no services.

HONEST PARITY AND INTENTIONAL PRIVACY HARDENING SPECIFICATION:
Native ArchivesSpace PUI indexing:
1. Leaves document-level root Solr doc['ancestors'] intact.
2. Serializes RecordInheritance.merge(remove_ancestors: true), which retains
   an empty 'ancestors': [] key inside doc['json'].
3. Serializes doc['json'] BEFORE deleting nested top_container 'internal_note'
   and container_profile 'notes', thereby leaking them inside public Solr JSON.

The Rust preparation path intentionally hardens these boundaries beyond native ArchivesSpace:
- Deletes document-level root doc['ancestors'] on the trusted projection path.
- Completely deletes the 'ancestors' key from doc['json'].
- Deletes nested top_container 'internal_note' and container_profile 'notes'
  prior to serialization, ensuring public JSON and fullrecord_published are privacy-clean.

Therefore, this comparator enforces a strict two-step verification rule:
1. Strict candidate privacy scan: The candidate is NEVER normalized to hide leaks.
   If the candidate contains root ancestors, JSON ancestors, internal_note, profile notes,
   unpublished fullrecord, or any _archivalist_* handoff metadata, the check FAILS IMMEDIATELY.
2. Baseline privacy normalization: ONLY the 4 explicitly documented native-private fields
   are normalized from the baseline (root ancestors, JSON ancestors, internal_note, profile notes).
3. Canonical structural JSON equality: Candidate parsed JSON must structurally match
   the privacy-normalized baseline JSON.
4. Exact type-strict parity: All remaining Solr fields, types, values, and array orderings
   must match exactly (excluding only '_version_' and 'score').
"""

from __future__ import annotations

import argparse
import base64
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
FORBIDDEN_PRIMARY_COLLECTION_NAMES: Set[str] = {"archivesspace", "primary", "production", "live"}


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


def validate_safety_and_distinct_collection_urls(baseline_url: str, candidate_url: str, allow_primary: bool = False) -> Tuple[str, str]:
    """Validate URLs, rejecting primary production collections and identical endpoints."""
    norm_base = normalize_collection_url(baseline_url)
    norm_cand = normalize_collection_url(candidate_url)

    if norm_base == norm_cand:
        raise ConfigurationError(
            f"Baseline and candidate URLs resolve to the same normalized Solr endpoint: "
            f"{redact_url(norm_base)}. The comparator cannot compare a collection to itself."
        )

    for label, url in [("baseline", norm_base), ("candidate", norm_cand)]:
        parsed = urllib.parse.urlsplit(url)
        path_segments = [s.lower() for s in parsed.path.split("/") if s]
        if not allow_primary:
            for forbidden in FORBIDDEN_PRIMARY_COLLECTION_NAMES:
                if forbidden in path_segments:
                    raise ConfigurationError(
                        f"Refusing to run parity comparator against protected collection name '{forbidden}' "
                        f"in {label} URL: {redact_url(url)}. Canary procedures must target isolated test cores."
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


def build_pui_repository_filter(scope: str) -> str:
    """Construct an exact Solr filter query for PUI repository records."""
    escaped = scope.replace("\\", "\\\\").replace('"', '\\"')
    return f'repository:"{escaped}" AND types:pui_record'


def get_auth_credentials(
    raw_url: str,
    specific_user_env: str,
    specific_pass_env: str,
    generic_user_env: str = "SOLR_USER",
    generic_pass_env: str = "SOLR_PASSWORD",
) -> Optional[Tuple[str, str]]:
    """Retrieve basic-auth credentials from env vars or URL userinfo without logging them."""
    if specific_user_env in os.environ and specific_pass_env in os.environ:
        return (os.environ[specific_user_env], os.environ[specific_pass_env])

    if generic_user_env in os.environ and generic_pass_env in os.environ:
        return (os.environ[generic_user_env], os.environ[generic_pass_env])

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


def fetch_pui_solr_documents(
    collection_url: str,
    repository_scope: str,
    auth: Optional[Tuple[str, str]] = None,
    batch_size: int = 1000,
    timeout: float = 30.0,
) -> Tuple[Dict[str, Dict[str, Any]], int]:
    """Fetch all matching PUI documents for the scoped repository using cursorMark pagination."""
    if batch_size <= 0:
        raise ConfigurationError(f"batch_size must be positive, got {batch_size}")

    clean_base = make_clean_request_url(collection_url)
    select_url = f"{clean_base}/select"
    fq = build_pui_repository_filter(repository_scope)

    docs_by_id: Dict[str, Dict[str, Any]] = {}
    seen_cursors: Set[str] = set()
    cursor_mark = "*"
    expected_num_found: Optional[int] = None
    page = 0

    while True:
        page += 1
        params = {
            "q": "*:*",
            "fq": fq,
            "sort": "id asc",
            "cursorMark": cursor_mark,
            "rows": str(batch_size),
            "wt": "json",
        }
        query_string = urllib.parse.urlencode(params)
        request_url = f"{select_url}?{query_string}"

        req = urllib.request.Request(request_url)
        if auth:
            user, pwd = auth
            token = base64.b64encode(f"{user}:{pwd}".encode("utf-8")).decode("ascii")
            req.add_header("Authorization", f"Basic {token}")

        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                status_code = resp.status
                body = resp.read()
        except urllib.error.HTTPError as err:
            raise TransportError(f"HTTP {err.code} while fetching {redact_url(request_url)}") from err
        except urllib.error.URLError as err:
            raise TransportError(f"Network error while connecting to {redact_url(clean_base)}: {err.reason}") from err
        except Exception as err:
            raise TransportError(f"Unexpected transport failure while contacting {redact_url(clean_base)}: {err}") from err

        if status_code != 200:
            raise TransportError(f"Unexpected HTTP status {status_code} from {redact_url(select_url)}")

        try:
            data = json.loads(body.decode("utf-8"))
        except Exception as err:
            raise TransportError(f"Malformed JSON response from {redact_url(select_url)}: {err}") from err

        response_obj = data.get("response")
        if not isinstance(response_obj, dict):
            raise TransportError(f"Solr response missing 'response' object from {redact_url(select_url)}")

        num_found = response_obj.get("numFound")
        if not isinstance(num_found, int):
            raise TransportError(f"Solr response missing integer 'numFound' from {redact_url(select_url)}")

        if expected_num_found is None:
            expected_num_found = num_found
        elif expected_num_found != num_found:
            raise TransportError(
                f"Concurrent index modification detected during pagination at {redact_url(clean_base)}: "
                f"numFound changed from {expected_num_found} to {num_found} on page {page}"
            )

        docs = response_obj.get("docs")
        if not isinstance(docs, list):
            raise TransportError(f"Solr response 'docs' is not a list from {redact_url(select_url)}")

        for doc in docs:
            if not isinstance(doc, dict):
                raise TransportError(f"Solr doc entry is not a dict from {redact_url(select_url)}")
            doc_id = doc.get("id")
            if not isinstance(doc_id, str) or not doc_id:
                raise TransportError(f"Solr doc missing string 'id' from {redact_url(select_url)}")
            if doc_id in docs_by_id:
                raise TransportError(f"Duplicate document id '{doc_id}' encountered during pagination from {redact_url(clean_base)}")
            docs_by_id[doc_id] = doc

        next_cursor = data.get("nextCursorMark")
        if not isinstance(next_cursor, str) or not next_cursor:
            raise TransportError(f"Solr response missing 'nextCursorMark' from {redact_url(select_url)}")

        if next_cursor == cursor_mark:
            # Reached end of pagination
            break

        if next_cursor in seen_cursors:
            raise TransportError(f"Cycle detected in cursorMark pagination at {redact_url(clean_base)} (cursor: {next_cursor})")

        seen_cursors.add(cursor_mark)
        cursor_mark = next_cursor

    if expected_num_found is not None and len(docs_by_id) != expected_num_found:
        raise TransportError(
            f"Pagination count mismatch from {redact_url(clean_base)}: retrieved {len(docs_by_id)} documents "
            f"but numFound was {expected_num_found}"
        )

    return docs_by_id, expected_num_found or 0


def detect_pui_privacy_leak(doc: Dict[str, Any]) -> List[str]:
    """Scan document for any privacy leak or forbidden public fields.

    Fails if:
    - 'ancestors' is present in document.
    - 'fullrecord' (unpublished) is present in document.
    - Any '_archivalist_*' field is present in document.
    - Inside parsed 'json':
      * 'ancestors' key is present.
      * 'internal_note' is present on any resolved top_container.
      * 'notes' is present on any resolved container_profile.
      * Any '_archivalist_*' key is present anywhere in the parsed JSON tree.
    """
    leaks: List[str] = []
    doc_id = doc.get("id", "<unknown>")

    if "ancestors" in doc:
        leaks.append(f"document root contains 'ancestors'")

    if "fullrecord" in doc:
        leaks.append(f"document root contains unpublished 'fullrecord'")

    for k in doc.keys():
        if k.startswith("_archivalist_"):
            leaks.append(f"document root contains private handoff field '{k}'")

    raw_json = doc.get("json")
    if isinstance(raw_json, str):
        try:
            parsed = json.loads(raw_json)
            if isinstance(parsed, dict):
                if "ancestors" in parsed:
                    leaks.append(f"parsed 'json' contains 'ancestors'")

                # Check instances
                instances = parsed.get("instances")
                if isinstance(instances, list):
                    for idx, inst in enumerate(instances):
                        if isinstance(inst, dict):
                            sub_c = inst.get("sub_container")
                            if isinstance(sub_c, dict):
                                tc = sub_c.get("top_container")
                                if isinstance(tc, dict):
                                    res_tc = tc.get("_resolved")
                                    if isinstance(res_tc, dict):
                                        if "internal_note" in res_tc:
                                            leaks.append(f"parsed 'json' instances[{idx}] top_container contains 'internal_note'")
                                        cp = res_tc.get("container_profile")
                                        if isinstance(cp, dict):
                                            res_cp = cp.get("_resolved")
                                            if isinstance(res_cp, dict) and "notes" in res_cp:
                                                leaks.append(f"parsed 'json' instances[{idx}] container_profile contains 'notes'")

                # Check for private keys in json tree
                def scan_keys(obj: Any, path: str = "json") -> None:
                    if isinstance(obj, dict):
                        for k, v in obj.items():
                            if str(k).startswith("_archivalist_"):
                                leaks.append(f"{path} contains private key '{k}'")
                            scan_keys(v, f"{path}.{k}")
                    elif isinstance(obj, list):
                        for i, item in enumerate(obj):
                            scan_keys(item, f"{path}[{i}]")

                scan_keys(parsed)
        except Exception:
            leaks.append(f"field 'json' is not valid JSON")

    return leaks


def deep_type_strict_equal(v1: Any, v2: Any, path: str = "") -> Tuple[bool, Optional[str]]:
    """Compare two values strictly by type and recursive value."""
    if type(v1) is not type(v2):
        return False, f"{path}: type mismatch ({type(v1).__name__} vs {type(v2).__name__})"

    if isinstance(v1, dict):
        keys1 = set(v1.keys())
        keys2 = set(v2.keys())
        if keys1 != keys2:
            diff = (keys1 ^ keys2)
            return False, f"{path}: dictionary keys mismatch (diff: {sorted(diff)})"
        for k in sorted(keys1):
            sub_path = f"{path}.{k}" if path else str(k)
            ok, reason = deep_type_strict_equal(v1[k], v2[k], sub_path)
            if not ok:
                return False, reason
        return True, None

    if isinstance(v1, list):
        if len(v1) != len(v2):
            return False, f"{path}: array length mismatch ({len(v1)} vs {len(v2)})"
        for idx, (item1, item2) in enumerate(zip(v1, v2)):
            sub_path = f"{path}[{idx}]"
            ok, reason = deep_type_strict_equal(item1, item2, sub_path)
            if not ok:
                return False, reason
        return True, None

    if isinstance(v1, float) and isinstance(v2, float):
        if math.isnan(v1) and math.isnan(v2):
            return True, None
        if v1 != v2:
            return False, f"{path}: float value mismatch ({v1} vs {v2})"
        return True, None

    if v1 != v2:
        return False, f"{path}: value mismatch ({repr(v1)} vs {repr(v2)})"

    return True, None


def normalize_baseline_pui_document(base_doc: Dict[str, Any]) -> Tuple[Dict[str, Any], Optional[Any], Optional[str]]:
    """Normalize ONLY explicitly documented native-private fields from the baseline.

    Native ArchivesSpace PUI indexing behavior:
    1. Retains root Solr doc['ancestors'].
    2. Retains empty 'ancestors': [] inside doc['json'].
    3. Serializes doc['json'] BEFORE stripping nested top_container 'internal_note'
       and container_profile 'notes'.

    The Rust preparation path intentionally hardens these privacy boundaries by:
    - Removing root doc['ancestors'] on the trusted projection path.
    - Completely removing the 'ancestors' key from doc['json'].
    - Stripping nested top-container 'internal_note' and container-profile 'notes'
      from doc['json'] and fullrecord_published.

    This function normalizes ONLY those 4 explicitly documented native-private fields
    from the baseline so canonical structural equality can be enforced without
    compromising privacy. It returns:
      (normalized_base_doc, parsed_normalized_json, parse_error)
    """
    normalized_doc = dict(base_doc)
    # 1. Root Solr ancestors
    normalized_doc.pop("ancestors", None)

    raw_json = normalized_doc.get("json")
    if not isinstance(raw_json, str):
        return normalized_doc, None, "baseline 'json' field is missing or not a string"

    try:
        parsed_json = json.loads(raw_json)
        if not isinstance(parsed_json, (dict, list)):
            return normalized_doc, None, "baseline 'json' must parse to a JSON object or array"
    except Exception as err:
        return normalized_doc, None, f"baseline 'json' parse error: {err}"

    if isinstance(parsed_json, dict):
        # 2. Parsed JSON ancestors
        parsed_json.pop("ancestors", None)

        # 3. Parsed JSON top_container internal_note and 4. container_profile notes
        instances = parsed_json.get("instances")
        if isinstance(instances, list):
            for inst in instances:
                if isinstance(inst, dict):
                    sub_c = inst.get("sub_container")
                    if isinstance(sub_c, dict):
                        tc = sub_c.get("top_container")
                        if isinstance(tc, dict):
                            res_tc = tc.get("_resolved")
                            if isinstance(res_tc, dict):
                                res_tc.pop("internal_note", None)
                                cp = res_tc.get("container_profile")
                                if isinstance(cp, dict):
                                    res_cp = cp.get("_resolved")
                                    if isinstance(res_cp, dict):
                                        res_cp.pop("notes", None)

    return normalized_doc, parsed_json, None


def compare_pui_document(
    base_doc: Dict[str, Any],
    cand_doc: Dict[str, Any],
    ignored_fields: Set[str],
) -> Tuple[bool, List[str]]:
    """Compare a single baseline and candidate PUI document strictly.

    Enforces the two-step verification rule:
    1. First, strictly scan the candidate for ANY privacy leaks. Never normalize the
       candidate to hide leaks. If any leak is detected, fail immediately.
    2. Normalize ONLY the explicitly documented native-private fields from the baseline
       (root ancestors, JSON ancestors, top-container internal_note, container-profile notes).
    3. Enforce canonical structural equality between candidate parsed JSON and
       the privacy-normalized baseline parsed JSON.
    4. Enforce exact type-strict equality on all remaining non-JSON Solr fields and values.
    """
    reasons: List[str] = []

    # Step 1: Strict scan of candidate for ANY privacy leaks
    cand_leaks = detect_pui_privacy_leak(cand_doc)
    if cand_leaks:
        for leak in cand_leaks:
            reasons.append(f"candidate privacy leak: {leak}")
        return False, reasons

    # Parse candidate JSON
    cand_raw_json = cand_doc.get("json")
    if not isinstance(cand_raw_json, str):
        reasons.append("candidate 'json' field is missing or not a string")
        return False, reasons

    try:
        cand_parsed_json = json.loads(cand_raw_json)
        if not isinstance(cand_parsed_json, (dict, list)):
            reasons.append("candidate 'json' must parse to a JSON object or array")
            return False, reasons
    except Exception as err:
        reasons.append(f"candidate 'json' parse error: {err}")
        return False, reasons

    # Step 2: Normalize ONLY documented native-private fields from baseline
    norm_base_doc, base_parsed_json, base_err = normalize_baseline_pui_document(base_doc)
    if base_err:
        reasons.append(base_err)
        return False, reasons

    # Step 3: Canonical structural JSON equality
    ok, reason = deep_type_strict_equal(base_parsed_json, cand_parsed_json, "json")
    if not ok:
        reasons.append(f"canonical structural JSON mismatch: {reason}")

    # Step 4: Solr field-level comparison (excluding ignored fields and 'json' which is compared structurally)
    base_keys = {k for k in norm_base_doc.keys() if k not in ignored_fields and k != "json"}
    cand_keys = {k for k in cand_doc.keys() if k not in ignored_fields and k != "json"}

    if base_keys != cand_keys:
        missing_in_cand = base_keys - cand_keys
        extra_in_cand = cand_keys - base_keys
        if missing_in_cand:
            reasons.append(f"missing fields in candidate: {sorted(missing_in_cand)}")
        if extra_in_cand:
            reasons.append(f"extra fields in candidate: {sorted(extra_in_cand)}")

    common_keys = base_keys & cand_keys
    for key in sorted(common_keys):
        base_val = norm_base_doc[key]
        cand_val = cand_doc[key]
        ok, reason = deep_type_strict_equal(base_val, cand_val, key)
        if not ok:
            reasons.append(reason or f"field '{key}' mismatch")

    return len(reasons) == 0, reasons


def run_pui_parity_comparison(
    baseline_url: str,
    candidate_url: str,
    repository: str,
    batch_size: int = 1000,
    timeout: float = 30.0,
    allow_primary: bool = False,
    ignored_fields: Optional[Set[str]] = None,
) -> int:
    """Execute complete privacy-normalized PUI parity comparison between baseline and candidate Solr cores."""
    if ignored_fields is None:
        ignored_fields = DEFAULT_IGNORED_FIELDS

    norm_base, norm_cand = validate_safety_and_distinct_collection_urls(baseline_url, candidate_url, allow_primary=allow_primary)
    repo_scope = resolve_repository_scope(repository)

    base_auth = get_auth_credentials(baseline_url, "SOLR_BASELINE_USER", "SOLR_BASELINE_PASSWORD")
    cand_auth = get_auth_credentials(candidate_url, "SOLR_CANDIDATE_USER", "SOLR_CANDIDATE_PASSWORD")

    print(f"=== Archivalist Privacy-Normalized PUI Parity Comparator ===")
    print(f"Baseline Solr URL:   {redact_url(norm_base)}")
    print(f"Candidate Solr URL:  {redact_url(norm_cand)}")
    print(f"Repository Scope:    {repo_scope}")
    print(f"Ignored Solr Fields: {sorted(ignored_fields)}")
    print(f"PUI Filter:          {build_pui_repository_filter(repo_scope)}")
    print(f"Safety Check:        Primary production collections forbidden (passed)")
    print()

    print(f"Fetching PUI documents from baseline collection...")
    base_docs, base_total = fetch_pui_solr_documents(norm_base, repo_scope, auth=base_auth, batch_size=batch_size, timeout=timeout)
    print(f"Retrieved {len(base_docs)} documents from baseline (numFound: {base_total}).")

    print(f"Fetching PUI documents from candidate collection...")
    cand_docs, cand_total = fetch_pui_solr_documents(norm_cand, repo_scope, auth=cand_auth, batch_size=batch_size, timeout=timeout)
    print(f"Retrieved {len(cand_docs)} documents from candidate (numFound: {cand_total}).")
    print()

    base_ids = set(base_docs.keys())
    cand_ids = set(cand_docs.keys())

    missing_in_candidate = sorted(base_ids - cand_ids)
    extra_in_candidate = sorted(cand_ids - base_ids)
    common_ids = sorted(base_ids & cand_ids)

    matching_count = 0
    mismatch_count = 0
    privacy_leak_count = 0
    mismatch_samples: List[Tuple[str, List[str]]] = []

    for doc_id in common_ids:
        is_equal, reasons = compare_pui_document(base_docs[doc_id], cand_docs[doc_id], ignored_fields)
        if any("privacy leak" in r for r in reasons):
            privacy_leak_count += 1
        if is_equal:
            matching_count += 1
        else:
            mismatch_count += 1
            if len(mismatch_samples) < 20:
                mismatch_samples.append((doc_id, reasons))

    print(f"=== Parity Analysis Summary ===")
    print(f"Baseline total docs:       {len(base_ids)}")
    print(f"Candidate total docs:      {len(cand_ids)}")
    print(f"Common docs compared:      {len(common_ids)}")
    print(f"Privacy-normalized matching docs: {matching_count}")
    print(f"Mismatched docs:           {mismatch_count}")
    print(f"Docs with privacy leaks:   {privacy_leak_count}")
    print(f"Missing in candidate:      {len(missing_in_candidate)}")
    print(f"Extra in candidate:        {len(extra_in_candidate)}")

    if missing_in_candidate:
        print()
        print(f"Sample missing IDs in candidate ({len(missing_in_candidate)} total):")
        for mid in missing_in_candidate[:10]:
            print(f"  - {mid}")

    if extra_in_candidate:
        print()
        print(f"Sample extra IDs in candidate ({len(extra_in_candidate)} total):")
        for eid in extra_in_candidate[:10]:
            print(f"  - {eid}")

    if mismatch_samples:
        print()
        print(f"Sample document mismatches ({len(mismatch_samples)} shown):")
        for doc_id, reasons in mismatch_samples:
            print(f"  Document ID: {doc_id}")
            for reason in reasons[:5]:
                print(f"    * {reason}")
            if len(reasons) > 5:
                print(f"    * ... and {len(reasons) - 5} more discrepancies")

    success = (
        len(missing_in_candidate) == 0
        and len(extra_in_candidate) == 0
        and mismatch_count == 0
        and privacy_leak_count == 0
    )

    print()
    if success:
        print(f"RESULT: SUCCESS - Privacy-normalized PUI parity verified. All non-private fields match and privacy invariants hold.")
        return EXIT_PARITY_SUCCESS
    else:
        print(f"RESULT: FAILURE - Privacy-normalized PUI parity mismatch or privacy invariant violation detected.")
        return EXIT_PARITY_MISMATCH


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Type-strict offline privacy-normalized PUI Solr index parity comparator for ArchivesSpace / Archivalist."
    )
    parser.add_argument(
        "--baseline-url",
        required=True,
        help="Solr base collection URL for baseline native PUI index (e.g. http://localhost:8983/solr/test_pui_baseline)",
    )
    parser.add_argument(
        "--candidate-url",
        required=True,
        help="Solr base collection URL for candidate Rust-prepared PUI index (e.g. http://localhost:8983/solr/test_pui_candidate)",
    )
    parser.add_argument(
        "--repository",
        required=True,
        help="Repository scope to verify (e.g. 2 or /repositories/2)",
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=1000,
        help="Solr pagination rows batch size (default: 1000)",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=30.0,
        help="HTTP timeout in seconds (default: 30.0)",
    )
    parser.add_argument(
        "--allow-primary",
        action="store_true",
        default=False,
        help="FOR TESTING ONLY: Allow testing against URLs with primary collection names. NEVER use in production.",
    )

    args = parser.parse_args()

    try:
        return run_pui_parity_comparison(
            baseline_url=args.baseline_url,
            candidate_url=args.candidate_url,
            repository=args.repository,
            batch_size=args.batch_size,
            timeout=args.timeout,
            allow_primary=args.allow_primary,
        )
    except ConfigurationError as err:
        print(f"Configuration Error: {err}", file=sys.stderr)
        return EXIT_CONFIG_ERROR
    except TransportError as err:
        print(f"Transport Error: {err}", file=sys.stderr)
        return EXIT_TRANSPORT_ERROR
    except KeyboardInterrupt:
        print(f"\nAborted by user.", file=sys.stderr)
        return EXIT_CONFIG_ERROR


if __name__ == "__main__":
    sys.exit(main())
