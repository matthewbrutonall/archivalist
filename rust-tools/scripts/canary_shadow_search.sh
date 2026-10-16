#!/usr/bin/env bash

set -euo pipefail

BACKEND_URL="${BACKEND_URL:-http://127.0.0.1:8089}"
SIDECAR_URL="${SIDECAR_URL:-http://127.0.0.1:4570}"
REPO_ID="${REPO_ID:?Set REPO_ID to a non-production repository ID}"
RESOURCE_ID="${RESOURCE_ID:?Set RESOURCE_ID to a representative resource ID}"
QUERY="${QUERY:?Set QUERY to a term present beneath the resource}"
ADMIN_USER="${ADMIN_USER:-admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:?Set ADMIN_PASSWORD for the non-production instance}"
ITERATIONS="${ITERATIONS:-10}"
PAGE_SIZE="${PAGE_SIZE:-25}"
OUTPUT_DIR="${OUTPUT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/benchmarks}"

for command in curl jq sort awk date; do
    command -v "${command}" >/dev/null || {
        printf 'Required command not found: %s\n' "${command}" >&2
        exit 1
    }
done

case "${ITERATIONS}" in
    ''|*[!0-9]*) printf 'ITERATIONS must be a positive integer\n' >&2; exit 1 ;;
esac
if [ "${ITERATIONS}" -lt 1 ]; then
    printf 'ITERATIONS must be a positive integer\n' >&2
    exit 1
fi

curl --fail --silent --show-error "${SIDECAR_URL%/}/ready" >/dev/null

LOGIN_RESPONSE="$(curl --fail --silent --show-error \
    -F "password=${ADMIN_PASSWORD}" \
    "${BACKEND_URL%/}/users/${ADMIN_USER}/login")"
SESSION_TOKEN="$(jq -r '.session // empty' <<<"${LOGIN_RESPONSE}")"
if [ -z "${SESSION_TOKEN}" ]; then
    printf 'Backend login did not return a session token\n' >&2
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
REPORT="${OUTPUT_DIR}/canary-shadow-search-${RUN_ID}.csv"
printf 'iteration,native_seconds,rust_seconds,total_hits,result_count\n' > "${REPORT}"

RESOURCE_URI="/repositories/${REPO_ID}/resources/${RESOURCE_ID}"
request_search() {
    local mode="$1"
    local output_file="$2"
    local time_file="$3"
    local args=(
        --get --fail --silent --show-error
        -H "X-ArchivesSpace-Session: ${SESSION_TOKEN}"
        --data-urlencode "q=${QUERY}"
        --data-urlencode "root_record=${RESOURCE_URI}"
        --data-urlencode "page=1"
        --data-urlencode "page_size=${PAGE_SIZE}"
    )
    if [ "${mode}" = "rust" ]; then
        args+=(--data-urlencode "shadow_sidecar=true")
    fi
    curl "${args[@]}" -o "${output_file}" -w '%{time_total}' \
        "${BACKEND_URL%/}/repositories/${REPO_ID}/search" > "${time_file}"
}

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

for iteration in $(seq 1 "${ITERATIONS}"); do
    request_search native "${TMP_DIR}/native.json" "${TMP_DIR}/native.time"
    request_search rust "${TMP_DIR}/rust.json" "${TMP_DIR}/rust.time"

    jq -S '{total_hits, results: [.results[] | {uri, title, primary_type}]}' \
        "${TMP_DIR}/native.json" > "${TMP_DIR}/native.canonical.json"
    jq -S '{total_hits, results: [.results[] | {uri, title, primary_type}]}' \
        "${TMP_DIR}/rust.json" > "${TMP_DIR}/rust.canonical.json"

    if ! cmp -s "${TMP_DIR}/native.canonical.json" "${TMP_DIR}/rust.canonical.json"; then
        printf 'Canary parity failed at iteration %s\n' "${iteration}" >&2
        diff -u "${TMP_DIR}/native.canonical.json" "${TMP_DIR}/rust.canonical.json" >&2 || true
        exit 1
    fi

    if jq -e 'has("facets")' "${TMP_DIR}/rust.json" >/dev/null; then
        printf 'Rust request fell back to the native path at iteration %s\n' "${iteration}" >&2
        exit 1
    fi

    native_seconds="$(<"${TMP_DIR}/native.time")"
    rust_seconds="$(<"${TMP_DIR}/rust.time")"
    total_hits="$(jq -r '.total_hits' "${TMP_DIR}/rust.json")"
    result_count="$(jq -r '.results | length' "${TMP_DIR}/rust.json")"
    printf '%s,%s,%s,%s,%s\n' \
        "${iteration}" "${native_seconds}" "${rust_seconds}" "${total_hits}" "${result_count}" >> "${REPORT}"
done

median() {
    sort -n | awk '{ values[NR] = $1 } END { if (NR % 2) print values[(NR + 1) / 2]; else printf "%.6f\n", (values[NR / 2] + values[NR / 2 + 1]) / 2 }'
}

NATIVE_MEDIAN="$(awk -F, 'NR > 1 {print $2}' "${REPORT}" | median)"
RUST_MEDIAN="$(awk -F, 'NR > 1 {print $3}' "${REPORT}" | median)"
IMPROVEMENT="$(awk -v native="${NATIVE_MEDIAN}" -v rust="${RUST_MEDIAN}" 'BEGIN { if (native == 0) print "0.0"; else printf "%.1f", ((native - rust) / native) * 100 }')"

printf 'Parity: PASS (%s iterations)\n' "${ITERATIONS}"
printf 'Native median: %ss\n' "${NATIVE_MEDIAN}"
printf 'Rust median:   %ss\n' "${RUST_MEDIAN}"
printf 'Improvement:   %s%%\n' "${IMPROVEMENT}"
printf 'Report: %s\n' "${REPORT}"
