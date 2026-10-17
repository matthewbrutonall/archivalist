#!/usr/bin/env bash
set -euo pipefail

BACKEND_URL="${BACKEND_URL:-http://127.0.0.1:4567}"
SIDECAR_URL="${SIDECAR_URL:-http://127.0.0.1:4570}"
SOLR_URL="${SOLR_URL:-http://127.0.0.1:8985/solr/archivesspace}"
REPO_ID="${REPO_ID:?Set REPO_ID to a non-production repository ID}"
RESOURCE_ID="${RESOURCE_ID:-}"
ACCESSION_ID="${ACCESSION_ID:-}"
SENTINEL_ACCESSION_ID="${SENTINEL_ACCESSION_ID:-}"
ACCESSION_DATE_UNKNOWN_LABEL="${ACCESSION_DATE_UNKNOWN_LABEL:-Date Unknown}"
ITERATIONS="${ITERATIONS:-10}"
ARKS_ENABLED="${ARKS_ENABLED:-false}"

if [[ -z "${RESOURCE_ID}" && -z "${ACCESSION_ID}" && -z "${SENTINEL_ACCESSION_ID}" ]]; then
    printf 'Set at least one of RESOURCE_ID, ACCESSION_ID, or SENTINEL_ACCESSION_ID\n' >&2
    exit 1
fi

case "${ARKS_ENABLED}" in
    true|TRUE|1|yes|YES)
        arks_enabled_bool=true
        ;;
    false|FALSE|0|no|NO|"")
        arks_enabled_bool=false
        ;;
    *)
        printf 'Invalid ARKS_ENABLED value: %s (expected boolean true or false)\n' "${ARKS_ENABLED}" >&2
        exit 1
        ;;
esac

if [[ -z "${ARCHIVESSPACE_SESSION:-}" ]]; then
    : "${ADMIN_PASSWORD:?Set ARCHIVESSPACE_SESSION or ADMIN_PASSWORD}"
    ARCHIVESSPACE_SESSION="$(
        curl --fail --silent --show-error \
            --data-urlencode "password=${ADMIN_PASSWORD}" \
            --data-urlencode 'expiring=false' \
            "${BACKEND_URL%/}/users/admin/login" | jq -er '.session'
    )"
fi

if [[ -n "${RESOURCE_ID}" ]]; then
    record="$(
        curl --fail --silent --show-error --get \
            -H "X-ArchivesSpace-Session: ${ARCHIVESSPACE_SESSION}" \
            --data-urlencode 'resolve[]=subjects' \
            --data-urlencode 'resolve[]=linked_agents' \
            --data-urlencode 'resolve[]=repository' \
            --data-urlencode 'resolve[]=classifications' \
            "${BACKEND_URL%/}/repositories/${REPO_ID}/resources/${RESOURCE_ID}"
    )"

    native_document="$(
        curl --fail --silent --show-error --get \
            --data-urlencode "q=id:\"/repositories/${REPO_ID}/resources/${RESOURCE_ID}\"" \
            --data-urlencode 'rows=1' \
            --data-urlencode 'wt=json' \
            "${SOLR_URL%/}/select" | jq -ec '.response.docs[0]'
    )"

    if [[ "${native_document}" == "null" || -z "${native_document}" ]]; then
        printf 'Could not find native Solr document for resource /repositories/%s/resources/%s at %s\n' \
            "${REPO_ID}" "${RESOURCE_ID}" "${SOLR_URL}" >&2
        exit 1
    fi

    dynamic_enum_fields="$(
        printf '%s' "${native_document}" | jq -c '
            [keys[] | select(endswith("_enum_s")) | sub("_enum_s$"; "")] | sort | unique
        '
    )"

    payload="$(
        printf '%s' "${record}" | jq -c \
            --argjson repository "${REPO_ID}" \
            --argjson enum_fields "${dynamic_enum_fields}" \
            --argjson arks_enabled "${arks_enabled_bool}" \
            '{records:[{
                uri:.uri,
                record_type:.jsonmodel_type,
                repository:$repository,
                record:.,
                enrich:true,
                dynamic_enum_fields:$enum_fields,
                arks_enabled:$arks_enabled
            }]}'
    )"

    for iteration in $(seq 1 "${ITERATIONS}"); do
        rust_document="$(
            curl --fail --silent --show-error \
                -H 'Content-Type: application/json' \
                --data-binary "${payload}" \
                "${SIDECAR_URL%/}/index/documents" | jq -ec '.documents[0]'
        )"

        if ! printf '%s\n%s\n' "${rust_document}" "${native_document}" | jq -se \
            --argjson arks_enabled "${arks_enabled_bool}" '
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
            ($rust | [keys[] | select(endswith("_enum_s"))] | sort) as $rust_enum_keys |
            ($native | [keys[] | select(endswith("_enum_s"))] | sort) as $native_enum_keys |
            ($fields | all(($rust[.] | ary) == ($native[.] | ary))) and
            (((($rust.notes_published // []) + ($rust.notes // [])) | sort) ==
              (($native.notes // []) | sort)) and
            ($rust_enum_keys == $native_enum_keys) and
            ($rust_enum_keys | all($rust[.] == $native[.])) and
            (
              if $arks_enabled then
                (($rust | has("ark_name")) == ($native | has("ark_name"))) and
                ($rust.ark_name == $native.ark_name)
              else
                (($rust | has("ark_name") | not) and ($native | has("ark_name") | not))
              end
            ) and
            ($rust._archivalist_rust_fields == [
              "common_metadata", "fullrecord", "resource_archival_object_metadata", "dynamic_enum_and_ark_metadata"
            ])
          ' >/dev/null; then
            printf 'index-document resource enrichment parity failed at iteration %s\n' "${iteration}" >&2
            exit 1
        fi
    done

    printf 'index-document enrichment passed: %s normalized exact iterations for resource %s/%s (arks_enabled=%s)\n' \
        "${ITERATIONS}" "${REPO_ID}" "${RESOURCE_ID}" "${arks_enabled_bool}"
fi

if [[ -n "${ACCESSION_ID}" ]]; then
    acc_record="$(
        curl --fail --silent --show-error --get \
            -H "X-ArchivesSpace-Session: ${ARCHIVESSPACE_SESSION}" \
            --data-urlencode 'resolve[]=subjects' \
            --data-urlencode 'resolve[]=linked_agents' \
            --data-urlencode 'resolve[]=repository' \
            --data-urlencode 'resolve[]=classifications' \
            "${BACKEND_URL%/}/repositories/${REPO_ID}/accessions/${ACCESSION_ID}"
    )"

    acc_native_document="$(
        curl --fail --silent --show-error --get \
            --data-urlencode "q=id:\"/repositories/${REPO_ID}/accessions/${ACCESSION_ID}\"" \
            --data-urlencode 'rows=1' \
            --data-urlencode 'wt=json' \
            "${SOLR_URL%/}/select" | jq -ec '.response.docs[0]'
    )"

    if [[ "${acc_native_document}" == "null" || -z "${acc_native_document}" ]]; then
        printf 'Could not find native Solr document for accession /repositories/%s/accessions/%s at %s\n' \
            "${REPO_ID}" "${ACCESSION_ID}" "${SOLR_URL}" >&2
        exit 1
    fi

    acc_enum_fields="$(
        printf '%s' "${acc_native_document}" | jq -c '
            [keys[] | select(endswith("_enum_s")) | sub("_enum_s$"; "")] | sort | unique
        '
    )"

    acc_payload="$(
        printf '%s\n%s\n' "${acc_record}" "${acc_native_document}" | jq -cs \
            --argjson repository "${REPO_ID}" \
            --argjson enum_fields "${acc_enum_fields}" \
            --argjson arks_enabled "${arks_enabled_bool}" \
            --arg unknown_label "${ACCESSION_DATE_UNKNOWN_LABEL}" \
            '.[0] as $rec | .[1] as $native |
            {records:[{
                uri:$rec.uri,
                record_type:$rec.jsonmodel_type,
                repository:$repository,
                record:$rec,
                enrich:true,
                dynamic_enum_fields:$enum_fields,
                arks_enabled:$arks_enabled,
                accession_date_unknown_label:$unknown_label,
                parsed_accession_date_year:$native.accession_date_year
            }]}'
    )"

    for iteration in $(seq 1 "${ITERATIONS}"); do
        rust_document="$(
            curl --fail --silent --show-error \
                -H 'Content-Type: application/json' \
                --data-binary "${acc_payload}" \
                "${SIDECAR_URL%/}/index/documents" | jq -ec '.documents[0]'
        )"

        if ! printf '%s\n%s\n' "${rust_document}" "${acc_native_document}" | jq -se \
            --argjson arks_enabled "${arks_enabled_bool}" \
            --arg unknown_label "${ACCESSION_DATE_UNKNOWN_LABEL}" '
            def ary:
              if . == null then []
              elif type == "array" then map(tostring)
              else [tostring]
              end;
            .[0] as $rust | .[1] as $native |
            [
              "subjects", "subject_uris", "agents", "agent_uris",
              "published_agents", "published_agent_uris",
              "notes_published", "dates", "years", "level", "summary", "extents",
              "created_by", "last_modified_by", "user_mtime", "system_mtime", "create_time",
              "title", "identifier", "four_part_id", "acquisition_type", "resource_type",
              "restrictions_apply", "access_restrictions", "use_restrictions",
              "related_resource_uris", "related_accession_uris", "slug", "is_slug_auto",
              "processing_priority", "processors", "external_id", "classification_paths",
              "classification_uris", "ancestors", "langcode"
            ] as $fields |
            ($rust | [keys[] | select(endswith("_enum_s"))] | sort) as $rust_enum_keys |
            ($native | [keys[] | select(endswith("_enum_s"))] | sort) as $native_enum_keys |
            ($fields | all(($rust[.] | ary) == ($native[.] | ary))) and
            ($rust_enum_keys == $native_enum_keys) and
            ($rust_enum_keys | all($rust[.] == $native[.])) and
            ($rust.accession_date == $native.accession_date) and
            ($rust.accession_date_year == $native.accession_date_year) and
            (($rust.accession_date_year | type) == "number") and
            ($rust._archivalist_rust_fields == [
              "common_metadata", "fullrecord", "accession_digital_object_metadata", "accession_date_metadata", "dynamic_enum_and_ark_metadata"
            ])
          ' >/dev/null; then
            printf 'index-document accession enrichment parity failed at iteration %s\n' "${iteration}" >&2
            exit 1
        fi
    done

    printf 'index-document enrichment passed: %s normalized exact iterations for accession %s/%s\n' \
        "${ITERATIONS}" "${REPO_ID}" "${ACCESSION_ID}"
fi

if [[ -n "${SENTINEL_ACCESSION_ID}" ]]; then
    sentinel_record="$(
        curl --fail --silent --show-error --get \
            -H "X-ArchivesSpace-Session: ${ARCHIVESSPACE_SESSION}" \
            --data-urlencode 'resolve[]=subjects' \
            --data-urlencode 'resolve[]=linked_agents' \
            --data-urlencode 'resolve[]=repository' \
            --data-urlencode 'resolve[]=classifications' \
            "${BACKEND_URL%/}/repositories/${REPO_ID}/accessions/${SENTINEL_ACCESSION_ID}"
    )"

    sentinel_native_document="$(
        curl --fail --silent --show-error --get \
            --data-urlencode "q=id:\"/repositories/${REPO_ID}/accessions/${SENTINEL_ACCESSION_ID}\"" \
            --data-urlencode 'rows=1' \
            --data-urlencode 'wt=json' \
            "${SOLR_URL%/}/select" | jq -ec '.response.docs[0]'
    )"

    if [[ "${sentinel_native_document}" == "null" || -z "${sentinel_native_document}" ]]; then
        printf 'Could not find native Solr document for sentinel accession /repositories/%s/accessions/%s at %s\n' \
            "${REPO_ID}" "${SENTINEL_ACCESSION_ID}" "${SOLR_URL}" >&2
        exit 1
    fi

    sentinel_enum_fields="$(
        printf '%s' "${sentinel_native_document}" | jq -c '
            [keys[] | select(endswith("_enum_s")) | sub("_enum_s$"; "")] | sort | unique
        '
    )"

    sentinel_payload="$(
        printf '%s\n%s\n' "${sentinel_record}" "${sentinel_native_document}" | jq -cs \
            --argjson repository "${REPO_ID}" \
            --argjson enum_fields "${sentinel_enum_fields}" \
            --argjson arks_enabled "${arks_enabled_bool}" \
            --arg unknown_label "${ACCESSION_DATE_UNKNOWN_LABEL}" \
            '.[0] as $rec | .[1] as $native |
            {records:[{
                uri:$rec.uri,
                record_type:$rec.jsonmodel_type,
                repository:$repository,
                record:$rec,
                enrich:true,
                dynamic_enum_fields:$enum_fields,
                arks_enabled:$arks_enabled,
                accession_date_unknown_label:$unknown_label,
                parsed_accession_date_year:($native.accession_date_year // 9999)
            }]}'
    )"

    for iteration in $(seq 1 "${ITERATIONS}"); do
        rust_document="$(
            curl --fail --silent --show-error \
                -H 'Content-Type: application/json' \
                --data-binary "${sentinel_payload}" \
                "${SIDECAR_URL%/}/index/documents" | jq -ec '.documents[0]'
        )"

        if ! printf '%s\n%s\n' "${rust_document}" "${sentinel_native_document}" | jq -se \
            --argjson arks_enabled "${arks_enabled_bool}" \
            --arg unknown_label "${ACCESSION_DATE_UNKNOWN_LABEL}" '
            def ary:
              if . == null then []
              elif type == "array" then map(tostring)
              else [tostring]
              end;
            .[0] as $rust | .[1] as $native |
            ($rust.accession_date == $native.accession_date) and
            ($rust.accession_date == $unknown_label) and
            ($rust.accession_date_year == 9999) and
            (($rust.accession_date_year | type) == "number") and
            ((($rust.fullrecord // [])[-1]) == ($unknown_label + " ")) and
            ($rust._archivalist_rust_fields == [
              "common_metadata", "fullrecord", "accession_digital_object_metadata", "accession_date_metadata", "dynamic_enum_and_ark_metadata"
            ])
          ' >/dev/null; then
            printf 'index-document sentinel accession enrichment parity failed at iteration %s\n' "${iteration}" >&2
            exit 1
        fi
    done

    printf 'index-document enrichment passed: %s normalized exact iterations for sentinel accession %s/%s\n' \
        "${ITERATIONS}" "${REPO_ID}" "${SENTINEL_ACCESSION_ID}"
fi
