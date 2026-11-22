use anyhow::{bail, Context, Result};
use aspace_client::{
    build_index_document_batch, Client, IndexDocumentBatchRequest, IndexDocumentInput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Write};

use crate::staff_ao_builder::strip_archivalist_markers;
pub use crate::staff_candidate_runner::{
    partition_ids_into_batches, render_candidate_summary_to_writer, CandidateBatchOutput,
    StaffCandidateArgs, StaffCandidateArgs as StaffAoCandidateArgs, StaffCandidateExecutionResult,
    DEFAULT_CANDIDATE_BATCH_SIZE, DEFAULT_CANDIDATE_CONCURRENCY, MAX_CANDIDATE_BATCH_SIZE,
    MAX_CANDIDATE_CONCURRENCY,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StaffAoCandidateSummary {
    pub repository_id: u32,
    pub total_ids_discovered: usize,
    pub batch_size: usize,
    pub concurrency: usize,
    pub total_batches: usize,
    pub batches_completed: usize,
    pub batches_failed: usize,
    pub failures: usize,
    pub exact_ids_indexed: usize,
    pub unique_uris: usize,
    pub missing_ids: usize,
    pub duplicate_ids: usize,
    pub unexpected_ids: usize,
    pub candidate_core: String,
    pub solr_url: String,
    pub arks_enabled: bool,
    pub id_list_elapsed_sec: f64,
    /// Combined wall interval for concurrent batch fetch, document building,
    /// and pipelined streaming writes to Solr.
    #[serde(alias = "batch_fetch_build_elapsed_sec")]
    pub batch_loop_wall_elapsed_sec: f64,
    /// Cumulative duration spent awaiting Solr update POST requests.
    /// Note: This is an overlapping sub-component of `batch_loop_wall_elapsed_sec`
    /// (streaming writes occur concurrently with ongoing worker batch fetches);
    /// do NOT sum `solr_write_elapsed_sec` with `batch_loop_wall_elapsed_sec`.
    pub solr_write_elapsed_sec: f64,
    /// Total number of successful update POST requests dispatched to Solr prior to completion/failure.
    #[serde(default)]
    pub solr_update_requests_sent: usize,
    pub solr_commit_elapsed_sec: f64,
    pub total_wall_elapsed_sec: f64,
    pub records_per_second: f64,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl From<StaffCandidateExecutionResult> for StaffAoCandidateSummary {
    fn from(res: StaffCandidateExecutionResult) -> Self {
        Self {
            repository_id: res.repository_id,
            total_ids_discovered: res.total_ids_discovered,
            batch_size: res.batch_size,
            concurrency: res.concurrency,
            total_batches: res.total_batches,
            batches_completed: res.batches_completed,
            batches_failed: res.batches_failed,
            failures: res.failures,
            exact_ids_indexed: res.exact_ids_indexed,
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
            success: res.success,
            error_message: res.error_message,
        }
    }
}

pub async fn fetch_and_build_ao_candidate_batch(
    client: &Client,
    repo_id: u32,
    chunk: &[u32],
    batch_idx: usize,
    dynamic_enums: std::sync::Arc<Vec<String>>,
    arks_enabled: bool,
) -> Result<CandidateBatchOutput> {
    let raw_records = client
        .fetch_staff_archival_objects_by_ids(repo_id, chunk)
        .await
        .with_context(|| format!("Failed to fetch AO batch {}", batch_idx))?;

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
            record_type: "archival_object".to_string(),
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

    let mut docs = batch_resp.documents;
    // Marker removal is mandatory before adding documents / writing to Solr
    for doc in &mut docs {
        strip_archivalist_markers(doc);
    }

    let parent_uris = docs
        .iter()
        .map(|d| d.get("uri").and_then(Value::as_str).map(|s| s.to_string()))
        .collect();

    let parent_count = docs.len();
    Ok(CandidateBatchOutput {
        ordered_documents: docs,
        parent_uris,
        child_uris: Vec::new(),
        parent_count,
        child_count: 0,
    })
}

pub async fn execute_staff_ao_candidate(
    client: &Client,
    args: &StaffAoCandidateArgs,
) -> StaffAoCandidateSummary {
    execute_staff_ao_candidate_with_verifier(
        client,
        args,
        crate::profiling_stack::verify_profiling_stack,
    )
    .await
}

pub async fn execute_staff_ao_candidate_with_verifier<V>(
    client: &Client,
    args: &StaffAoCandidateArgs,
    verifier: V,
) -> StaffAoCandidateSummary
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
{
    let repo_id = args.repo;
    let arks_enabled = args.arks_enabled;
    let expected_prefix = format!("/repositories/{}/archival_objects/", repo_id);
    let descriptor = crate::staff_candidate_runner::StaffCandidateDescriptor {
        expected_parent_prefix: &expected_prefix,
        id_type_label: "archival object",
        id_list_label: "AO",
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
                .archival_object_ids_modified_since(repo_id, Some(0))
                .await
        },
        move |batch_idx, chunk, dynamic_enums| {
            let client = client_for_batches.clone();
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
    )
    .await;

    res.into()
}

pub fn print_human_candidate_summary(summary: &StaffAoCandidateSummary) {
    let mut stdout = io::stdout();
    let _ = print_human_candidate_summary_to_writer(summary, &mut stdout);
}

pub fn print_human_candidate_summary_to_writer<W: Write>(
    summary: &StaffAoCandidateSummary,
    writer: &mut W,
) -> io::Result<()> {
    writeln!(
        writer,
        "=== STAFF Archival Object Candidate Indexer Summary ==="
    )?;
    writeln!(writer, "Repository ID: {}", summary.repository_id)?;
    writeln!(writer, "Candidate Core: {}", summary.candidate_core)?;
    writeln!(
        writer,
        "Total Discovered IDs: {}",
        summary.total_ids_discovered
    )?;
    writeln!(writer, "Batch Size: {}", summary.batch_size)?;
    writeln!(writer, "Concurrency: {}", summary.concurrency)?;
    writeln!(writer, "Total Batches: {}", summary.total_batches)?;
    writeln!(writer, "Batches Completed: {}", summary.batches_completed)?;
    writeln!(writer, "Batches Failed: {}", summary.batches_failed)?;
    writeln!(writer, "Total Failures: {}", summary.failures)?;
    writeln!(writer, "Exact IDs Indexed: {}", summary.exact_ids_indexed)?;
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

pub async fn run_staff_ao_candidate(
    client: &Client,
    args: StaffAoCandidateArgs,
) -> Result<StaffAoCandidateSummary> {
    run_staff_ao_candidate_to_writer(client, args, &mut io::stdout()).await
}

pub async fn run_staff_ao_candidate_to_writer<W: Write>(
    client: &Client,
    args: StaffAoCandidateArgs,
    writer: &mut W,
) -> Result<StaffAoCandidateSummary> {
    run_staff_ao_candidate_to_writer_with_verifier(
        client,
        args,
        writer,
        crate::profiling_stack::verify_profiling_stack,
    )
    .await
}

pub async fn run_staff_ao_candidate_to_writer_with_verifier<W: Write, V>(
    client: &Client,
    args: StaffAoCandidateArgs,
    writer: &mut W,
    verifier: V,
) -> Result<StaffAoCandidateSummary>
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
{
    let summary = execute_staff_ao_candidate_with_verifier(client, &args, verifier).await;

    render_candidate_summary_to_writer(
        writer,
        &summary,
        args.json,
        summary.success,
        summary.error_message.as_deref(),
        "archival object",
        |w| print_human_candidate_summary_to_writer(&summary, w),
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
        let valid_args = StaffAoCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: "http://127.0.0.1:18983/solr".to_string(),
            candidate_core: "stage_staff_20261026_candidate".to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/profiling_state.json")),
            json: false,
            fail_after_batches: None,
        };
        assert!(valid_args.validate().is_ok());

        let missing_state = StaffAoCandidateArgs {
            state: None,
            ..valid_args.clone()
        };
        assert!(missing_state.validate().is_err());
        assert_eq!(
            missing_state.validate().unwrap_err().to_string(),
            "missing required profiling-stack state file for Solr write mode (--state)"
        );

        let empty_state = StaffAoCandidateArgs {
            state: Some(std::path::PathBuf::from("")),
            ..valid_args.clone()
        };
        assert!(empty_state.validate().is_err());

        let strip_markers_false = StaffAoCandidateArgs {
            strip_markers: false,
            ..valid_args.clone()
        };
        assert!(strip_markers_false.validate().is_err());
        assert_eq!(
            strip_markers_false.validate().unwrap_err().to_string(),
            "--strip-markers false is forbidden in Solr write mode; marker stripping is mandatory"
        );

        let fail_zero = StaffAoCandidateArgs {
            fail_after_batches: Some(0),
            ..valid_args.clone()
        };
        assert_eq!(
            fail_zero.validate().unwrap_err().to_string(),
            "fail_after_batches must be greater than 0 if specified"
        );

        let repo_zero = StaffAoCandidateArgs {
            repo: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            repo_zero.validate().unwrap_err().to_string(),
            "repo ID must be greater than 0"
        );

        let batch_zero = StaffAoCandidateArgs {
            batch_size: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            batch_zero.validate().unwrap_err().to_string(),
            "batch size must be greater than 0"
        );

        let batch_oversized = StaffAoCandidateArgs {
            batch_size: 26,
            ..valid_args.clone()
        };
        assert_eq!(
            batch_oversized.validate().unwrap_err().to_string(),
            "batch size cannot exceed 25 (requested: 26)"
        );

        let concurrency_zero = StaffAoCandidateArgs {
            concurrency: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            concurrency_zero.validate().unwrap_err().to_string(),
            "worker concurrency must be greater than 0"
        );

        let concurrency_oversized = StaffAoCandidateArgs {
            concurrency: 33,
            ..valid_args.clone()
        };
        assert_eq!(
            concurrency_oversized.validate().unwrap_err().to_string(),
            "worker concurrency cannot exceed 32 (requested: 33)"
        );

        let non_loopback_solr = StaffAoCandidateArgs {
            solr_url: "http://example.com:8983/solr".to_string(),
            ..valid_args.clone()
        };
        assert!(non_loopback_solr.validate().is_err());

        let invalid_core = StaffAoCandidateArgs {
            candidate_core: "archivesspace".to_string(),
            ..valid_args.clone()
        };
        assert!(invalid_core.validate().is_err());
    }

    #[test]
    fn test_partition_ids_into_batches_compat() {
        assert_eq!(partition_ids_into_batches(&[], 25), Vec::<Vec<u32>>::new());
        assert_eq!(
            partition_ids_into_batches(&[1, 2, 3], 0),
            Vec::<Vec<u32>>::new()
        );
        let ids: Vec<u32> = (1..=55).collect();
        let batches = partition_ids_into_batches(&ids, 25);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 25);
        assert_eq!(batches[1].len(), 25);
        assert_eq!(batches[2].len(), 5);
    }

    #[tokio::test]
    async fn test_candidate_refuses_invalid_core_or_solr() {
        let server = Server::new_async().await;
        let mut client = Client::new(server.url());
        client.session_token = Some("dummy-session".to_string());

        let args = StaffAoCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: "http://example.com/solr".to_string(), // non-loopback
            candidate_core: "stage_staff_20261026_candidate".to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: false,
            fail_after_batches: None,
        };

        let summary = execute_staff_ao_candidate(&client, &args).await;
        assert!(!summary.success);
        assert_eq!(summary.failures, 1);
        assert!(summary.error_message.unwrap().contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn test_candidate_full_success_mocked() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261026_1234_candidate";

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
                            "instanceDir": "/var/solr/data/stage_staff_20261026_1234_candidate"
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
            .with_body(
                json!({
                    "archival_object": {
                        "properties": {
                            "level": {
                                "type": "string",
                                "dynamic_enum": "archival_record_level"
                            }
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let _m_ids = aspace_server
            .mock(
                "GET",
                "/repositories/4/archival_objects?all_ids=true&modified_since=0",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[101, 102]")
            .create_async()
            .await;

        let _m_batch = aspace_server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/repositories/4/archival_objects\?id_set=".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!([
                    {
                        "uri": "/repositories/4/archival_objects/101",
                        "title": "Item 101",
                        "level": "item",
                        "jsonmodel_type": "archival_object",
                        "repository": {"ref": "/repositories/4"}
                    },
                    {
                        "uri": "/repositories/4/archival_objects/102",
                        "title": "Item 102",
                        "level": "item",
                        "jsonmodel_type": "archival_object",
                        "repository": {"ref": "/repositories/4"}
                    }
                ])
                .to_string(),
            )
            .create_async()
            .await;

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
        let solr_url = solr_server.url();

        let args = StaffAoCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url,
            candidate_core: candidate_core.to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: true,
            fail_after_batches: None,
        };

        let mut output = Vec::new();
        let summary = run_staff_ao_candidate_to_writer_with_verifier(
            &client,
            args,
            &mut output,
            |_state, _backend, _solr| Ok(()),
        )
        .await
        .unwrap();

        assert!(summary.success);
        assert_eq!(summary.failures, 0);
        assert_eq!(summary.total_ids_discovered, 2);
        assert_eq!(summary.exact_ids_indexed, 2);
        assert_eq!(summary.unique_uris, 2);
        assert_eq!(summary.missing_ids, 0);
        assert_eq!(summary.duplicate_ids, 0);
        assert_eq!(summary.unexpected_ids, 0);
        assert!(summary.id_list_elapsed_sec >= 0.0);
        assert!(summary.batch_loop_wall_elapsed_sec >= 0.0);
        assert!(summary.total_wall_elapsed_sec >= 0.0);
    }

    #[tokio::test]
    async fn test_candidate_injected_failure_after_batches() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261026_5678_candidate";

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
                            "instanceDir": "/var/solr/data/stage_staff_20261026_5678_candidate"
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
            .with_body(
                json!({
                    "archival_object": {
                        "properties": {
                            "level": {
                                "type": "string",
                                "dynamic_enum": "archival_record_level"
                            }
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let all_ids: Vec<u32> = (1..=525).collect();
        let _m_ids = aspace_server
            .mock(
                "GET",
                "/repositories/4/archival_objects?all_ids=true&modified_since=0",
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
                            "uri": format!("/repositories/4/archival_objects/{id}"),
                            "title": format!("Item {id}"),
                            "level": "item",
                            "jsonmodel_type": "archival_object",
                            "repository": {"ref": "/repositories/4"}
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();

            let enc_ids = id_str.replace(',', "%2C");
            let regex = format!(r"^/repositories/4/archival_objects\?id_set={enc_ids}(&|$)");
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
        let solr_url = solr_server.url();

        let args = StaffAoCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 1,
            solr_url,
            candidate_core: candidate_core.to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: true,
            fail_after_batches: Some(21),
        };

        let summary =
            execute_staff_ao_candidate_with_verifier(&client, &args, |_state, _backend, _solr| {
                Ok(())
            })
            .await;

        assert!(!summary.success);
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

    #[tokio::test]
    async fn test_candidate_verification_failure_fails_closed_before_core_check_or_solr_write() {
        let aspace_server = Server::new_async().await;
        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());

        let args = StaffAoCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: "http://127.0.0.1:18983/solr".to_string(),
            candidate_core: "stage_staff_20261026_verif99_candidate".to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: false,
            fail_after_batches: None,
        };

        let summary =
            execute_staff_ao_candidate_with_verifier(&client, &args, |_state, _backend, _solr| {
                bail!("Challenge-response verification failed: backend not on profiling stack");
            })
            .await;

        assert!(!summary.success);
        assert_eq!(summary.failures, 1);
        let err_msg = summary.error_message.expect("error message present");
        assert!(err_msg.contains("Profiling stack verification failed"));
        assert!(err_msg.contains("Challenge-response verification failed"));
    }

    #[tokio::test]
    async fn test_candidate_passes_exact_state_backend_solr_to_verifier() {
        let aspace_server = Server::new_async().await;
        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());

        let state_path = std::path::PathBuf::from("/benchmarks/profiling_stack_test123.json");
        let expected_backend = client.base_url().to_string();
        let expected_solr = "http://127.0.0.1:18983/solr".to_string();

        let args = StaffAoCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: expected_solr.clone(),
            candidate_core: "stage_staff_20261026_verif99_candidate".to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(state_path.clone()),
            json: false,
            fail_after_batches: None,
        };

        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let verified = Arc::new(AtomicBool::new(false));
        let verified_clone = Arc::clone(&verified);

        let exp_state_clone = state_path.clone();
        let exp_back_clone = expected_backend.clone();
        let exp_solr_clone = expected_solr.clone();

        let _summary = execute_staff_ao_candidate_with_verifier(
            &client,
            &args,
            move |state, backend, solr| {
                assert_eq!(state, exp_state_clone.as_path());
                assert_eq!(backend, exp_back_clone.as_str());
                assert_eq!(solr, exp_solr_clone.as_str());
                verified_clone.store(true, Ordering::SeqCst);
                bail!("Stop after verifier argument check");
            },
        )
        .await;

        assert!(
            verified.load(Ordering::SeqCst),
            "Verifier must have been invoked with exact arguments"
        );
    }

    #[tokio::test]
    async fn test_ao_batch_fetch_failure_without_commit() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261026_ao_fail_candidate";

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
                            "instanceDir": "/var/solr/data/stage_staff_20261026_ao_fail_candidate"
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

        let _m_ids = aspace_server
            .mock(
                "GET",
                "/repositories/4/archival_objects?all_ids=true&modified_since=0",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[101, 102]")
            .create_async()
            .await;

        let _m_batch = aspace_server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/repositories/4/archival_objects\?id_set=".to_string()),
            )
            .with_status(500)
            .with_body("Backend exploded on AO batch fetch")
            .create_async()
            .await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());

        let args = StaffAoCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: solr_server.url(),
            candidate_core: candidate_core.to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: false,
            fail_after_batches: None,
        };

        let summary =
            execute_staff_ao_candidate_with_verifier(&client, &args, |_state, _backend, _solr| {
                Ok(())
            })
            .await;

        assert!(!summary.success);
        assert_eq!(summary.batches_failed, 1);
        assert!(summary.failures >= 1);
        assert_eq!(summary.solr_commit_elapsed_sec, 0.0);
        assert!(summary
            .error_message
            .unwrap()
            .contains("Failed to fetch AO batch 0"));
    }
}
