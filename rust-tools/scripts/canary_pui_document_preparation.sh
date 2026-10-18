#!/usr/bin/env bash
# Low-level projection and privacy canary for Archivalist PUI Document Preparation.
#
# SAFETY NOTICE:
# This script is strictly for isolated non-production verification.
# It permanently forbids targeting any primary production Solr collection.
#
# SCOPE NOTICE:
# This canary verifies the low-level Rust sidecar document preparation contract directly.
# It does NOT perform Ruby RecordInheritance merge (which is performed in ArchivesSpace during indexing).
# The full two-index comparator (compare_pui_index_parity.py) is the end-to-end gate for complete privacy-normalized PUI parity.
set -euo pipefail

BACKEND_URL="${BACKEND_URL:-http://127.0.0.1:4567}"
SIDECAR_URL="${SIDECAR_URL:-http://127.0.0.1:4570}"
CANARY_SOLR_URL="${CANARY_SOLR_URL:-http://127.0.0.1:8985/solr/test_canary_pui}"
REPO_ID="${REPO_ID:?Set REPO_ID to a non-production repository ID}"
ARCHIVAL_OBJECT_ID="${ARCHIVAL_OBJECT_ID:-}"
RESOURCE_ID="${RESOURCE_ID:-}"
ITERATIONS="${ITERATIONS:-5}"

# 1. Safety check: Refuse primary production Solr collection
lower_solr_url="$(printf '%s' "${CANARY_SOLR_URL}" | tr '[:upper:]' '[:lower:]')"
for forbidden in "archivesspace" "production" "primary" "live"; do
    if [[ "${lower_solr_url}" == *"${forbidden}"* ]]; then
        printf 'ERROR: CANARY_SOLR_URL (%s) contains forbidden production keyword "%s".\n' "${CANARY_SOLR_URL}" "${forbidden}" >&2
        printf 'Safety protocol forbids targeting primary production Solr collections.\n' >&2
        exit 1
    fi
done

if [[ -z "${ARCHIVAL_OBJECT_ID}" && -z "${RESOURCE_ID}" ]]; then
    printf 'ERROR: Set at least one of ARCHIVAL_OBJECT_ID or RESOURCE_ID for canary verification.\n' >&2
    exit 1
fi

printf '=== Archivalist PUI Document Preparation Canary ===\n'
printf 'Backend URL:       %s\n' "${BACKEND_URL}"
printf 'Sidecar URL:       %s\n' "${SIDECAR_URL}"
printf 'Canary Solr URL:   %s\n' "${CANARY_SOLR_URL}"
printf 'Repository ID:     %s\n' "${REPO_ID}"
printf 'Archival Object:   %s\n' "${ARCHIVAL_OBJECT_ID:-none}"
printf 'Resource ID:       %s\n' "${RESOURCE_ID:-none}"
printf 'Feature Gate:      AppConfig[:rust_pui_document_preparation_enabled] (disabled by default)\n'
printf 'Safety Check:      Non-production target verified.\n\n'

# Authenticate if session not provided
if [[ -z "${ARCHIVESSPACE_SESSION:-}" ]]; then
    : "${ADMIN_PASSWORD:?Set ARCHIVESSPACE_SESSION or ADMIN_PASSWORD}"
    ARCHIVESSPACE_SESSION="$(
        curl --fail --silent --show-error \
            --data-urlencode "password=${ADMIN_PASSWORD}" \
            --data-urlencode 'expiring=false' \
            "${BACKEND_URL%/}/users/admin/login" | jq -er '.session'
    )"
fi

# Check sidecar health and served_by identity
sidecar_health="$(curl --fail --silent --show-error "${SIDECAR_URL%/}/health" 2>/dev/null || true)"
if [[ -z "${sidecar_health}" ]]; then
    printf 'ERROR: Sidecar is unreachable or unhealthy at %s/health\n' "${SIDECAR_URL}" >&2
    exit 1
fi
sidecar_served_by="$(printf '%s' "${sidecar_health}" | jq -r '.served_by // empty' 2>/dev/null || true)"
if [[ "${sidecar_served_by}" != "aspace-shadow-sidecar" ]]; then
    printf 'ERROR: Unexpected sidecar identity at %s/health: %s (expected aspace-shadow-sidecar)\n' "${SIDECAR_URL}" "${sidecar_served_by:-missing}" >&2
    exit 1
fi
printf 'Sidecar health response: %s\n' "${sidecar_health}"

# Function to run canary on a record
run_record_canary() {
    local record_type="$1"
    local record_id="$2"
    local uri="/repositories/${REPO_ID}/${record_type}s/${record_id}"

    printf 'Testing %s canary for %s...\n' "${record_type}" "${uri}"

    local raw_record
    raw_record="$(
        curl --fail --silent --show-error --get \
            -H "X-ArchivesSpace-Session: ${ARCHIVESSPACE_SESSION}" \
            --data-urlencode 'resolve[]=subjects' \
            --data-urlencode 'resolve[]=linked_agents' \
            --data-urlencode 'resolve[]=top_container' \
            --data-urlencode 'resolve[]=container_profile' \
            "${BACKEND_URL%/}${uri}"
    )"

    # Build sidecar batch input payload
    local candidate
    candidate="$(printf '%s' "${raw_record}" | jq -c '.')"

    local batch_payload
    batch_payload="$(
        jq -n \
            --arg uri "${uri}" \
            --arg rtype "${record_type}" \
            --argjson repo "${REPO_ID}" \
            --argjson rec "${candidate}" \
            '{
                records: [
                    {
                        uri: $uri,
                        record_type: $rtype,
                        repository: $repo,
                        record: $rec,
                        prepare_pui_final: true,
                        pui_candidate_record: $rec
                    }
                ]
            }'
    )"

    local sidecar_response
    sidecar_response="$(
        curl --fail --silent --show-error \
            -H 'Content-Type: application/json' \
            --data "${batch_payload}" \
            "${SIDECAR_URL%/}/index/documents"
    )"

    # 1. Assert response envelope
    local served_by
    served_by="$(printf '%s' "${sidecar_response}" | jq -r '.served_by')"
    if [[ "${served_by}" != "archivalist-rust-index-document-builder" ]]; then
        printf 'ERROR: Unexpected served_by: %s\n' "${served_by}" >&2
        exit 1
    fi

    local doc
    doc="$(printf '%s' "${sidecar_response}" | jq -c '.documents[0]')"

    # 2. Assert root document privacy invariants
    local has_root_ancestors
    has_root_ancestors="$(printf '%s' "${doc}" | jq 'has("ancestors")')"
    if [[ "${has_root_ancestors}" != "false" ]]; then
        printf 'ERROR: Privacy violation: document retains root ancestors!\n' >&2
        exit 1
    fi

    local has_fullrecord
    has_fullrecord="$(printf '%s' "${doc}" | jq 'has("fullrecord")')"
    if [[ "${has_fullrecord}" != "false" ]]; then
        printf 'ERROR: Privacy violation: document has unpublished fullrecord field!\n' >&2
        exit 1
    fi

    # Assert only expected handoff markers exist on root doc
    local extra_handoff_keys
    extra_handoff_keys="$(printf '%s' "${doc}" | jq -r '[keys[] | select(startswith("_archivalist_") and . != "_archivalist_rust_fields" and . != "_archivalist_pui_final" and . != "_archivalist_pui_routing" and . != "_archivalist_pui_notes")] | join(", ")')"
    if [[ -n "${extra_handoff_keys}" ]]; then
        printf 'ERROR: Privacy violation: unexpected handoff fields on document: %s\n' "${extra_handoff_keys}" >&2
        exit 1
    fi

    # 3. Assert handoff markers
    local rust_fields
    rust_fields="$(printf '%s' "${doc}" | jq -r '._archivalist_rust_fields == ["pui_final_document_metadata", "pui_routing_metadata", "pui_notes_metadata"]')"
    if [[ "${rust_fields}" != "true" ]]; then
        printf 'ERROR: Missing or invalid _archivalist_rust_fields marker: %s\n' "$(printf '%s' "${doc}" | jq -c '._archivalist_rust_fields')" >&2
        exit 1
    fi

    local pui_marker
    pui_marker="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_final.marker')"
    if [[ "${pui_marker}" != "pui_final_document_metadata" ]]; then
        printf 'ERROR: Missing or invalid _archivalist_pui_final.marker: %s\n' "${pui_marker}" >&2
        exit 1
    fi

    # 3b. Assert PUI routing projection
    local routing_marker
    routing_marker="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_routing.marker')"
    if [[ "${routing_marker}" != "pui_routing_metadata" ]]; then
        printf 'ERROR: Missing or invalid _archivalist_pui_routing.marker: %s\n' "${routing_marker}" >&2
        exit 1
    fi

    local routing_keys_match
    routing_keys_match="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_routing | keys | sort == ["id", "marker", "pui_parent_id", "types"]')"
    if [[ "${routing_keys_match}" != "true" ]]; then
        printf 'ERROR: Unexpected or missing keys in _archivalist_pui_routing: %s\n' "$(printf '%s' "${doc}" | jq -c '._archivalist_pui_routing | keys')" >&2
        exit 1
    fi

    local expected_pui_id="${uri}#pui"
    local actual_pui_id
    actual_pui_id="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_routing.id')"
    if [[ "${actual_pui_id}" != "${expected_pui_id}" ]]; then
        printf 'ERROR: Invalid _archivalist_pui_routing.id: expected %s, got %s\n' "${expected_pui_id}" "${actual_pui_id}" >&2
        exit 1
    fi

    local actual_pui_parent_id
    actual_pui_parent_id="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_routing.pui_parent_id')"
    if [[ "${actual_pui_parent_id}" != "${uri}" ]]; then
        printf 'ERROR: Invalid _archivalist_pui_routing.pui_parent_id: expected %s, got %s\n' "${uri}" "${actual_pui_parent_id}" >&2
        exit 1
    fi

    local routing_types_match
    routing_types_match="$(printf '%s' "${doc}" | jq -r --arg rtype "${record_type}" '._archivalist_pui_routing.types == ["pui", ("pui_" + $rtype), "pui_record", "pui_only"]')"
    if [[ "${routing_types_match}" != "true" ]]; then
        printf 'ERROR: Invalid _archivalist_pui_routing.types: %s\n' "$(printf '%s' "${doc}" | jq -c '._archivalist_pui_routing.types')" >&2
        exit 1
    fi

    # 3c. Assert PUI notes projection
    local notes_marker
    notes_marker="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_notes.marker')"
    if [[ "${notes_marker}" != "pui_notes_metadata" ]]; then
        printf 'ERROR: Missing or invalid _archivalist_pui_notes.marker: %s\n' "${notes_marker}" >&2
        exit 1
    fi

    local notes_keys_match
    notes_keys_match="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_notes | keys | sort == ["marker", "notes_published"]')"
    if [[ "${notes_keys_match}" != "true" ]]; then
        printf 'ERROR: Unexpected or missing keys in _archivalist_pui_notes: %s\n' "$(printf '%s' "${doc}" | jq -c '._archivalist_pui_notes | keys')" >&2
        exit 1
    fi

    local has_notes_published_key
    has_notes_published_key="$(printf '%s' "${doc}" | jq '._archivalist_pui_notes | has("notes_published")')"
    if [[ "${has_notes_published_key}" != "true" ]]; then
        printf 'ERROR: _archivalist_pui_notes missing notes_published key\n' >&2
        exit 1
    fi

    # 4. Assert clean JSON (no ancestors, no internal_note, no container_profile notes, no _archivalist_* keys)
    local pui_json_str
    pui_json_str="$(printf '%s' "${doc}" | jq -r '._archivalist_pui_final.json')"
    local pui_json
    pui_json="$(printf '%s' "${pui_json_str}" | jq -c '.')"

    if printf '%s' "${pui_json}" | jq -e 'has("ancestors")' >/dev/null; then
        printf 'ERROR: Privacy violation: candidate JSON retains ancestors!\n' >&2
        exit 1
    fi

    local internal_note_count
    internal_note_count="$(printf '%s' "${pui_json}" | jq '[.. | .internal_note? // empty] | length')"
    if [[ "${internal_note_count}" -ne 0 ]]; then
        printf 'ERROR: Privacy violation: candidate JSON retains top_container internal_note (%s found)!\n' "${internal_note_count}" >&2
        exit 1
    fi

    local cp_notes_count
    cp_notes_count="$(printf '%s' "${pui_json}" | jq '[.instances[]?.sub_container?.top_container?._resolved?.container_profile?._resolved?.notes? // empty] | length')"
    if [[ "${cp_notes_count}" -ne 0 ]]; then
        printf 'ERROR: Privacy violation: candidate JSON retains container_profile notes (%s found)!\n' "${cp_notes_count}" >&2
        exit 1
    fi

    local json_archivalist_count
    json_archivalist_count="$(printf '%s' "${pui_json}" | jq '[paths | map(tostring)[] | select(startswith("_archivalist_"))] | length')"
    if [[ "${json_archivalist_count}" -ne 0 ]]; then
        printf 'ERROR: Privacy violation: candidate JSON contains _archivalist_* keys (%s found)!\n' "${json_archivalist_count}" >&2
        exit 1
    fi

    # 5. Assert clean fullrecord_published
    local pub_array_type
    pub_array_type="$(printf '%s' "${doc}" | jq '._archivalist_pui_final.fullrecord_published | type')"
    if [[ "${pub_array_type}" != '"array"' ]]; then
        printf 'ERROR: fullrecord_published must be an array, got %s\n' "${pub_array_type}" >&2
        exit 1
    fi

    local pub_leak_count
    pub_leak_count="$(printf '%s' "${doc}" | jq '[._archivalist_pui_final.fullrecord_published[] | select(tostring | (contains("internal_note") or contains("_archivalist_")))] | length')"
    if [[ "${pub_leak_count}" -ne 0 ]]; then
        printf 'ERROR: Privacy violation: fullrecord_published leaks private internal or handoff strings!\n' >&2
        exit 1
    fi

    printf '  [PASS] Privacy assertions verified: root/JSON ancestors removed, internal_note/profile notes removed, fullrecord suppressed.\n'
    printf '  [PASS] Published strings count: %s\n' "$(printf '%s' "${doc}" | jq '._archivalist_pui_final.fullrecord_published | length')"
}

if [[ -n "${ARCHIVAL_OBJECT_ID}" ]]; then
    run_record_canary "archival_object" "${ARCHIVAL_OBJECT_ID}"
fi

if [[ -n "${RESOURCE_ID}" ]]; then
    run_record_canary "resource" "${RESOURCE_ID}"
fi

printf '\nCanary run completed successfully. Privacy-normalized PUI document preparation verified.\n'
