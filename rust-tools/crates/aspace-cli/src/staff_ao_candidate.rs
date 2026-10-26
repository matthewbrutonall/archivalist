use anyhow::{bail, Context, Result};
use aspace_client::isolated_solr_writer::{
    validate_isolated_solr_base_url, validate_stage_candidate_core, IsolatedSolrStaffAoWriter,
};
use aspace_client::{
    build_index_document_batch, Client, IndexDocumentBatchRequest, IndexDocumentInput,
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{self, Write};
use std::time::Instant;

use crate::staff_ao_builder::strip_archivalist_markers;

pub const DEFAULT_CANDIDATE_BATCH_SIZE: usize = 25;
pub const MAX_CANDIDATE_BATCH_SIZE: usize = 25;
pub const DEFAULT_CANDIDATE_CONCURRENCY: usize = 4;
pub const MAX_CANDIDATE_CONCURRENCY: usize = 32;

#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct StaffAoCandidateArgs {
    #[arg(
        short,
        long,
        alias = "repo-id",
        help = "Repository ID",
        default_value_t = 4
    )]
    pub repo: u32,

    #[arg(
        long,
        help = "Batch size for AO hydration (default: 25, max: 25)",
        default_value_t = DEFAULT_CANDIDATE_BATCH_SIZE
    )]
    pub batch_size: usize,

    #[arg(
        long,
        help = "Worker concurrency for batch fetching (default: 4, max: 32)",
        default_value_t = DEFAULT_CANDIDATE_CONCURRENCY
    )]
    pub concurrency: usize,

    #[arg(
        long,
        help = "Isolated Solr base URL (e.g. http://127.0.0.1:18983/solr)"
    )]
    pub solr_url: String,

    #[arg(
        long,
        help = "Target stage candidate core name (format: stage_staff_<nonce>_candidate)"
    )]
    pub candidate_core: String,

    #[arg(
        long,
        default_value_t = false,
        num_args = 0..=1,
        default_missing_value = "true",
        action = clap::ArgAction::Set,
        help = "Whether ARKs are enabled (default: false)"
    )]
    pub arks_enabled: bool,

    #[arg(
        long,
        default_value_t = true,
        num_args = 0..=1,
        default_missing_value = "true",
        action = clap::ArgAction::Set,
        help = "Strip private _archivalist_* capability markers (default: true)"
    )]
    pub strip_markers: bool,

    #[arg(
        long,
        help = "Output machine-readable JSON summary (default: human-readable)"
    )]
    pub json: bool,

    #[arg(
        long,
        help = "Test-only: inject deterministic failure after completing N batches"
    )]
    pub fail_after_batches: Option<usize>,
}

impl StaffAoCandidateArgs {
    pub fn validate(&self) -> Result<()> {
        if self.repo == 0 {
            bail!("repo ID must be greater than 0");
        }
        if self.batch_size == 0 {
            bail!("batch size must be greater than 0");
        }
        if self.batch_size > MAX_CANDIDATE_BATCH_SIZE {
            bail!(
                "batch size cannot exceed {} (requested: {})",
                MAX_CANDIDATE_BATCH_SIZE,
                self.batch_size
            );
        }
        if self.concurrency == 0 {
            bail!("worker concurrency must be greater than 0");
        }
        if self.concurrency > MAX_CANDIDATE_CONCURRENCY {
            bail!(
                "worker concurrency cannot exceed {} (requested: {})",
                MAX_CANDIDATE_CONCURRENCY,
                self.concurrency
            );
        }
        if let Some(limit) = self.fail_after_batches {
            if limit == 0 {
                bail!("fail_after_batches must be greater than 0 if specified");
            }
        }
        validate_isolated_solr_base_url(&self.solr_url)?;
        validate_stage_candidate_core(&self.candidate_core, None)?;
        Ok(())
    }
}

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

pub fn partition_ids_into_batches(ids: &[u32], batch_size: usize) -> Vec<Vec<u32>> {
    if ids.is_empty() || batch_size == 0 {
        return Vec::new();
    }
    ids.chunks(batch_size).map(|c| c.to_vec()).collect()
}

pub async fn execute_staff_ao_candidate(
    client: &Client,
    args: &StaffAoCandidateArgs,
) -> StaffAoCandidateSummary {
    if let Err(err) = args.validate() {
        return StaffAoCandidateSummary {
            repository_id: args.repo,
            total_ids_discovered: 0,
            batch_size: args.batch_size,
            concurrency: args.concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 1,
            exact_ids_indexed: 0,
            unique_uris: 0,
            missing_ids: 0,
            duplicate_ids: 0,
            unexpected_ids: 0,
            candidate_core: args.candidate_core.clone(),
            solr_url: args.solr_url.clone(),
            arks_enabled: args.arks_enabled,
            id_list_elapsed_sec: 0.0,
            batch_loop_wall_elapsed_sec: 0.0,
            solr_write_elapsed_sec: 0.0,
            solr_update_requests_sent: 0,
            solr_commit_elapsed_sec: 0.0,
            total_wall_elapsed_sec: 0.0,
            records_per_second: 0.0,
            success: false,
            error_message: Some(format!("Invalid arguments: {err}")),
        };
    }

    // Pre-flight check: construct writer and verify candidate core is ready and empty
    let mut writer = match IsolatedSolrStaffAoWriter::new(&args.solr_url, &args.candidate_core) {
        Ok(w) => w,
        Err(err) => {
            return StaffAoCandidateSummary {
                repository_id: args.repo,
                total_ids_discovered: 0,
                batch_size: args.batch_size,
                concurrency: args.concurrency,
                total_batches: 0,
                batches_completed: 0,
                batches_failed: 0,
                failures: 1,
                exact_ids_indexed: 0,
                unique_uris: 0,
                missing_ids: 0,
                duplicate_ids: 0,
                unexpected_ids: 0,
                candidate_core: args.candidate_core.clone(),
                solr_url: args.solr_url.clone(),
                arks_enabled: args.arks_enabled,
                id_list_elapsed_sec: 0.0,
                batch_loop_wall_elapsed_sec: 0.0,
                solr_write_elapsed_sec: 0.0,
                solr_update_requests_sent: 0,
                solr_commit_elapsed_sec: 0.0,
                total_wall_elapsed_sec: 0.0,
                records_per_second: 0.0,
                success: false,
                error_message: Some(format!(
                    "Failed to initialize IsolatedSolrStaffAoWriter: {err}"
                )),
            };
        }
    };

    if let Err(err) = writer.verify_target_core_ready().await {
        return StaffAoCandidateSummary {
            repository_id: args.repo,
            total_ids_discovered: 0,
            batch_size: args.batch_size,
            concurrency: args.concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 1,
            exact_ids_indexed: 0,
            unique_uris: 0,
            missing_ids: 0,
            duplicate_ids: 0,
            unexpected_ids: 0,
            candidate_core: args.candidate_core.clone(),
            solr_url: args.solr_url.clone(),
            arks_enabled: args.arks_enabled,
            id_list_elapsed_sec: 0.0,
            batch_loop_wall_elapsed_sec: 0.0,
            solr_write_elapsed_sec: 0.0,
            solr_update_requests_sent: 0,
            solr_commit_elapsed_sec: 0.0,
            total_wall_elapsed_sec: 0.0,
            records_per_second: 0.0,
            success: false,
            error_message: Some(format!(
                "Candidate core '{}' readiness verification failed: {err}",
                args.candidate_core
            )),
        };
    }

    // Pre-flight check: derive dynamic enum fields once from GET /schemas
    let dynamic_enums = match client.dynamic_enum_fields_from_schemas().await {
        Ok(e) => e,
        Err(err) => {
            return StaffAoCandidateSummary {
                repository_id: args.repo,
                total_ids_discovered: 0,
                batch_size: args.batch_size,
                concurrency: args.concurrency,
                total_batches: 0,
                batches_completed: 0,
                batches_failed: 0,
                failures: 1,
                exact_ids_indexed: 0,
                unique_uris: 0,
                missing_ids: 0,
                duplicate_ids: 0,
                unexpected_ids: 0,
                candidate_core: args.candidate_core.clone(),
                solr_url: args.solr_url.clone(),
                arks_enabled: args.arks_enabled,
                id_list_elapsed_sec: 0.0,
                batch_loop_wall_elapsed_sec: 0.0,
                solr_write_elapsed_sec: 0.0,
                solr_update_requests_sent: 0,
                solr_commit_elapsed_sec: 0.0,
                total_wall_elapsed_sec: 0.0,
                records_per_second: 0.0,
                success: false,
                error_message: Some(format!(
                    "Failed to derive dynamic enum fields from GET /schemas: {err}"
                )),
            };
        }
    };

    // Measurement envelope begins here (matches native baseline: starts before ID enumeration)
    let wall_start = Instant::now();

    // 1. Enumerate all AO IDs modified since 0
    let id_start = Instant::now();
    let id_result = client
        .archival_object_ids_modified_since(args.repo, Some(0))
        .await;
    let id_list_elapsed_sec = id_start.elapsed().as_secs_f64();

    let ids = match id_result {
        Ok(ids) => ids,
        Err(err) => {
            let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
            return StaffAoCandidateSummary {
                repository_id: args.repo,
                total_ids_discovered: 0,
                batch_size: args.batch_size,
                concurrency: args.concurrency,
                total_batches: 0,
                batches_completed: 0,
                batches_failed: 0,
                failures: 1,
                exact_ids_indexed: 0,
                unique_uris: 0,
                missing_ids: 0,
                duplicate_ids: 0,
                unexpected_ids: 0,
                candidate_core: args.candidate_core.clone(),
                solr_url: args.solr_url.clone(),
                arks_enabled: args.arks_enabled,
                id_list_elapsed_sec,
                batch_loop_wall_elapsed_sec: 0.0,
                solr_write_elapsed_sec: 0.0,
                solr_update_requests_sent: 0,
                solr_commit_elapsed_sec: 0.0,
                total_wall_elapsed_sec,
                records_per_second: 0.0,
                success: false,
                error_message: Some(format!(
                    "Failed to fetch archival object ID list from backend: {err}"
                )),
            };
        }
    };

    let mut initial_ids_set = HashSet::with_capacity(ids.len());
    let mut duplicate_input_ids = 0;
    for &id in &ids {
        if !initial_ids_set.insert(id) {
            duplicate_input_ids += 1;
        }
    }

    if duplicate_input_ids > 0 {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        return StaffAoCandidateSummary {
            repository_id: args.repo,
            total_ids_discovered: ids.len(),
            batch_size: args.batch_size,
            concurrency: args.concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 1,
            exact_ids_indexed: 0,
            unique_uris: 0,
            missing_ids: 0,
            duplicate_ids: duplicate_input_ids,
            unexpected_ids: 0,
            candidate_core: args.candidate_core.clone(),
            solr_url: args.solr_url.clone(),
            arks_enabled: args.arks_enabled,
            id_list_elapsed_sec,
            batch_loop_wall_elapsed_sec: 0.0,
            solr_write_elapsed_sec: 0.0,
            solr_update_requests_sent: 0,
            solr_commit_elapsed_sec: 0.0,
            total_wall_elapsed_sec,
            records_per_second: 0.0,
            success: false,
            error_message: Some(format!(
                "Backend returned {} duplicate ID(s) in repository AO ID list",
                duplicate_input_ids
            )),
        };
    }

    if ids.is_empty() {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        return StaffAoCandidateSummary {
            repository_id: args.repo,
            total_ids_discovered: 0,
            batch_size: args.batch_size,
            concurrency: args.concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 0,
            exact_ids_indexed: 0,
            unique_uris: 0,
            missing_ids: 0,
            duplicate_ids: 0,
            unexpected_ids: 0,
            candidate_core: args.candidate_core.clone(),
            solr_url: args.solr_url.clone(),
            arks_enabled: args.arks_enabled,
            id_list_elapsed_sec,
            batch_loop_wall_elapsed_sec: 0.0,
            solr_write_elapsed_sec: 0.0,
            solr_update_requests_sent: 0,
            solr_commit_elapsed_sec: 0.0,
            total_wall_elapsed_sec,
            records_per_second: 0.0,
            success: true,
            error_message: None,
        };
    }

    let batches = partition_ids_into_batches(&ids, args.batch_size);
    let total_batches = batches.len();
    let batch_start = Instant::now();

    let repo_id = args.repo;
    let arks_enabled = args.arks_enabled;
    let strip_markers = args.strip_markers;

    let stream = stream::iter(batches.into_iter().enumerate().map(|(batch_idx, chunk)| {
        let client = client.clone();
        let dynamic_enums = dynamic_enums.clone();
        async move {
            let res: Result<Vec<Value>> = async {
                let raw_records = client
                    .fetch_staff_archival_objects_by_ids(repo_id, &chunk)
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
                        dynamic_enum_fields: dynamic_enums.clone(),
                        arks_enabled,
                        ..Default::default()
                    };
                    inputs.push(input);
                }

                let batch_resp =
                    build_index_document_batch(IndexDocumentBatchRequest { records: inputs })
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to build index document batch: {e}")
                        })?;

                let mut docs = batch_resp.documents;
                if strip_markers {
                    for doc in &mut docs {
                        strip_archivalist_markers(doc);
                    }
                }
                Ok(docs)
            }
            .await;

            (batch_idx, chunk, res)
        }
    }))
    .buffer_unordered(args.concurrency);

    let mut batches_completed = 0;
    let mut batches_failed = 0;
    let mut returned_ids = HashSet::with_capacity(ids.len());
    let mut unique_uris = HashSet::with_capacity(ids.len());
    let mut duplicate_ids_count = 0;
    let mut unexpected_ids_count = 0;
    let mut first_error: Option<String> = None;
    let mut solr_write_elapsed_sec = 0.0;

    let mut stream = stream;
    let expected_prefix = format!("/repositories/{}/archival_objects/", repo_id);

    while let Some((batch_idx, chunk, res)) = stream.next().await {
        match res {
            Ok(docs) => {
                batches_completed += 1;
                for (rec_idx, doc) in docs.iter().enumerate() {
                    let uri = match doc.get("uri").and_then(|v| v.as_str()) {
                        Some(u) => u.to_string(),
                        None => {
                            unexpected_ids_count += 1;
                            if first_error.is_none() {
                                first_error = Some(format!(
                                    "Doc at index {} in batch {} missing string 'uri'",
                                    rec_idx, batch_idx
                                ));
                            }
                            continue;
                        }
                    };

                    if !uri.starts_with(&expected_prefix) {
                        unexpected_ids_count += 1;
                        if first_error.is_none() {
                            first_error = Some(format!(
                                "Doc URI '{}' does not match expected prefix '{}'",
                                uri, expected_prefix
                            ));
                        }
                        continue;
                    }

                    let id_str = &uri[expected_prefix.len()..];
                    let record_id: u32 = match id_str.parse() {
                        Ok(id) => id,
                        Err(_) => {
                            unexpected_ids_count += 1;
                            if first_error.is_none() {
                                first_error = Some(format!(
                                    "Doc URI '{}' contains non-numeric ID '{}'",
                                    uri, id_str
                                ));
                            }
                            continue;
                        }
                    };

                    if !initial_ids_set.contains(&record_id) {
                        unexpected_ids_count += 1;
                        if first_error.is_none() {
                            first_error = Some(format!(
                                "Returned ID {} was not present in repository AO ID list",
                                record_id
                            ));
                        }
                    }

                    if !returned_ids.insert(record_id) {
                        duplicate_ids_count += 1;
                        if first_error.is_none() {
                            first_error = Some(format!(
                                "Duplicate record returned for ID {} across batches",
                                record_id
                            ));
                        }
                    }

                    unique_uris.insert(uri);
                }

                // Add documents to writer in bounded memory chunks
                let t_write = Instant::now();
                if let Err(w_err) = writer.add_documents(docs).await {
                    batches_failed += 1;
                    if first_error.is_none() {
                        first_error = Some(format!(
                            "Failed to write documents to candidate Solr core: {w_err}"
                        ));
                    }
                    break;
                }
                solr_write_elapsed_sec += t_write.elapsed().as_secs_f64();

                if let Some(fail_limit) = args.fail_after_batches {
                    if batches_completed >= fail_limit {
                        batches_failed += 1;
                        if first_error.is_none() {
                            first_error = Some(format!(
                                "Injected deterministic test failure after {} completed batch(es) (fail_after_batches={})",
                                batches_completed, fail_limit
                            ));
                        }
                        break;
                    }
                }
            }
            Err(err) => {
                batches_failed += 1;
                if first_error.is_none() {
                    first_error = Some(format!("Batch {} (chunk {:?}): {}", batch_idx, chunk, err));
                }
                break;
            }
        }
    }

    let batch_loop_wall_elapsed_sec = batch_start.elapsed().as_secs_f64();

    // Commit only if all batches succeeded and zero unexpected/duplicate anomalies occurred
    let mut solr_commit_elapsed_sec = 0.0;
    let mut commit_failed = false;

    if batches_failed == 0
        && unexpected_ids_count == 0
        && duplicate_ids_count == 0
        && returned_ids.len() == ids.len()
    {
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
    let exact_ids_indexed = returned_ids.len();
    let missing_ids = ids.len().saturating_sub(exact_ids_indexed);

    let failures = batches_failed
        + if commit_failed { 1 } else { 0 }
        + if duplicate_ids_count > 0 { 1 } else { 0 }
        + if unexpected_ids_count > 0 { 1 } else { 0 }
        + if missing_ids > 0 && batches_failed == 0 {
            1
        } else {
            0
        };

    let success = failures == 0
        && exact_ids_indexed == ids.len()
        && missing_ids == 0
        && duplicate_ids_count == 0
        && unexpected_ids_count == 0
        && first_error.is_none();

    let records_per_second = if total_wall_elapsed_sec > 0.0 && success {
        exact_ids_indexed as f64 / total_wall_elapsed_sec
    } else {
        0.0
    };

    StaffAoCandidateSummary {
        repository_id: repo_id,
        total_ids_discovered: ids.len(),
        batch_size: args.batch_size,
        concurrency: args.concurrency,
        total_batches,
        batches_completed,
        batches_failed,
        failures,
        exact_ids_indexed,
        unique_uris: unique_uris.len(),
        missing_ids,
        duplicate_ids: duplicate_ids_count,
        unexpected_ids: unexpected_ids_count,
        candidate_core: args.candidate_core.clone(),
        solr_url: args.solr_url.clone(),
        arks_enabled: args.arks_enabled,
        id_list_elapsed_sec,
        batch_loop_wall_elapsed_sec,
        solr_write_elapsed_sec,
        solr_update_requests_sent: writer.update_requests_sent(),
        solr_commit_elapsed_sec,
        total_wall_elapsed_sec,
        records_per_second,
        success,
        error_message: first_error,
    }
}

pub fn print_human_candidate_summary(summary: &StaffAoCandidateSummary) {
    println!("=== STAFF Archival Object Candidate Indexer Summary ===");
    println!("Repository ID: {}", summary.repository_id);
    println!("Candidate Core: {}", summary.candidate_core);
    println!("Total Discovered IDs: {}", summary.total_ids_discovered);
    println!("Batch Size: {}", summary.batch_size);
    println!("Concurrency: {}", summary.concurrency);
    println!("Total Batches: {}", summary.total_batches);
    println!("Batches Completed: {}", summary.batches_completed);
    println!("Batches Failed: {}", summary.batches_failed);
    println!("Total Failures: {}", summary.failures);
    println!("Exact IDs Indexed: {}", summary.exact_ids_indexed);
    println!("Unique URIs: {}", summary.unique_uris);
    println!("Missing IDs: {}", summary.missing_ids);
    println!("Duplicate IDs: {}", summary.duplicate_ids);
    println!("Unexpected IDs: {}", summary.unexpected_ids);
    println!("ARKs Enabled: {}", summary.arks_enabled);
    println!("ID List Elapsed: {:.4}s", summary.id_list_elapsed_sec);
    println!(
        "Combined Batch Loop Wall Elapsed: {:.4}s (pipelined fetch, build & writes)",
        summary.batch_loop_wall_elapsed_sec
    );
    println!(
        "  Solr Writes Elapsed: {:.4}s (awaiting Solr update POSTs; overlaps batch loop)",
        summary.solr_write_elapsed_sec
    );
    println!(
        "  Solr Update POST Requests: {}",
        summary.solr_update_requests_sent
    );
    println!(
        "Solr Hard Commit Elapsed: {:.4}s",
        summary.solr_commit_elapsed_sec
    );
    println!(
        "Total Wall Elapsed: {:.4}s (id_list + batch_loop_wall + solr_commit)",
        summary.total_wall_elapsed_sec
    );
    println!("Records / Sec: {:.2}", summary.records_per_second);
    if let Some(ref err) = summary.error_message {
        println!("Error: {}", err);
    }
    println!(
        "Status: {}",
        if summary.success { "SUCCESS" } else { "FAILED" }
    );
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
    let summary = execute_staff_ao_candidate(client, &args).await;

    if args.json {
        let json_out = serde_json::to_string_pretty(&summary)?;
        writeln!(writer, "{}", json_out)?;
    } else {
        // Output human readable summary to the writer
        let mut buffer = Vec::new();
        {
            let mut cursor = io::Cursor::new(&mut buffer);
            writeln!(
                cursor,
                "=== STAFF Archival Object Candidate Indexer Summary ==="
            )?;
            writeln!(cursor, "Repository ID: {}", summary.repository_id)?;
            writeln!(cursor, "Candidate Core: {}", summary.candidate_core)?;
            writeln!(
                cursor,
                "Total Discovered IDs: {}",
                summary.total_ids_discovered
            )?;
            writeln!(cursor, "Batch Size: {}", summary.batch_size)?;
            writeln!(cursor, "Concurrency: {}", summary.concurrency)?;
            writeln!(cursor, "Total Batches: {}", summary.total_batches)?;
            writeln!(cursor, "Batches Completed: {}", summary.batches_completed)?;
            writeln!(cursor, "Batches Failed: {}", summary.batches_failed)?;
            writeln!(cursor, "Total Failures: {}", summary.failures)?;
            writeln!(cursor, "Exact IDs Indexed: {}", summary.exact_ids_indexed)?;
            writeln!(cursor, "Unique URIs: {}", summary.unique_uris)?;
            writeln!(cursor, "Missing IDs: {}", summary.missing_ids)?;
            writeln!(cursor, "Duplicate IDs: {}", summary.duplicate_ids)?;
            writeln!(cursor, "Unexpected IDs: {}", summary.unexpected_ids)?;
            writeln!(cursor, "ARKs Enabled: {}", summary.arks_enabled)?;
            writeln!(
                cursor,
                "ID List Elapsed: {:.4}s",
                summary.id_list_elapsed_sec
            )?;
            writeln!(
                cursor,
                "Combined Batch Loop Wall Elapsed: {:.4}s (pipelined fetch, build & writes)",
                summary.batch_loop_wall_elapsed_sec
            )?;
            writeln!(
                cursor,
                "  Solr Writes Elapsed: {:.4}s (awaiting Solr update POSTs; overlaps batch loop)",
                summary.solr_write_elapsed_sec
            )?;
            writeln!(
                cursor,
                "Solr Hard Commit Elapsed: {:.4}s",
                summary.solr_commit_elapsed_sec
            )?;
            writeln!(
                cursor,
                "Total Wall Elapsed: {:.4}s (id_list + batch_loop_wall + solr_commit)",
                summary.total_wall_elapsed_sec
            )?;
            writeln!(cursor, "Records / Sec: {:.2}", summary.records_per_second)?;
            if let Some(ref err) = summary.error_message {
                writeln!(cursor, "Error: {}", err)?;
            }
            writeln!(
                cursor,
                "Status: {}",
                if summary.success { "SUCCESS" } else { "FAILED" }
            )?;
        }
        writer.write_all(&buffer)?;
    }

    if !summary.success {
        bail!(
            "STAFF archival object candidate indexing failed: {}",
            summary
                .error_message
                .as_deref()
                .unwrap_or("validation, missing ID, or batch failure")
        );
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

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
            json: false,
            fail_after_batches: None,
        };
        assert!(valid_args.validate().is_ok());

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
    fn test_partition_ids_into_batches() {
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

        // Mock Solr STATUS
        let _m_status = solr_server
            .mock(
                "GET",
                format!("/admin/cores?action=STATUS&core={candidate_core}&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
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

        // Mock Solr select empty check
        let _m_empty = solr_server
            .mock(
                "GET",
                format!("/{candidate_core}/select?q=*:*&rows=0&wt=json").as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
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

        // Mock ASpace schemas for dynamic enums
        let _m_schemas = aspace_server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
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

        // Mock ASpace AO IDs modified since 0
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

        // Mock batch fetch
        let _m_batch = aspace_server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/repositories/4/archival_objects\?id_set=".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!([
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

        // Mock Solr updates and commit
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
            json: true,
            fail_after_batches: None,
        };

        let mut output = Vec::new();
        let summary = run_staff_ao_candidate_to_writer(&client, args, &mut output)
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
                serde_json::json!({
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
                serde_json::json!({
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
                serde_json::json!({
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
                        serde_json::json!({
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
            json: true,
            fail_after_batches: Some(21),
        };

        let summary = execute_staff_ao_candidate(&client, &args).await;

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
}
