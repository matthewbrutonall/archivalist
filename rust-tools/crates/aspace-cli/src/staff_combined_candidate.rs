use anyhow::Result;
use aspace_client::isolated_solr_writer::IsolatedSolrStaffAoWriter;
use aspace_client::Client;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
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
    let client_for_ao_batches = client.clone();
    let client_for_res_batches = client.clone();

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
    )
    .await
}

pub async fn execute_staff_combined_candidate_custom<
    V,
    FAoIds,
    FutAoIds,
    EAoIds,
    FResIds,
    FutResIds,
    EResIds,
    FAoBatch,
    FutAoBatch,
    FResBatch,
    FutResBatch,
>(
    client: &Client,
    args: &StaffCombinedCandidateArgs,
    verifier: V,
    fetch_ao_ids: FAoIds,
    fetch_res_ids: FResIds,
    fetch_and_build_ao_batch: FAoBatch,
    fetch_and_build_res_batch: FResBatch,
) -> StaffCombinedCandidateSummary
where
    V: FnOnce(&std::path::Path, &str, &str) -> Result<()>,
    FAoIds: FnOnce() -> FutAoIds,
    FutAoIds: Future<Output = std::result::Result<Vec<u32>, EAoIds>>,
    EAoIds: std::fmt::Display,
    FResIds: FnOnce() -> FutResIds,
    FutResIds: Future<Output = std::result::Result<Vec<u32>, EResIds>>,
    EResIds: std::fmt::Display,
    FAoBatch: Fn(usize, Vec<u32>, Arc<Vec<String>>) -> FutAoBatch + Send + Sync + 'static,
    FutAoBatch: Future<Output = Result<CandidateBatchOutput>> + Send,
    FResBatch: Fn(usize, Vec<u32>, Arc<Vec<String>>) -> FutResBatch + Send + Sync + 'static,
    FutResBatch: Future<Output = Result<CandidateBatchOutput>> + Send,
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

    // 1. Enumerate IDs for both types
    let id_start = Instant::now();
    let ao_ids_res = fetch_ao_ids().await;
    let res_ids_res = fetch_res_ids().await;
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
        res.total_ids_discovered = ao_ids.len() + resource_ids.len();
        res.archival_objects.ids_discovered = ao_ids.len();
        res.resources.ids_discovered = resource_ids.len();
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
        res.total_ids_discovered = ao_ids.len() + resource_ids.len();
        res.archival_objects.ids_discovered = ao_ids.len();
        res.resources.ids_discovered = resource_ids.len();
        res.resources.duplicate_ids = res_dup_input;
        res.duplicate_ids = res_dup_input;
        res.id_list_elapsed_sec = id_list_elapsed_sec;
        res.total_wall_elapsed_sec = total_wall_elapsed_sec;
        return res;
    }

    let total_discovered_ids = ao_ids.len() + resource_ids.len();

    let res_batches = partition_ids_into_batches(&resource_ids, args.batch_size);
    let ao_batches = partition_ids_into_batches(&ao_ids, args.batch_size);
    let total_batches = res_batches.len() + ao_batches.len();

    let expected_ao_prefix = format!("/repositories/{}/archival_objects/", args.repo);
    let expected_res_prefix = format!("/repositories/{}/resources/", args.repo);

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

    // 3. Process Archival Object Batches (if resources succeeded)
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

    let total_batches_failed = res_slice_summary.batches_failed + ao_slice_summary.batches_failed;
    let total_missing_ids = res_slice_summary.missing_ids + ao_slice_summary.missing_ids;
    let total_duplicate_ids = res_slice_summary.duplicate_ids + ao_slice_summary.duplicate_ids;
    let total_unexpected_ids = res_slice_summary.unexpected_ids + ao_slice_summary.unexpected_ids;
    let total_exact_ids_indexed =
        res_slice_summary.exact_ids_indexed + ao_slice_summary.exact_ids_indexed;
    let total_parent_docs = res_slice_summary.parent_docs + ao_slice_summary.parent_docs;
    let total_child_docs = res_slice_summary.child_docs + ao_slice_summary.child_docs;
    let total_docs_indexed = total_parent_docs + total_child_docs;

    // 4. Commit Decision:
    // Only commit if all batches succeeded, zero unexpected/duplicate anomalies occurred,
    // exact ID counts match for both slices, and no errors occurred.
    let mut solr_commit_elapsed_sec = 0.0;
    let mut commit_failed = false;

    let can_commit = total_batches_failed == 0
        && total_unexpected_ids == 0
        && total_duplicate_ids == 0
        && total_missing_ids == 0
        && res_returned_ids.len() == resource_ids.len()
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
        "combined (AO + Resource)",
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
    async fn test_combined_candidate_clean_success() {
        let candidate_core = "stage_staff_20261027_success_candidate";
        let (_aspace, _solr, client, args) = setup_test_harness(candidate_core).await;

        let res = execute_staff_combined_candidate_custom(
            &client,
            &args,
            |_state, _backend, _solr| Ok(()),
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![101]) },
            || async { Ok::<Vec<u32>, anyhow::Error>(vec![1]) },
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
        )
        .await;

        if !res.success {
            eprintln!("Test failed with error_message: {:?}", res.error_message);
            eprintln!("Failures: {}", res.failures);
            eprintln!("Committed: {}", res.committed);
            eprintln!("AO: {:?}", res.archival_objects);
            eprintln!("Res: {:?}", res.resources);
        }
        assert!(res.success);
        assert!(res.committed);
        assert_eq!(res.failures, 0);
        assert_eq!(res.total_ids_discovered, 2);
        assert_eq!(res.exact_ids_indexed, 2);
        assert_eq!(res.parent_docs, 2);
        assert_eq!(res.child_docs, 0);
        assert_eq!(res.total_docs_indexed, 2);
        assert_eq!(res.unique_uris, 2);
        assert_eq!(res.archival_objects.exact_ids_indexed, 1);
        assert_eq!(res.resources.exact_ids_indexed, 1);
    }
}
