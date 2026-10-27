use anyhow::{bail, Result};
use aspace_client::isolated_solr_writer::{
    validate_isolated_solr_base_url, validate_stage_candidate_core, IsolatedSolrStaffAoWriter,
};
use aspace_client::Client;
use futures::stream::{self, StreamExt};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

pub const DEFAULT_CANDIDATE_BATCH_SIZE: usize = 25;
pub const MAX_CANDIDATE_BATCH_SIZE: usize = 25;
pub const DEFAULT_CANDIDATE_CONCURRENCY: usize = 4;
pub const MAX_CANDIDATE_CONCURRENCY: usize = 32;

#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct StaffCandidateArgs {
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
        help = "Batch size for candidate hydration (default: 25, max: 25)",
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
        alias = "profiling-stack-state",
        help = "Path to profiling-stack state JSON file (required for Solr write verification)"
    )]
    pub state: Option<std::path::PathBuf>,

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

impl StaffCandidateArgs {
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
        if !self.strip_markers {
            bail!("--strip-markers false is forbidden in Solr write mode; marker stripping is mandatory");
        }
        match &self.state {
            Some(path) if !path.as_os_str().is_empty() => {}
            _ => bail!("missing required profiling-stack state file for Solr write mode (--state)"),
        }
        Ok(())
    }
}

pub fn partition_ids_into_batches(ids: &[u32], batch_size: usize) -> Vec<Vec<u32>> {
    if ids.is_empty() || batch_size == 0 {
        return Vec::new();
    }
    ids.chunks(batch_size).map(|c| c.to_vec()).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateBatchOutput {
    pub ordered_documents: Vec<Value>,
    pub parent_uris: Vec<Option<String>>,
    pub child_uris: Vec<String>,
    pub parent_count: usize,
    pub child_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaffCandidateExecutionResult {
    pub repository_id: u32,
    pub total_ids_discovered: usize,
    pub exact_ids_indexed: usize,
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
    pub error_message: Option<String>,
}

impl StaffCandidateExecutionResult {
    pub fn failure_preflight(args: &StaffCandidateArgs, error_message: String) -> Self {
        Self {
            repository_id: args.repo,
            total_ids_discovered: 0,
            exact_ids_indexed: 0,
            parent_docs: 0,
            child_docs: 0,
            total_docs_indexed: 0,
            batch_size: args.batch_size,
            concurrency: args.concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 1,
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
            committed: false,
            success: false,
            error_message: Some(error_message),
        }
    }
}

pub fn render_candidate_summary_to_writer<W: Write, S: Serialize, H>(
    writer: &mut W,
    summary: &S,
    json: bool,
    success: bool,
    error_message: Option<&str>,
    type_label: &str,
    human_renderer: H,
) -> Result<()>
where
    H: FnOnce(&mut W) -> io::Result<()>,
{
    if json {
        let json_out = serde_json::to_string_pretty(summary)?;
        writeln!(writer, "{}", json_out)?;
    } else {
        human_renderer(writer)?;
    }

    if !success {
        bail!(
            "STAFF {} candidate indexing failed: {}",
            type_label,
            error_message.unwrap_or("validation, missing ID, or batch failure")
        );
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaffCandidateDescriptor<'a> {
    pub expected_parent_prefix: &'a str,
    pub id_type_label: &'a str,
    pub id_list_label: &'a str,
}

pub async fn execute_staff_candidate<V, FId, FutId, EId, FBatch, FutBatch>(
    client: &Client,
    args: &StaffCandidateArgs,
    descriptor: StaffCandidateDescriptor<'_>,
    verifier: V,
    fetch_ids: FId,
    fetch_and_build_batch: FBatch,
) -> StaffCandidateExecutionResult
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
    FId: FnOnce() -> FutId,
    FutId: Future<Output = std::result::Result<Vec<u32>, EId>>,
    EId: std::fmt::Display,
    FBatch: Fn(usize, Vec<u32>, Arc<Vec<String>>) -> FutBatch + Send + Sync + 'static,
    FutBatch: Future<Output = Result<CandidateBatchOutput>> + Send,
{
    let expected_parent_prefix = descriptor.expected_parent_prefix;
    let id_type_label = descriptor.id_type_label;
    let id_list_label = descriptor.id_list_label;
    if let Err(err) = args.validate() {
        return StaffCandidateExecutionResult::failure_preflight(
            args,
            format!("Invalid arguments: {err}"),
        );
    }

    let state_path = args.state.as_ref().unwrap();

    // Verify profiling stack identity before any Solr write or core readiness check
    if let Err(err) = verifier(state_path, client.base_url(), &args.solr_url) {
        return StaffCandidateExecutionResult::failure_preflight(
            args,
            format!("Profiling stack verification failed: {err}"),
        );
    }

    // Pre-flight check: construct writer and verify candidate core is ready and empty
    let mut writer = match IsolatedSolrStaffAoWriter::new(&args.solr_url, &args.candidate_core) {
        Ok(w) => w,
        Err(err) => {
            return StaffCandidateExecutionResult::failure_preflight(
                args,
                format!("Failed to initialize IsolatedSolrStaffAoWriter: {err}"),
            );
        }
    };

    if let Err(err) = writer.verify_target_core_ready().await {
        return StaffCandidateExecutionResult::failure_preflight(
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
            return StaffCandidateExecutionResult::failure_preflight(
                args,
                format!("Failed to derive dynamic enum fields from GET /schemas: {err}"),
            );
        }
    };

    // Measurement envelope begins here (matches native baseline reindex contract)
    let wall_start = Instant::now();

    // 1. Enumerate all IDs modified since 0
    let id_start = Instant::now();
    let id_result = fetch_ids().await;
    let id_list_elapsed_sec = id_start.elapsed().as_secs_f64();

    let ids = match id_result {
        Ok(ids) => ids,
        Err(err) => {
            let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
            let mut res = StaffCandidateExecutionResult::failure_preflight(
                args,
                format!(
                    "Failed to fetch {} ID list from backend: {err}",
                    id_type_label
                ),
            );
            res.id_list_elapsed_sec = id_list_elapsed_sec;
            res.total_wall_elapsed_sec = total_wall_elapsed_sec;
            return res;
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
        let mut res = StaffCandidateExecutionResult::failure_preflight(
            args,
            format!(
                "Backend returned {} duplicate ID(s) in repository {} ID list",
                duplicate_input_ids, id_list_label
            ),
        );
        res.total_ids_discovered = ids.len();
        res.duplicate_ids = duplicate_input_ids;
        res.id_list_elapsed_sec = id_list_elapsed_sec;
        res.total_wall_elapsed_sec = total_wall_elapsed_sec;
        return res;
    }

    if ids.is_empty() {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        return StaffCandidateExecutionResult {
            repository_id: args.repo,
            total_ids_discovered: 0,
            exact_ids_indexed: 0,
            parent_docs: 0,
            child_docs: 0,
            total_docs_indexed: 0,
            batch_size: args.batch_size,
            concurrency: args.concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 0,
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
            committed: false,
            success: true,
            error_message: None,
        };
    }

    let batches = partition_ids_into_batches(&ids, args.batch_size);
    let total_batches = batches.len();
    let batch_start = Instant::now();

    let stream = stream::iter(batches.into_iter().enumerate().map(|(batch_idx, chunk)| {
        let dynamic_enums = Arc::clone(&dynamic_enums);
        let fut = fetch_and_build_batch(batch_idx, chunk.clone(), dynamic_enums);
        async move {
            let res = fut.await;
            (batch_idx, chunk, res)
        }
    }))
    .buffer_unordered(args.concurrency);

    let mut batches_completed = 0;
    let mut batches_failed = 0;
    let mut returned_ids = HashSet::with_capacity(ids.len());
    let mut unique_uris = HashSet::with_capacity(ids.len() * 2);
    let mut duplicate_ids_count = 0;
    let mut unexpected_ids_count = 0;
    let mut first_error: Option<String> = None;
    let mut solr_write_elapsed_sec = 0.0;
    let mut total_parent_docs = 0;
    let mut total_child_docs = 0;

    let mut stream = stream;

    while let Some((batch_idx, chunk, res)) = stream.next().await {
        match res {
            Ok(output) => {
                batches_completed += 1;
                total_parent_docs += output.parent_count;
                total_child_docs += output.child_count;

                for (rec_idx, maybe_uri) in output.parent_uris.into_iter().enumerate() {
                    let uri = match maybe_uri {
                        Some(u) => u,
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

                    if !uri.starts_with(expected_parent_prefix) {
                        unexpected_ids_count += 1;
                        if first_error.is_none() {
                            first_error = Some(format!(
                                "Doc URI '{}' does not match expected prefix '{}'",
                                uri, expected_parent_prefix
                            ));
                        }
                        continue;
                    }

                    let id_str = &uri[expected_parent_prefix.len()..];
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
                                "Returned ID {} was not present in repository {} ID list",
                                record_id, id_list_label
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

                for (child_idx, child_uri) in output.child_uris.into_iter().enumerate() {
                    if !unique_uris.insert(child_uri.clone()) {
                        duplicate_ids_count += 1;
                        if first_error.is_none() {
                            first_error = Some(format!(
                                "Duplicate child URI '{}' at index {} in batch {}",
                                child_uri, child_idx, batch_idx
                            ));
                        }
                    }
                }

                // Add documents to writer in bounded memory chunks
                let t_write = Instant::now();
                if let Err(w_err) = writer.add_documents(output.ordered_documents).await {
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

    let committed = batches_failed == 0
        && !commit_failed
        && returned_ids.len() == ids.len()
        && duplicate_ids_count == 0
        && unexpected_ids_count == 0;

    let success = failures == 0
        && exact_ids_indexed == ids.len()
        && missing_ids == 0
        && duplicate_ids_count == 0
        && unexpected_ids_count == 0
        && committed
        && first_error.is_none();

    let records_per_second = if total_wall_elapsed_sec > 0.0 && success {
        exact_ids_indexed as f64 / total_wall_elapsed_sec
    } else {
        0.0
    };

    StaffCandidateExecutionResult {
        repository_id: args.repo,
        total_ids_discovered: ids.len(),
        exact_ids_indexed,
        parent_docs: total_parent_docs,
        child_docs: total_child_docs,
        total_docs_indexed: total_parent_docs + total_child_docs,
        batch_size: args.batch_size,
        concurrency: args.concurrency,
        total_batches,
        batches_completed,
        batches_failed,
        failures,
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
        committed,
        success,
        error_message: first_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use mockito::Server;
    use serde_json::json;

    #[test]
    fn test_candidate_args_validation() {
        let valid_args = StaffCandidateArgs {
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

        let missing_state = StaffCandidateArgs {
            state: None,
            ..valid_args.clone()
        };
        assert_eq!(
            missing_state.validate().unwrap_err().to_string(),
            "missing required profiling-stack state file for Solr write mode (--state)"
        );

        let empty_state = StaffCandidateArgs {
            state: Some(std::path::PathBuf::from("")),
            ..valid_args.clone()
        };
        assert!(empty_state.validate().is_err());

        let strip_markers_false = StaffCandidateArgs {
            strip_markers: false,
            ..valid_args.clone()
        };
        assert_eq!(
            strip_markers_false.validate().unwrap_err().to_string(),
            "--strip-markers false is forbidden in Solr write mode; marker stripping is mandatory"
        );

        let fail_zero = StaffCandidateArgs {
            fail_after_batches: Some(0),
            ..valid_args.clone()
        };
        assert_eq!(
            fail_zero.validate().unwrap_err().to_string(),
            "fail_after_batches must be greater than 0 if specified"
        );

        let repo_zero = StaffCandidateArgs {
            repo: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            repo_zero.validate().unwrap_err().to_string(),
            "repo ID must be greater than 0"
        );

        let batch_zero = StaffCandidateArgs {
            batch_size: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            batch_zero.validate().unwrap_err().to_string(),
            "batch size must be greater than 0"
        );

        let batch_oversized = StaffCandidateArgs {
            batch_size: 26,
            ..valid_args.clone()
        };
        assert_eq!(
            batch_oversized.validate().unwrap_err().to_string(),
            "batch size cannot exceed 25 (requested: 26)"
        );

        let concurrency_zero = StaffCandidateArgs {
            concurrency: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            concurrency_zero.validate().unwrap_err().to_string(),
            "worker concurrency must be greater than 0"
        );

        let concurrency_oversized = StaffCandidateArgs {
            concurrency: 33,
            ..valid_args.clone()
        };
        assert_eq!(
            concurrency_oversized.validate().unwrap_err().to_string(),
            "worker concurrency cannot exceed 32 (requested: 33)"
        );

        let non_loopback_solr = StaffCandidateArgs {
            solr_url: "http://example.com:8983/solr".to_string(),
            ..valid_args.clone()
        };
        assert!(non_loopback_solr.validate().is_err());

        let invalid_core = StaffCandidateArgs {
            candidate_core: "archivesspace".to_string(),
            ..valid_args.clone()
        };
        assert!(invalid_core.validate().is_err());
    }

    #[test]
    fn test_partition_ids_into_batches_logic() {
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
    async fn test_preflight_rejection_solr_or_core() {
        let server = Server::new_async().await;
        let client = Client::new(server.url());

        let args = StaffCandidateArgs {
            repo: 4,
            batch_size: 25,
            concurrency: 4,
            solr_url: "http://example.com/solr".to_string(),
            candidate_core: "stage_staff_20261027_candidate".to_string(),
            arks_enabled: false,
            strip_markers: true,
            state: Some(std::path::PathBuf::from("/path/to/state.json")),
            json: false,
            fail_after_batches: None,
        };

        let descriptor = StaffCandidateDescriptor {
            expected_parent_prefix: "/repositories/4/archival_objects/",
            id_type_label: "archival object",
            id_list_label: "AO",
        };

        let res = execute_staff_candidate(
            &client,
            &args,
            descriptor,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(!res.success);
        assert_eq!(res.failures, 1);
        assert!(res.error_message.unwrap().contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn test_preflight_rejection_verifier_failure() {
        let server = Server::new_async().await;
        let client = Client::new(server.url());

        let args = StaffCandidateArgs {
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

        let descriptor = StaffCandidateDescriptor {
            expected_parent_prefix: "/repositories/4/archival_objects/",
            id_type_label: "archival object",
            id_list_label: "AO",
        };

        let res = execute_staff_candidate(
            &client,
            &args,
            descriptor,
            |_state, _backend, _solr| bail!("Injected verifier failure"),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(!res.success);
        assert_eq!(res.failures, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Profiling stack verification failed: Injected verifier failure"));
    }

    #[tokio::test]
    async fn test_duplicate_input_ids_fails_closed() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261027_dup_candidate";

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
                format!("/{candidate_core}/select?q=*:*&rows=0&wt=json").as_str(),
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

        let _m_schemas = aspace_server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{}"#)
            .create_async()
            .await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());
        let args = StaffCandidateArgs {
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

        let descriptor = StaffCandidateDescriptor {
            expected_parent_prefix: "/repositories/4/archival_objects/",
            id_type_label: "archival object",
            id_list_label: "AO",
        };

        let res = execute_staff_candidate(
            &client,
            &args,
            descriptor,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101, 102, 101]) }, // Duplicate 101
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(!res.success);
        assert_eq!(res.failures, 1);
        assert_eq!(res.duplicate_ids, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Backend returned 1 duplicate ID(s) in repository AO ID list"));
    }

    #[tokio::test]
    async fn test_empty_ids_discovered_success() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261027_empty_candidate";

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
                format!("/{candidate_core}/select?q=*:*&rows=0&wt=json").as_str(),
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

        let _m_schemas = aspace_server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{}"#)
            .create_async()
            .await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());
        let args = StaffCandidateArgs {
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

        let descriptor = StaffCandidateDescriptor {
            expected_parent_prefix: "/repositories/4/archival_objects/",
            id_type_label: "archival object",
            id_list_label: "AO",
        };

        let res = execute_staff_candidate(
            &client,
            &args,
            descriptor,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(Vec::new()) }, // Empty IDs
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(res.success);
        assert_eq!(res.failures, 0);
        assert_eq!(res.total_ids_discovered, 0);
        assert_eq!(res.exact_ids_indexed, 0);
    }

    #[tokio::test]
    async fn test_preflight_rejection_core_readiness_failure() {
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261027_notready_candidate";

        // Solr returns status indicating core does not exist
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
                    "status": {}
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new("http://127.0.0.1:8089");
        let args = StaffCandidateArgs {
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

        let descriptor = StaffCandidateDescriptor {
            expected_parent_prefix: "/repositories/4/archival_objects/",
            id_type_label: "archival object",
            id_list_label: "AO",
        };

        let res = execute_staff_candidate(
            &client,
            &args,
            descriptor,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(!res.success);
        assert_eq!(res.failures, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("readiness verification failed"));
    }

    #[tokio::test]
    async fn test_preflight_rejection_schemas_failure() {
        let mut aspace_server = Server::new_async().await;
        let mut solr_server = Server::new_async().await;

        let candidate_core = "stage_staff_20261027_schema_fail_candidate";

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
                format!("/{candidate_core}/select?q=*:*&rows=0&wt=json").as_str(),
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

        // /schemas returns 500 error
        let _m_schemas = aspace_server
            .mock("GET", "/schemas")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let mut client = Client::new(aspace_server.url());
        client.session_token = Some("dummy-session".to_string());

        let args = StaffCandidateArgs {
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

        let descriptor = StaffCandidateDescriptor {
            expected_parent_prefix: "/repositories/4/archival_objects/",
            id_type_label: "archival object",
            id_list_label: "AO",
        };

        let res = execute_staff_candidate(
            &client,
            &args,
            descriptor,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
            |_idx, _chunk, _enums| async { unreachable!() },
        )
        .await;

        assert!(!res.success);
        assert_eq!(res.failures, 1);
        assert!(res
            .error_message
            .unwrap()
            .contains("Failed to derive dynamic enum fields from GET /schemas"));
    }
}
