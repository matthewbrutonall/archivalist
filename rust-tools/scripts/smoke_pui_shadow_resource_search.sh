#!/usr/bin/env bash
#
# smoke_pui_shadow_resource_search.sh
#
# Conservative read-only bash smoke helper for the opt-in PUI resource shadow search path.
# Validates:
# 1. Shadow search response (q, root_record, shadow_sidecar=true, page=1, page_size)
#    verifying total_hits and results are present, plus asserting absence of facets
#    when EXPECT_FAST_PATH=true.
# 2. Fallback search response with unsupported facet[]=primary_type
#    verifying fallback shape with facets, total_hits, and results.
#
# Read-only: never writes to Solr, ArchivesSpace records, DB, or Docker.

set -euo pipefail

BACKEND_URL="${BACKEND_URL:-http://localhost:4567}"
REPO_ID="${REPO_ID:?Set REPO_ID to a test repository ID}"
RESOURCE_URI="${RESOURCE_URI:?Set RESOURCE_URI to a resource in the test repository}"
QUERY="${QUERY:?Set QUERY to a term present in the test repository}"
PAGE_SIZE="${PAGE_SIZE:-5}"
EXPECT_FAST_PATH="${EXPECT_FAST_PATH:-false}"

if ! command -v curl >/dev/null 2>&1; then
    echo "Error: curl is required but not installed." >&2
    exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "Error: jq is required but not installed." >&2
    exit 1
fi

echo "============================================================"
echo " Starting PUI shadow resource search smoke test"
echo "============================================================"
echo " Backend URL:      ${BACKEND_URL}"
echo " Repo ID:          ${REPO_ID}"
echo " Resource URI:     ${RESOURCE_URI}"
echo " Query:            ${QUERY}"
echo " Page Size:        ${PAGE_SIZE}"
echo " Expect Fast-Path: ${EXPECT_FAST_PATH}"
echo "============================================================"

# 1. Login to backend as admin/admin and get a session token
LOGIN_URL="${BACKEND_URL%/}/users/admin/login"
echo -n "1. Authenticating to backend at ${LOGIN_URL}... "

LOGIN_RESPONSE=$(curl -s -S --fail-with-body -F "password=admin" "${LOGIN_URL}" 2>&1) || {
    echo "FAILED"
    echo "Error: Failed to connect or authenticate to ArchivesSpace backend at ${LOGIN_URL}." >&2
    echo "Output: ${LOGIN_RESPONSE}" >&2
    exit 1
}

SESSION_TOKEN=$(echo "${LOGIN_RESPONSE}" | jq -r '.session // empty' 2>/dev/null || true)
if [ -z "${SESSION_TOKEN}" ] || [ "${SESSION_TOKEN}" = "null" ]; then
    echo "FAILED"
    echo "Error: Authentication response did not contain a valid session token." >&2
    echo "Response: ${LOGIN_RESPONSE}" >&2
    exit 1
fi
echo "OK"

# 2. Call /repositories/:repo_id/search with shadow_sidecar=true params
SHADOW_SEARCH_URL="${BACKEND_URL%/}/repositories/${REPO_ID}/search?q=${QUERY}&root_record=${RESOURCE_URI}&shadow_sidecar=true&page=1&page_size=${PAGE_SIZE}"
echo -n "2. Querying shadow search endpoint (shadow_sidecar=true)... "

SHADOW_RESPONSE=$(curl -g -s -S -H "X-ArchivesSpace-Session: ${SESSION_TOKEN}" "${SHADOW_SEARCH_URL}" 2>&1) || {
    echo "FAILED"
    echo "Error: Failed to execute HTTP GET for shadow search at ${SHADOW_SEARCH_URL}." >&2
    echo "Output: ${SHADOW_RESPONSE}" >&2
    exit 1
}

# Verify shadow search response has results and total_hits
SHADOW_TOTAL_HITS=$(echo "${SHADOW_RESPONSE}" | jq -r '.total_hits // empty' 2>/dev/null || true)
SHADOW_HAS_RESULTS=$(echo "${SHADOW_RESPONSE}" | jq -r 'if .results != null and (.results | type == "array") then "true" else "false" end' 2>/dev/null || true)

if [ -z "${SHADOW_TOTAL_HITS}" ] || [ "${SHADOW_HAS_RESULTS}" != "true" ]; then
    echo "FAILED"
    echo "Error: Shadow search response is missing required 'results' array or 'total_hits'." >&2
    echo "Response: ${SHADOW_RESPONSE}" >&2
    exit 1
fi

SHADOW_FIRST_URI=$(echo "${SHADOW_RESPONSE}" | jq -r '.results[0].uri // empty' 2>/dev/null || true)
echo "OK"

if [ "${EXPECT_FAST_PATH}" = "true" ]; then
    SHADOW_HAS_FACETS=$(echo "${SHADOW_RESPONSE}" | jq -r 'if has("facets") or .facets != null then "true" else "false" end' 2>/dev/null || true)
    if [ "${SHADOW_HAS_FACETS}" = "true" ]; then
        echo "   Fast-path assertion: FAILED" >&2
        echo "   Error: Expected fast-path response, but '.facets' is present (backend used fallback Solr path)." >&2
        exit 1
    fi
    echo "   Fast-path assertion: OK"
    echo "   Fast-path total_hits: ${SHADOW_TOTAL_HITS}"
    echo "   Fast-path first_uri:  ${SHADOW_FIRST_URI:-none}"
else
    echo "   Shadow-request total_hits: ${SHADOW_TOTAL_HITS}"
    echo "   Shadow-request first_uri:  ${SHADOW_FIRST_URI:-none}"
fi

# 3. Call same endpoint with unsupported facet[]=primary_type to prove fallback works
FALLBACK_URL="${SHADOW_SEARCH_URL}&facet[]=primary_type"
echo -n "3. Querying fallback search endpoint (facet[]=primary_type)... "

FALLBACK_RESPONSE=$(curl -g -s -S -H "X-ArchivesSpace-Session: ${SESSION_TOKEN}" "${FALLBACK_URL}" 2>&1) || {
    echo "FAILED"
    echo "Error: Failed to execute HTTP GET for fallback search at ${FALLBACK_URL}." >&2
    echo "Output: ${FALLBACK_RESPONSE}" >&2
    exit 1
}

# Verify fallback response has facets, results, and total_hits
FALLBACK_HAS_FACETS=$(echo "${FALLBACK_RESPONSE}" | jq -r 'if .facets != null then "true" else "false" end' 2>/dev/null || true)
if [ "${FALLBACK_HAS_FACETS}" != "true" ]; then
    echo "FAILED"
    echo "Error: Fallback response is missing required 'facets' field." >&2
    echo "Response: ${FALLBACK_RESPONSE}" >&2
    exit 1
fi

FALLBACK_TOTAL_HITS=$(echo "${FALLBACK_RESPONSE}" | jq -r '.total_hits // empty' 2>/dev/null || true)
FALLBACK_HAS_RESULTS=$(echo "${FALLBACK_RESPONSE}" | jq -r 'if .results != null and (.results | type == "array") then "true" else "false" end' 2>/dev/null || true)

if [ -z "${FALLBACK_TOTAL_HITS}" ] || [ "${FALLBACK_HAS_RESULTS}" != "true" ]; then
    echo "FAILED"
    echo "Error: Fallback response is missing required 'results' array or 'total_hits'." >&2
    echo "Response: ${FALLBACK_RESPONSE}" >&2
    exit 1
fi

FALLBACK_FIRST_URI=$(echo "${FALLBACK_RESPONSE}" | jq -r '.results[0].uri // empty' 2>/dev/null || true)
echo "OK"
echo "   Fallback total_hits:  ${FALLBACK_TOTAL_HITS}"
echo "   Fallback first_uri:   ${FALLBACK_FIRST_URI:-none}"

echo "============================================================"
echo " Smoke test completed successfully."
echo "============================================================"
