use anyhow::{bail, Context, Result};
use aspace_client::index_document::COLLECTION_MANAGEMENT_DOCUMENTS_KEY;
use aspace_client::{
    build_index_document_batch, Client, IndexDocumentBatchRequest, IndexDocumentInput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Write};

pub use crate::staff_candidate_runner::{
    partition_ids_into_batches, render_candidate_summary_to_writer, CandidateBatchOutput,
    StaffCandidateArgs, StaffCandidateArgs as StaffResourceCandidateArgs,
    StaffCandidateExecutionResult, DEFAULT_CANDIDATE_BATCH_SIZE, DEFAULT_CANDIDATE_CONCURRENCY,
    MAX_CANDIDATE_BATCH_SIZE, MAX_CANDIDATE_CONCURRENCY,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StaffResourceCandidateSummary {
    pub repository_id: u32,
    pub total_ids_discovered: usize,
    pub source_ids_discovered: usize,
    pub exact_ids_indexed: usize,
    pub source_ids_indexed: usize,
    pub parent_docs: usize,
    pub child_docs: usize,
    pub total_docs_indexed: usize,
    pub batch_size: usize,
    pub concurrency: usize,
    pub total_batches: usize,
    pub batches_completed: usize,
    pub batches_failed: usize,
    pub failures: usize,
    pub unique_uris: usize,
    pub missing_ids: usize,
    pub duplicate_ids: usize,
    pub unexpected_ids: usize,
    pub candidate_core: String,
    pub solr_url: String,
    pub arks_enabled: bool,
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

impl From<StaffCandidateExecutionResult> for StaffResourceCandidateSummary {
    fn from(res: StaffCandidateExecutionResult) -> Self {
        Self {
            repository_id: res.repository_id,
            total_ids_discovered: res.total_ids_discovered,
            source_ids_discovered: res.total_ids_discovered,
            exact_ids_indexed: res.exact_ids_indexed,
            source_ids_indexed: res.exact_ids_indexed,
            parent_docs: res.parent_docs,
            child_docs: res.child_docs,
            total_docs_indexed: res.total_docs_indexed,
            batch_size: res.batch_size,
            concurrency: res.concurrency,
            total_batches: res.total_batches,
            batches_completed: res.batches_completed,
            batches_failed: res.batches_failed,
            failures: res.failures,
            unique_uris: res.unique_uris,
            missing_ids: res.missing_ids,
            duplicate_ids: res.duplicate_ids,
            unexpected_ids: res.unexpected_ids,
            candidate_core: res.candidate_core,
            solr_url: res.solr_url,
            arks_enabled: res.arks_enabled,
            id_list_elapsed_sec: res.id_list_elapsed_sec,
            batch_loop_wall_elapsed_sec: res.batch_loop_wall_elapsed_sec,
            solr_write_elapsed_sec: res.solr_write_elapsed_sec,
            solr_update_requests_sent: res.solr_update_requests_sent,
            solr_commit_elapsed_sec: res.solr_commit_elapsed_sec,
            total_wall_elapsed_sec: res.total_wall_elapsed_sec,
            records_per_second: res.records_per_second,
            committed: res.committed,
            success: res.success,
            error_message: res.error_message,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedResourceDocuments {
    pub ordered_documents: Vec<Value>,
    pub parent_count: usize,
    pub child_count: usize,
    pub parent_uris: Vec<String>,
    pub child_uris: Vec<String>,
}

impl From<ExtractedResourceDocuments> for CandidateBatchOutput {
    fn from(ext: ExtractedResourceDocuments) -> Self {
        Self {
            ordered_documents: ext.ordered_documents,
            parent_uris: ext.parent_uris.into_iter().map(Some).collect(),
            child_uris: ext.child_uris,
            parent_count: ext.parent_count,
            child_count: ext.child_count,
        }
    }
}

/// Extracts synthetic `collection_management` child documents from built parent documents
/// in native hook order (parent, then child docs), preserving exact child JSON and identity,
/// and strips all top-level `_archivalist_*` private markers from both parent and child docs.
///
/// Fails closed if child envelope is malformed, not an array, child is not an object,
/// or child identity/envelope does not match expected parent URI.
pub fn extract_resource_batch_documents(
    batch_documents: Vec<Value>,
    repo_id: u32,
) -> Result<ExtractedResourceDocuments> {
    let expected_resource_prefix = format!("/repositories/{}/resources/", repo_id);
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

        if !parent_uri.starts_with(&expected_resource_prefix) {
            bail!(
                "Parent resource URI '{}' does not match expected prefix '{}'",
                parent_uri,
                expected_resource_prefix
            );
        }

        let parent_id_suffix = &parent_uri[expected_resource_prefix.len()..];
        if parent_id_suffix.parse::<u32>().is_err() {
            bail!(
                "Parent resource URI '{}' contains non-numeric ID suffix '{}'",
                parent_uri,
                parent_id_suffix
            );
        }

        // Extract collection management child documents BEFORE stripping markers
        let cm_value = parent_obj.remove(COLLECTION_MANAGEMENT_DOCUMENTS_KEY);

        // Strip all top-level _archivalist_* markers from parent document
        parent_obj.retain(|k, _| !k.starts_with("_archivalist_"));

        // Match native staff indexer apply_pui_fields: published resources receive PUI availability
        // types ["pui_collection", "pui"] in addition to ["resource"].
        if parent_obj.get("publish").and_then(Value::as_bool) == Some(true) {
            let types = parent_obj
                .entry("types".to_string())
                .or_insert_with(|| Value::Array(vec![Value::String("resource".to_string())]));
            if let Value::Array(types_arr) = types {
                if !types_arr
                    .iter()
                    .any(|v| v.as_str() == Some("pui_collection"))
                {
                    types_arr.push(Value::String("pui_collection".to_string()));
                }
                if !types_arr.iter().any(|v| v.as_str() == Some("pui")) {
                    types_arr.push(Value::String("pui".to_string()));
                }
            }
        }

        let mut children = Vec::new();
        if let Some(cm_val) = cm_value {
            let cm_array = match cm_val {
                Value::Array(arr) => arr,
                _ => bail!(
                    "Parent resource '{}' has malformed {}: expected JSON array",
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
                if parent_type_val != "resource" {
                    bail!(
                        "Child document '{}' parent_type '{}' must be 'resource'",
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
        parent_uris.push(parent_uri);
        ordered_documents.push(parent_doc);
        parent_count += 1;

        for child in children {
            ordered_documents.push(child);
            child_count += 1;
        }
    }

    Ok(ExtractedResourceDocuments {
        ordered_documents,
        parent_count,
        child_count,
        parent_uris,
        child_uris,
    })
}

pub async fn fetch_and_build_resource_candidate_batch(
    client: &Client,
    repo_id: u32,
    chunk: &[u32],
    batch_idx: usize,
    batch_size: usize,
    dynamic_enums: std::sync::Arc<Vec<String>>,
    arks_enabled: bool,
) -> Result<CandidateBatchOutput> {
    let raw_records = client
        .fetch_staff_resources_by_ids_bounded(repo_id, chunk, batch_size)
        .await
        .with_context(|| format!("Failed to fetch resource batch {}", batch_idx))?;

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
            record_type: "resource".to_string(),
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

    let extracted = extract_resource_batch_documents(batch_resp.documents, repo_id)?;
    Ok(extracted.into())
}

pub async fn execute_staff_resource_candidate(
    client: &Client,
    args: &StaffResourceCandidateArgs,
) -> StaffResourceCandidateSummary {
    execute_staff_resource_candidate_with_verifier(
        client,
        args,
        crate::profiling_stack::verify_profiling_stack,
    )
    .await
}

pub async fn execute_staff_resource_candidate_with_verifier<V>(
    client: &Client,
    args: &StaffResourceCandidateArgs,
    verifier: V,
) -> StaffResourceCandidateSummary
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
{
    let repo_id = args.repo;
    let arks_enabled = args.arks_enabled;
    let batch_size = args.batch_size;
    let expected_prefix = format!("/repositories/{}/resources/", repo_id);
    let descriptor = crate::staff_candidate_runner::StaffCandidateDescriptor {
        expected_parent_prefix: &expected_prefix,
        id_type_label: "resource",
        id_list_label: "resource",
    };

    let client_for_ids = client.clone();
    let client_for_batches = client.clone();

    let res = crate::staff_candidate_runner::execute_staff_candidate(
        client,
        args,
        descriptor,
        verifier,
        || async move {
            client_for_ids
                .resource_ids_modified_since(repo_id, Some(0))
                .await
        },
        move |batch_idx, chunk, dynamic_enums| {
            let client = client_for_batches.clone();
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
    )
    .await;

    res.into()
}

pub fn print_human_resource_candidate_summary(summary: &StaffResourceCandidateSummary) {
    let mut stdout = io::stdout();
    let _ = print_human_resource_candidate_summary_to_writer(summary, &mut stdout);
}

pub fn print_human_resource_candidate_summary_to_writer<W: Write>(
    summary: &StaffResourceCandidateSummary,
    writer: &mut W,
) -> io::Result<()> {
    writeln!(writer, "=== STAFF Resource Candidate Indexer Summary ===")?;
    writeln!(writer, "Repository ID: {}", summary.repository_id)?;
    writeln!(writer, "Candidate Core: {}", summary.candidate_core)?;
    writeln!(
        writer,
        "Total Discovered IDs: {}",
        summary.total_ids_discovered
    )?;
    writeln!(writer, "Exact IDs Indexed: {}", summary.exact_ids_indexed)?;
    writeln!(writer, "Parent Docs: {}", summary.parent_docs)?;
    writeln!(
        writer,
        "Child Docs (Collection Management): {}",
        summary.child_docs
    )?;
    writeln!(writer, "Total Docs Indexed: {}", summary.total_docs_indexed)?;
    writeln!(writer, "Batch Size: {}", summary.batch_size)?;
    writeln!(writer, "Concurrency: {}", summary.concurrency)?;
    writeln!(writer, "Total Batches: {}", summary.total_batches)?;
    writeln!(writer, "Batches Completed: {}", summary.batches_completed)?;
    writeln!(writer, "Batches Failed: {}", summary.batches_failed)?;
    writeln!(writer, "Total Failures: {}", summary.failures)?;
    writeln!(writer, "Unique URIs: {}", summary.unique_uris)?;
    writeln!(writer, "Missing IDs: {}", summary.missing_ids)?;
    writeln!(writer, "Duplicate IDs: {}", summary.duplicate_ids)?;
    writeln!(writer, "Unexpected IDs: {}", summary.unexpected_ids)?;
    writeln!(writer, "ARKs Enabled: {}", summary.arks_enabled)?;
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
        "  Solr Writes Elapsed: {:.4}s (awaiting Solr update POSTs; overlaps batch loop)",
        summary.solr_write_elapsed_sec
    )?;
    writeln!(
        writer,
        "  Solr Update POST Requests: {}",
        summary.solr_update_requests_sent
    )?;
    writeln!(
        writer,
        "Solr Hard Commit Elapsed: {:.4}s",
        summary.solr_commit_elapsed_sec
    )?;
    writeln!(
        writer,
        "Total Wall Elapsed: {:.4}s (id_list + batch_loop_wall + solr_commit)",
        summary.total_wall_elapsed_sec
    )?;
    writeln!(writer, "Records / Sec: {:.2}", summary.records_per_second)?;
    writeln!(writer, "Committed: {}", summary.committed)?;
    if let Some(ref err) = summary.error_message {
        writeln!(writer, "Error: {}", err)?;
    }
    writeln!(
        writer,
        "Status: {}",
        if summary.success { "SUCCESS" } else { "FAILED" }
    )?;
    Ok(())
}

pub async fn run_staff_resource_candidate(
    client: &Client,
    args: StaffResourceCandidateArgs,
) -> Result<StaffResourceCandidateSummary> {
    run_staff_resource_candidate_to_writer(client, args, &mut io::stdout()).await
}

pub async fn run_staff_resource_candidate_to_writer<W: Write>(
    client: &Client,
    args: StaffResourceCandidateArgs,
    writer: &mut W,
) -> Result<StaffResourceCandidateSummary> {
    run_staff_resource_candidate_to_writer_with_verifier(
        client,
        args,
        writer,
        crate::profiling_stack::verify_profiling_stack,
    )
    .await
}

pub async fn run_staff_resource_candidate_to_writer_with_verifier<W: Write, V>(
    client: &Client,
    args: StaffResourceCandidateArgs,
    writer: &mut W,
    verifier: V,
) -> Result<StaffResourceCandidateSummary>
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
{
    let summary = execute_staff_resource_candidate_with_verifier(client, &args, verifier).await;

    render_candidate_summary_to_writer(
        writer,
        &summary,
        args.json,
        summary.success,
        summary.error_message.as_deref(),
        "resource",
        |w| print_human_resource_candidate_summary_to_writer(&summary, w),
    )?;

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use serde_json::json;

    #[test]
    fn test_cli_argument_bounds() {
        let valid_args = StaffResourceCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: "http://127.0.0.1:18983/solr".to_string(),
            candidate_core: "stage_staff_20261027_candidate".to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/profiling_state.json")),
            json: false,
            fail_after_batches: None,
        };
        assert!(valid_args.validate().is_ok());

        let missing_state = StaffResourceCandidateArgs {
            state: None,
            ..valid_args.clone()
        };
        assert!(missing_state.validate().is_err());
        assert_eq!(
            missing_state.validate().unwrap_err().to_string(),
            "missing required profiling-stack state file for Solr write mode (--state)"
        );

        let empty_state = StaffResourceCandidateArgs {
            state: Some(std::path::PathBuf::from("")),
            ..valid_args.clone()
        };
        assert!(empty_state.validate().is_err());

        let strip_markers_false = StaffResourceCandidateArgs {
            strip_markers: false,
            ..valid_args.clone()
        };
        assert!(strip_markers_false.validate().is_err());
        assert_eq!(
            strip_markers_false.validate().unwrap_err().to_string(),
            "--strip-markers false is forbidden in Solr write mode; marker stripping is mandatory"
        );

        let fail_zero = StaffResourceCandidateArgs {
            fail_after_batches: Some(0),
            ..valid_args.clone()
        };
        assert_eq!(
            fail_zero.validate().unwrap_err().to_string(),
            "fail_after_batches must be greater than 0 if specified"
        );

        let repo_zero = StaffResourceCandidateArgs {
            repo: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            repo_zero.validate().unwrap_err().to_string(),
            "repo ID must be greater than 0"
        );

        let batch_zero = StaffResourceCandidateArgs {
            batch_size: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            batch_zero.validate().unwrap_err().to_string(),
            "batch size must be greater than 0"
        );

        let batch_oversized = StaffResourceCandidateArgs {
            batch_size: 26,
            ..valid_args.clone()
        };
        assert_eq!(
            batch_oversized.validate().unwrap_err().to_string(),
            "batch size cannot exceed 25 (requested: 26)"
        );

        let concurrency_zero = StaffResourceCandidateArgs {
            concurrency: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            concurrency_zero.validate().unwrap_err().to_string(),
            "worker concurrency must be greater than 0"
        );

        let concurrency_oversized = StaffResourceCandidateArgs {
            concurrency: 33,
            ..valid_args.clone()
        };
        assert_eq!(
            concurrency_oversized.validate().unwrap_err().to_string(),
            "worker concurrency cannot exceed 32 (requested: 33)"
        );

        let non_loopback_solr = StaffResourceCandidateArgs {
            solr_url: "http://example.com:8983/solr".to_string(),
            ..valid_args.clone()
        };
        assert!(non_loopback_solr.validate().is_err());

        let invalid_core = StaffResourceCandidateArgs {
            candidate_core: "archivesspace".to_string(),
            ..valid_args.clone()
        };
        assert!(invalid_core.validate().is_err());
    }

    #[test]
    fn test_parent_and_child_extraction_order_and_marker_stripping() {
        let parent_json = json!({
            "id": "/repositories/4/resources/10",
            "uri": "/repositories/4/resources/10",
            "title": "Resource Ten",
            "primary_type": "resource",
            "_archivalist_marker_1": "private_marker_parent",
            "_archivalist_collection_management_documents": [
                {
                    "id": "/repositories/4/collection_management/50",
                    "uri": "/repositories/4/collection_management/50",
                    "parent_id": "/repositories/4/resources/10",
                    "parent_type": "resource",
                    "primary_type": "collection_management",
                    "title": "Resource Ten",
                    "json": "{\"processing_priority\": \"high\"}",
                    "_archivalist_marker_2": "private_marker_child"
                }
            ]
        });

        let extracted = extract_resource_batch_documents(vec![parent_json], 4).unwrap();
        assert_eq!(extracted.parent_count, 1);
        assert_eq!(extracted.child_count, 1);
        assert_eq!(extracted.ordered_documents.len(), 2);
        assert_eq!(
            extracted.parent_uris,
            vec!["/repositories/4/resources/10".to_string()]
        );
        assert_eq!(
            extracted.child_uris,
            vec!["/repositories/4/collection_management/50".to_string()]
        );

        // Native hook order: parent first, then child
        let doc0 = &extracted.ordered_documents[0];
        let doc1 = &extracted.ordered_documents[1];
        assert_eq!(doc0["id"], "/repositories/4/resources/10");
        assert_eq!(doc1["id"], "/repositories/4/collection_management/50");

        // Verify all _archivalist_* markers stripped from parent
        let parent_map = doc0.as_object().unwrap();
        assert!(!parent_map.contains_key("_archivalist_marker_1"));
        assert!(!parent_map.contains_key("_archivalist_collection_management_documents"));
        assert!(!parent_map.keys().any(|k| k.starts_with("_archivalist_")));

        // Verify all _archivalist_* markers stripped from child
        let child_map = doc1.as_object().unwrap();
        assert!(!child_map.contains_key("_archivalist_marker_2"));
        assert!(!child_map.keys().any(|k| k.starts_with("_archivalist_")));

        // Exact child identity preserved
        assert_eq!(doc1["parent_id"], "/repositories/4/resources/10");
        assert_eq!(doc1["parent_type"], "resource");
        assert_eq!(doc1["primary_type"], "collection_management");
        assert_eq!(doc1["json"], "{\"processing_priority\": \"high\"}");
    }

    #[test]
    fn test_resource_without_child_emits_parent_only() {
        let parent_json = json!({
            "id": "/repositories/4/resources/11",
            "uri": "/repositories/4/resources/11",
            "title": "Resource Eleven",
            "primary_type": "resource",
            "_archivalist_some_flag": true
        });

        let extracted = extract_resource_batch_documents(vec![parent_json], 4).unwrap();
        assert_eq!(extracted.parent_count, 1);
        assert_eq!(extracted.child_count, 0);
        assert_eq!(extracted.ordered_documents.len(), 1);
        assert_eq!(
            extracted.ordered_documents[0]["id"],
            "/repositories/4/resources/11"
        );
        assert!(!extracted.ordered_documents[0]
            .as_object()
            .unwrap()
            .contains_key("_archivalist_some_flag"));
    }

    #[test]
    fn test_published_resource_appends_pui_availability_types() {
        // 1. Published resource gets ["pui_collection", "pui"] appended to types
        let published_parent = json!({
            "id": "/repositories/4/resources/20",
            "uri": "/repositories/4/resources/20",
            "title": "Published Resource",
            "primary_type": "resource",
            "publish": true,
            "types": ["resource"],
        });
        let extracted_pub = extract_resource_batch_documents(vec![published_parent], 4).unwrap();
        assert_eq!(
            extracted_pub.ordered_documents[0]["types"],
            json!(["resource", "pui_collection", "pui"])
        );

        // 2. Unpublished resource preserves types without pui additions
        let unpublished_parent = json!({
            "id": "/repositories/4/resources/21",
            "uri": "/repositories/4/resources/21",
            "title": "Unpublished Resource",
            "primary_type": "resource",
            "publish": false,
            "types": ["resource"],
        });
        let extracted_unpub =
            extract_resource_batch_documents(vec![unpublished_parent], 4).unwrap();
        assert_eq!(
            extracted_unpub.ordered_documents[0]["types"],
            json!(["resource"])
        );

        // 3. Collection management child does NOT receive pui additions
        let parent_with_cm = json!({
            "id": "/repositories/4/resources/22",
            "uri": "/repositories/4/resources/22",
            "title": "Published Resource With CM",
            "primary_type": "resource",
            "publish": true,
            "types": ["resource"],
            "_archivalist_collection_management_documents": [
                {
                    "id": "/repositories/4/collection_management/99",
                    "uri": "/repositories/4/collection_management/99",
                    "parent_id": "/repositories/4/resources/22",
                    "parent_type": "resource",
                    "primary_type": "collection_management",
                    "types": ["collection_management"],
                    "title": "Published Resource With CM",
                    "json": "{}"
                }
            ]
        });
        let extracted_cm = extract_resource_batch_documents(vec![parent_with_cm], 4).unwrap();
        assert_eq!(
            extracted_cm.ordered_documents[0]["types"],
            json!(["resource", "pui_collection", "pui"])
        );
        assert_eq!(
            extracted_cm.ordered_documents[1]["types"],
            json!(["collection_management"])
        );
    }

    #[test]
    fn test_malformed_child_envelope_fails_closed() {
        // Child envelope is a string instead of an array
        let bad_envelope = json!({
            "id": "/repositories/4/resources/12",
            "uri": "/repositories/4/resources/12",
            "title": "Bad Envelope",
            "_archivalist_collection_management_documents": "not_an_array"
        });

        let res = extract_resource_batch_documents(vec![bad_envelope], 4);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("expected JSON array"));
    }

    #[test]
    fn test_malformed_child_not_object_fails_closed() {
        let child_not_obj = json!({
            "id": "/repositories/4/resources/13",
            "uri": "/repositories/4/resources/13",
            "title": "Child not object",
            "_archivalist_collection_management_documents": [123]
        });

        let res = extract_resource_batch_documents(vec![child_not_obj], 4);
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("is not a JSON object"));
    }

    #[test]
    fn test_malformed_child_identity_and_parent_mismatch_fails_closed() {
        // Missing id
        let missing_id = json!({
            "id": "/repositories/4/resources/14",
            "uri": "/repositories/4/resources/14",
            "_archivalist_collection_management_documents": [
                {
                    "uri": "/repositories/4/collection_management/50",
                    "parent_id": "/repositories/4/resources/14",
                    "parent_type": "resource",
                    "primary_type": "collection_management"
                }
            ]
        });
        assert!(extract_resource_batch_documents(vec![missing_id], 4).is_err());

        // id != uri
        let id_uri_mismatch = json!({
            "id": "/repositories/4/resources/15",
            "uri": "/repositories/4/resources/15",
            "_archivalist_collection_management_documents": [
                {
                    "id": "/repositories/4/collection_management/999",
                    "uri": "/repositories/4/collection_management/50",
                    "parent_id": "/repositories/4/resources/15",
                    "parent_type": "resource",
                    "primary_type": "collection_management"
                }
            ]
        });
        assert!(extract_resource_batch_documents(vec![id_uri_mismatch], 4).is_err());

        // parent_id != parent URI
        let parent_mismatch = json!({
            "id": "/repositories/4/resources/16",
            "uri": "/repositories/4/resources/16",
            "_archivalist_collection_management_documents": [
                {
                    "id": "/repositories/4/collection_management/50",
                    "uri": "/repositories/4/collection_management/50",
                    "parent_id": "/repositories/4/resources/999",
                    "parent_type": "resource",
                    "primary_type": "collection_management"
                }
            ]
        });
        assert!(extract_resource_batch_documents(vec![parent_mismatch], 4).is_err());

        // parent_type != resource
        let wrong_parent_type = json!({
            "id": "/repositories/4/resources/17",
            "uri": "/repositories/4/resources/17",
            "_archivalist_collection_management_documents": [
                {
                    "id": "/repositories/4/collection_management/50",
                    "uri": "/repositories/4/collection_management/50",
                    "parent_id": "/repositories/4/resources/17",
                    "parent_type": "accession",
                    "primary_type": "collection_management"
                }
            ]
        });
        assert!(extract_resource_batch_documents(vec![wrong_parent_type], 4).is_err());

        // primary_type != collection_management
        let wrong_primary_type = json!({
            "id": "/repositories/4/resources/18",
            "uri": "/repositories/4/resources/18",
            "_archivalist_collection_management_documents": [
                {
                    "id": "/repositories/4/collection_management/50",
                    "uri": "/repositories/4/collection_management/50",
                    "parent_id": "/repositories/4/resources/18",
                    "parent_type": "resource",
                    "primary_type": "resource"
                }
            ]
        });
        assert!(extract_resource_batch_documents(vec![wrong_primary_type], 4).is_err());

        // Wrong URI prefix
        let wrong_child_prefix = json!({
            "id": "/repositories/4/resources/19",
            "uri": "/repositories/4/resources/19",
            "_archivalist_collection_management_documents": [
                {
                    "id": "/repositories/9/collection_management/50",
                    "uri": "/repositories/9/collection_management/50",
                    "parent_id": "/repositories/4/resources/19",
                    "parent_type": "resource",
                    "primary_type": "collection_management"
                }
            ]
        });
        assert!(extract_resource_batch_documents(vec![wrong_child_prefix], 4).is_err());
    }

    #[test]
    fn test_wrong_parent_uri_fails_closed() {
        let wrong_repo = json!({
            "id": "/repositories/99/resources/10",
            "uri": "/repositories/99/resources/10",
            "title": "Wrong Repo"
        });
        assert!(extract_resource_batch_documents(vec![wrong_repo], 4).is_err());

        let non_numeric = json!({
            "id": "/repositories/4/resources/abc",
            "uri": "/repositories/4/resources/abc",
            "title": "Non Numeric"
        });
        assert!(extract_resource_batch_documents(vec![non_numeric], 4).is_err());
    }

    #[tokio::test]
    async fn test_candidate_refuses_invalid_core_or_solr() {
        let server = Server::new_async().await;
        let mut client = Client::new(server.url());
        client.session_token = Some("token".to_string());

        let bad_core_args = StaffResourceCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: "http://127.0.0.1:18983/solr".to_string(),
            candidate_core: "stage_staff_candidate".to_string(), // missing nonce
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: false,
            fail_after_batches: None,
        };

        let summary = execute_staff_resource_candidate_with_verifier(
            &client,
            &bad_core_args,
            |_path, _backend, _solr| Ok(()),
        )
        .await;
        assert!(!summary.success);
        assert_eq!(summary.failures, 1);
        assert!(!summary.committed);
        assert!(summary
            .error_message
            .unwrap()
            .contains("Refusing core name"));
    }

    #[tokio::test]
    async fn test_verification_failure_fails_closed_before_core_readiness_or_solr_write() {
        let server = Server::new_async().await;
        let mut client = Client::new(server.url());
        client.session_token = Some("token".to_string());

        let args = StaffResourceCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: "http://127.0.0.1:18983/solr".to_string(),
            candidate_core: "stage_staff_20261027_candidate".to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: false,
            fail_after_batches: None,
        };

        let summary = execute_staff_resource_candidate_with_verifier(
            &client,
            &args,
            |_path, _backend, _solr| bail!("injected verification failure"),
        )
        .await;

        assert!(!summary.success);
        assert_eq!(summary.failures, 1);
        assert!(!summary.committed);
        assert!(summary
            .error_message
            .unwrap()
            .contains("Profiling stack verification failed: injected verification failure"));
    }

    #[tokio::test]
    async fn test_full_mocked_resource_candidate_success_and_child_accounting() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("test-session".to_string());

        // 1. Mock schemas endpoint
        let _schemas_mock = aspace_server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "test-session")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"resource": {"properties": {"level": {"dynamic_enum": "archival_record_level"}}}}"#)
            .create_async()
            .await;

        // 2. Mock resource IDs modified since 0
        let _ids_mock = aspace_server
            .mock("GET", "/repositories/4/resources")
            .match_header("X-ArchivesSpace-Session", "test-session")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("all_ids".to_string(), "true".to_string()),
                mockito::Matcher::UrlEncoded("modified_since".to_string(), "0".to_string()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[101, 102]")
            .create_async()
            .await;

        // 3. Mock resource hydration batch (101 has collection_management, 102 does not)
        let resource_batch_body = r#"[
            {
                "uri": "/repositories/4/resources/101",
                "title": "Papers of Alice",
                "id_0": "MS101",
                "level": "collection",
                "dates": [{"date_type": "single", "expression": "1940"}],
                "extents": [{"number": "1", "extent_type": "box"}],
                "collection_management": {
                    "uri": "/repositories/4/collection_management/501",
                    "processing_priority": "high"
                }
            },
            {
                "uri": "/repositories/4/resources/102",
                "title": "Papers of Bob",
                "id_0": "MS102",
                "level": "collection",
                "dates": [{"date_type": "single", "expression": "1950"}],
                "extents": [{"number": "2", "extent_type": "boxes"}]
            }
        ]"#;

        let _hydrate_mock = aspace_server
            .mock("GET", "/repositories/4/resources")
            .match_header("X-ArchivesSpace-Session", "test-session")
            .match_query(mockito::Matcher::AllOf(vec![mockito::Matcher::UrlEncoded(
                "id_set".to_string(),
                "101,102".to_string(),
            )]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(resource_batch_body)
            .create_async()
            .await;

        let core_name = "stage_staff_20261027_candidate";

        // Solr core readiness: STATUS
        let _solr_status = solr_server
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
                            "instanceDir": "/var/solr/data/stage_staff_20261027_candidate"
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        // Solr core readiness: empty check
        let _solr_select = solr_server
            .mock(
                "GET",
                format!("/{core_name}/select?q=*:*&rows=0&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "responseHeader": {"status": 0},
                    "response": {
                        "numFound": 0,
                        "start": 0,
                        "docs": []
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        // Solr updates: update POST and commit POST
        let _solr_update = solr_server
            .mock("POST", format!("/{core_name}/update").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader": {"status": 0, "QTime": 1}}"#)
            .expect_at_least(1)
            .create_async()
            .await;

        let args = StaffResourceCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: solr_server.url(),
            candidate_core: core_name.to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: true,
            fail_after_batches: None,
        };

        let summary = execute_staff_resource_candidate_with_verifier(
            &client,
            &args,
            |_path, _backend, _solr| Ok(()),
        )
        .await;

        assert!(
            summary.success,
            "Summary error: {:?}",
            summary.error_message
        );
        assert!(summary.committed);
        assert_eq!(summary.failures, 0);
        assert_eq!(summary.total_ids_discovered, 2);
        assert_eq!(summary.exact_ids_indexed, 2);
        assert_eq!(summary.parent_docs, 2);
        assert_eq!(summary.child_docs, 1); // Only 101 had CM
        assert_eq!(summary.total_docs_indexed, 3); // 2 parents + 1 child
        assert_eq!(summary.unique_uris, 3); // 2 resource URIs + 1 CM URI
        assert_eq!(summary.missing_ids, 0);
        assert_eq!(summary.duplicate_ids, 0);
        assert_eq!(summary.unexpected_ids, 0);
        assert!(summary.solr_update_requests_sent >= 1);
    }

    #[tokio::test]
    async fn test_writer_non_commit_on_batch_error() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("test-session".to_string());

        let _schemas_mock = aspace_server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{}"#)
            .create_async()
            .await;

        let _ids_mock = aspace_server
            .mock("GET", "/repositories/4/resources")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[101, 102]")
            .create_async()
            .await;

        // Backend fails during hydration
        let _hydrate_mock = aspace_server
            .mock("GET", "/repositories/4/resources")
            .match_query(mockito::Matcher::AllOf(vec![mockito::Matcher::UrlEncoded(
                "id_set".to_string(),
                "101,102".to_string(),
            )]))
            .with_status(500)
            .with_body("Hydration exploded")
            .create_async()
            .await;

        let core_name = "stage_staff_20261027_candidate";

        let _solr_status = solr_server
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
                            "instanceDir": "/var/solr/data/stage_staff_20261027_candidate"
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let _solr_select = solr_server
            .mock(
                "GET",
                format!("/{core_name}/select?q=*:*&rows=0&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "responseHeader": {"status": 0},
                    "response": {
                        "numFound": 0,
                        "start": 0,
                        "docs": []
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let args = StaffResourceCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: solr_server.url(),
            candidate_core: core_name.to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: true,
            fail_after_batches: None,
        };

        let summary = execute_staff_resource_candidate_with_verifier(
            &client,
            &args,
            |_path, _backend, _solr| Ok(()),
        )
        .await;

        assert!(!summary.success);
        assert!(!summary.committed);
        assert_eq!(summary.batches_failed, 1);
        assert!(summary.failures >= 1);
    }

    #[tokio::test]
    async fn test_resource_candidate_injected_failure_without_commit() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261027_res_fail_candidate";

        let _m_status = solr_server
            .mock(
                "GET",
                format!("/admin/cores?action=STATUS&core={candidate_core}&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "responseHeader": {"status": 0},
                    "status": {
                        candidate_core: {
                            "name": candidate_core,
                            "instanceDir": "/var/solr/data/stage_staff_20261027_res_fail_candidate"
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
                format!("/{candidate_core}/select?q=*:*&rows=0&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "responseHeader": {"status": 0},
                    "response": {
                        "numFound": 0,
                        "start": 0,
                        "docs": []
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let _m_schemas = aspace_server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{}"#)
            .create_async()
            .await;

        let all_ids: Vec<u32> = (1..=525).collect();
        let _m_ids = aspace_server
            .mock(
                "GET",
                "/repositories/4/resources?all_ids=true&modified_since=0",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&all_ids).unwrap())
            .create_async()
            .await;

        let mut _batch_mocks = Vec::new();
        for chunk in all_ids.chunks(25) {
            let id_str = chunk
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let body = serde_json::to_string(
                &chunk
                    .iter()
                    .map(|id| {
                        json!({
                            "uri": format!("/repositories/4/resources/{id}"),
                            "title": format!("Resource {id}"),
                            "id_0": format!("MS{id}"),
                            "level": "collection",
                            "dates": [{"date_type": "single", "expression": "1940"}],
                            "extents": [{"number": "1", "extent_type": "box"}]
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();

            let enc_ids = id_str.replace(',', "%2C");
            let regex = format!(r"^/repositories/4/resources\?id_set={enc_ids}(&|$)");
            let m = aspace_server
                .mock("GET", mockito::Matcher::Regex(regex))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(body)
                .create_async()
                .await;
            _batch_mocks.push(m);
        }

        let _m_update = solr_server
            .mock("POST", format!("/{candidate_core}/update").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .expect_at_least(1)
            .create_async()
            .await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());

        let args = StaffResourceCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 1,
            solr_url: solr_server.url(),
            candidate_core: candidate_core.to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: true,
            fail_after_batches: Some(21),
        };

        let summary = execute_staff_resource_candidate_with_verifier(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
        )
        .await;

        assert!(!summary.success);
        assert!(!summary.committed);
        assert!(summary.failures >= 1);
        assert_eq!(summary.batches_completed, 21);
        assert_eq!(summary.exact_ids_indexed, 525);
        assert!(summary.solr_update_requests_sent >= 1);
        assert_eq!(summary.solr_commit_elapsed_sec, 0.0);
        assert!(summary
            .error_message
            .unwrap()
            .contains("Injected deterministic test failure after 21 completed batch"));
        _m_update.assert_async().await;
    }
}
