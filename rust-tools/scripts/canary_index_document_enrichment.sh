#!/usr/bin/env bash
set -euo pipefail

BACKEND_URL="${BACKEND_URL:-http://127.0.0.1:4567}"
SIDECAR_URL="${SIDECAR_URL:-http://127.0.0.1:4570}"
SOLR_URL="${SOLR_URL:-http://127.0.0.1:8985/solr/archivesspace}"
REPO_ID="${REPO_ID:?Set REPO_ID to a non-production repository ID}"
RESOURCE_ID="${RESOURCE_ID:?Set RESOURCE_ID to an already natively indexed resource ID}"
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
    curl --fail --silent --show-error --get \
        -H "X-ArchivesSpace-Session: ${ARCHIVESSPACE_SESSION}" \
        --data-urlencode 'resolve[]=subjects' \
        --data-urlencode 'resolve[]=linked_agents' \
        --data-urlencode 'resolve[]=repository' \
        --data-urlencode 'resolve[]=classifications' \
        "${BACKEND_URL%/}/repositories/${REPO_ID}/resources/${RESOURCE_ID}"
)"

payload="$(
    printf '%s' "${record}" | jq -c --argjson repository "${REPO_ID}" \
        '{records:[{uri:.uri,record_type:.jsonmodel_type,repository:$repository,record:.,enrich:true}]}'
)"

native_document="$(
    curl --fail --silent --show-error --get \
        --data-urlencode "q=id:\"/repositories/${REPO_ID}/resources/${RESOURCE_ID}\"" \
        --data-urlencode 'rows=1' \
        --data-urlencode 'wt=json' \
        "${SOLR_URL%/}/select" | jq -ec '.response.docs[0]'
)"

for iteration in $(seq 1 "${ITERATIONS}"); do
    rust_document="$(
        curl --fail --silent --show-error \
            -H 'Content-Type: application/json' \
            --data-binary "${payload}" \
            "${SIDECAR_URL%/}/index/documents" | jq -ec '.documents[0]'
    )"

    if ! printf '%s\n%s\n' "${rust_document}" "${native_document}" | jq -se '
        def ary:
          if . == null then []
          elif type == "array" then map(tostring)
          else [tostring]
          end;
        .[0] as $rust | .[1] as $native |
        [
          "subjects", "subject_uris", "agents", "agent_uris",
          "published_agents", "published_agent_uris", "creators",
          "notes_published", "dates", "years", "level", "summary", "extents",
          "created_by", "last_modified_by", "user_mtime", "system_mtime", "create_time",
          "finding_aid_title", "finding_aid_filing_title", "identifier", "four_part_id",
          "resource_type", "restrictions", "ead_id", "finding_aid_status",
          "related_accession_uris", "slug", "is_slug_auto", "processing_priority",
          "processors", "external_id", "classification_paths", "classification_uris",
          "digital_object_uris", "top_container_uri_u_sstr", "child_container_u_sstr",
          "grand_child_container_u_sstr", "ancestors", "rights_statement_agent_uris",
          "langcode"
        ] as $fields |
        ($fields | all(($rust[.] | ary) == ($native[.] | ary))) and
        (((($rust.notes_published // []) + ($rust.notes // [])) | sort) ==
          (($native.notes // []) | sort)) and
        ($rust._archivalist_rust_fields == [
          "common_metadata", "fullrecord", "resource_archival_object_metadata"
        ])
      ' >/dev/null; then
        printf 'index-document enrichment parity failed at iteration %s\n' "${iteration}" >&2
        exit 1
    fi
done

printf 'index-document enrichment passed: %s normalized exact iterations for resource %s/%s\n' \
    "${ITERATIONS}" "${REPO_ID}" "${RESOURCE_ID}"
