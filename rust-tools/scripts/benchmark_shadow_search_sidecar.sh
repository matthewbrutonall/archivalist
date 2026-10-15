#!/usr/bin/env bash
#
# benchmark_shadow_search_sidecar.sh
#
# Deterministic, safe/read-only benchmark script for the aspace-shadow-sidecar
# keyword search endpoint (/shadow/search).
#
# Requirements:
# - Default PORT: 4575 (overridable with PORT env var)
# - Default REPEAT: 20 (overridable with REPEAT env var)
# - Starts aspace-shadow-sidecar using cargo
# - Waits until it responds
# - Runs repeated curl calls against:
#   /shadow/search?q=<QUERY>&resource_uri=<RESOURCE_URI>&rows=5&tree=true&include_ancestors=true
# - Prints min/mean/max elapsed milliseconds using curl timing
# - Confirms num_found appears in response
# - Confirms "orphans":[] appears in response
# - Confirms collection=archivesspace returns 403
# - Stops the sidecar cleanly on exit
# - Standard shell tools only (no jq)
#

set -euo pipefail

PORT="${PORT:-4575}"
REPEAT="${REPEAT:-20}"
QUERY="${QUERY:?Set QUERY to a term present in the test index}"
RESOURCE_URI="${RESOURCE_URI:?Set RESOURCE_URI to a resource in the test index}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

HOST="127.0.0.1"
BASE_URL="http://${HOST}:${PORT}"
TARGET_PATH="/shadow/search?q=${QUERY}&resource_uri=${RESOURCE_URI}&rows=5&tree=true&include_ancestors=true"
TARGET_URL="${BASE_URL}${TARGET_PATH}"
FORBIDDEN_URL="${BASE_URL}/shadow/search?collection=archivesspace"

SIDECAR_PID=""

cleanup() {
    local exit_code=$?
    if [ -n "${SIDECAR_PID}" ] && kill -0 "${SIDECAR_PID}" 2>/dev/null; then
        echo "Stopping sidecar process (PID ${SIDECAR_PID})..."
        kill "${SIDECAR_PID}" 2>/dev/null || true
        wait "${SIDECAR_PID}" 2>/dev/null || true
    fi
    exit "${exit_code}"
}

trap cleanup EXIT INT TERM

echo "============================================================"
echo " Starting aspace-shadow-sidecar benchmark"
echo "============================================================"
echo " Host:        ${HOST}"
echo " Port:        ${PORT}"
echo " Repeat:      ${REPEAT}"
echo " Target URL:  ${TARGET_URL}"
echo "============================================================"

# Ensure the binary is built first so cargo run doesn't delay startup loop
echo "Building aspace-shadow-sidecar binary..."
cargo build --release -q --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" -p aspace-cli --bin aspace-shadow-sidecar

echo "Launching aspace-shadow-sidecar on port ${PORT}..."
cargo run --release -q --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" -p aspace-cli --bin aspace-shadow-sidecar -- --port "${PORT}" &
SIDECAR_PID=$!

# Wait until sidecar responds
echo "Waiting for sidecar to become ready..."
READY=0
for i in $(seq 1 50); do
    if curl -s -f "${BASE_URL}/shadow/search?rows=0" >/dev/null 2>&1; then
        READY=1
        break
    fi
    sleep 0.1
done

if [ "${READY}" -ne 1 ]; then
    echo "Error: Sidecar failed to respond on ${BASE_URL} within 5 seconds."
    exit 1
fi
echo "Sidecar is ready."

# 1. Verification: test forbidden collection return code 403
echo -n "Checking live Solr collection refusal (HTTP 403)... "
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "${FORBIDDEN_URL}")
if [ "${HTTP_CODE}" != "403" ]; then
    echo "FAILED (expected 403, got ${HTTP_CODE})"
    exit 1
fi
echo "OK (got 403)"

# 2. Functional assertions on response body
echo "Validating search response shape on target query..."
SAMPLE_RESPONSE=$(curl -s "${TARGET_URL}")

if ! echo "${SAMPLE_RESPONSE}" | grep -q '"num_found":'; then
    echo "Error: 'num_found' not found in sidecar response."
    echo "Response: ${SAMPLE_RESPONSE}"
    exit 1
fi

if ! echo "${SAMPLE_RESPONSE}" | grep -q '"orphans":\[\]'; then
    echo "Error: '\"orphans\":[]' not found in sidecar response."
    echo "Response: ${SAMPLE_RESPONSE}"
    exit 1
fi
echo "Response contract verified (num_found present, orphans:[] present)."

# 3. Repeated benchmark runs
echo "Running ${REPEAT} benchmark iterations..."
TIMINGS=()
for ((i=1; i<=REPEAT; i++)); do
    TIME_SEC=$(curl -s -o /dev/null -w "%{time_total}" "${TARGET_URL}")
    TIMINGS+=("${TIME_SEC}")
done

# 4. Compute min/mean/max in milliseconds using awk
STATS=$(printf "%s\n" "${TIMINGS[@]}" | awk '
BEGIN {
    min = 999999.0;
    max = 0.0;
    sum = 0.0;
    count = 0;
}
{
    val = $1 * 1000.0; # convert seconds to milliseconds
    if (val < min) min = val;
    if (val > max) max = val;
    sum += val;
    count++;
}
END {
    if (count > 0) {
        printf "%.2f %.2f %.2f\n", min, (sum / count), max;
    } else {
        printf "0.00 0.00 0.00\n";
    }
}
')

MIN_MS=$(echo "${STATS}" | awk '{print $1}')
MEAN_MS=$(echo "${STATS}" | awk '{print $2}')
MAX_MS=$(echo "${STATS}" | awk '{print $3}')

echo "============================================================"
echo " Benchmark Results (${REPEAT} requests)"
echo "============================================================"
echo " Min:  ${MIN_MS} ms"
echo " Mean: ${MEAN_MS} ms"
echo " Max:  ${MAX_MS} ms"
echo "============================================================"
echo "Benchmark completed successfully."
