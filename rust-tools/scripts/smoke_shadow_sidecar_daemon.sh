#!/usr/bin/env bash
#
# smoke_shadow_sidecar_daemon.sh
#
# Safe, read-only smoke test script for validating aspace-shadow-sidecar
# daemon readiness.
#
# Checks:
# 1. GET /health confirms process liveness and configured dependency target.
# 2. GET /ready proves the configured shadow Solr collection is queryable.
# 3. GET /metrics exposes process counters in Prometheus format.
# 4. GET /shadow/search with caller-supplied test data returns valid tree JSON.
# 5. Live collection queries (collection=archivesspace) are refused with HTTP 403 Forbidden.
#
# Shell-only, read-only, no jq requirement.
# Stops sidecar cleanly on exit.

set -euo pipefail

PORT="${PORT:-4577}"
QUERY="${QUERY:?Set QUERY to a term present in the test index}"
RESOURCE_URI="${RESOURCE_URI:?Set RESOURCE_URI to a resource in the test index}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

HOST="127.0.0.1"
BASE_URL="http://${HOST}:${PORT}"
HEALTH_URL="${BASE_URL}/health"
READY_URL="${BASE_URL}/ready"
METRICS_URL="${BASE_URL}/metrics"
SEARCH_URL="${BASE_URL}/shadow/search?q=${QUERY}&resource_uri=${RESOURCE_URI}&rows=5&tree=true&include_ancestors=true"
FORBIDDEN_URL="${BASE_URL}/shadow/search?collection=archivesspace"

SIDECAR_PID=""

cleanup() {
    local exit_code=$?
    if [ -n "${SIDECAR_PID}" ] && kill -0 "${SIDECAR_PID}" 2>/dev/null; then
        echo "Stopping sidecar daemon process (PID ${SIDECAR_PID})..."
        kill "${SIDECAR_PID}" 2>/dev/null || true
        wait "${SIDECAR_PID}" 2>/dev/null || true
    fi
    exit "${exit_code}"
}

trap cleanup EXIT INT TERM

echo "============================================================"
echo " Starting aspace-shadow-sidecar daemon smoke test"
echo "============================================================"
echo " Host: ${HOST}"
echo " Port: ${PORT}"
echo " Base URL: ${BASE_URL}"
echo "============================================================"

echo "Building release binary..."
cargo build --release -q --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" -p aspace-cli --bin aspace-shadow-sidecar

echo "Starting aspace-shadow-sidecar on ${HOST}:${PORT}..."
"${WORKSPACE_ROOT}/target/release/aspace-shadow-sidecar" --host "${HOST}" --port "${PORT}" &
SIDECAR_PID=$!

echo "Waiting for sidecar to become ready..."
READY=0
for i in $(seq 1 50); do
    if curl -s -f "${HEALTH_URL}" >/dev/null 2>&1; then
        READY=1
        break
    fi
    sleep 0.1
done

if [ "${READY}" -ne 1 ]; then
    echo "Error: Sidecar failed to start on ${BASE_URL} within 5 seconds."
    exit 1
fi
echo "Sidecar is running."

# 1. Health check
echo -n "1. Checking GET /health... "
HEALTH_RESPONSE=$(curl -s -f "${HEALTH_URL}")

if ! echo "${HEALTH_RESPONSE}" | grep -q '"status":"ok"'; then
    echo "FAILED: 'status':'ok' not found in health response"
    echo "Response: ${HEALTH_RESPONSE}"
    exit 1
fi

if ! echo "${HEALTH_RESPONSE}" | grep -q '"served_by":"aspace-shadow-sidecar"'; then
    echo "FAILED: 'served_by':'aspace-shadow-sidecar' not found in health response"
    echo "Response: ${HEALTH_RESPONSE}"
    exit 1
fi

if ! echo "${HEALTH_RESPONSE}" | grep -q '"solr_url":'; then
    echo "FAILED: 'solr_url' field not found in health response"
    echo "Response: ${HEALTH_RESPONSE}"
    exit 1
fi

if ! echo "${HEALTH_RESPONSE}" | grep -q '"collection":'; then
    echo "FAILED: 'collection' field not found in health response"
    echo "Response: ${HEALTH_RESPONSE}"
    exit 1
fi
echo "PASSED"

# 2. Dependency readiness check
echo -n "2. Checking GET /ready against shadow Solr... "
READY_RESPONSE=$(curl -s -f "${READY_URL}")

if ! echo "${READY_RESPONSE}" | grep -q '"status":"ready"'; then
    echo "FAILED: 'status':'ready' not found in readiness response"
    echo "Response: ${READY_RESPONSE}"
    exit 1
fi

if ! echo "${READY_RESPONSE}" | grep -q '"num_found":'; then
    echo "FAILED: 'num_found' not found in readiness response"
    echo "Response: ${READY_RESPONSE}"
    exit 1
fi
echo "PASSED"

# 3. Metrics check
echo -n "3. Checking GET /metrics... "
METRICS_RESPONSE=$(curl -s -f "${METRICS_URL}")
if ! echo "${METRICS_RESPONSE}" | grep -q '^archivalist_sidecar_requests_total '; then
    echo "FAILED: request counter not found in metrics response"
    echo "Response: ${METRICS_RESPONSE}"
    exit 1
fi
echo "PASSED"

# 4. Shadow search check
echo -n "4. Checking GET /shadow/search (configured query with tree & ancestors)... "
SEARCH_RESPONSE=$(curl -s -f "${SEARCH_URL}")

if ! echo "${SEARCH_RESPONSE}" | grep -q '"num_found":'; then
    echo "FAILED: 'num_found' not found in search response"
    echo "Response: ${SEARCH_RESPONSE}"
    exit 1
fi

if ! echo "${SEARCH_RESPONSE}" | grep -q '"orphans":\[\]'; then
    echo "FAILED: 'orphans':[] not found in search response"
    echo "Response: ${SEARCH_RESPONSE}"
    exit 1
fi

if ! echo "${SEARCH_RESPONSE}" | grep -q '"tree":\['; then
    echo "FAILED: 'tree':[...] not found in search response"
    echo "Response: ${SEARCH_RESPONSE}"
    exit 1
fi
echo "PASSED"

# 5. Live collection refusal check
echo -n "5. Checking live collection refusal returns 403... "
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "${FORBIDDEN_URL}")
if [ "${HTTP_CODE}" != "403" ]; then
    echo "FAILED: Expected HTTP 403, got ${HTTP_CODE}"
    exit 1
fi
echo "PASSED (got 403)"

echo "============================================================"
echo " Smoke test completed successfully: sidecar daemon is ready."
echo "============================================================"
