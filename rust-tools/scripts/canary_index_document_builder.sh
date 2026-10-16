#!/usr/bin/env bash
set -euo pipefail

BACKEND_URL="${BACKEND_URL:-http://127.0.0.1:4567}"
SIDECAR_URL="${SIDECAR_URL:-http://127.0.0.1:4570}"
REPO_ID="${REPO_ID:?Set REPO_ID to a non-production repository ID}"
RESOURCE_ID="${RESOURCE_ID:?Set RESOURCE_ID to a representative resource ID}"
ITERATIONS="${ITERATIONS:-10}"

if [[ -z "${ARCHIVESSPACE_SESSION:-}" ]]; then
    : "${ADMIN_PASSWORD:?Set ARCHIVESSPACE_SESSION or ADMIN_PASSWORD}"
    ARCHIVESSPACE_SESSION="$(
        curl --fail --silent --show-error \
            --data-urlencode "password=${ADMIN_PASSWORD}" \
            --data-urlencode 'expiring=false' \
            "${BACKEND_URL%/}/users/admin/login" | jq -er '.session'
    )"
fi

record="$(
    curl --fail --silent --show-error \
        -H "X-ArchivesSpace-Session: ${ARCHIVESSPACE_SESSION}" \
        "${BACKEND_URL%/}/repositories/${REPO_ID}/resources/${RESOURCE_ID}"
)"

payload="$(
    printf '%s' "${record}" | jq -c --argjson repository "${REPO_ID}" \
        '{records:[{uri:.uri,record_type:.jsonmodel_type,repository:$repository,record:.}]}'
)"

for iteration in $(seq 1 "${ITERATIONS}"); do
    response="$(
        curl --fail --silent --show-error \
            -H 'Content-Type: application/json' \
            --data-binary "${payload}" \
            "${SIDECAR_URL%/}/index/documents"
    )"

    if ! printf '%s\n%s\n' "${record}" "${response}" | jq -se \
        --argjson repository "${REPO_ID}" '
          .[0] as $record | .[1] as $response |
          ($response.served_by == "archivalist-rust-index-document-builder") and
          ($response.documents | length == 1) and
          ($response.documents[0] as $document |
            ($document.id == $record.uri) and
            ($document.uri == $record.uri) and
            ($document.title == $record.title) and
            ($document.primary_type == $record.jsonmodel_type) and
            ($document.types == [$record.jsonmodel_type]) and
            ($document.repository == $repository) and
            (($document.json | fromjson) == $record) and
            ($document.suppressed == ($record.suppressed // false)) and
            ($document.publish == (
              (($record.suppressed // false) | not) and
              (($record.has_unpublished_ancestor // false) | not) and
              (($record.repository._resolved.publish == false) | not) and
              ($record.publish // false)
            )))
        ' >/dev/null; then
        printf 'index-document parity failed at iteration %s\n' "${iteration}" >&2
        exit 1
    fi
done

printf 'index-document contract passed: %s exact iterations for resource %s/%s\n' \
    "${ITERATIONS}" "${REPO_ID}" "${RESOURCE_ID}"
