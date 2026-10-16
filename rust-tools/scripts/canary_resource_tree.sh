#!/usr/bin/env bash

set -euo pipefail

BASELINE_BACKEND_URL="${BASELINE_BACKEND_URL:-http://127.0.0.1:4567}"
CANARY_BACKEND_URL="${CANARY_BACKEND_URL:-http://127.0.0.1:4569}"
SIDECAR_URL="${SIDECAR_URL:-http://127.0.0.1:4570}"
REPO_ID="${REPO_ID:?Set REPO_ID to a non-production repository ID}"
RESOURCE_ID="${RESOURCE_ID:?Set RESOURCE_ID to a representative resource ID}"
NODE_URI="${NODE_URI:?Set NODE_URI to a nested node with at least 100 children}"
NODE_IDS="${NODE_IDS:?Set NODE_IDS to 64-128 unique comma-separated archival object IDs}"
ADMIN_USER="${ADMIN_USER:-admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:?Set ADMIN_PASSWORD for the non-production instance}"
ITERATIONS="${ITERATIONS:-10}"
WARMUP_ITERATIONS="${WARMUP_ITERATIONS:-3}"
EXPECT_FAST_PATH="${EXPECT_FAST_PATH:-true}"
GATES="${GATES:-root_waypoint,nested_node,root_paths}"
OUTPUT_DIR="${OUTPUT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/benchmarks}"

for command in curl jq sort awk date cmp diff; do
    command -v "${command}" >/dev/null || {
        printf 'Required command not found: %s\n' "${command}" >&2
        exit 1
    }
done

case "${ITERATIONS}" in
    ''|*[!0-9]*) printf 'ITERATIONS must be a positive integer\n' >&2; exit 1 ;;
esac
case "${WARMUP_ITERATIONS}" in
    ''|*[!0-9]*) printf 'WARMUP_ITERATIONS must be a non-negative integer\n' >&2; exit 1 ;;
esac
if [ "${ITERATIONS}" -lt 1 ]; then
    printf 'ITERATIONS must be a positive integer\n' >&2
    exit 1
fi
if [ "${EXPECT_FAST_PATH}" != "true" ] && [ "${EXPECT_FAST_PATH}" != "false" ]; then
    printf 'EXPECT_FAST_PATH must be true or false\n' >&2
    exit 1
fi
if ! [[ "${NODE_URI}" =~ ^/repositories/${REPO_ID}/archival_objects/[1-9][0-9]*$ ]]; then
    printf 'NODE_URI must belong to REPO_ID and identify an archival object\n' >&2
    exit 1
fi

IFS=',' read -r -a GATE_ARRAY <<<"${GATES}"
declare -A SEEN_GATES=()
for gate in "${GATE_ARRAY[@]}"; do
    case "${gate}" in
        root_waypoint|nested_node|root_paths) ;;
        *) printf 'Unknown tree gate: %s\n' "${gate}" >&2; exit 1 ;;
    esac
    if [ -n "${SEEN_GATES[${gate}]:-}" ]; then
        printf 'GATES contains a duplicate: %s\n' "${gate}" >&2
        exit 1
    fi
    SEEN_GATES[${gate}]=1
done
if [ "${#GATE_ARRAY[@]}" -eq 0 ]; then
    printf 'GATES must select at least one tree gate\n' >&2
    exit 1
fi

IFS=',' read -r -a NODE_ID_ARRAY <<<"${NODE_IDS}"
declare -A SEEN_NODE_IDS=()
for node_id in "${NODE_ID_ARRAY[@]}"; do
    if ! [[ "${node_id}" =~ ^[1-9][0-9]*$ ]]; then
        printf 'NODE_IDS contains an invalid ID: %s\n' "${node_id}" >&2
        exit 1
    fi
    if [ -n "${SEEN_NODE_IDS[${node_id}]:-}" ]; then
        printf 'NODE_IDS contains a duplicate ID: %s\n' "${node_id}" >&2
        exit 1
    fi
    SEEN_NODE_IDS[${node_id}]=1
done
if [ "${#NODE_ID_ARRAY[@]}" -lt 64 ] || [ "${#NODE_ID_ARRAY[@]}" -gt 128 ]; then
    printf 'NODE_IDS must contain between 64 and 128 IDs\n' >&2
    exit 1
fi

login() {
    local backend_url="$1"
    curl --fail --silent --show-error \
        -F "password=${ADMIN_PASSWORD}" \
        "${backend_url%/}/users/${ADMIN_USER}/login" | jq -r '.session // empty'
}

BASELINE_SESSION="$(login "${BASELINE_BACKEND_URL}")"
CANARY_SESSION="$(login "${CANARY_BACKEND_URL}")"
if [ -z "${BASELINE_SESSION}" ] || [ -z "${CANARY_SESSION}" ]; then
    printf 'Backend login did not return both session tokens\n' >&2
    exit 1
fi

if [ "${EXPECT_FAST_PATH}" = "true" ]; then
    curl --fail --silent --show-error "${SIDECAR_URL%/}/ready" >/dev/null
fi

mkdir -p "${OUTPUT_DIR}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
REPORT="${OUTPUT_DIR}/canary-resource-tree-${RUN_ID}.csv"
printf 'gate,iteration,native_seconds,rust_seconds\n' > "${REPORT}"

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

metric_requests_total() {
    curl --fail --silent --show-error "${SIDECAR_URL%/}/metrics" |
        awk '$1 == "archivalist_sidecar_requests_total" { print $2 }'
}

request_tree() {
    local backend_url="$1"
    local session="$2"
    local gate="$3"
    local output_file="$4"
    local time_file="$5"
    local endpoint="${backend_url%/}/repositories/${REPO_ID}/resources/${RESOURCE_ID}/tree"
    local args=(--get --fail --silent --show-error -H "X-ArchivesSpace-Session: ${session}")

    case "${gate}" in
        root_waypoint)
            endpoint="${endpoint}/waypoint"
            args+=(--data-urlencode 'offset=0')
            ;;
        nested_node)
            endpoint="${endpoint}/node"
            args+=(--data-urlencode "node_uri=${NODE_URI}")
            ;;
        root_paths)
            endpoint="${endpoint}/node_from_root"
            for node_id in "${NODE_ID_ARRAY[@]}"; do
                args+=(--data-urlencode "node_ids[]=${node_id}")
            done
            ;;
        *)
            printf 'Unknown tree gate: %s\n' "${gate}" >&2
            return 1
            ;;
    esac

    curl "${args[@]}" -o "${output_file}" -w '%{time_total}' "${endpoint}" > "${time_file}"
}

for gate in "${GATE_ARRAY[@]}"; do
    total_iterations=$((WARMUP_ITERATIONS + ITERATIONS))
    for iteration in $(seq 1 "${total_iterations}"); do
        if [ "${EXPECT_FAST_PATH}" = "true" ]; then
            metrics_before="$(metric_requests_total)"
        fi

        if [ $((iteration % 2)) -eq 1 ]; then
            request_tree "${BASELINE_BACKEND_URL}" "${BASELINE_SESSION}" "${gate}" \
                "${TMP_DIR}/native.json" "${TMP_DIR}/native.time"
            request_tree "${CANARY_BACKEND_URL}" "${CANARY_SESSION}" "${gate}" \
                "${TMP_DIR}/rust.json" "${TMP_DIR}/rust.time"
        else
            request_tree "${CANARY_BACKEND_URL}" "${CANARY_SESSION}" "${gate}" \
                "${TMP_DIR}/rust.json" "${TMP_DIR}/rust.time"
            request_tree "${BASELINE_BACKEND_URL}" "${BASELINE_SESSION}" "${gate}" \
                "${TMP_DIR}/native.json" "${TMP_DIR}/native.time"
        fi

        jq -S . "${TMP_DIR}/native.json" > "${TMP_DIR}/native.canonical.json"
        jq -S . "${TMP_DIR}/rust.json" > "${TMP_DIR}/rust.canonical.json"
        if ! cmp -s "${TMP_DIR}/native.canonical.json" "${TMP_DIR}/rust.canonical.json"; then
            printf '%s parity failed at iteration %s\n' "${gate}" "${iteration}" >&2
            diff -u "${TMP_DIR}/native.canonical.json" "${TMP_DIR}/rust.canonical.json" >&2 || true
            exit 1
        fi

        if [ "${EXPECT_FAST_PATH}" = "true" ]; then
            metrics_after="$(metric_requests_total)"
            if [ $((metrics_after - metrics_before)) -lt 2 ]; then
                printf '%s did not reach the Rust sidecar at iteration %s\n' "${gate}" "${iteration}" >&2
                exit 1
            fi
        fi

        if [ "${iteration}" -gt "${WARMUP_ITERATIONS}" ]; then
            measured_iteration=$((iteration - WARMUP_ITERATIONS))
            printf '%s,%s,%s,%s\n' \
                "${gate}" "${measured_iteration}" "$(<"${TMP_DIR}/native.time")" "$(<"${TMP_DIR}/rust.time")" >> "${REPORT}"
        fi
    done
done

median() {
    sort -n | awk '{ values[NR] = $1 } END { if (NR % 2) print values[(NR + 1) / 2]; else printf "%.6f\n", (values[NR / 2] + values[NR / 2 + 1]) / 2 }'
}

for gate in "${GATE_ARRAY[@]}"; do
    native_median="$(awk -F, -v gate="${gate}" 'NR > 1 && $1 == gate {print $3}' "${REPORT}" | median)"
    rust_median="$(awk -F, -v gate="${gate}" 'NR > 1 && $1 == gate {print $4}' "${REPORT}" | median)"
    improvement="$(awk -v native="${native_median}" -v rust="${rust_median}" 'BEGIN { if (native == 0) print "0.0"; else printf "%.1f", ((native - rust) / native) * 100 }')"
    printf '%-14s parity=PASS native=%ss rust=%ss improvement=%s%%\n' \
        "${gate}" "${native_median}" "${rust_median}" "${improvement}"
done
printf 'Report: %s\n' "${REPORT}"
