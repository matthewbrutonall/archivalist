use anyhow::{bail, Context, Result};
use aspace_client::index_document::COLLECTION_MANAGEMENT_DOCUMENTS_KEY;
use aspace_client::isolated_solr_writer::IsolatedSolrStaffAoWriter;
use aspace_client::{
    build_index_document_batch, Client, IndexDocumentBatchRequest, IndexDocumentInput,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

pub use crate::staff_ao_candidate::fetch_and_build_ao_candidate_batch;
pub use crate::staff_candidate_runner::{
    partition_ids_into_batches, render_candidate_summary_to_writer, CandidateBatchOutput,
    StaffCandidateArgs, StaffCandidateArgs as StaffCombinedCandidateArgs,
    DEFAULT_CANDIDATE_BATCH_SIZE, DEFAULT_CANDIDATE_CONCURRENCY, MAX_CANDIDATE_BATCH_SIZE,
    MAX_CANDIDATE_CONCURRENCY,
};
pub use crate::staff_resource_candidate::fetch_and_build_resource_candidate_batch;

/// Extracts synthetic `collection_management` child documents from built parent digital object documents
/// in native hook order (parent, then child docs), preserving exact child JSON and identity,
/// applies PUI availability types for published digital objects,
/// and strips all top-level `_archivalist_*` private markers from both parent and child docs.
///
/// Fails closed if child envelope is malformed, not an array, child is not an object,
/// or child identity/envelope does not match expected parent URI.
pub fn extract_digital_object_batch_documents(
    batch_documents: Vec<Value>,
    repo_id: u32,
) -> Result<CandidateBatchOutput> {
    let expected_do_prefix = format!("/repositories/{}/digital_objects/", repo_id);
    let expected_cm_prefix = format!("/repositories/{}/collection_management/", repo_id);

    let mut ordered_documents = Vec::new();
    let mut parent_count = 0;
    let mut child_count = 0;
    let mut parent_uris = Vec::new();
    let mut child_uris = Vec::new();

    for mut parent_doc in batch_documents {
        let parent_obj = parent_doc
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Built parent document must be a JSON object"))?;

        let parent_uri = parent_obj
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Built parent document missing string 'uri'"))?
            .to_string();

        if !parent_uri.starts_with(&expected_do_prefix) {
            bail!(
                "Parent digital_object URI '{}' does not match expected prefix '{}'",
                parent_uri,
                expected_do_prefix
            );
        }

        let parent_id_suffix = &parent_uri[expected_do_prefix.len()..];
        if parent_id_suffix.parse::<u32>().is_err() {
            bail!(
                "Parent digital_object URI '{}' contains non-numeric ID suffix '{}'",
                parent_uri,
                parent_id_suffix
            );
        }

        // Extract collection management child documents BEFORE stripping markers
        let cm_value = parent_obj.remove(COLLECTION_MANAGEMENT_DOCUMENTS_KEY);

        // Strip all top-level _archivalist_* markers from parent document
        parent_obj.retain(|k, _| !k.starts_with("_archivalist_"));

        // Match native staff indexer apply_pui_fields: published digital objects receive PUI availability
        // types ["pui_record", "pui_digital_object", "pui"] in addition to ["digital_object"].
        if parent_obj.get("publish").and_then(Value::as_bool) == Some(true) {
            let types = parent_obj
                .entry("types".to_string())
                .or_insert_with(|| Value::Array(vec![Value::String("digital_object".to_string())]));
            if let Value::Array(types_arr) = types {
                for pui_type in ["pui_record", "pui_digital_object", "pui"] {
                    if !types_arr.iter().any(|v| v.as_str() == Some(pui_type)) {
                        types_arr.push(Value::String(pui_type.to_string()));
                    }
                }
            }
        }

        let mut children = Vec::new();
        if let Some(cm_val) = cm_value {
            let cm_array = match cm_val {
                Value::Array(arr) => arr,
                _ => bail!(
                    "Parent digital_object '{}' has malformed {}: expected JSON array",
                    parent_uri,
                    COLLECTION_MANAGEMENT_DOCUMENTS_KEY
                ),
            };

            for (idx, mut child_doc) in cm_array.into_iter().enumerate() {
                let child_obj = child_doc.as_object_mut().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Child document at index {} for parent '{}' is not a JSON object",
                        idx,
                        parent_uri
                    )
                })?;

                let child_id = child_obj
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Child document at index {} for parent '{}' missing string 'id'",
                            idx,
                            parent_uri
                        )
                    })?
                    .to_string();

                let child_uri = child_obj
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Child document at index {} for parent '{}' missing string 'uri'",
                            idx,
                            parent_uri
                        )
                    })?
                    .to_string();

                if child_id != child_uri {
                    bail!(
                        "Child document 'id' ('{}') does not match 'uri' ('{}')",
                        child_id,
                        child_uri
                    );
                }

                if !child_uri.starts_with(&expected_cm_prefix) {
                    bail!(
                        "Child document URI '{}' does not match expected prefix '{}'",
                        child_uri,
                        expected_cm_prefix
                    );
                }

                let child_id_suffix = &child_uri[expected_cm_prefix.len()..];
                if child_id_suffix.parse::<u32>().is_err() {
                    bail!(
                        "Child document URI '{}' contains non-numeric ID suffix '{}'",
                        child_uri,
                        child_id_suffix
                    );
                }

                let parent_id_val = child_obj
                    .get("parent_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Child document '{}' missing string 'parent_id'", child_uri)
                    })?;
                if parent_id_val != parent_uri {
                    bail!(
                        "Child document '{}' parent_id '{}' does not match parent URI '{}'",
                        child_uri,
                        parent_id_val,
                        parent_uri
                    );
                }

                let parent_type_val = child_obj
                    .get("parent_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Child document '{}' missing string 'parent_type'",
                            child_uri
                        )
                    })?;
                if parent_type_val != "digital_object" {
                    bail!(
                        "Child document '{}' parent_type '{}' must be 'digital_object'",
                        child_uri,
                        parent_type_val
                    );
                }

                let primary_type_val = child_obj
                    .get("primary_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Child document '{}' missing string 'primary_type'",
                            child_uri
                        )
                    })?;
                if primary_type_val != "collection_management" {
                    bail!(
                        "Child document '{}' primary_type '{}' must be 'collection_management'",
                        child_uri,
                        primary_type_val
                    );
                }

                // Strip all top-level _archivalist_* markers from child document
                child_obj.retain(|k, _| !k.starts_with("_archivalist_"));

                child_uris.push(child_uri);
                children.push(child_doc);
            }
        }

        // Native hook order: emit parent document first, followed by its child docs
        parent_uris.push(Some(parent_uri));
        ordered_documents.push(parent_doc);
        parent_count += 1;

        for child in children {
            ordered_documents.push(child);
            child_count += 1;
        }
    }

    Ok(CandidateBatchOutput {
        ordered_documents,
        parent_uris,
        child_uris,
        parent_count,
        child_count,
    })
}

pub async fn fetch_and_build_digital_object_candidate_batch(
    client: &Client,
    repo_id: u32,
    chunk: &[u32],
    batch_idx: usize,
    batch_size: usize,
    dynamic_enums: Arc<Vec<String>>,
    arks_enabled: bool,
) -> Result<CandidateBatchOutput> {
    let raw_records = client
        .fetch_staff_digital_objects_by_ids_bounded(repo_id, chunk, batch_size)
        .await
        .with_context(|| format!("Failed to fetch digital_object batch {}", batch_idx))?;

    if raw_records.len() != chunk.len() {
        bail!(
            "Backend returned {} records for batch {}, expected {}",
            raw_records.len(),
            batch_idx,
            chunk.len()
        );
    }

    let repo_scope = Value::String(format!("/repositories/{}", repo_id));
    let mut inputs = Vec::with_capacity(raw_records.len());
    for record in raw_records {
        let uri = record
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Record missing valid string uri"))?
            .to_string();

        let input = IndexDocumentInput {
            uri,
            record_type: "digital_object".to_string(),
            repository: repo_scope.clone(),
            record,
            enrich: true,
            dynamic_enum_fields: (*dynamic_enums).clone(),
            arks_enabled,
            ..Default::default()
        };
        inputs.push(input);
    }

    let batch_resp = build_index_document_batch(IndexDocumentBatchRequest { records: inputs })
        .map_err(|e| anyhow::anyhow!("Failed to build index document batch: {e}"))?;

    extract_digital_object_batch_documents(batch_resp.documents, repo_id)
}

/// Validates repository scope and numeric ID suffix for top containers,
/// strips all top-level `_archivalist_*` private markers,
/// and applies publication status and PUI availability types matching native staff indexer:
/// `doc['publish'] = is_linked_to_published_record`
/// and if published, adds `["pui_container", "pui"]` to `types`.
/// Fails closed if document is malformed or URI/ID does not match repository scope.
pub fn extract_top_container_batch_documents(
    batch_documents: Vec<Value>,
    repo_id: u32,
) -> Result<CandidateBatchOutput> {
    let expected_tc_prefix = format!("/repositories/{}/top_containers/", repo_id);

    let mut ordered_documents = Vec::new();
    let mut parent_count = 0;
    let mut parent_uris = Vec::new();

    for mut parent_doc in batch_documents {
        let parent_obj = parent_doc
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Built parent document must be a JSON object"))?;

        let parent_uri = parent_obj
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Built parent document missing string 'uri'"))?
            .to_string();

        if !parent_uri.starts_with(&expected_tc_prefix) {
            bail!(
                "Parent top_container URI '{}' does not match expected prefix '{}'",
                parent_uri,
                expected_tc_prefix
            );
        }

        let parent_id_suffix = &parent_uri[expected_tc_prefix.len()..];
        if parent_id_suffix.parse::<u32>().is_err() {
            bail!(
                "Parent top_container URI '{}' contains non-numeric ID suffix '{}'",
                parent_uri,
                parent_id_suffix
            );
        }

        if let Some(id_val) = parent_obj.get("id").and_then(Value::as_str) {
            if id_val != parent_uri {
                bail!(
                    "Parent top_container 'id' ('{}') does not match 'uri' ('{}')",
                    id_val,
                    parent_uri
                );
            }
        }

        if let Some(pt) = parent_obj.get("primary_type").and_then(Value::as_str) {
            if pt != "top_container" {
                bail!(
                    "Parent top_container URI '{}' has unexpected primary_type '{}'",
                    parent_uri,
                    pt
                );
            }
        }

        // Strip all top-level _archivalist_* markers from document
        parent_obj.retain(|k, _| !k.starts_with("_archivalist_"));

        // Match native staff indexer apply_pui_fields:
        // doc['publish'] = record['record']['is_linked_to_published_record']
        // if doc['publish']
        //   doc['types'] << 'pui_container'
        //   doc['types'] << 'pui'
        // end
        let is_linked = parent_obj
            .get("is_linked_to_published_record")
            .and_then(Value::as_bool)
            .or_else(|| {
                parent_obj
                    .get("json")
                    .and_then(Value::as_str)
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .and_then(|v| {
                        v.get("is_linked_to_published_record")
                            .and_then(Value::as_bool)
                    })
            })
            .unwrap_or(false);

        parent_obj.insert("publish".to_string(), Value::Bool(is_linked));
        if is_linked {
            let types = parent_obj
                .entry("types".to_string())
                .or_insert_with(|| Value::Array(vec![Value::String("top_container".to_string())]));
            if let Value::Array(types_arr) = types {
                for pui_type in ["pui_container", "pui"] {
                    if !types_arr.iter().any(|v| v.as_str() == Some(pui_type)) {
                        types_arr.push(Value::String(pui_type.to_string()));
                    }
                }
            }
        } else if let Some(Value::Array(types_arr)) = parent_obj.get_mut("types") {
            types_arr.retain(|v| v.as_str() != Some("pui_container") && v.as_str() != Some("pui"));
        }

        parent_uris.push(Some(parent_uri));
        ordered_documents.push(parent_doc);
        parent_count += 1;
    }

    Ok(CandidateBatchOutput {
        ordered_documents,
        parent_uris,
        child_uris: Vec::new(),
        parent_count,
        child_count: 0,
    })
}

pub async fn fetch_and_build_top_container_candidate_batch(
    client: &Client,
    repo_id: u32,
    chunk: &[u32],
    batch_idx: usize,
    batch_size: usize,
    dynamic_enums: Arc<Vec<String>>,
    arks_enabled: bool,
) -> Result<CandidateBatchOutput> {
    let raw_records = client
        .fetch_staff_top_containers_by_ids_bounded(repo_id, chunk, batch_size)
        .await
        .with_context(|| format!("Failed to fetch top_container batch {}", batch_idx))?;

    if raw_records.len() != chunk.len() {
        bail!(
            "Backend returned {} records for batch {}, expected {}",
            raw_records.len(),
            batch_idx,
            chunk.len()
        );
    }

    let repo_scope = Value::String(format!("/repositories/{}", repo_id));
    let mut inputs = Vec::with_capacity(raw_records.len());
    for record in raw_records {
        let uri = record
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Record missing valid string uri"))?
            .to_string();

        let input = IndexDocumentInput {
            uri,
            record_type: "top_container".to_string(),
            repository: repo_scope.clone(),
            record,
            enrich: true,
            dynamic_enum_fields: (*dynamic_enums).clone(),
            arks_enabled,
            ..Default::default()
        };
        inputs.push(input);
    }

    let batch_resp = build_index_document_batch(IndexDocumentBatchRequest { records: inputs })
        .map_err(|e| anyhow::anyhow!("Failed to build index document batch: {e}"))?;

    extract_top_container_batch_documents(batch_resp.documents, repo_id)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypeSliceSummary {
    pub record_type: String,
    pub ids_discovered: usize,
    pub exact_ids_indexed: usize,
    pub parent_docs: usize,
    pub child_docs: usize,
    pub total_docs: usize,
    pub batches: usize,
    pub batches_completed: usize,
    pub batches_failed: usize,
    pub missing_ids: usize,
    pub duplicate_ids: usize,
    pub unexpected_ids: usize,
}

impl TypeSliceSummary {
    pub fn empty(record_type: &str) -> Self {
        Self {
            record_type: record_type.to_string(),
            ids_discovered: 0,
            exact_ids_indexed: 0,
            parent_docs: 0,
            child_docs: 0,
            total_docs: 0,
            batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            missing_ids: 0,
            duplicate_ids: 0,
            unexpected_ids: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StaffCombinedCandidateSummary {
    pub repository_id: u32,
    pub candidate_core: String,
    pub solr_url: String,
    pub arks_enabled: bool,
    pub batch_size: usize,
    pub concurrency: usize,

    pub archival_objects: TypeSliceSummary,
    pub resources: TypeSliceSummary,
    pub digital_objects: TypeSliceSummary,
    pub top_containers: TypeSliceSummary,

    pub total_ids_discovered: usize,
    pub exact_ids_indexed: usize,
    pub parent_docs: usize,
    pub child_docs: usize,
    pub total_docs_indexed: usize,
    pub unique_uris: usize,
    pub total_batches: usize,
    pub batches_completed: usize,
    pub batches_failed: usize,
    pub missing_ids: usize,
    pub duplicate_ids: usize,
    pub unexpected_ids: usize,
    pub failures: usize,

    pub id_list_elapsed_sec: f64,
    pub batch_loop_wall_elapsed_sec: f64,
    pub solr_write_elapsed_sec: f64,
    pub solr_update_requests_sent: usize,
    pub solr_commit_elapsed_sec: f64,
    pub total_wall_elapsed_sec: f64,
    pub records_per_second: f64,

    pub committed: bool,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl StaffCombinedCandidateSummary {
    pub fn failure_preflight(args: &StaffCombinedCandidateArgs, error_message: String) -> Self {
        Self {
            repository_id: args.repo,
            candidate_core: args.candidate_core.clone(),
            solr_url: args.solr_url.clone(),
            arks_enabled: args.arks_enabled,
            batch_size: args.batch_size,
            concurrency: args.concurrency,
            archival_objects: TypeSliceSummary::empty("archival_object"),
            resources: TypeSliceSummary::empty("resource"),
            digital_objects: TypeSliceSummary::empty("digital_object"),
            top_containers: TypeSliceSummary::empty("top_container"),
            total_ids_discovered: 0,
            exact_ids_indexed: 0,
            parent_docs: 0,
            child_docs: 0,
            total_docs_indexed: 0,
            unique_uris: 0,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            missing_ids: 0,
            duplicate_ids: 0,
            unexpected_ids: 0,
            failures: 1,
            id_list_elapsed_sec: 0.0,
            batch_loop_wall_elapsed_sec: 0.0,
            solr_write_elapsed_sec: 0.0,
            solr_update_requests_sent: 0,
            solr_commit_elapsed_sec: 0.0,
            total_wall_elapsed_sec: 0.0,
            records_per_second: 0.0,
            committed: false,
            success: false,
            error_message: Some(error_message),
        }
    }
}

/// Shared mutable state and configuration for the candidate pipeline across all record types.
pub struct SharedCandidateState<'a> {
    pub writer: &'a mut IsolatedSolrStaffAoWriter,
    pub global_unique_uris: &'a mut HashSet<String>,
    pub cross_type_duplicate_uris_count: &'a mut usize,
    pub total_completed_batches: &'a mut usize,
    pub solr_write_elapsed_sec: &'a mut f64,
    pub first_error: &'a mut Option<String>,
    pub concurrency: usize,
    pub fail_after_batches: Option<usize>,
}

/// Executes candidate batch fetching, document construction, URI/ID validation,
/// Solr stage writing, and failure injection for a single record type slice.
///
/// Preserves:
/// - Single writer / single stage core destination
/// - Parent then child document order within each batch
/// - Per-type and global URI uniqueness validation
/// - Exact ID set tracking against expected input IDs
/// - Deterministic failure-injection (`fail_after_batches`)
pub async fn process_type_candidate_batches<F, Fut>(
    type_label: &str,
    expected_prefix: &str,
    expected_ids: &[u32],
    batches: Vec<Vec<u32>>,
    shared: &mut SharedCandidateState<'_>,
    fetch_and_build: F,
) -> (TypeSliceSummary, HashSet<u32>)
where
    F: Fn(usize, Vec<u32>) -> Fut,
    Fut: Future<Output = Result<CandidateBatchOutput>> + Send,
{
    let expected_ids_set: HashSet<u32> = expected_ids.iter().copied().collect();
    let mut slice_summary = TypeSliceSummary {
        record_type: type_label.to_string(),
        ids_discovered: expected_ids.len(),
        exact_ids_indexed: 0,
        parent_docs: 0,
        child_docs: 0,
        total_docs: 0,
        batches: batches.len(),
        batches_completed: 0,
        batches_failed: 0,
        missing_ids: 0,
        duplicate_ids: 0,
        unexpected_ids: 0,
    };
    let mut returned_ids = HashSet::with_capacity(expected_ids.len());

    if batches.is_empty() || shared.first_error.is_some() {
        slice_summary.missing_ids = expected_ids.len().saturating_sub(returned_ids.len());
        return (slice_summary, returned_ids);
    }

    let stream = stream::iter(batches.into_iter().enumerate().map(|(batch_idx, chunk)| {
        let fut = fetch_and_build(batch_idx, chunk.clone());
        async move {
            let res = fut.await;
            (batch_idx, chunk, res)
        }
    }))
    .buffer_unordered(shared.concurrency);

    let mut stream = stream;
    while let Some((batch_idx, chunk, res)) = stream.next().await {
        match res {
            Ok(output) => {
                slice_summary.batches_completed += 1;
                *shared.total_completed_batches += 1;
                slice_summary.parent_docs += output.parent_count;
                slice_summary.child_docs += output.child_count;
                slice_summary.total_docs += output.parent_count + output.child_count;

                for (rec_idx, maybe_uri) in output.parent_uris.into_iter().enumerate() {
                    let uri = match maybe_uri {
                        Some(u) => u,
                        None => {
                            slice_summary.unexpected_ids += 1;
                            if shared.first_error.is_none() {
                                *shared.first_error = Some(format!(
                                    "{type_label} doc at index {rec_idx} in batch {batch_idx} missing string 'uri'"
                                ));
                            }
                            continue;
                        }
                    };

                    if !uri.starts_with(expected_prefix) {
                        slice_summary.unexpected_ids += 1;
                        if shared.first_error.is_none() {
                            *shared.first_error = Some(format!(
                                "{type_label} doc URI '{uri}' does not match expected prefix '{expected_prefix}'"
                            ));
                        }
                        continue;
                    }

                    let id_str = &uri[expected_prefix.len()..];
                    let record_id: u32 = match id_str.parse() {
                        Ok(id) => id,
                        Err(_) => {
                            slice_summary.unexpected_ids += 1;
                            if shared.first_error.is_none() {
                                *shared.first_error = Some(format!(
                                    "{type_label} doc URI '{uri}' contains non-numeric ID '{id_str}'"
                                ));
                            }
                            continue;
                        }
                    };

                    if !expected_ids_set.contains(&record_id) {
                        slice_summary.unexpected_ids += 1;
                        if shared.first_error.is_none() {
                            *shared.first_error = Some(format!(
                                "Returned {type_label} ID {record_id} was not present in repository {type_label} ID list"
                            ));
                        }
                    }

                    if !returned_ids.insert(record_id) {
                        slice_summary.duplicate_ids += 1;
                        if shared.first_error.is_none() {
                            *shared.first_error = Some(format!(
                                "Duplicate record returned for {type_label} ID {record_id} across batches"
                            ));
                        }
                    }

                    if !shared.global_unique_uris.insert(uri.clone()) {
                        *shared.cross_type_duplicate_uris_count += 1;
                        slice_summary.duplicate_ids += 1;
                        if shared.first_error.is_none() {
                            *shared.first_error = Some(format!(
                                "Duplicate document URI '{uri}' encountered in candidate stream"
                            ));
                        }
                    }
                }

                for (child_idx, child_uri) in output.child_uris.into_iter().enumerate() {
                    if !shared.global_unique_uris.insert(child_uri.clone()) {
                        *shared.cross_type_duplicate_uris_count += 1;
                        slice_summary.duplicate_ids += 1;
                        if shared.first_error.is_none() {
                            *shared.first_error = Some(format!(
                                "Duplicate child URI '{child_uri}' at index {child_idx} in {type_label} batch {batch_idx}"
                            ));
                        }
                    }
                }

                // Add documents to writer in bounded chunks
                let t_write = Instant::now();
                if let Err(w_err) = shared.writer.add_documents(output.ordered_documents).await {
                    slice_summary.batches_failed += 1;
                    if shared.first_error.is_none() {
                        *shared.first_error = Some(format!(
                            "Failed to write {type_label} documents to candidate Solr core: {w_err}"
                        ));
                    }
                    break;
                }
                *shared.solr_write_elapsed_sec += t_write.elapsed().as_secs_f64();

                if let Some(fail_limit) = shared.fail_after_batches {
                    if *shared.total_completed_batches >= fail_limit {
                        slice_summary.batches_failed += 1;
                        if shared.first_error.is_none() {
                            *shared.first_error = Some(format!(
                                "Injected deterministic test failure after {} completed batch(es) (fail_after_batches={fail_limit})",
                                *shared.total_completed_batches
                            ));
                        }
                        break;
                    }
                }
            }
            Err(err) => {
                slice_summary.batches_failed += 1;
                if shared.first_error.is_none() {
                    *shared.first_error = Some(format!(
                        "{type_label} batch {batch_idx} (chunk {chunk:?}): {err}"
                    ));
                }
                break;
            }
        }
    }

    slice_summary.exact_ids_indexed = returned_ids.len();
    slice_summary.missing_ids = expected_ids.len().saturating_sub(returned_ids.len());
    (slice_summary, returned_ids)
}

fn check_input_id_duplicates(ids: &[u32]) -> usize {
    let mut set = HashSet::with_capacity(ids.len());
    let mut dups = 0;
    for &id in ids {
        if !set.insert(id) {
            dups += 1;
        }
    }
    dups
}

pub async fn execute_staff_combined_candidate(
    client: &Client,
    args: &StaffCombinedCandidateArgs,
) -> StaffCombinedCandidateSummary {
    execute_staff_combined_candidate_with_verifier(
        client,
        args,
        crate::profiling_stack::verify_profiling_stack,
    )
    .await
}

pub async fn execute_staff_combined_candidate_with_verifier<V>(
    client: &Client,
    args: &StaffCombinedCandidateArgs,
    verifier: V,
) -> StaffCombinedCandidateSummary
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
{
    let repo_id = args.repo;
    let arks_enabled = args.arks_enabled;
    let batch_size = args.batch_size;

    let client_for_ao_ids = client.clone();
    let client_for_res_ids = client.clone();
    let client_for_do_ids = client.clone();
    let client_for_tc_ids = client.clone();
    let client_for_ao_batches = client.clone();
    let client_for_res_batches = client.clone();
    let client_for_do_batches = client.clone();
    let client_for_tc_batches = client.clone();

    execute_staff_combined_candidate_custom(
        client,
        args,
        verifier,
        move || {
            let client = client_for_ao_ids.clone();
            async move {
                client
                    .archival_object_ids_modified_since(repo_id, Some(0))
                    .await
            }
        },
        move || {
            let client = client_for_res_ids.clone();
            async move { client.resource_ids_modified_since(repo_id, Some(0)).await }
        },
        move || {
            let client = client_for_do_ids.clone();
            async move {
                client
                    .digital_object_ids_modified_since(repo_id, Some(0))
                    .await
            }
        },
        move || {
            let client = client_for_tc_ids.clone();
            async move {
                client
                    .top_container_ids_modified_since(repo_id, Some(0))
                    .await
            }
        },
        move |batch_idx, chunk, dynamic_enums| {
            let client = client_for_ao_batches.clone();
            async move {
                fetch_and_build_ao_candidate_batch(
                    &client,
                    repo_id,
                    &chunk,
                    batch_idx,
                    dynamic_enums,
                    arks_enabled,
                )
                .await
            }
        },
        move |batch_idx, chunk, dynamic_enums| {
            let client = client_for_res_batches.clone();
            async move {
                fetch_and_build_resource_candidate_batch(
                    &client,
                    repo_id,
                    &chunk,
                    batch_idx,
                    batch_size,
                    dynamic_enums,
                    arks_enabled,
                )
                .await
            }
        },
        move |batch_idx, chunk, dynamic_enums| {
            let client = client_for_do_batches.clone();
            async move {
                fetch_and_build_digital_object_candidate_batch(
                    &client,
                    repo_id,
                    &chunk,
                    batch_idx,
                    batch_size,
                    dynamic_enums,
                    arks_enabled,
                )
                .await
            }
        },
        move |batch_idx, chunk, dynamic_enums| {
            let client = client_for_tc_batches.clone();
            async move {
                fetch_and_build_top_container_candidate_batch(
                    &client,
                    repo_id,
                    &chunk,
                    batch_idx,
                    batch_size,
                    dynamic_enums,
                    arks_enabled,
                )
                .await
            }
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_staff_combined_candidate_custom<
    V,
    FAoIds,
    FutAoIds,
    EAoIds,
    FResIds,
    FutResIds,
    EResIds,
    FDoIds,
    FutDoIds,
    EDoIds,
    FTcIds,
    FutTcIds,
    ETcIds,
    FAoBatch,
    FutAoBatch,
    FResBatch,
    FutResBatch,
    FDoBatch,
    FutDoBatch,
    FTcBatch,
    FutTcBatch,
>(
    client: &Client,
    args: &StaffCombinedCandidateArgs,
    verifier: V,
    fetch_ao_ids: FAoIds,
    fetch_res_ids: FResIds,
    fetch_do_ids: FDoIds,
    fetch_tc_ids: FTcIds,
    fetch_and_build_ao_batch: FAoBatch,
    fetch_and_build_res_batch: FResBatch,
    fetch_and_build_do_batch: FDoBatch,
    fetch_and_build_tc_batch: FTcBatch,
) -> StaffCombinedCandidateSummary
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
    FAoIds: FnOnce() -> FutAoIds,
    FutAoIds: Future<Output = std::result::Result<Vec<u32>, EAoIds>>,
    EAoIds: std::fmt::Display,
    FResIds: FnOnce() -> FutResIds,
    FutResIds: Future<Output = std::result::Result<Vec<u32>, EResIds>>,
    EResIds: std::fmt::Display,
    FDoIds: FnOnce() -> FutDoIds,
    FutDoIds: Future<Output = std::result::Result<Vec<u32>, EDoIds>>,
    EDoIds: std::fmt::Display,
    FTcIds: FnOnce() -> FutTcIds,
    FutTcIds: Future<Output = std::result::Result<Vec<u32>, ETcIds>>,
    ETcIds: std::fmt::Display,
    FAoBatch: Fn(usize, Vec<u32>, Arc<Vec<String>>) -> FutAoBatch + Send + Sync + 'static,
    FutAoBatch: Future<Output = Result<CandidateBatchOutput>> + Send,
    FResBatch: Fn(usize, Vec<u32>, Arc<Vec<String>>) -> FutResBatch + Send + Sync + 'static,
    FutResBatch: Future<Output = Result<CandidateBatchOutput>> + Send,
    FDoBatch: Fn(usize, Vec<u32>, Arc<Vec<String>>) -> FutDoBatch + Send + Sync + 'static,
    FutDoBatch: Future<Output = Result<CandidateBatchOutput>> + Send,
    FTcBatch: Fn(usize, Vec<u32>, Arc<Vec<String>>) -> FutTcBatch + Send + Sync + 'static,
    FutTcBatch: Future<Output = Result<CandidateBatchOutput>> + Send,
{
    if let Err(err) = args.validate() {
        return StaffCombinedCandidateSummary::failure_preflight(
            args,
            format!("Invalid arguments: {err}"),
        );
    }

    let state_path = args.state.as_ref().unwrap();

    // Verify profiling stack identity BEFORE any Solr write or core readiness check
    if let Err(err) = verifier(state_path, client.base_url(), &args.solr_url) {
        return StaffCombinedCandidateSummary::failure_preflight(
            args,
            format!("Profiling stack verification failed: {err}"),
        );
    }

    // Pre-flight check: construct writer and verify candidate core is ready and empty
    let mut writer = match IsolatedSolrStaffAoWriter::new(&args.solr_url, &args.candidate_core) {
        Ok(w) => w,
        Err(err) => {
            return StaffCombinedCandidateSummary::failure_preflight(
                args,
                format!("Failed to initialize IsolatedSolrStaffAoWriter: {err}"),
            );
        }
    };

    if let Err(err) = writer.verify_target_core_ready().await {
        return StaffCombinedCandidateSummary::failure_preflight(
            args,
            format!(
                "Candidate core '{}' readiness verification failed: {err}",
                args.candidate_core
            ),
        );
    }

    // Pre-flight check: derive dynamic enum fields once from GET /schemas
    let dynamic_enums = match client.dynamic_enum_fields_from_schemas().await {
        Ok(e) => Arc::new(e),
        Err(err) => {
            return StaffCombinedCandidateSummary::failure_preflight(
                args,
                format!("Failed to derive dynamic enum fields from GET /schemas: {err}"),
            );
        }
    };

    // Measurement envelope begins here
    let wall_start = Instant::now();

    // 1. Enumerate IDs for all four types
    let id_start = Instant::now();
    let ao_ids_res = fetch_ao_ids().await;
    let res_ids_res = fetch_res_ids().await;
    let do_ids_res = fetch_do_ids().await;
    let tc_ids_res = fetch_tc_ids().await;
    let id_list_elapsed_sec = id_start.elapsed().as_secs_f64();

    let ao_ids = match ao_ids_res {
        Ok(ids) => ids,
        Err(err) => {
            let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
            let mut res = StaffCombinedCandidateSummary::failure_preflight(
                args,
                format!("Failed to fetch archival object ID list from backend: {err}"),
            );
            res.id_list_elapsed_sec = id_list_elapsed_sec;
            res.total_wall_elapsed_sec = total_wall_elapsed_sec;
            return res;
        }
    };

    let resource_ids = match res_ids_res {
        Ok(ids) => ids,
        Err(err) => {
            let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
            let mut res = StaffCombinedCandidateSummary::failure_preflight(
                args,
                format!("Failed to fetch resource ID list from backend: {err}"),
            );
            res.id_list_elapsed_sec = id_list_elapsed_sec;
            res.total_wall_elapsed_sec = total_wall_elapsed_sec;
            return res;
        }
    };

    let do_ids = match do_ids_res {
        Ok(ids) => ids,
        Err(err) => {
            let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
            let mut res = StaffCombinedCandidateSummary::failure_preflight(
                args,
                format!("Failed to fetch digital object ID list from backend: {err}"),
            );
            res.id_list_elapsed_sec = id_list_elapsed_sec;
            res.total_wall_elapsed_sec = total_wall_elapsed_sec;
            return res;
        }
    };

    let tc_ids = match tc_ids_res {
        Ok(ids) => ids,
        Err(err) => {
            let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
            let mut res = StaffCombinedCandidateSummary::failure_preflight(
                args,
                format!("Failed to fetch top container ID list from backend: {err}"),
            );
            res.id_list_elapsed_sec = id_list_elapsed_sec;
            res.total_wall_elapsed_sec = total_wall_elapsed_sec;
            return res;
        }
    };

    // Check duplicate input IDs in discovered sets
    let ao_dup_input = check_input_id_duplicates(&ao_ids);
    if ao_dup_input > 0 {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        let mut res = StaffCombinedCandidateSummary::failure_preflight(
            args,
            format!(
                "Backend returned {} duplicate ID(s) in repository AO ID list",
                ao_dup_input
            ),
        );
        res.total_ids_discovered = ao_ids.len() + resource_ids.len() + do_ids.len() + tc_ids.len();
        res.archival_objects.ids_discovered = ao_ids.len();
        res.resources.ids_discovered = resource_ids.len();
        res.digital_objects.ids_discovered = do_ids.len();
        res.top_containers.ids_discovered = tc_ids.len();
        res.archival_objects.duplicate_ids = ao_dup_input;
        res.duplicate_ids = ao_dup_input;
        res.id_list_elapsed_sec = id_list_elapsed_sec;
        res.total_wall_elapsed_sec = total_wall_elapsed_sec;
        return res;
    }

    let res_dup_input = check_input_id_duplicates(&resource_ids);
    if res_dup_input > 0 {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        let mut res = StaffCombinedCandidateSummary::failure_preflight(
            args,
            format!(
                "Backend returned {} duplicate ID(s) in repository resource ID list",
                res_dup_input
            ),
        );
        res.total_ids_discovered = ao_ids.len() + resource_ids.len() + do_ids.len() + tc_ids.len();
        res.archival_objects.ids_discovered = ao_ids.len();
        res.resources.ids_discovered = resource_ids.len();
        res.digital_objects.ids_discovered = do_ids.len();
        res.top_containers.ids_discovered = tc_ids.len();
        res.resources.duplicate_ids = res_dup_input;
        res.duplicate_ids = res_dup_input;
        res.id_list_elapsed_sec = id_list_elapsed_sec;
        res.total_wall_elapsed_sec = total_wall_elapsed_sec;
        return res;
    }

    let do_dup_input = check_input_id_duplicates(&do_ids);
    if do_dup_input > 0 {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        let mut res = StaffCombinedCandidateSummary::failure_preflight(
            args,
            format!(
                "Backend returned {} duplicate ID(s) in repository DO ID list",
                do_dup_input
            ),
        );
        res.total_ids_discovered = ao_ids.len() + resource_ids.len() + do_ids.len() + tc_ids.len();
        res.archival_objects.ids_discovered = ao_ids.len();
        res.resources.ids_discovered = resource_ids.len();
        res.digital_objects.ids_discovered = do_ids.len();
        res.top_containers.ids_discovered = tc_ids.len();
        res.digital_objects.duplicate_ids = do_dup_input;
        res.duplicate_ids = do_dup_input;
        res.id_list_elapsed_sec = id_list_elapsed_sec;
        res.total_wall_elapsed_sec = total_wall_elapsed_sec;
        return res;
    }

    let tc_dup_input = check_input_id_duplicates(&tc_ids);
    if tc_dup_input > 0 {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        let mut res = StaffCombinedCandidateSummary::failure_preflight(
            args,
            format!(
                "Backend returned {} duplicate ID(s) in repository top container ID list",
                tc_dup_input
            ),
        );
        res.total_ids_discovered = ao_ids.len() + resource_ids.len() + do_ids.len() + tc_ids.len();
        res.archival_objects.ids_discovered = ao_ids.len();
        res.resources.ids_discovered = resource_ids.len();
        res.digital_objects.ids_discovered = do_ids.len();
        res.top_containers.ids_discovered = tc_ids.len();
        res.top_containers.duplicate_ids = tc_dup_input;
        res.duplicate_ids = tc_dup_input;
        res.id_list_elapsed_sec = id_list_elapsed_sec;
        res.total_wall_elapsed_sec = total_wall_elapsed_sec;
        return res;
    }

    let total_discovered_ids = ao_ids.len() + resource_ids.len() + do_ids.len() + tc_ids.len();

    let res_batches = partition_ids_into_batches(&resource_ids, args.batch_size);
    let do_batches = partition_ids_into_batches(&do_ids, args.batch_size);
    let tc_batches = partition_ids_into_batches(&tc_ids, args.batch_size);
    let ao_batches = partition_ids_into_batches(&ao_ids, args.batch_size);
    let total_batches = res_batches.len() + do_batches.len() + tc_batches.len() + ao_batches.len();

    let expected_ao_prefix = format!("/repositories/{}/archival_objects/", args.repo);
    let expected_res_prefix = format!("/repositories/{}/resources/", args.repo);
    let expected_do_prefix = format!("/repositories/{}/digital_objects/", args.repo);
    let expected_tc_prefix = format!("/repositories/{}/top_containers/", args.repo);

    let mut global_unique_uris = HashSet::with_capacity(total_discovered_ids * 2);
    let mut cross_type_duplicate_uris_count = 0;
    let mut first_error: Option<String> = None;
    let mut solr_write_elapsed_sec = 0.0;
    let mut total_completed_batches = 0;

    let batch_loop_start = Instant::now();

    let mut shared = SharedCandidateState {
        writer: &mut writer,
        global_unique_uris: &mut global_unique_uris,
        cross_type_duplicate_uris_count: &mut cross_type_duplicate_uris_count,
        total_completed_batches: &mut total_completed_batches,
        solr_write_elapsed_sec: &mut solr_write_elapsed_sec,
        first_error: &mut first_error,
        concurrency: args.concurrency,
        fail_after_batches: args.fail_after_batches,
    };

    // 2. Process Resource Batches (preserve native order: resources first)
    let (res_slice_summary, res_returned_ids) = process_type_candidate_batches(
        "resource",
        &expected_res_prefix,
        &resource_ids,
        res_batches,
        &mut shared,
        |batch_idx, chunk| {
            let dynamic_enums = Arc::clone(&dynamic_enums);
            fetch_and_build_res_batch(batch_idx, chunk, dynamic_enums)
        },
    )
    .await;

    // 3. Process Digital Object Batches (digital objects second)
    let (do_slice_summary, do_returned_ids) = process_type_candidate_batches(
        "digital_object",
        &expected_do_prefix,
        &do_ids,
        do_batches,
        &mut shared,
        |batch_idx, chunk| {
            let dynamic_enums = Arc::clone(&dynamic_enums);
            fetch_and_build_do_batch(batch_idx, chunk, dynamic_enums)
        },
    )
    .await;

    // 4. Process Top Container Batches (top containers third)
    let (tc_slice_summary, tc_returned_ids) = process_type_candidate_batches(
        "top_container",
        &expected_tc_prefix,
        &tc_ids,
        tc_batches,
        &mut shared,
        |batch_idx, chunk| {
            let dynamic_enums = Arc::clone(&dynamic_enums);
            fetch_and_build_tc_batch(batch_idx, chunk, dynamic_enums)
        },
    )
    .await;

    // 5. Process Archival Object Batches (archival objects fourth)
    let (ao_slice_summary, ao_returned_ids) = process_type_candidate_batches(
        "archival_object",
        &expected_ao_prefix,
        &ao_ids,
        ao_batches,
        &mut shared,
        |batch_idx, chunk| {
            let dynamic_enums = Arc::clone(&dynamic_enums);
            fetch_and_build_ao_batch(batch_idx, chunk, dynamic_enums)
        },
    )
    .await;

    let batch_loop_wall_elapsed_sec = batch_loop_start.elapsed().as_secs_f64();

    let total_batches_failed = res_slice_summary.batches_failed
        + do_slice_summary.batches_failed
        + tc_slice_summary.batches_failed
        + ao_slice_summary.batches_failed;
    let total_missing_ids = res_slice_summary.missing_ids
        + do_slice_summary.missing_ids
        + tc_slice_summary.missing_ids
        + ao_slice_summary.missing_ids;
    let total_duplicate_ids = res_slice_summary.duplicate_ids
        + do_slice_summary.duplicate_ids
        + tc_slice_summary.duplicate_ids
        + ao_slice_summary.duplicate_ids;
    let total_unexpected_ids = res_slice_summary.unexpected_ids
        + do_slice_summary.unexpected_ids
        + tc_slice_summary.unexpected_ids
        + ao_slice_summary.unexpected_ids;
    let total_exact_ids_indexed = res_slice_summary.exact_ids_indexed
        + do_slice_summary.exact_ids_indexed
        + tc_slice_summary.exact_ids_indexed
        + ao_slice_summary.exact_ids_indexed;
    let total_parent_docs = res_slice_summary.parent_docs
        + do_slice_summary.parent_docs
        + tc_slice_summary.parent_docs
        + ao_slice_summary.parent_docs;
    let total_child_docs = res_slice_summary.child_docs
        + do_slice_summary.child_docs
        + tc_slice_summary.child_docs
        + ao_slice_summary.child_docs;
    let total_docs_indexed = total_parent_docs + total_child_docs;

    // 6. Commit Decision:
    // Only commit if all batches succeeded, zero unexpected/duplicate anomalies occurred,
    // exact ID counts match for all slices, and no errors occurred.
    let mut solr_commit_elapsed_sec = 0.0;
    let mut commit_failed = false;

    let can_commit = total_batches_failed == 0
        && total_unexpected_ids == 0
        && total_duplicate_ids == 0
        && total_missing_ids == 0
        && res_returned_ids.len() == resource_ids.len()
        && do_returned_ids.len() == do_ids.len()
        && tc_returned_ids.len() == tc_ids.len()
        && ao_returned_ids.len() == ao_ids.len()
        && global_unique_uris.len() == total_docs_indexed
        && first_error.is_none();

    if can_commit {
        let t_commit = Instant::now();
        if let Err(c_err) = writer.commit().await {
            commit_failed = true;
            if first_error.is_none() {
                first_error = Some(format!("Failed to commit candidate Solr core: {c_err}"));
            }
        }
        solr_commit_elapsed_sec = t_commit.elapsed().as_secs_f64();
    }

    let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();

    let failures = total_batches_failed
        + if commit_failed { 1 } else { 0 }
        + if total_duplicate_ids > 0 { 1 } else { 0 }
        + if total_unexpected_ids > 0 { 1 } else { 0 }
        + if total_missing_ids > 0 && total_batches_failed == 0 {
            1
        } else {
            0
        };

    let committed = can_commit && !commit_failed;

    let success = failures == 0
        && total_exact_ids_indexed == total_discovered_ids
        && total_missing_ids == 0
        && total_duplicate_ids == 0
        && total_unexpected_ids == 0
        && committed
        && first_error.is_none();

    let records_per_second = if total_wall_elapsed_sec > 0.0 && success {
        total_exact_ids_indexed as f64 / total_wall_elapsed_sec
    } else {
        0.0
    };

    StaffCombinedCandidateSummary {
        repository_id: args.repo,
        candidate_core: args.candidate_core.clone(),
        solr_url: args.solr_url.clone(),
        arks_enabled: args.arks_enabled,
        batch_size: args.batch_size,
        concurrency: args.concurrency,
        archival_objects: ao_slice_summary,
        resources: res_slice_summary,
        digital_objects: do_slice_summary,
        top_containers: tc_slice_summary,
        total_ids_discovered: total_discovered_ids,
        exact_ids_indexed: total_exact_ids_indexed,
        parent_docs: total_parent_docs,
        child_docs: total_child_docs,
        total_docs_indexed,
        unique_uris: global_unique_uris.len(),
        total_batches,
        batches_completed: total_completed_batches,
        batches_failed: total_batches_failed,
        missing_ids: total_missing_ids,
        duplicate_ids: total_duplicate_ids,
        unexpected_ids: total_unexpected_ids,
        failures,
        id_list_elapsed_sec,
        batch_loop_wall_elapsed_sec,
        solr_write_elapsed_sec,
        solr_update_requests_sent: writer.update_requests_sent(),
        solr_commit_elapsed_sec,
        total_wall_elapsed_sec,
        records_per_second,
        committed,
        success,
        error_message: first_error,
    }
}

pub async fn run_staff_combined_candidate(
    client: &Client,
    args: StaffCombinedCandidateArgs,
) -> Result<()> {
    let json_output = args.json;
    let summary = execute_staff_combined_candidate(client, &args).await;
    let success = summary.success;
    let error_message = summary.error_message.clone();

    let mut stdout = io::stdout();
    render_candidate_summary_to_writer(
        &mut stdout,
        &summary,
        json_output,
        success,
        error_message.as_deref(),
        "combined (AO + Resource + Digital Object + Top Container)",
        |w| print_human_combined_candidate_summary_to_writer(&summary, w),
    )
}

pub fn print_human_combined_candidate_summary(summary: &StaffCombinedCandidateSummary) {
    let mut stdout = io::stdout();
    let _ = print_human_combined_candidate_summary_to_writer(summary, &mut stdout);
}

pub fn print_human_combined_candidate_summary_to_writer<W: Write>(
    summary: &StaffCombinedCandidateSummary,
    writer: &mut W,
) -> io::Result<()> {
    writeln!(writer, "=== STAFF Combined Candidate Indexer Summary ===")?;
    writeln!(writer, "Repository ID: {}", summary.repository_id)?;
    writeln!(writer, "Candidate Core: {}", summary.candidate_core)?;
    writeln!(writer, "Solr URL: {}", summary.solr_url)?;
    writeln!(writer, "ARKs Enabled: {}", summary.arks_enabled)?;
    writeln!(
        writer,
        "Batch Size: {} | Concurrency: {}",
        summary.batch_size, summary.concurrency
    )?;
    writeln!(writer)?;
    writeln!(writer, "--- Record Type Breakdowns ---")?;
    writeln!(
        writer,
        "Archival Objects:\n  Discovered: {} | Indexed: {} | Parent Docs: {} | Child Docs: {} | Total Docs: {}\n  Batches: {} (completed: {}, failed: {}) | Missing: {} | Duplicate: {} | Unexpected: {}",
        summary.archival_objects.ids_discovered,
        summary.archival_objects.exact_ids_indexed,
        summary.archival_objects.parent_docs,
        summary.archival_objects.child_docs,
        summary.archival_objects.total_docs,
        summary.archival_objects.batches,
        summary.archival_objects.batches_completed,
        summary.archival_objects.batches_failed,
        summary.archival_objects.missing_ids,
        summary.archival_objects.duplicate_ids,
        summary.archival_objects.unexpected_ids,
    )?;
    writeln!(
        writer,
        "Resources:\n  Discovered: {} | Indexed: {} | Parent Docs: {} | Child Docs: {} | Total Docs: {}\n  Batches: {} (completed: {}, failed: {}) | Missing: {} | Duplicate: {} | Unexpected: {}",
        summary.resources.ids_discovered,
        summary.resources.exact_ids_indexed,
        summary.resources.parent_docs,
        summary.resources.child_docs,
        summary.resources.total_docs,
        summary.resources.batches,
        summary.resources.batches_completed,
        summary.resources.batches_failed,
        summary.resources.missing_ids,
        summary.resources.duplicate_ids,
        summary.resources.unexpected_ids,
    )?;
    writeln!(
        writer,
        "Digital Objects:\n  Discovered: {} | Indexed: {} | Parent Docs: {} | Child Docs: {} | Total Docs: {}\n  Batches: {} (completed: {}, failed: {}) | Missing: {} | Duplicate: {} | Unexpected: {}",
        summary.digital_objects.ids_discovered,
        summary.digital_objects.exact_ids_indexed,
        summary.digital_objects.parent_docs,
        summary.digital_objects.child_docs,
        summary.digital_objects.total_docs,
        summary.digital_objects.batches,
        summary.digital_objects.batches_completed,
        summary.digital_objects.batches_failed,
        summary.digital_objects.missing_ids,
        summary.digital_objects.duplicate_ids,
        summary.digital_objects.unexpected_ids,
    )?;
    writeln!(
        writer,
        "Top Containers:\n  Discovered: {} | Indexed: {} | Parent Docs: {} | Child Docs: {} | Total Docs: {}\n  Batches: {} (completed: {}, failed: {}) | Missing: {} | Duplicate: {} | Unexpected: {}",
        summary.top_containers.ids_discovered,
        summary.top_containers.exact_ids_indexed,
        summary.top_containers.parent_docs,
        summary.top_containers.child_docs,
        summary.top_containers.total_docs,
        summary.top_containers.batches,
        summary.top_containers.batches_completed,
        summary.top_containers.batches_failed,
        summary.top_containers.missing_ids,
        summary.top_containers.duplicate_ids,
        summary.top_containers.unexpected_ids,
    )?;
    writeln!(writer)?;
    writeln!(writer, "--- Overall Combined Totals ---")?;
    writeln!(
        writer,
        "Total Discovered IDs: {}",
        summary.total_ids_discovered
    )?;
    writeln!(writer, "Exact IDs Indexed: {}", summary.exact_ids_indexed)?;
    writeln!(writer, "Parent Documents: {}", summary.parent_docs)?;
    writeln!(writer, "Child Documents: {}", summary.child_docs)?;
    writeln!(
        writer,
        "Total Documents Indexed: {}",
        summary.total_docs_indexed
    )?;
    writeln!(writer, "Unique URIs: {}", summary.unique_uris)?;
    writeln!(
        writer,
        "Missing IDs: {} | Duplicate IDs: {} | Unexpected IDs: {}",
        summary.missing_ids, summary.duplicate_ids, summary.unexpected_ids
    )?;
    writeln!(
        writer,
        "Total Batches: {} (completed: {}, failed: {})",
        summary.total_batches, summary.batches_completed, summary.batches_failed
    )?;
    writeln!(writer, "Failures: {}", summary.failures)?;
    writeln!(writer, "Committed: {}", summary.committed)?;
    writeln!(
        writer,
        "Status: {}",
        if summary.success {
            "SUCCESS"
        } else {
            "FAILURE"
        }
    )?;
    if let Some(err) = &summary.error_message {
        writeln!(writer, "Error Message: {}", err)?;
    }
    writeln!(writer)?;
    writeln!(writer, "--- Timings ---")?;
    writeln!(
        writer,
        "ID List Elapsed: {:.4}s",
        summary.id_list_elapsed_sec
    )?;
    writeln!(
        writer,
        "Combined Batch Loop Wall Elapsed: {:.4}s (pipelined fetch, build & writes)",
        summary.batch_loop_wall_elapsed_sec
    )?;
    writeln!(
        writer,
        "Solr Update Requests Sent: {}",
        summary.solr_update_requests_sent
    )?;
    writeln!(
        writer,
        "Cumulative Solr Write Wait: {:.4}s",
        summary.solr_write_elapsed_sec
    )?;
    writeln!(
        writer,
        "Solr Hard Commit Elapsed: {:.4}s",
        summary.solr_commit_elapsed_sec
    )?;
    writeln!(
        writer,
        "Total Wall Elapsed: {:.4}s",
        summary.total_wall_elapsed_sec
    )?;
    writeln!(
        writer,
        "Throughput: {:.2} records/sec",
        summary.records_per_second
    )?;
    writeln!(writer, "================================================")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use mockito::Server;
    use serde_json::json;

    async fn setup_mock_solr(core_name: &str) -> (mockito::ServerGuard, mockito::Mock) {
        let mut solr_server = Server::new_async().await;

        let _m_status = solr_server
            .mock(
                "GET",
                format!("/admin/cores?action=STATUS&core={core_name}&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "responseHeader": {"status": 0},
                    "status": {
                        core_name: {
                            "name": core_name,
                            "instanceDir": "/var/solr/data"
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let _m_empty = solr_server
            .mock(
                "GET",
                format!("/{core_name}/select?q=*:*&rows=0&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "responseHeader": {"status": 0},
                    "response": {"numFound": 0, "start": 0, "docs": []}
                })
                .to_string(),
            )
            .create_async()
            .await;

        let m_update = solr_server
            .mock("POST", format!("/{core_name}/update").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .create_async()
            .await;

        (solr_server, m_update)
    }

    fn make_test_args(candidate_core: &str, solr_url: &str) -> StaffCombinedCandidateArgs {
        StaffCombinedCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: solr_url.to_string(),
            candidate_core: candidate_core.to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: false,
            fail_after_batches: None,
        }
    }

    async fn setup_test_harness(
        candidate_core: &str,
    ) -> (
        mockito::ServerGuard,
        mockito::ServerGuard,
        Client,
        StaffCombinedCandidateArgs,
    ) {
        let mut aspace_server = Server::new_async().await;
        let (solr_server, _m_update) = setup_mock_solr(candidate_core).await;

        let _m_schemas = aspace_server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{}"#)
            .create_async()
            .await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());
        let args = make_test_args(candidate_core, &solr_server.url());
        (aspace_server, solr_server, client, args)
    }

    #[tokio::test]
    async fn test_verifier_failure_before_writer_fails_closed() {
        let server = Server::new_async().await;
        let client = Client::new(server.url());
        let args = make_test_args(
            "stage_staff_20261027_verifier_candidate",
            "http://127.0.0.1:18983/solr",
        );

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| bail!("Injected verifier failure before writer"),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            |_idx, _chunk, _enums| async { unreachable!() },
            |_idx, _chunk, _enums| async { unreachable!() },
            |_idx, _chunk, _enums| async { unreachable!() },
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.failures, 1);
        assert!(res.error_message.unwrap().contains(
            "Profiling stack verification failed: Injected verifier failure before writer"
        ));
    }

    #[tokio::test]
    async fn test_cross_type_duplicate_uri_fails_closed() {
        let candidate_core = "stage_staff_20261027_crosstype_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Resource returns parent URI "/repositories/4/resources/1"
        // AO batch returns parent URI "/repositories/4/archival_objects/101" with child URI "/repositories/4/resources/1" (cross-type URI collision!)
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![
                        json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"}),
                        json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"}),
                    ],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec!["/repositories/4/resources/1".to_string()],
                    parent_count: 1,
                    child_count: 1,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert!(res.failures > 0);
        assert!(res.duplicate_ids > 0);
        assert!(res.error_message.unwrap().contains(
            "Duplicate child URI '/repositories/4/resources/1' at index 0 in archival_object batch 0"
        ));
    }

    #[tokio::test]
    async fn test_cross_type_parent_duplicate_uri_fails_closed() {
        let candidate_core = "stage_staff_20261027_crosstype_parent_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Resource returns parent URI "/repositories/4/resources/1" and child URI "/repositories/4/archival_objects/101"
        // AO batch then returns parent URI "/repositories/4/archival_objects/101" -> duplicate document URI in stream!
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![
                        json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"}),
                        json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"}),
                    ],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec!["/repositories/4/archival_objects/101".to_string()],
                    parent_count: 1,
                    child_count: 1,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert!(res.failures > 0);
        assert!(res.duplicate_ids > 0);
        assert!(res.error_message.unwrap().contains(
            "Duplicate document URI '/repositories/4/archival_objects/101' encountered in candidate stream"
        ));
    }

    #[tokio::test]
    async fn test_resource_duplicate_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_res_dup_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Resource batch returns duplicate parent record for ID 1
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not be called when resource stage encounters duplicate ID");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![
                        json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"}),
                        json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"}),
                    ],
                    parent_uris: vec![
                        Some("/repositories/4/resources/1".to_string()),
                        Some("/repositories/4/resources/1".to_string()),
                    ],
                    child_uris: vec![],
                    parent_count: 2,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert!(res.resources.duplicate_ids > 0);
        assert!(res.duplicate_ids > 0);
        assert!(res
            .error_message
            .unwrap()
            .contains("Duplicate record returned for resource ID 1 across batches"));
    }

    #[tokio::test]
    async fn test_resource_unexpected_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_res_unexp_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Discovered Resource ID is 1, but batch returns doc for 999
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not run when resource stage has unexpected ID");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/999", "uri": "/repositories/4/resources/999"})],
                    parent_uris: vec![Some("/repositories/4/resources/999".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.unexpected_ids, 1);
        assert_eq!(res.unexpected_ids, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Returned resource ID 999 was not present in repository resource ID list"));
    }

    #[tokio::test]
    async fn test_ao_duplicate_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_ao_dup_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Resource succeeds, but AO returns duplicate record for ID 101
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![
                        json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"}),
                        json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"}),
                    ],
                    parent_uris: vec![
                        Some("/repositories/4/archival_objects/101".to_string()),
                        Some("/repositories/4/archival_objects/101".to_string()),
                    ],
                    child_uris: vec![],
                    parent_count: 2,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.resources.duplicate_ids, 0);
        assert!(res.archival_objects.duplicate_ids > 0);
        assert!(res.duplicate_ids > 0);
        assert!(res
            .error_message
            .unwrap()
            .contains("Duplicate record returned for archival_object ID 101 across batches"));
    }

    #[tokio::test]
    async fn test_ao_unexpected_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_ao_unexp_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Discovered AO ID is 101, but batch returns doc for 999
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/999", "uri": "/repositories/4/archival_objects/999"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/999".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.archival_objects.unexpected_ids, 1);
        assert_eq!(res.unexpected_ids, 1);
        assert!(res.error_message.unwrap().contains(
            "Returned archival_object ID 999 was not present in repository archival_object ID list"
        ));
    }

    #[tokio::test]
    async fn test_missing_ids_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_missing_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Discovered AO IDs: [101, 102], but AO batch only returns doc for [101]
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101, 102]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.missing_ids, 1);
        assert_eq!(res.archival_objects.missing_ids, 1);
        assert_eq!(res.resources.missing_ids, 0);
    }

    #[tokio::test]
    async fn test_missing_resource_ids_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_missing_res_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Discovered Resource IDs: [1, 2], but batch only returns doc for [1]
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1, 2]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.missing_ids, 1);
        assert_eq!(res.missing_ids, 1);
    }

    #[tokio::test]
    async fn test_malformed_child_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_malformed_child_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Resource batch returns malformed child error
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                bail!("Child document missing string 'parent_id'")
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.batches_failed, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Child document missing string 'parent_id'"));
    }

    #[tokio::test]
    async fn test_ao_batch_failure_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_ao_batch_err_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Resource succeeds, but AO batch fails with error
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                bail!("Injected AO network timeout")
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.archival_objects.batches_failed, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Injected AO network timeout"));
    }

    #[tokio::test]
    async fn test_fail_after_batches_never_commits() {
        let candidate_core = "stage_staff_20261027_fail_after_candidate";
        let (_aspace, _solr, client, mut args) = setup_test_harness(candidate_core).await;
        args.fail_after_batches = Some(1); // Fail after first batch

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert!(res.failures > 0);
        assert!(res
            .error_message
            .unwrap()
            .contains("Injected deterministic test failure after 1 completed batch(es)"));
    }

    #[tokio::test]
    async fn test_extract_digital_object_batch_documents_clean_and_marker_stripping() {
        let repo_id = 4;
        let batch = vec![
            json!({
                "uri": "/repositories/4/digital_objects/50",
                "title": "Published DO",
                "publish": true,
                "primary_type": "digital_object",
                "_archivalist_rust_fields": ["common_metadata"],
                "_archivalist_private": "marker_parent",
                COLLECTION_MANAGEMENT_DOCUMENTS_KEY: [
                    {
                        "id": "/repositories/4/collection_management/500",
                        "uri": "/repositories/4/collection_management/500",
                        "parent_id": "/repositories/4/digital_objects/50",
                        "parent_type": "digital_object",
                        "primary_type": "collection_management",
                        "_archivalist_marker": "marker_child",
                        "processing_priority": "high"
                    }
                ]
            }),
            json!({
                "uri": "/repositories/4/digital_objects/51",
                "title": "Unpublished DO",
                "publish": false,
                "primary_type": "digital_object",
                "_archivalist_marker": "strip_me"
            }),
        ];

        let extracted = extract_digital_object_batch_documents(batch, repo_id).unwrap();
        assert_eq!(extracted.parent_count, 2);
        assert_eq!(extracted.child_count, 1);
        assert_eq!(extracted.ordered_documents.len(), 3);

        // Document 0: parent DO 50 (parent before child)
        let doc0 = &extracted.ordered_documents[0];
        assert_eq!(doc0["uri"], "/repositories/4/digital_objects/50");
        assert!(doc0.get("_archivalist_rust_fields").is_none());
        assert!(doc0.get("_archivalist_private").is_none());
        assert!(doc0.get(COLLECTION_MANAGEMENT_DOCUMENTS_KEY).is_none());
        // Verify PUI types added for published DO
        let types0 = doc0["types"].as_array().unwrap();
        assert!(types0.iter().any(|v| v == "pui_record"));
        assert!(types0.iter().any(|v| v == "pui_digital_object"));
        assert!(types0.iter().any(|v| v == "pui"));

        // Document 1: child collection_management 500
        let doc1 = &extracted.ordered_documents[1];
        assert_eq!(doc1["uri"], "/repositories/4/collection_management/500");
        assert_eq!(doc1["parent_id"], "/repositories/4/digital_objects/50");
        assert_eq!(doc1["parent_type"], "digital_object");
        assert_eq!(doc1["primary_type"], "collection_management");
        assert!(doc1.get("_archivalist_marker").is_none());

        // Document 2: parent DO 51 (unpublished, no PUI additions)
        let doc2 = &extracted.ordered_documents[2];
        assert_eq!(doc2["uri"], "/repositories/4/digital_objects/51");
        assert!(doc2.get("_archivalist_marker").is_none());
        assert!(doc2.get("types").is_none());
    }

    #[tokio::test]
    async fn test_extract_digital_object_batch_documents_validation_failures() {
        let repo_id = 4;

        // 1. Wrong parent URI prefix
        let bad_prefix = vec![json!({"uri": "/repositories/4/resources/50"})];
        assert!(extract_digital_object_batch_documents(bad_prefix, repo_id).is_err());

        // 2. Non-numeric parent ID
        let bad_id = vec![json!({"uri": "/repositories/4/digital_objects/abc"})];
        assert!(extract_digital_object_batch_documents(bad_id, repo_id).is_err());

        // 3. Child not object
        let bad_child = vec![json!({
            "uri": "/repositories/4/digital_objects/50",
            COLLECTION_MANAGEMENT_DOCUMENTS_KEY: ["not an object"]
        })];
        assert!(extract_digital_object_batch_documents(bad_child, repo_id).is_err());

        // 4. Child id != uri
        let bad_child_id = vec![json!({
            "uri": "/repositories/4/digital_objects/50",
            COLLECTION_MANAGEMENT_DOCUMENTS_KEY: [{
                "id": "/repositories/4/collection_management/1",
                "uri": "/repositories/4/collection_management/2",
                "parent_id": "/repositories/4/digital_objects/50",
                "parent_type": "digital_object",
                "primary_type": "collection_management"
            }]
        })];
        assert!(extract_digital_object_batch_documents(bad_child_id, repo_id).is_err());

        // 5. Child parent_type not digital_object
        let bad_parent_type = vec![json!({
            "uri": "/repositories/4/digital_objects/50",
            COLLECTION_MANAGEMENT_DOCUMENTS_KEY: [{
                "id": "/repositories/4/collection_management/1",
                "uri": "/repositories/4/collection_management/1",
                "parent_id": "/repositories/4/digital_objects/50",
                "parent_type": "resource",
                "primary_type": "collection_management"
            }]
        })];
        assert!(extract_digital_object_batch_documents(bad_parent_type, repo_id).is_err());

        // 6. Child parent_id mismatch
        let bad_parent_id = vec![json!({
            "uri": "/repositories/4/digital_objects/50",
            COLLECTION_MANAGEMENT_DOCUMENTS_KEY: [{
                "id": "/repositories/4/collection_management/1",
                "uri": "/repositories/4/collection_management/1",
                "parent_id": "/repositories/4/digital_objects/99",
                "parent_type": "digital_object",
                "primary_type": "collection_management"
            }]
        })];
        assert!(extract_digital_object_batch_documents(bad_parent_id, repo_id).is_err());
    }

    #[tokio::test]
    async fn test_do_duplicate_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_do_dup_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not run when DO stage encounters duplicate");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![
                        json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"}),
                        json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"}),
                    ],
                    parent_uris: vec![
                        Some("/repositories/4/digital_objects/50".to_string()),
                        Some("/repositories/4/digital_objects/50".to_string()),
                    ],
                    child_uris: vec![],
                    parent_count: 2,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                panic!("TC stage must not run when DO stage encounters duplicate");
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert!(res.digital_objects.duplicate_ids > 0);
        assert!(res.duplicate_ids > 0);
        assert!(res
            .error_message
            .unwrap()
            .contains("Duplicate record returned for digital_object ID 50 across batches"));
    }

    #[tokio::test]
    async fn test_do_unexpected_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_do_unexp_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not run when DO stage encounters unexpected ID");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/digital_objects/999", "uri": "/repositories/4/digital_objects/999"})],
                    parent_uris: vec![Some("/repositories/4/digital_objects/999".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                panic!("TC stage must not run when DO stage encounters unexpected ID");
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.digital_objects.unexpected_ids, 1);
        assert_eq!(res.unexpected_ids, 1);
        assert!(res.error_message.unwrap().contains(
            "Returned digital_object ID 999 was not present in repository digital_object ID list"
        ));
    }

    #[tokio::test]
    async fn test_missing_digital_object_ids_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_missing_do_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Discovered DO IDs: [50, 51], but batch only returns doc for [50]
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50, 51]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"})],
                    parent_uris: vec![Some("/repositories/4/digital_objects/50".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![],
                    parent_uris: vec![],
                    child_uris: vec![],
                    parent_count: 0,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.digital_objects.missing_ids, 1);
        assert_eq!(res.missing_ids, 1);
        assert_eq!(res.resources.missing_ids, 0);
        assert_eq!(res.archival_objects.missing_ids, 0);
    }

    #[tokio::test]
    async fn test_do_batch_failure_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_do_batch_err_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not run when DO batch fails");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                bail!("Injected DO batch network error")
            },
            |_idx, _chunk, _enums| async {
                panic!("TC stage must not run when DO batch fails");
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.digital_objects.batches_failed, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Injected DO batch network error"));
    }

    #[tokio::test]
    async fn test_extract_top_container_batch_documents_clean_and_marker_stripping() {
        let repo_id = 4;
        let batch = vec![
            json!({
                "uri": "/repositories/4/top_containers/100",
                "id": "/repositories/4/top_containers/100",
                "primary_type": "top_container",
                "types": ["top_container"],
                "is_linked_to_published_record": true,
                "indicator": "Box 1",
                "_archivalist_marker": "marker_val",
                "_archivalist_rust_fields": ["tc_metadata"]
            }),
            json!({
                "uri": "/repositories/4/top_containers/101",
                "id": "/repositories/4/top_containers/101",
                "primary_type": "top_container",
                "types": ["top_container", "pui_container", "pui"],
                "is_linked_to_published_record": false,
                "indicator": "Box 2",
                "_archivalist_temp": "strip_this"
            }),
            json!({
                "uri": "/repositories/4/top_containers/102",
                "id": "/repositories/4/top_containers/102",
                "primary_type": "top_container",
                "indicator": "Box 3"
            }),
        ];

        let extracted = extract_top_container_batch_documents(batch, repo_id).unwrap();
        assert_eq!(extracted.parent_count, 3);
        assert_eq!(extracted.child_count, 0);
        assert_eq!(extracted.ordered_documents.len(), 3);

        // Doc 0: linked to published record -> publish: true, types: ["top_container", "pui_container", "pui"]
        let doc0 = &extracted.ordered_documents[0];
        assert_eq!(doc0["uri"], "/repositories/4/top_containers/100");
        assert_eq!(doc0["publish"], true);
        assert!(doc0.get("_archivalist_marker").is_none());
        assert!(doc0.get("_archivalist_rust_fields").is_none());
        let types0 = doc0["types"].as_array().unwrap();
        assert_eq!(
            types0,
            &vec![json!("top_container"), json!("pui_container"), json!("pui")]
        );

        // Doc 1: linked = false -> publish: false, pui types stripped: ["top_container"]
        let doc1 = &extracted.ordered_documents[1];
        assert_eq!(doc1["uri"], "/repositories/4/top_containers/101");
        assert_eq!(doc1["publish"], false);
        assert!(doc1.get("_archivalist_temp").is_none());
        let types1 = doc1["types"].as_array().unwrap();
        assert_eq!(types1, &vec![json!("top_container")]);

        // Doc 2: no linked field (None) and no types -> publish: false, types is None
        let doc2 = &extracted.ordered_documents[2];
        assert_eq!(doc2["uri"], "/repositories/4/top_containers/102");
        assert_eq!(doc2["publish"], false);
        assert!(doc2.get("types").is_none());
    }

    #[tokio::test]
    async fn test_extract_top_container_batch_documents_validation_failures() {
        let repo_id = 4;

        // 1. Wrong URI prefix
        let bad_prefix = vec![
            json!({"uri": "/repositories/4/resources/100", "id": "/repositories/4/resources/100", "primary_type": "top_container"}),
        ];
        assert!(extract_top_container_batch_documents(bad_prefix, repo_id).is_err());

        // 2. Non-numeric ID
        let bad_id = vec![
            json!({"uri": "/repositories/4/top_containers/abc", "id": "/repositories/4/top_containers/abc", "primary_type": "top_container"}),
        ];
        assert!(extract_top_container_batch_documents(bad_id, repo_id).is_err());

        // 3. Different repository ID
        let bad_repo = vec![
            json!({"uri": "/repositories/2/top_containers/100", "id": "/repositories/2/top_containers/100", "primary_type": "top_container"}),
        ];
        assert!(extract_top_container_batch_documents(bad_repo, repo_id).is_err());

        // 4. primary_type mismatch
        let bad_type = vec![
            json!({"uri": "/repositories/4/top_containers/100", "id": "/repositories/4/top_containers/100", "primary_type": "archival_object"}),
        ];
        assert!(extract_top_container_batch_documents(bad_type, repo_id).is_err());

        // 5. id != uri mismatch
        let bad_id_uri = vec![
            json!({"uri": "/repositories/4/top_containers/100", "id": "/repositories/4/top_containers/101", "primary_type": "top_container"}),
        ];
        assert!(extract_top_container_batch_documents(bad_id_uri, repo_id).is_err());
    }

    #[tokio::test]
    async fn test_tc_duplicate_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_tc_dup_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![100]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not run when TC stage encounters duplicate");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"})],
                    parent_uris: vec![Some("/repositories/4/digital_objects/50".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![
                        json!({"id": "/repositories/4/top_containers/100", "uri": "/repositories/4/top_containers/100"}),
                        json!({"id": "/repositories/4/top_containers/100", "uri": "/repositories/4/top_containers/100"}),
                    ],
                    parent_uris: vec![
                        Some("/repositories/4/top_containers/100".to_string()),
                        Some("/repositories/4/top_containers/100".to_string()),
                    ],
                    child_uris: vec![],
                    parent_count: 2,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.digital_objects.exact_ids_indexed, 1);
        assert!(res.top_containers.duplicate_ids > 0);
        assert!(res.duplicate_ids > 0);
        assert!(res
            .error_message
            .unwrap()
            .contains("Duplicate record returned for top_container ID 100 across batches"));
    }

    #[tokio::test]
    async fn test_tc_unexpected_id_fails_closed() {
        let candidate_core = "stage_staff_20261027_tc_unexp_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![100]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not run when TC stage encounters unexpected ID");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"})],
                    parent_uris: vec![Some("/repositories/4/digital_objects/50".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/top_containers/999", "uri": "/repositories/4/top_containers/999"})],
                    parent_uris: vec![Some("/repositories/4/top_containers/999".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.digital_objects.exact_ids_indexed, 1);
        assert_eq!(res.top_containers.unexpected_ids, 1);
        assert_eq!(res.unexpected_ids, 1);
        assert!(res.error_message.unwrap().contains(
            "Returned top_container ID 999 was not present in repository top_container ID list"
        ));
    }

    #[tokio::test]
    async fn test_missing_top_container_ids_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_missing_tc_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        // Discovered TC IDs: [100, 101], but batch only returns doc for [100]
        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![100, 101]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"})],
                    parent_uris: vec![Some("/repositories/4/digital_objects/50".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/top_containers/100", "uri": "/repositories/4/top_containers/100"})],
                    parent_uris: vec![Some("/repositories/4/top_containers/100".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.top_containers.missing_ids, 1);
        assert_eq!(res.missing_ids, 1);
        assert_eq!(res.digital_objects.missing_ids, 0);
        assert_eq!(res.resources.missing_ids, 0);
        assert_eq!(res.archival_objects.missing_ids, 0);
    }

    #[tokio::test]
    async fn test_tc_batch_failure_fails_closed_and_no_commit() {
        let candidate_core = "stage_staff_20261027_tc_batch_err_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![100]) },
            |_idx, _chunk, _enums| async {
                panic!("AO stage must not run when TC batch fails");
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"})],
                    parent_uris: vec![Some("/repositories/4/digital_objects/50".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                bail!("Injected TC batch network error")
            },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.digital_objects.exact_ids_indexed, 1);
        assert_eq!(res.top_containers.batches_failed, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Injected TC batch network error"));
    }

    #[tokio::test]
    async fn test_tc_duplicate_input_ids_fail_closed() {
        let candidate_core = "stage_staff_20261027_tc_dup_input_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![100, 100]) },
            |_idx, _chunk, _enums| async { unreachable!() },
            |_idx, _chunk, _enums| async { unreachable!() },
            |_idx, _chunk, _enums| async { unreachable!() },
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(!res.success);
        assert!(!res.committed);
        assert_eq!(res.top_containers.duplicate_ids, 1);
        assert_eq!(res.duplicate_ids, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("duplicate ID(s) in repository top container ID list"));
    }

    #[tokio::test]
    async fn test_combined_candidate_clean_success() {
        let candidate_core = "stage_staff_20261027_success_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![50]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![100]) },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/archival_objects/101", "uri": "/repositories/4/archival_objects/101"})],
                    parent_uris: vec![Some("/repositories/4/archival_objects/101".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/resources/1", "uri": "/repositories/4/resources/1"})],
                    parent_uris: vec![Some("/repositories/4/resources/1".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![
                        json!({"id": "/repositories/4/digital_objects/50", "uri": "/repositories/4/digital_objects/50"}),
                        json!({
                            "id": "/repositories/4/collection_management/500",
                            "uri": "/repositories/4/collection_management/500",
                            "parent_id": "/repositories/4/digital_objects/50",
                            "parent_type": "digital_object",
                            "primary_type": "collection_management"
                        }),
                    ],
                    parent_uris: vec![Some("/repositories/4/digital_objects/50".to_string())],
                    child_uris: vec!["/repositories/4/collection_management/500".to_string()],
                    parent_count: 1,
                    child_count: 1,
                })
            },
            |_idx, _chunk, _enums| async {
                Ok(CandidateBatchOutput {
                    ordered_documents: vec![json!({"id": "/repositories/4/top_containers/100", "uri": "/repositories/4/top_containers/100"})],
                    parent_uris: vec![Some("/repositories/4/top_containers/100".to_string())],
                    child_uris: vec![],
                    parent_count: 1,
                    child_count: 0,
                })
            },
        )
        .await;

        if !res.success {
            eprintln!("Test failed with error_message: {:?}", res.error_message);
            eprintln!("Failures: {}", res.failures);
            eprintln!("Committed: {}", res.committed);
            eprintln!("AO: {:?}", res.archival_objects);
            eprintln!("Res: {:?}", res.resources);
            eprintln!("DO: {:?}", res.digital_objects);
            eprintln!("TC: {:?}", res.top_containers);
        }
        assert!(res.success);
        assert!(res.committed);
        assert_eq!(res.failures, 0);
        assert_eq!(res.total_ids_discovered, 4);
        assert_eq!(res.exact_ids_indexed, 4);
        assert_eq!(res.parent_docs, 4);
        assert_eq!(res.child_docs, 1);
        assert_eq!(res.total_docs_indexed, 5);
        assert_eq!(res.unique_uris, 5);
        assert_eq!(res.archival_objects.exact_ids_indexed, 1);
        assert_eq!(res.resources.exact_ids_indexed, 1);
        assert_eq!(res.digital_objects.exact_ids_indexed, 1);
        assert_eq!(res.digital_objects.parent_docs, 1);
        assert_eq!(res.digital_objects.child_docs, 1);
        assert_eq!(res.top_containers.exact_ids_indexed, 1);
        assert_eq!(res.top_containers.parent_docs, 1);
        assert_eq!(res.top_containers.child_docs, 0);
    }
}
