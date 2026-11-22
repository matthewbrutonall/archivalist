use anyhow::Result;
use aspace_client::Client;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;

pub const DEFAULT_PROBE_BATCH_SIZE: usize = 25;
pub const MAX_PROBE_BATCH_SIZE: usize = 25;
pub const DEFAULT_PROBE_CONCURRENCY: usize = 4;
pub const MAX_PROBE_CONCURRENCY: usize = 32;
pub const PROBE_NOTE: &str =
    "Source-fetch timing and validity probe only. No Solr build or write is measured.";

#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct StaffAoProbeArgs {
    #[arg(short, long, alias = "repo-id", help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Batch size for AO hydration (default: 25, max: 25)",
        default_value_t = DEFAULT_PROBE_BATCH_SIZE
    )]
    pub batch_size: usize,

    #[arg(
        long,
        help = "Worker concurrency for batch fetching (default: 4, max: 32)",
        default_value_t = DEFAULT_PROBE_CONCURRENCY
    )]
    pub concurrency: usize,

    #[arg(
        long,
        help = "Output machine-readable JSON summary (default: human-readable)"
    )]
    pub json: bool,
}

impl StaffAoProbeArgs {
    pub fn validate(&self) -> Result<()> {
        if self.batch_size == 0 {
            anyhow::bail!("batch size must be greater than 0");
        }
        if self.batch_size > MAX_PROBE_BATCH_SIZE {
            anyhow::bail!(
                "batch size cannot exceed {} (requested: {})",
                MAX_PROBE_BATCH_SIZE,
                self.batch_size
            );
        }
        if self.concurrency == 0 {
            anyhow::bail!("worker concurrency must be greater than 0");
        }
        if self.concurrency > MAX_PROBE_CONCURRENCY {
            anyhow::bail!(
                "worker concurrency cannot exceed {} (requested: {})",
                MAX_PROBE_CONCURRENCY,
                self.concurrency
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StaffAoProbeSummary {
    pub repository_id: u32,
    pub total_ids_discovered: usize,
    pub batch_size: usize,
    pub concurrency: usize,
    pub total_batches: usize,
    pub batches_completed: usize,
    pub batches_failed: usize,
    /// Total count of failures encountered during the probe.
    /// Unambiguously defined as the number of failed batches (`batches_failed`) plus any
    /// pre-batch abort incidents (ID list fetch error or duplicate input IDs; where batches_failed=0)
    /// or post-batch validation incidents (cross-batch duplicate IDs, unexpected IDs, URI mismatches,
    /// or missing IDs when batches_failed=0). Guaranteed to be 0 if and only if `success` is true.
    pub failures: usize,
    pub exact_ids_returned: usize,
    pub unique_uris: usize,
    pub missing_ids: usize,
    pub duplicate_ids: usize,
    pub unexpected_ids: usize,
    pub id_list_fetch_elapsed_sec: f64,
    pub batch_fetches_elapsed_sec: f64,
    /// Total wall clock elapsed seconds covering ID list discovery and all batch hydration fetches.
    /// Explicitly excludes authentication (main.rs logs in prior to probe execution).
    pub total_wall_elapsed_sec: f64,
    pub records_per_second: f64,
    pub solr_build_or_write_measured: bool,
    pub note: String,
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

pub async fn execute_staff_ao_probe(
    client: &Client,
    repo_id: u32,
    batch_size: usize,
    concurrency: usize,
) -> StaffAoProbeSummary {
    let wall_start = Instant::now();
    let id_start = Instant::now();
    let id_result = client.archival_object_ids(repo_id).await;
    let id_list_fetch_elapsed_sec = id_start.elapsed().as_secs_f64();

    let ids = match id_result {
        Ok(ids) => ids,
        Err(err) => {
            let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
            return StaffAoProbeSummary {
                repository_id: repo_id,
                total_ids_discovered: 0,
                batch_size,
                concurrency,
                total_batches: 0,
                batches_completed: 0,
                batches_failed: 0,
                failures: 1,
                exact_ids_returned: 0,
                unique_uris: 0,
                missing_ids: 0,
                duplicate_ids: 0,
                unexpected_ids: 0,
                id_list_fetch_elapsed_sec,
                batch_fetches_elapsed_sec: 0.0,
                total_wall_elapsed_sec,
                records_per_second: 0.0,
                solr_build_or_write_measured: false,
                note: PROBE_NOTE.to_string(),
                success: false,
                error_message: Some(format!("Failed to fetch archival object ID list: {}", err)),
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
        return StaffAoProbeSummary {
            repository_id: repo_id,
            total_ids_discovered: ids.len(),
            batch_size,
            concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 1,
            exact_ids_returned: 0,
            unique_uris: 0,
            missing_ids: 0,
            duplicate_ids: duplicate_input_ids,
            unexpected_ids: 0,
            id_list_fetch_elapsed_sec,
            batch_fetches_elapsed_sec: 0.0,
            total_wall_elapsed_sec,
            records_per_second: 0.0,
            solr_build_or_write_measured: false,
            note: PROBE_NOTE.to_string(),
            success: false,
            error_message: Some(format!(
                "Backend returned {} duplicate ID(s) in repository AO ID list",
                duplicate_input_ids
            )),
        };
    }

    if ids.is_empty() {
        let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
        return StaffAoProbeSummary {
            repository_id: repo_id,
            total_ids_discovered: 0,
            batch_size,
            concurrency,
            total_batches: 0,
            batches_completed: 0,
            batches_failed: 0,
            failures: 0,
            exact_ids_returned: 0,
            unique_uris: 0,
            missing_ids: 0,
            duplicate_ids: 0,
            unexpected_ids: 0,
            id_list_fetch_elapsed_sec,
            batch_fetches_elapsed_sec: 0.0,
            total_wall_elapsed_sec,
            records_per_second: 0.0,
            solr_build_or_write_measured: false,
            note: PROBE_NOTE.to_string(),
            success: true,
            error_message: None,
        };
    }

    let batches = partition_ids_into_batches(&ids, batch_size);
    let total_batches = batches.len();
    let batch_start = Instant::now();

    let stream = stream::iter(batches.into_iter().enumerate().map(|(batch_idx, chunk)| {
        let client = client.clone();
        async move {
            let res = client
                .fetch_staff_archival_objects_by_ids(repo_id, &chunk)
                .await;
            (batch_idx, chunk, res)
        }
    }))
    .buffer_unordered(concurrency);

    let mut batches_completed = 0;
    let mut batches_failed = 0;
    let mut returned_ids = HashSet::with_capacity(ids.len());
    let mut unique_uris = HashSet::with_capacity(ids.len());
    let mut duplicate_ids_count = 0;
    let mut unexpected_ids_count = 0;
    let mut first_error: Option<String> = None;

    let mut stream = stream;
    let expected_prefix = format!("/repositories/{}/archival_objects/", repo_id);

    while let Some((batch_idx, chunk, res)) = stream.next().await {
        match res {
            Ok(records) => {
                batches_completed += 1;
                for (rec_idx, record) in records.into_iter().enumerate() {
                    let uri = match record.get("uri").and_then(|v| v.as_str()) {
                        Some(u) => u.to_string(),
                        None => {
                            unexpected_ids_count += 1;
                            if first_error.is_none() {
                                first_error = Some(format!(
                                    "Record at index {} in batch {} missing string 'uri'",
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
                                "Record URI '{}' does not match expected prefix '{}'",
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
                                    "Record URI '{}' contains non-numeric ID '{}'",
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
            }
            Err(err) => {
                batches_failed += 1;
                if first_error.is_none() {
                    first_error = Some(format!(
                        "Batch {} (size {}) failed: {}",
                        batch_idx,
                        chunk.len(),
                        err
                    ));
                }
            }
        }
    }

    let missing_ids_count = initial_ids_set
        .iter()
        .filter(|id| !returned_ids.contains(id))
        .count();
    if missing_ids_count > 0 && first_error.is_none() {
        first_error = Some(format!(
            "Missing {} archival object ID(s) from hydration response",
            missing_ids_count
        ));
    }

    let batch_fetches_elapsed_sec = batch_start.elapsed().as_secs_f64();
    let total_wall_elapsed_sec = wall_start.elapsed().as_secs_f64();
    let records_per_second = if total_wall_elapsed_sec > 0.0 {
        returned_ids.len() as f64 / total_wall_elapsed_sec
    } else {
        0.0
    };

    // Unambiguous failure semantics: failures = number of failed batches (`batches_failed`)
    // plus any post-batch validation incidents (e.g. cross-batch duplicate IDs, unexpected IDs,
    // URI count mismatches, or missing IDs when batches_failed == 0).
    // Batches are never counted as both completed and failed.
    let mut post_validation_incidents = 0;
    if duplicate_ids_count > 0 {
        post_validation_incidents += 1;
    }
    if unexpected_ids_count > 0 {
        post_validation_incidents += 1;
    }
    if unique_uris.len() != returned_ids.len() {
        post_validation_incidents += 1;
    }
    if missing_ids_count > 0 && batches_failed == 0 {
        post_validation_incidents += 1;
    }

    let failures = batches_failed + post_validation_incidents;
    let success = failures == 0;

    StaffAoProbeSummary {
        repository_id: repo_id,
        total_ids_discovered: ids.len(),
        batch_size,
        concurrency,
        total_batches,
        batches_completed,
        batches_failed,
        failures,
        exact_ids_returned: returned_ids.len(),
        unique_uris: unique_uris.len(),
        missing_ids: missing_ids_count,
        duplicate_ids: duplicate_ids_count,
        unexpected_ids: unexpected_ids_count,
        id_list_fetch_elapsed_sec,
        batch_fetches_elapsed_sec,
        total_wall_elapsed_sec,
        records_per_second,
        solr_build_or_write_measured: false,
        note: PROBE_NOTE.to_string(),
        success,
        error_message: first_error,
    }
}

pub fn print_human_summary(summary: &StaffAoProbeSummary) {
    println!(
        "\n--- STAFF Archival Object Hydration Probe (Repo {}) ---",
        summary.repository_id
    );
    println!("Repository ID: {}", summary.repository_id);
    println!("Total IDs Discovered: {}", summary.total_ids_discovered);
    println!("Batch Size: {}", summary.batch_size);
    println!("Worker Concurrency: {}", summary.concurrency);
    println!("Total Batches: {}", summary.total_batches);
    println!("Batches Completed: {}", summary.batches_completed);
    println!("Batches Failed: {}", summary.batches_failed);
    println!("Failures: {}", summary.failures);
    println!("Exact IDs Returned: {}", summary.exact_ids_returned);
    println!("Unique URIs: {}", summary.unique_uris);
    println!("Missing IDs: {}", summary.missing_ids);
    println!("Duplicate IDs: {}", summary.duplicate_ids);
    println!("Unexpected IDs: {}", summary.unexpected_ids);
    println!("ID List Elapsed: {:.4}s", summary.id_list_fetch_elapsed_sec);
    println!(
        "Batch Fetches Elapsed: {:.4}s",
        summary.batch_fetches_elapsed_sec
    );
    println!(
        "Total Wall Elapsed: {:.4}s (excludes auth)",
        summary.total_wall_elapsed_sec
    );
    println!("Records / Sec: {:.2}", summary.records_per_second);
    println!(
        "Solr Build/Write Measured: {}",
        summary.solr_build_or_write_measured
    );
    println!("Note: {}", summary.note);
    if let Some(ref err) = summary.error_message {
        println!("Error: {}", err);
    }
    println!(
        "Status: {}",
        if summary.success { "SUCCESS" } else { "FAILED" }
    );
}

pub async fn run_staff_ao_probe(
    client: &Client,
    args: StaffAoProbeArgs,
) -> Result<StaffAoProbeSummary> {
    args.validate()?;
    let summary =
        execute_staff_ao_probe(client, args.repo, args.batch_size, args.concurrency).await;

    if args.json {
        let json_out = serde_json::to_string_pretty(&summary)?;
        println!("{}", json_out);
    } else {
        print_human_summary(&summary);
    }

    if !summary.success {
        anyhow::bail!(
            "STAFF archival object hydration probe failed: {}",
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
    use aspace_client::STAFF_INDEXER_RESOLVED_ATTRIBUTES;
    use mockito::Server;

    #[test]
    fn test_cli_argument_bounds() {
        let valid_args = StaffAoProbeArgs {
            repo: 2,
            batch_size: 25,
            concurrency: 4,
            json: false,
        };
        assert!(valid_args.validate().is_ok());

        let min_batch = StaffAoProbeArgs {
            batch_size: 1,
            ..valid_args.clone()
        };
        assert!(min_batch.validate().is_ok());

        let zero_batch = StaffAoProbeArgs {
            batch_size: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            zero_batch.validate().unwrap_err().to_string(),
            "batch size must be greater than 0"
        );

        let oversized_batch = StaffAoProbeArgs {
            batch_size: 26,
            ..valid_args.clone()
        };
        assert_eq!(
            oversized_batch.validate().unwrap_err().to_string(),
            "batch size cannot exceed 25 (requested: 26)"
        );

        let min_concurrency = StaffAoProbeArgs {
            concurrency: 1,
            ..valid_args.clone()
        };
        assert!(min_concurrency.validate().is_ok());

        let max_concurrency = StaffAoProbeArgs {
            concurrency: 32,
            ..valid_args.clone()
        };
        assert!(max_concurrency.validate().is_ok());

        let zero_concurrency = StaffAoProbeArgs {
            concurrency: 0,
            ..valid_args.clone()
        };
        assert_eq!(
            zero_concurrency.validate().unwrap_err().to_string(),
            "worker concurrency must be greater than 0"
        );

        let oversized_concurrency = StaffAoProbeArgs {
            concurrency: 33,
            ..valid_args.clone()
        };
        assert_eq!(
            oversized_concurrency.validate().unwrap_err().to_string(),
            "worker concurrency cannot exceed 32 (requested: 33)"
        );
    }

    #[test]
    fn test_batching_boundaries() {
        assert_eq!(partition_ids_into_batches(&[], 25), Vec::<Vec<u32>>::new());
        assert_eq!(partition_ids_into_batches(&[1], 0), Vec::<Vec<u32>>::new());

        let one_id = vec![101];
        assert_eq!(partition_ids_into_batches(&one_id, 25), vec![vec![101]]);

        let under_batch: Vec<u32> = (1..=24).collect();
        let chunks_24 = partition_ids_into_batches(&under_batch, 25);
        assert_eq!(chunks_24.len(), 1);
        assert_eq!(chunks_24[0].len(), 24);

        let exact_batch: Vec<u32> = (1..=25).collect();
        let chunks_25 = partition_ids_into_batches(&exact_batch, 25);
        assert_eq!(chunks_25.len(), 1);
        assert_eq!(chunks_25[0].len(), 25);

        let just_over_batch: Vec<u32> = (1..=26).collect();
        let chunks_26 = partition_ids_into_batches(&just_over_batch, 25);
        assert_eq!(chunks_26.len(), 2);
        assert_eq!(chunks_26[0].len(), 25);
        assert_eq!(chunks_26[1].len(), 1);

        let two_exact_batches: Vec<u32> = (1..=50).collect();
        let chunks_50 = partition_ids_into_batches(&two_exact_batches, 25);
        assert_eq!(chunks_50.len(), 2);
        assert_eq!(chunks_50[0].len(), 25);
        assert_eq!(chunks_50[1].len(), 25);

        let uneven_batches: Vec<u32> = (1..=63).collect();
        let chunks_63 = partition_ids_into_batches(&uneven_batches, 25);
        assert_eq!(chunks_63.len(), 3);
        assert_eq!(chunks_63[0].len(), 25);
        assert_eq!(chunks_63[1].len(), 25);
        assert_eq!(chunks_63[2].len(), 13);

        let small_batch_size = partition_ids_into_batches(&[1, 2, 3, 4, 5], 2);
        assert_eq!(small_batch_size, vec![vec![1, 2], vec![3, 4], vec![5]]);
    }

    #[tokio::test]
    async fn test_probe_empty_id_list() {
        let mut server = Server::new_async().await;

        let _mock_ao_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 25, 4).await;
        assert_eq!(summary.repository_id, 2);
        assert_eq!(summary.total_ids_discovered, 0);
        assert_eq!(summary.total_batches, 0);
        assert_eq!(summary.batches_completed, 0);
        assert_eq!(summary.batches_failed, 0);
        assert_eq!(summary.failures, 0);
        assert_eq!(summary.exact_ids_returned, 0);
        assert_eq!(summary.unique_uris, 0);
        assert_eq!(summary.missing_ids, 0);
        assert_eq!(summary.duplicate_ids, 0);
        assert_eq!(summary.unexpected_ids, 0);
        assert!(!summary.solr_build_or_write_measured);
        assert_eq!(summary.note, PROBE_NOTE);
        assert!(summary.success);
        assert!(summary.error_message.is_none());
    }

    #[tokio::test]
    async fn test_probe_multi_batch_and_concurrency_success() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[101, 102, 103, 104, 105]"#)
            .create_async()
            .await;

        let make_query = |ids_str: &str| -> String {
            format!(
                "id_set={}&{}",
                ids_str,
                STAFF_INDEXER_RESOLVED_ATTRIBUTES
                    .iter()
                    .map(|attr| format!("resolve%5B%5D={}", attr.replace("::", "%3A%3A")))
                    .collect::<Vec<_>>()
                    .join("&")
            )
        };

        let _mock_batch_1 = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Exact(make_query("101%2C102")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/101", "title": "Item 101"},
                {"uri": "/repositories/2/archival_objects/102", "title": "Item 102"}
            ]"#,
            )
            .create_async()
            .await;

        let _mock_batch_2 = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Exact(make_query("103%2C104")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/103", "title": "Item 103"},
                {"uri": "/repositories/2/archival_objects/104", "title": "Item 104"}
            ]"#,
            )
            .create_async()
            .await;

        let _mock_batch_3 = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Exact(make_query("105")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/105", "title": "Item 105"}
            ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 2, 2).await;
        assert_eq!(summary.repository_id, 2);
        assert_eq!(summary.total_ids_discovered, 5);
        assert_eq!(summary.batch_size, 2);
        assert_eq!(summary.concurrency, 2);
        assert_eq!(summary.total_batches, 3);
        assert_eq!(summary.batches_completed, 3);
        assert_eq!(summary.batches_failed, 0);
        assert_eq!(summary.failures, 0);
        assert_eq!(summary.exact_ids_returned, 5);
        assert_eq!(summary.unique_uris, 5);
        assert_eq!(summary.missing_ids, 0);
        assert_eq!(summary.duplicate_ids, 0);
        assert_eq!(summary.unexpected_ids, 0);
        assert!(!summary.solr_build_or_write_measured);
        assert!(summary.success);
        assert!(summary.error_message.is_none());
    }

    #[tokio::test]
    async fn test_probe_duplicate_detection() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[101, 102]"#)
            .create_async()
            .await;

        let _mock_batch = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/repositories/2/archival_objects.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/101", "title": "Item 101"},
                {"uri": "/repositories/2/archival_objects/101", "title": "Item 101 Dup"}
            ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 25, 4).await;
        assert_eq!(summary.batches_completed, 0);
        assert_eq!(summary.batches_failed, 1);
        assert_eq!(summary.failures, 1);
        assert!(!summary.success);
        assert!(summary.error_message.is_some());
    }

    #[tokio::test]
    async fn test_probe_missing_id_detection() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[101, 102]"#)
            .create_async()
            .await;

        let _mock_batch = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/repositories/2/archival_objects.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/101", "title": "Item 101"}
            ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 25, 4).await;
        assert_eq!(summary.batches_completed, 0);
        assert_eq!(summary.batches_failed, 1);
        assert_eq!(summary.failures, 1);
        assert!(!summary.success);
        assert!(summary.error_message.is_some());
    }

    #[tokio::test]
    async fn test_probe_unexpected_id_detection() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[101]"#)
            .create_async()
            .await;

        let _mock_batch = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/repositories/2/archival_objects.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/999", "title": "Item 999"}
            ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 25, 4).await;
        assert_eq!(summary.batches_completed, 0);
        assert_eq!(summary.batches_failed, 1);
        assert_eq!(summary.failures, 1);
        assert!(!summary.success);
        assert!(summary.error_message.is_some());
    }

    #[tokio::test]
    async fn test_probe_error_propagation_id_list() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Backend crashed"}"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 25, 4).await;
        assert_eq!(summary.total_ids_discovered, 0);
        assert_eq!(summary.total_batches, 0);
        assert_eq!(summary.batches_completed, 0);
        assert_eq!(summary.batches_failed, 0);
        assert_eq!(summary.failures, 1);
        assert!(!summary.success);
        assert!(summary.error_message.is_some());

        let args = StaffAoProbeArgs {
            repo: 2,
            batch_size: 25,
            concurrency: 4,
            json: true,
        };
        let err = run_staff_ao_probe(&client, args).await.unwrap_err();
        assert!(err
            .to_string()
            .contains("STAFF archival object hydration probe failed"));
    }

    #[tokio::test]
    async fn test_probe_duplicate_input_id_list() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[101, 102, 101]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 25, 4).await;
        assert_eq!(summary.total_ids_discovered, 3);
        assert_eq!(summary.total_batches, 0);
        assert_eq!(summary.batches_completed, 0);
        assert_eq!(summary.batches_failed, 0);
        assert_eq!(summary.failures, 1);
        assert_eq!(summary.duplicate_ids, 1);
        assert!(!summary.success);
        assert!(summary
            .error_message
            .as_deref()
            .unwrap_or("")
            .contains("duplicate ID(s) in repository AO ID list"));
    }

    #[tokio::test]
    async fn test_probe_error_propagation_batch_fetch() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[101]"#)
            .create_async()
            .await;

        let _mock_batch = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/repositories/2/archival_objects.*".to_string()),
            )
            .with_status(500)
            .with_body(r#"Server error"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 25, 4).await;
        assert_eq!(summary.total_ids_discovered, 1);
        assert_eq!(summary.total_batches, 1);
        assert_eq!(summary.batches_completed, 0);
        assert_eq!(summary.batches_failed, 1);
        assert_eq!(summary.failures, 1);
        assert!(!summary.success);
        assert!(summary.error_message.is_some());

        let args = StaffAoProbeArgs {
            repo: 2,
            batch_size: 25,
            concurrency: 4,
            json: false,
        };
        let err = run_staff_ao_probe(&client, args).await.unwrap_err();
        assert!(err
            .to_string()
            .contains("STAFF archival object hydration probe failed"));
    }

    #[tokio::test]
    async fn test_probe_validation_failures_never_zero_when_unsuccessful() {
        let mut server = Server::new_async().await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[101, 102]"#)
            .create_async()
            .await;

        let make_query = |ids_str: &str| -> String {
            format!(
                "id_set={}&{}",
                ids_str,
                STAFF_INDEXER_RESOLVED_ATTRIBUTES
                    .iter()
                    .map(|attr| format!("resolve%5B%5D={}", attr.replace("::", "%3A%3A")))
                    .collect::<Vec<_>>()
                    .join("&")
            )
        };

        // Batch 1 succeeds
        let _mock_batch_1 = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Exact(make_query("101")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[{"uri": "/repositories/2/archival_objects/101", "title": "Item 101"}]"#)
            .create_async()
            .await;

        // Batch 2 fails with 500
        let _mock_batch_2 = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Exact(make_query("102")))
            .with_status(500)
            .with_body(r#"Server error"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("test-session".to_string());

        let summary = execute_staff_ao_probe(&client, 2, 1, 1).await;
        assert_eq!(summary.total_batches, 2);
        assert_eq!(summary.batches_completed, 1);
        assert_eq!(summary.batches_failed, 1);
        assert_eq!(summary.failures, 1);
        assert!(!summary.success);
        assert_eq!(
            summary.batches_completed + summary.batches_failed,
            summary.total_batches
        );
    }

    #[test]
    fn test_metrics_shape_and_solr_note() {
        let summary = StaffAoProbeSummary {
            repository_id: 2,
            total_ids_discovered: 10,
            batch_size: 25,
            concurrency: 4,
            total_batches: 1,
            batches_completed: 1,
            batches_failed: 0,
            failures: 0,
            exact_ids_returned: 10,
            unique_uris: 10,
            missing_ids: 0,
            duplicate_ids: 0,
            unexpected_ids: 0,
            id_list_fetch_elapsed_sec: 0.1234,
            batch_fetches_elapsed_sec: 0.5678,
            total_wall_elapsed_sec: 0.6912,
            records_per_second: 14.47,
            solr_build_or_write_measured: false,
            note: PROBE_NOTE.to_string(),
            success: true,
            error_message: None,
        };

        let json_str = serde_json::to_string_pretty(&summary).expect("Serialization succeeded");
        let val: serde_json::Value = serde_json::from_str(&json_str).expect("Valid JSON");

        assert_eq!(val["repository_id"], 2);
        assert_eq!(val["total_ids_discovered"], 10);
        assert_eq!(val["batch_size"], 25);
        assert_eq!(val["concurrency"], 4);
        assert_eq!(val["total_batches"], 1);
        assert_eq!(val["batches_completed"], 1);
        assert_eq!(val["batches_failed"], 0);
        assert_eq!(val["failures"], 0);
        assert_eq!(val["exact_ids_returned"], 10);
        assert_eq!(val["unique_uris"], 10);
        assert_eq!(val["missing_ids"], 0);
        assert_eq!(val["duplicate_ids"], 0);
        assert_eq!(val["unexpected_ids"], 0);
        assert!(val["id_list_fetch_elapsed_sec"].is_f64());
        assert!(val["batch_fetches_elapsed_sec"].is_f64());
        assert!(val["total_wall_elapsed_sec"].is_f64());
        assert!(val["records_per_second"].is_f64());
        assert_eq!(val["solr_build_or_write_measured"], false);
        assert_eq!(val["note"], PROBE_NOTE);
        assert_eq!(val["success"], true);
        assert!(val.get("error_message").is_none());

        assert!(!json_str.contains("secret"));
        assert!(!json_str.contains("password"));
        assert!(!json_str.contains("session"));
        assert!(!json_str.contains("payload"));
    }
}
