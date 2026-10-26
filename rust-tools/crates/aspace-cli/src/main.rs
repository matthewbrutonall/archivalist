use anyhow::Result;
use aspace_client::validation::{
    validate_archival_object, validate_digital_object, validate_resource, ValidationIssueRow,
    ValidationMetrics, ValidationReport, ValidationSummary,
};
use aspace_client::Client;
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;
use std::{collections::BTreeMap, io};

pub mod shadow;
pub mod staff_ao_builder;
pub mod staff_ao_candidate;
pub mod staff_ao_probe;

#[derive(Serialize)]
struct PerfPingReport {
    endpoint_url: String,
    auth_elapsed_sec: f64,
    ping_elapsed_sec: f64,
    total_elapsed_sec: f64,
    success: bool,
    summary: Option<String>,
}

#[derive(Serialize, Clone)]
struct PerfScanTargetReport {
    record_type: String,
    total_records: usize,
    total_batches: usize,
    batch_size: usize,
    id_fetch_elapsed_sec: f64,
    batch_fetch_elapsed_sec: f64,
    total_elapsed_sec: f64,
    records_per_sec: f64,
}

#[derive(Serialize, Clone)]
struct PerfRepoScanRun {
    run_index: usize,
    total_elapsed_sec: f64,
    overall_records_per_sec: f64,
    targets: Vec<PerfScanTargetReport>,
}

#[derive(Serialize)]
struct PerfScanAggregateTarget {
    record_type: String,
    min_total_elapsed_sec: f64,
    mean_total_elapsed_sec: f64,
    max_total_elapsed_sec: f64,
    min_records_per_sec: f64,
    mean_records_per_sec: f64,
    max_records_per_sec: f64,
}

#[derive(Serialize)]
struct PerfRepoScanAggregate {
    min_total_elapsed_sec: f64,
    mean_total_elapsed_sec: f64,
    max_total_elapsed_sec: f64,
    min_overall_records_per_sec: f64,
    mean_overall_records_per_sec: f64,
    max_overall_records_per_sec: f64,
    targets: Vec<PerfScanAggregateTarget>,
}

#[derive(Serialize)]
struct PerfRepoScanReport {
    endpoint_url: String,
    repo_id: u32,
    auth_elapsed_sec: f64,
    concurrency: usize,
    total_records: usize,
    warmup_used: bool,
    runs: Vec<PerfRepoScanRun>,
    aggregate: PerfRepoScanAggregate,
}

#[derive(Serialize)]
struct PerfEndpointSample {
    endpoint_label: String,
    endpoint_path: String,
    repeat_count: usize,
    success_count: usize,
    error_count: usize,
    min_elapsed_ms: f64,
    mean_elapsed_ms: f64,
    max_elapsed_ms: f64,
}

#[derive(Serialize)]
struct PerfEndpointsReport {
    endpoint_url: String,
    repo_id: u32,
    auth_elapsed_sec: f64,
    endpoints: Vec<PerfEndpointSample>,
}

#[derive(Serialize)]
struct PerfSearchCheckReport {
    endpoint_url: String,
    repo_id: u32,
    search_path: String,
    repeat_count: usize,
    success_count: usize,
    error_count: usize,
    min_elapsed_ms: f64,
    mean_elapsed_ms: f64,
    max_elapsed_ms: f64,
    total_hits: Option<u64>,
}

#[derive(Serialize)]
struct PerfRecordTypesReport {
    endpoint_url: String,
    repo_id: u32,
    search_path: String,
    total_hits: Option<u64>,
    counts_by_type: BTreeMap<String, u64>,
    elapsed_ms: f64,
    success: bool,
}

#[derive(Serialize)]
struct PerfHealthReport {
    endpoint_url: String,
    repo_id: u32,
    overall_success: bool,
    ping: PerfPingReport,
    record_types: PerfRecordTypesReport,
    search: PerfSearchCheckReport,
    solr: PerfSolrCheckReport,
}

#[derive(Serialize)]
struct PerfConsistencyCheck {
    record_type: String,
    api_count: usize,
    search_count: Option<u64>,
    status: String,
}

#[derive(Serialize)]
struct PerfConsistencyReport {
    endpoint_url: String,
    repo_id: u32,
    overall_status: String,
    checks: Vec<PerfConsistencyCheck>,
    search_total_hits: Option<u64>,
    solr_num_found: Option<u64>,
    notes: Vec<String>,
}

#[derive(clap::Args)]
struct PerfEndpointsArgs {
    #[arg(short, long)]
    repo: u32,

    #[arg(long, help = "Output as JSON")]
    json: bool,

    #[arg(long, help = "Number of measured runs", default_value = "3")]
    repeat: std::num::NonZeroUsize,
}

#[derive(clap::Args)]
struct PerfSearchCheckArgs {
    #[arg(short, long)]
    repo: u32,

    #[arg(long, help = "Output as JSON")]
    json: bool,

    #[arg(long, help = "Number of measured runs", default_value = "3")]
    repeat: std::num::NonZeroUsize,
}

#[derive(clap::Args)]
struct PerfRecordTypesArgs {
    #[arg(short, long)]
    repo: u32,

    #[arg(long, help = "Output as JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct PerfHealthArgs {
    #[arg(short, long)]
    repo: u32,

    #[arg(long, default_value = "http://localhost:8985/solr/archivesspace")]
    solr_url: String,

    #[arg(long, help = "Output as JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct PerfConsistencyArgs {
    #[arg(short, long)]
    repo: u32,

    #[arg(long, default_value = "http://localhost:8985/solr/archivesspace")]
    solr_url: String,

    #[arg(long, help = "Output as JSON")]
    json: bool,
}

#[derive(Serialize)]
struct PerfRepoScanCsvRow {
    repo_id: u32,
    run_index: usize,
    record_type: String,
    total_records: usize,
    total_batches: usize,
    batch_size: usize,
    concurrency: usize,
    warmup_used: bool,
    elapsed_time_sec: f64,
    records_per_sec: f64,
    success: bool,
}

#[derive(clap::Args)]
struct PerfRepoScanArgs {
    #[arg(short, long)]
    repo: u32,

    #[arg(long, help = "Output as JSON", conflicts_with = "csv")]
    json: bool,

    #[arg(long, help = "Output as CSV", conflicts_with = "json")]
    csv: bool,

    #[arg(
        short,
        long,
        help = "File to output CSV to (defaults to stdout)",
        requires = "csv",
        conflicts_with = "json"
    )]
    output: Option<String>,

    #[arg(long, help = "Number of measured runs", default_value = "1")]
    repeat: std::num::NonZeroUsize,

    #[arg(
        long,
        help = "Run an unreported warmup scan first",
        default_value_t = false
    )]
    warmup: bool,

    #[arg(
        long,
        help = "Number of records to fetch per batch",
        default_value = "50"
    )]
    batch_size: std::num::NonZeroUsize,

    #[arg(
        long,
        help = "Bounded concurrency for batch fetching",
        default_value = "1"
    )]
    concurrency: std::num::NonZeroUsize,
}

#[derive(Parser)]
#[command(name = "aspace")]
#[command(about = "Safe, non-destructive ArchivesSpace administration CLI", long_about = None)]
struct Cli {
    #[arg(
        short,
        long,
        env = "ASPACE_URL",
        default_value = "http://localhost:8089"
    )]
    url: String,

    #[arg(short = 'U', long, env = "ASPACE_USER", default_value = "admin")]
    user: String,

    #[arg(short = 'P', long, env = "ASPACE_PASSWORD", default_value = "admin")]
    password: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    ShadowIndex(shadow::ShadowIndexCommand),
    ShadowQuery {
        #[command(subcommand)]
        target: ShadowQueryTarget,
    },
    Ping,
    Perf {
        #[command(subcommand)]
        target: PerfTarget,
    },
    Validate {
        #[command(subcommand)]
        target: ValidateTarget,
    },
    #[command(name = "staff-ao-probe", alias = "staff-ao-hydration-probe")]
    StaffAoProbe(staff_ao_probe::StaffAoProbeArgs),
    #[command(name = "staff-ao-candidate")]
    StaffAoCandidate(staff_ao_candidate::StaffAoCandidateArgs),
    #[command(name = "staff-ao-builder")]
    StaffAoBuilder(staff_ao_builder::StaffAoBuilderArgs),
    #[command(name = "build-index-document")]
    BuildIndexDocument(staff_ao_builder::BuildIndexDocumentArgs),
}

#[derive(Subcommand)]
enum ShadowQueryTarget {
    Descendants(ShadowQueryDescendantsArgs),
    Search(ShadowQuerySearchArgs),
}

#[derive(clap::Args, Clone)]
struct ShadowQuerySearchArgs {
    #[arg(long)]
    q: Option<String>,

    #[arg(long)]
    resource_uri: Option<String>,

    #[arg(long)]
    date_filter: Option<String>,

    #[arg(long)]
    title_filter: Option<String>,

    #[arg(long, default_value = "50")]
    rows: usize,

    #[arg(long, default_value = "0")]
    start: usize,

    #[arg(long, default_value = "http://localhost:8985/solr")]
    solr_url: String,

    #[arg(long, default_value = "archivesspace_shadow")]
    collection: String,

    #[arg(long)]
    json: bool,

    #[arg(long, help = "Output nested tree structure")]
    tree: bool,

    #[arg(
        long,
        help = "Include ancestor context documents to connect matches to root"
    )]
    include_ancestors: bool,
}

#[derive(clap::Args, Clone)]
struct ShadowQueryDescendantsArgs {
    #[arg(long)]
    resource_uri: String,

    #[arg(long)]
    date_filter: Option<String>,

    #[arg(long)]
    title_filter: Option<String>,

    #[arg(long, default_value = "50")]
    rows: usize,

    #[arg(long, default_value = "0")]
    start: usize,

    #[arg(long, default_value = "http://localhost:8985/solr")]
    solr_url: String,

    #[arg(long, default_value = "archivesspace_shadow")]
    collection: String,

    #[arg(long)]
    json: bool,

    #[arg(long, help = "Output nested tree structure")]
    tree: bool,

    #[arg(
        long,
        help = "Include ancestor context documents to connect matches to root"
    )]
    include_ancestors: bool,
}

#[derive(Serialize)]
struct ShadowQueryOutput {
    resource_uri: String,
    date_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_filter: Option<String>,
    solr_url: String,
    collection: String,
    rows: usize,
    start: usize,
    num_found: u64,
    docs: Vec<aspace_client::ShadowArchivalObjectHit>,
}

#[derive(Serialize)]
struct ShadowSearchOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_filter: Option<String>,
    solr_url: String,
    collection: String,
    rows: usize,
    start: usize,
    num_found: u64,
    query_string: String,
    docs: Vec<aspace_client::ShadowArchivalObjectHit>,
}

#[derive(Serialize)]
struct ShadowTreeQueryOutput {
    resource_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    q: Option<String>,
    date_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_filter: Option<String>,
    solr_url: String,
    collection: String,
    rows: usize,
    start: usize,
    num_found: u64,
    total_nodes: usize,
    tree: Vec<aspace_client::ShadowTreeNode>,
    orphans: Vec<aspace_client::ShadowTreeNode>,
}

#[derive(clap::Args)]
struct PerfSolrCheckArgs {
    #[arg(long, default_value = "http://localhost:8985/solr/archivesspace")]
    solr_url: String,

    #[arg(long, default_value = "3")]
    repeat: std::num::NonZeroUsize,

    #[arg(long, help = "Output as JSON")]
    json: bool,
}

#[derive(serde::Serialize)]
struct PerfSolrCheckReport {
    solr_url: String,
    repeat_count: usize,
    success_count: usize,
    error_count: usize,
    min_elapsed_ms: f64,
    mean_elapsed_ms: f64,
    max_elapsed_ms: f64,
    num_found: Option<u64>,
}

#[derive(Subcommand)]
enum PerfTarget {
    Ping {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    RepositoryScan(PerfRepoScanArgs),
    Consistency(PerfConsistencyArgs),
    Endpoints(PerfEndpointsArgs),
    Health(PerfHealthArgs),
    RecordTypes(PerfRecordTypesArgs),
    SearchCheck(PerfSearchCheckArgs),
    SolrCheck(PerfSolrCheckArgs),
    #[command(name = "staff-ao-probe", alias = "staff-ao-hydration-probe")]
    StaffAoProbe(staff_ao_probe::StaffAoProbeArgs),
}

#[derive(clap::Args)]
struct ValidateArgs {
    #[arg(short, long)]
    repo: u32,

    #[arg(long, help = "Output as JSON", conflicts_with = "csv")]
    json: bool,

    #[arg(long, help = "Output as CSV", conflicts_with = "json")]
    csv: bool,

    #[arg(
        short,
        long,
        help = "File to output CSV to (defaults to stdout)",
        requires = "csv",
        conflicts_with = "json"
    )]
    output: Option<String>,
}

async fn run_shadow_query_descendants(args: ShadowQueryDescendantsArgs) -> Result<()> {
    if args.collection == "archivesspace" {
        anyhow::bail!("refusing to query live Solr collection 'archivesspace'");
    }

    if args.include_ancestors && !args.tree {
        anyhow::bail!("--include-ancestors requires --tree");
    }

    if args.tree && args.rows == 0 {
        anyhow::bail!("cannot assemble tree with rows=0 (documents must be fetched)");
    }

    let client = aspace_client::ShadowSolrClient::new(&args.solr_url, &args.collection);
    let params = aspace_client::ShadowSearchParams {
        base_url: args.solr_url.clone(),
        collection: args.collection.clone(),
        resource_uri: args.resource_uri.clone(),
        date_filter: args.date_filter.clone(),
        title_filter: args.title_filter.clone(),
        rows: args.rows,
        start: args.start,
    };
    let result = client.search(&params).await?;

    if args.tree {
        let tree_res = if args.include_ancestors {
            let missing_ancestors =
                aspace_client::collect_missing_ancestor_uris(&result.docs, &args.resource_uri);
            let context_hits = client.docs_by_ids(&missing_ancestors).await?;
            aspace_client::assemble_shadow_tree_with_context(
                result.docs,
                context_hits,
                &args.resource_uri,
            )
        } else {
            aspace_client::assemble_shadow_tree(result.docs, &args.resource_uri)
        };

        let output = ShadowTreeQueryOutput {
            resource_uri: args.resource_uri,
            q: None,
            date_filter: args.date_filter,
            title_filter: args.title_filter,
            solr_url: args.solr_url,
            collection: args.collection,
            rows: args.rows,
            start: args.start,
            num_found: result.num_found,
            total_nodes: tree_res.total_nodes,
            tree: tree_res.tree,
            orphans: tree_res.orphans,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if args.json {
        let output = ShadowQueryOutput {
            resource_uri: args.resource_uri,
            date_filter: args.date_filter,
            title_filter: args.title_filter,
            solr_url: args.solr_url,
            collection: args.collection,
            rows: args.rows,
            start: args.start,
            num_found: result.num_found,
            docs: result.docs,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Resource URI: {}", args.resource_uri);
    println!("Collection: {}", args.collection);
    if let Some(date_filter) = &args.date_filter {
        println!("Date Filter: {}", date_filter);
    }
    if let Some(title_filter) = &args.title_filter {
        println!("Title Filter: {}", title_filter);
    }
    println!("Total Found: {}", result.num_found);
    println!("Start: {}", result.start);
    println!("Returned Docs: {}", result.docs.len());

    if args.rows > 0 {
        for doc in result.docs {
            let title = doc
                .title
                .as_deref()
                .or(doc.display_string.as_deref())
                .unwrap_or("<untitled>");
            println!("{} | {}", doc.id, title);
        }
    }

    Ok(())
}

async fn run_shadow_query_search(args: ShadowQuerySearchArgs) -> Result<()> {
    if args.collection == "archivesspace" {
        anyhow::bail!("refusing to query live Solr collection 'archivesspace'");
    }

    if args.include_ancestors && !args.tree {
        anyhow::bail!("--include-ancestors requires --tree");
    }

    if args.tree {
        if args.resource_uri.is_none() {
            anyhow::bail!("--tree requires --resource-uri");
        }
        if args.rows == 0 {
            anyhow::bail!("cannot assemble tree with rows=0 (documents must be fetched)");
        }
    }

    let client = aspace_client::ShadowSolrClient::new(&args.solr_url, &args.collection);
    let params = aspace_client::ShadowKeywordSearchParams {
        base_url: args.solr_url.clone(),
        collection: args.collection.clone(),
        query: args.q.clone(),
        resource_uri: args.resource_uri.clone(),
        date_filter: args.date_filter.clone(),
        title_filter: args.title_filter.clone(),
        rows: args.rows,
        start: args.start,
    };
    let result = client.keyword_search(&params).await?;

    if args.tree {
        let root_resource_uri = args
            .resource_uri
            .as_deref()
            .expect("resource_uri validated for tree");

        let tree_res = if args.include_ancestors {
            let missing_ancestors =
                aspace_client::collect_missing_ancestor_uris(&result.docs, root_resource_uri);
            let context_hits = client.docs_by_ids(&missing_ancestors).await?;
            aspace_client::assemble_shadow_tree_with_context(
                result.docs,
                context_hits,
                root_resource_uri,
            )
        } else {
            aspace_client::assemble_shadow_tree(result.docs, root_resource_uri)
        };

        let output = ShadowTreeQueryOutput {
            resource_uri: root_resource_uri.to_string(),
            q: args.q,
            date_filter: args.date_filter,
            title_filter: args.title_filter,
            solr_url: args.solr_url,
            collection: args.collection,
            rows: args.rows,
            start: args.start,
            num_found: result.num_found,
            total_nodes: tree_res.total_nodes,
            tree: tree_res.tree,
            orphans: tree_res.orphans,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if args.json {
        let output = ShadowSearchOutput {
            q: args.q,
            resource_uri: args.resource_uri,
            date_filter: args.date_filter,
            title_filter: args.title_filter,
            solr_url: args.solr_url,
            collection: args.collection,
            rows: args.rows,
            start: args.start,
            num_found: result.num_found,
            query_string: result.query_string,
            docs: result.docs,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if let Some(q) = &args.q {
        println!("Query: {}", q);
    }
    if let Some(resource_uri) = &args.resource_uri {
        println!("Resource URI: {}", resource_uri);
    }
    println!("Collection: {}", args.collection);
    if let Some(date_filter) = &args.date_filter {
        println!("Date Filter: {}", date_filter);
    }
    if let Some(title_filter) = &args.title_filter {
        println!("Title Filter: {}", title_filter);
    }
    println!("Solr Query: {}", result.query_string);
    println!("Total Found: {}", result.num_found);
    println!("Start: {}", result.start);
    println!("Returned Docs: {}", result.docs.len());

    if args.rows > 0 {
        for doc in result.docs {
            let title = doc
                .title
                .as_deref()
                .or(doc.display_string.as_deref())
                .unwrap_or("<untitled>");
            println!("{} | {}", doc.id, title);
        }
    }

    Ok(())
}

#[derive(Subcommand)]
enum ValidateTarget {
    Repositories,
    Resources(ValidateArgs),
    ArchivalObjects(ValidateArgs),
    DigitalObjects(ValidateArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Commands::ShadowQuery { target } = &cli.command {
        match target {
            ShadowQueryTarget::Descendants(args) => {
                run_shadow_query_descendants(args.clone()).await?;
            }
            ShadowQueryTarget::Search(args) => {
                run_shadow_query_search(args.clone()).await?;
            }
        }
        return Ok(());
    }

    if let Commands::BuildIndexDocument(args) = &cli.command {
        staff_ao_builder::run_build_index_document(args)?;
        return Ok(());
    }

    let mut client = Client::new(cli.url.clone());

    let auth_start = std::time::Instant::now();
    client.login(&cli.user, &cli.password).await?;
    let auth_elapsed_sec = auth_start.elapsed().as_secs_f64();

    match &cli.command {
        Commands::ShadowQuery { .. } => unreachable!("shadow-query returns before API login"),
        Commands::ShadowIndex(args) => {
            shadow::run_shadow_index(&client, args.clone()).await?;
        }
        Commands::Ping => {
            let res = client.get("version")?.send().await?;
            if res.status().is_success() {
                println!(
                    "✅ Successfully connected to ArchivesSpace (version: {}).",
                    res.text().await?
                );
            } else {
                println!("❌ API error: {}", res.status());
            }
        }
        Commands::Perf { target } => {
            match target {
                PerfTarget::Ping { json } => {
                    let report = run_perf_ping(&client, cli.url.clone(), auth_elapsed_sec).await;
                    if *json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else {
                        println!("Endpoint URL: {}", report.endpoint_url);
                        println!("Auth Elapsed: {:.4}s", report.auth_elapsed_sec);
                        println!("Ping Elapsed: {:.4}s", report.ping_elapsed_sec);
                        println!("Total Elapsed: {:.4}s", report.total_elapsed_sec);
                        println!("Success: {}", report.success);
                        if let Some(ref s) = report.summary {
                            println!("Summary: {}", s);
                        }
                    }
                }
                PerfTarget::RepositoryScan(args) => {
                    let report = run_perf_repository_scan(
                        &client,
                        cli.url.clone(),
                        args.repo,
                        auth_elapsed_sec,
                        args.repeat.get(),
                        args.warmup,
                        args.batch_size.get(),
                        args.concurrency.get(),
                    )
                    .await?;
                    if args.json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else if args.csv {
                        let mut wtr = match &args.output {
                            Some(path) => {
                                let file = std::fs::File::create(path)?;
                                csv::WriterBuilder::new()
                                    .has_headers(true)
                                    .from_writer(Box::new(file) as Box<dyn io::Write>)
                            }
                            None => csv::WriterBuilder::new()
                                .has_headers(true)
                                .from_writer(Box::new(io::stdout()) as Box<dyn io::Write>),
                        };
                        // We emit simple per-run rows as requested. Aggregate rows are not included
                        // in the CSV because statistical summaries (min/mean/max) have a different
                        // schema than individual runs and can be easily computed from the raw CSV data.
                        for run in &report.runs {
                            for t in &run.targets {
                                wtr.serialize(PerfRepoScanCsvRow {
                                    repo_id: report.repo_id,
                                    run_index: run.run_index,
                                    record_type: t.record_type.clone(),
                                    total_records: t.total_records,
                                    total_batches: t.total_batches,
                                    batch_size: t.batch_size,
                                    concurrency: report.concurrency,
                                    warmup_used: report.warmup_used,
                                    elapsed_time_sec: t.total_elapsed_sec,
                                    records_per_sec: t.records_per_sec,
                                    success: true,
                                })?;
                            }
                        }
                        wtr.flush()?;
                    } else {
                        println!(
                            "\n--- Performance Repository Scan (Repo {}) ---",
                            report.repo_id
                        );
                        println!("Endpoint URL: {}", report.endpoint_url);
                        println!("Auth Elapsed: {:.4}s", report.auth_elapsed_sec);
                        println!("Concurrency: {}", report.concurrency);
                        println!(
                            "Warmup Used: {}",
                            if report.warmup_used { "Yes" } else { "No" }
                        );
                        println!("Total Records: {}", report.total_records);

                        if report.runs.len() == 1 {
                            let run = &report.runs[0];
                            println!("Total Elapsed: {:.4}s", run.total_elapsed_sec);
                            println!("Overall Records/Sec: {:.2}", run.overall_records_per_sec);
                            for t in &run.targets {
                                println!(
                                    "
  Target: {}",
                                    t.record_type
                                );
                                println!(
                                    "  Records: {} ({} batches of {})",
                                    t.total_records, t.total_batches, t.batch_size
                                );
                                println!("  ID Fetch Elapsed: {:.4}s", t.id_fetch_elapsed_sec);
                                println!(
                                    "  Batch Fetch+Decode Elapsed: {:.4}s",
                                    t.batch_fetch_elapsed_sec
                                );
                                println!("  Total Elapsed: {:.4}s", t.total_elapsed_sec);
                                println!("  Records/Sec: {:.2}", t.records_per_sec);
                            }
                        } else {
                            for run in &report.runs {
                                println!(
                                    "
--- Run {} ---",
                                    run.run_index
                                );
                                println!("Total Elapsed: {:.4}s", run.total_elapsed_sec);
                                println!("Overall Records/Sec: {:.2}", run.overall_records_per_sec);
                                for t in &run.targets {
                                    println!(
                                        "
  Target: {}",
                                        t.record_type
                                    );
                                    println!(
                                        "  Records: {} ({} batches of {})",
                                        t.total_records, t.total_batches, t.batch_size
                                    );
                                    println!("  Total Elapsed: {:.4}s", t.total_elapsed_sec);
                                    println!("  Records/Sec: {:.2}", t.records_per_sec);
                                }
                            }

                            println!(
                                "
--- Aggregate Summary ({} runs) ---",
                                report.runs.len()
                            );
                            println!(
                                "Total Elapsed (sec)   - Min: {:.4}, Mean: {:.4}, Max: {:.4}",
                                report.aggregate.min_total_elapsed_sec,
                                report.aggregate.mean_total_elapsed_sec,
                                report.aggregate.max_total_elapsed_sec
                            );
                            println!(
                                "Overall Records/Sec   - Min: {:.2}, Mean: {:.2}, Max: {:.2}",
                                report.aggregate.min_overall_records_per_sec,
                                report.aggregate.mean_overall_records_per_sec,
                                report.aggregate.max_overall_records_per_sec
                            );

                            for t in &report.aggregate.targets {
                                println!(
                                    "
  Target: {}",
                                    t.record_type
                                );
                                println!(
                                    "    Total Elapsed (sec) - Min: {:.4}, Mean: {:.4}, Max: {:.4}",
                                    t.min_total_elapsed_sec,
                                    t.mean_total_elapsed_sec,
                                    t.max_total_elapsed_sec
                                );
                                println!(
                                    "    Records/Sec         - Min: {:.2}, Mean: {:.2}, Max: {:.2}",
                                    t.min_records_per_sec,
                                    t.mean_records_per_sec,
                                    t.max_records_per_sec
                                );
                            }
                        }
                    }
                }
                PerfTarget::Endpoints(args) => {
                    let report = run_perf_endpoints(
                        &client,
                        cli.url.clone(),
                        args.repo,
                        auth_elapsed_sec,
                        args.repeat.get(),
                    )
                    .await?;
                    if args.json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else {
                        println!("\n--- Performance Endpoints (Repo {}) ---", report.repo_id);
                        println!("Endpoint URL: {}", report.endpoint_url);
                        println!("Auth Elapsed: {:.4}s", report.auth_elapsed_sec);
                        println!(
                            "{:<22} | {:<50} | {:<7} | {:<7} | {:<7} | {:<8} | {:<8} | {:<8}",
                            "Label",
                            "Path",
                            "Repeats",
                            "Success",
                            "Error",
                            "Min(ms)",
                            "Mean(ms)",
                            "Max(ms)"
                        );
                        println!("{:-<110}", "");
                        for ep in &report.endpoints {
                            println!("{:<22} | {:<50} | {:<7} | {:<7} | {:<7} | {:<8.2} | {:<8.2} | {:<8.2}",
                            ep.endpoint_label, ep.endpoint_path, ep.repeat_count, ep.success_count, ep.error_count,
                            ep.min_elapsed_ms, ep.mean_elapsed_ms, ep.max_elapsed_ms);
                        }
                    }
                }
                PerfTarget::Consistency(args) => {
                    let report = run_perf_consistency(
                        &client,
                        cli.url.clone(),
                        args.repo,
                        args.solr_url.clone(),
                    )
                    .await?;
                    if args.json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else {
                        println!(
                            "\n--- Performance Consistency (Repo {}) ---",
                            report.repo_id
                        );
                        println!("Endpoint URL: {}", report.endpoint_url);
                        println!("Overall Status: {}", report.overall_status);
                        println!(
                            "Search Total Hits: {}",
                            report.search_total_hits.unwrap_or(0)
                        );
                        println!("Solr numFound: {}", report.solr_num_found.unwrap_or(0));
                        println!(
                            "{:<18} | {:<9} | {:<12} | {:<8}",
                            "Type", "API", "Search", "Status"
                        );
                        println!("{:-<58}", "");
                        for check in &report.checks {
                            let search_count = check
                                .search_count
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "missing".to_string());
                            println!(
                                "{:<18} | {:<9} | {:<12} | {:<8}",
                                check.record_type, check.api_count, search_count, check.status
                            );
                        }
                        for note in &report.notes {
                            println!("Note: {}", note);
                        }
                    }
                }
                PerfTarget::Health(args) => {
                    let report = run_perf_health(
                        &client,
                        cli.url.clone(),
                        args.repo,
                        auth_elapsed_sec,
                        args.solr_url.clone(),
                    )
                    .await?;
                    if args.json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else {
                        println!("\n--- Performance Health (Repo {}) ---", report.repo_id);
                        println!("Endpoint URL: {}", report.endpoint_url);
                        println!("Overall Success: {}", report.overall_success);
                        println!(
                            "Backend Ping: {} ({:.2}ms)",
                            report.ping.success,
                            report.ping.ping_elapsed_sec * 1000.0
                        );
                        println!(
                            "Search: {}/{} successful, mean {:.2}ms",
                            report.search.success_count,
                            report.search.repeat_count,
                            report.search.mean_elapsed_ms
                        );
                        println!(
                            "Record Types: {} total hits, {:.2}ms",
                            report.record_types.total_hits.unwrap_or(0),
                            report.record_types.elapsed_ms
                        );
                        for (record_type, count) in &report.record_types.counts_by_type {
                            println!("  {}: {}", record_type, count);
                        }
                        println!(
                            "Solr: {}/{} successful, mean {:.2}ms, numFound {}",
                            report.solr.success_count,
                            report.solr.repeat_count,
                            report.solr.mean_elapsed_ms,
                            report.solr.num_found.unwrap_or(0)
                        );
                    }
                }
                PerfTarget::RecordTypes(args) => {
                    let report = run_perf_record_types(&client, cli.url.clone(), args.repo).await?;
                    if args.json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else {
                        println!(
                            "\n--- Performance Record Types (Repo {}) ---",
                            report.repo_id
                        );
                        println!("Endpoint URL: {}", report.endpoint_url);
                        println!("Search Path: {}", report.search_path);
                        println!("Success: {}", report.success);
                        if let Some(total_hits) = report.total_hits {
                            println!("Total Hits: {}", total_hits);
                        }
                        println!("Elapsed Time: {:.2}ms", report.elapsed_ms);
                        println!("Counts By Type:");
                        for (record_type, count) in &report.counts_by_type {
                            println!("  {}: {}", record_type, count);
                        }
                    }
                }
                PerfTarget::SearchCheck(args) => {
                    let report = run_perf_search_check(
                        &client,
                        cli.url.clone(),
                        args.repo,
                        args.repeat.get(),
                    )
                    .await?;
                    if args.json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else {
                        println!(
                            "\n--- Performance Search Check (Repo {}) ---",
                            report.repo_id
                        );
                        println!("Endpoint URL: {}", report.endpoint_url);
                        println!("Search Path: {}", report.search_path);
                        println!("Repeats: {}", report.repeat_count);
                        println!("Success: {}", report.success_count);
                        println!("Error: {}", report.error_count);
                        if let Some(total_hits) = report.total_hits {
                            println!("Total Hits: {}", total_hits);
                        }
                        println!(
                            "Elapsed Time (ms) - Min: {:.2}, Mean: {:.2}, Max: {:.2}",
                            report.min_elapsed_ms, report.mean_elapsed_ms, report.max_elapsed_ms
                        );
                    }
                }
                PerfTarget::SolrCheck(args) => {
                    let report =
                        run_perf_solr_check(args.solr_url.clone(), args.repeat.get()).await?;
                    if args.json {
                        let json_out = serde_json::to_string_pretty(&report)?;
                        println!("{}", json_out);
                    } else {
                        println!("\n--- Performance Solr Check ---");
                        println!("Solr URL: {}", report.solr_url);
                        println!("Repeats: {}", report.repeat_count);
                        println!("Success: {}", report.success_count);
                        println!("Error: {}", report.error_count);
                        if let Some(nf) = report.num_found {
                            println!("numFound: {}", nf);
                        }
                        println!(
                            "Elapsed Time (ms) - Min: {:.2}, Mean: {:.2}, Max: {:.2}",
                            report.min_elapsed_ms, report.mean_elapsed_ms, report.max_elapsed_ms
                        );
                    }
                }
                PerfTarget::StaffAoProbe(args) => {
                    staff_ao_probe::run_staff_ao_probe(&client, args.clone()).await?;
                }
            }
        }
        Commands::StaffAoProbe(args) => {
            staff_ao_probe::run_staff_ao_probe(&client, args.clone()).await?;
        }
        Commands::StaffAoCandidate(args) => {
            staff_ao_candidate::run_staff_ao_candidate(&client, args.clone()).await?;
        }
        Commands::BuildIndexDocument(_) => {
            unreachable!("build-index-document returns before API login")
        }
        Commands::StaffAoBuilder(args) => {
            staff_ao_builder::run_staff_ao_builder(&client, args.clone()).await?;
        }
        Commands::Validate { target } => match target {
            ValidateTarget::Repositories => {
                println!("🔍 Fetching repositories...");
                let repos = client.repositories().await?;
                for repo in repos {
                    println!("- {} ({})", repo.name, repo.uri);
                }
            }
            ValidateTarget::Resources(args) => {
                validate_records(&client, "resource", args, &cli.url).await?;
            }
            ValidateTarget::ArchivalObjects(args) => {
                validate_records(&client, "archival_object", args, &cli.url).await?;
            }
            ValidateTarget::DigitalObjects(args) => {
                validate_records(&client, "digital_object", args, &cli.url).await?;
            }
        },
    }
    Ok(())
}

async fn validate_records(
    client: &Client,
    target_name: &str,
    args: &ValidateArgs,
    url: &str,
) -> Result<()> {
    let silent = args.json || args.csv;
    let display_name = target_name.replace("_", " ");
    let display_name_plural = match target_name {
        "resource" => "resources",
        "archival_object" => "archival objects",
        "digital_object" => "digital objects",
        _ => unreachable!(),
    };
    if !silent {
        println!(
            "🔍 Fetching {} IDs for Repository {}...",
            display_name, args.repo
        );
    }
    let start_time = std::time::Instant::now();

    let ids = match target_name {
        "resource" => client.resource_ids(args.repo).await?,
        "archival_object" => client.archival_object_ids(args.repo).await?,
        "digital_object" => client.digital_object_ids(args.repo).await?,
        _ => unreachable!(),
    };

    if !silent {
        println!(
            "Found {} {}. Validating in batches...",
            ids.len(),
            display_name_plural
        );
    }

    let batch_start_time = std::time::Instant::now();
    let batch_size = 50;
    let total_batches = ids.chunks(batch_size).len();

    let mut summary = ValidationSummary::default();
    let mut report_issues = Vec::new();

    for chunk in ids.chunks(batch_size) {
        match target_name {
            "resource" => {
                let records = client.resources_by_ids(args.repo, chunk).await?;
                for r in records {
                    let res = validate_resource(&r, &mut summary);
                    if !res.is_valid {
                        for (issue, severity) in res.issues {
                            report_issues.push(ValidationIssueRow {
                                repo_id: args.repo,
                                uri: r.uri.clone(),
                                title: r.title.clone(),
                                issue: issue.clone(),
                                severity: severity.clone(),
                                parent_resource: None,
                            });
                            if !silent {
                                println!("❌ [{:?}] {}", issue, r.uri);
                            }
                        }
                    }
                }
            }
            "archival_object" => {
                let records = client.archival_objects_by_ids(args.repo, chunk).await?;
                for ao in records {
                    let res = validate_archival_object(&ao, &mut summary);
                    if !res.is_valid {
                        let title = ao.title.clone().or_else(|| ao.display_string.clone());
                        let parent = ao.resource.as_ref().map(|r| r.ref_uri.clone());
                        for (issue, severity) in res.issues {
                            report_issues.push(ValidationIssueRow {
                                repo_id: args.repo,
                                uri: ao.uri.clone(),
                                title: title.clone(),
                                issue: issue.clone(),
                                severity: severity.clone(),
                                parent_resource: parent.clone(),
                            });
                            if !silent {
                                println!("❌ [{:?}] {}", issue, ao.uri);
                            }
                        }
                    }
                }
            }
            "digital_object" => {
                let records = client.digital_objects_by_ids(args.repo, chunk).await?;
                for do_obj in records {
                    let res = validate_digital_object(&do_obj, &mut summary);
                    if !res.is_valid {
                        for (issue, severity) in res.issues {
                            report_issues.push(ValidationIssueRow {
                                repo_id: args.repo,
                                uri: do_obj.uri.clone(),
                                title: do_obj.title.clone(),
                                issue: issue.clone(),
                                severity: severity.clone(),
                                parent_resource: None,
                            });
                            if !silent {
                                println!("❌ [{:?}] {}", issue, do_obj.uri);
                            }
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    let elapsed_time_sec = start_time.elapsed().as_secs_f64();
    let batch_elapsed_time_sec = batch_start_time.elapsed().as_secs_f64();
    let records_per_sec = if elapsed_time_sec > 0.0 {
        ids.len() as f64 / elapsed_time_sec
    } else {
        0.0
    };
    let output_mode = if args.json {
        "json"
    } else if args.csv {
        "csv"
    } else {
        "human"
    };

    let metrics = ValidationMetrics {
        endpoint_url: url.to_string(),
        record_type: target_name.to_string(),
        repo_id: args.repo,
        total_ids: ids.len(),
        total_batches,
        batch_size,
        elapsed_time_sec,
        batch_elapsed_time_sec,
        records_per_sec,
        output_mode: output_mode.to_string(),
    };

    output_report(
        args.repo,
        summary,
        metrics,
        report_issues,
        args.json,
        args.csv,
        &args.output,
    )?;
    Ok(())
}

async fn do_scan_run(
    client: &Client,
    repo_id: u32,
    run_index: usize,
    batch_size: usize,
    concurrency: usize,
) -> Result<(usize, PerfRepoScanRun)> {
    let mut targets = Vec::new();
    let mut total_records = 0;

    let overall_start = std::time::Instant::now();

    macro_rules! scan_target {
        ($record_type:expr, $id_call:expr, |$chunk:ident| $batch_call:expr) => {{
            let id_start = std::time::Instant::now();
            let ids = $id_call.await?;
            let id_fetch_elapsed = id_start.elapsed().as_secs_f64();
            let batch_start = std::time::Instant::now();
            let chunks: Vec<_> = ids.chunks(batch_size).collect();
            let total_batches = chunks.len();
            stream::iter(chunks)
                .map(|$chunk| async move { $batch_call.await })
                .buffer_unordered(concurrency)
                .try_collect::<Vec<_>>()
                .await?;
            let batch_fetch_elapsed = batch_start.elapsed().as_secs_f64();
            let total_target_elapsed = id_start.elapsed().as_secs_f64();
            let total = ids.len();
            let report = PerfScanTargetReport {
                record_type: $record_type.to_string(),
                total_records: total,
                total_batches,
                batch_size,
                id_fetch_elapsed_sec: id_fetch_elapsed,
                batch_fetch_elapsed_sec: batch_fetch_elapsed,
                total_elapsed_sec: total_target_elapsed,
                records_per_sec: if total_target_elapsed > 0.0 {
                    total as f64 / total_target_elapsed
                } else {
                    0.0
                },
            };
            (total, report)
        }};
    }

    // Resources
    let (res_total, res_report) =
        scan_target!("resource", client.resource_ids(repo_id), |chunk| client
            .resources_by_ids(repo_id, chunk));
    targets.push(res_report);
    total_records += res_total;

    // Archival Objects
    let (ao_total, ao_report) = scan_target!(
        "archival_object",
        client.archival_object_ids(repo_id),
        |chunk| client.archival_objects_by_ids(repo_id, chunk)
    );
    targets.push(ao_report);
    total_records += ao_total;

    // Digital Objects
    let (do_total, do_report) = scan_target!(
        "digital_object",
        client.digital_object_ids(repo_id),
        |chunk| client.digital_objects_by_ids(repo_id, chunk)
    );
    targets.push(do_report);
    total_records += do_total;

    let total_elapsed = overall_start.elapsed().as_secs_f64();

    Ok((
        total_records,
        PerfRepoScanRun {
            run_index,
            total_elapsed_sec: total_elapsed,
            overall_records_per_sec: if total_elapsed > 0.0 {
                total_records as f64 / total_elapsed
            } else {
                0.0
            },
            targets,
        },
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run_perf_repository_scan(
    client: &Client,
    endpoint_url: String,
    repo_id: u32,
    auth_elapsed_sec: f64,
    repeat: usize,
    warmup: bool,
    batch_size: usize,
    concurrency: usize,
) -> Result<PerfRepoScanReport> {
    if warmup {
        let _ = do_scan_run(client, repo_id, 0, batch_size, concurrency).await?;
    }

    let mut runs = Vec::new();
    let mut final_total_records = 0;

    for i in 1..=repeat {
        let (total_records, run) = do_scan_run(client, repo_id, i, batch_size, concurrency).await?;
        final_total_records = total_records;
        runs.push(run);
    }

    let min_total_elapsed_sec = runs
        .iter()
        .map(|r| r.total_elapsed_sec)
        .fold(f64::INFINITY, |a, b| a.min(b));
    let max_total_elapsed_sec = runs
        .iter()
        .map(|r| r.total_elapsed_sec)
        .fold(f64::NEG_INFINITY, |a, b| a.max(b));
    let mean_total_elapsed_sec =
        runs.iter().map(|r| r.total_elapsed_sec).sum::<f64>() / runs.len() as f64;

    let min_overall_records_per_sec = runs
        .iter()
        .map(|r| r.overall_records_per_sec)
        .fold(f64::INFINITY, |a, b| a.min(b));
    let max_overall_records_per_sec = runs
        .iter()
        .map(|r| r.overall_records_per_sec)
        .fold(f64::NEG_INFINITY, |a, b| a.max(b));
    let mean_overall_records_per_sec =
        runs.iter().map(|r| r.overall_records_per_sec).sum::<f64>() / runs.len() as f64;

    let mut agg_targets = Vec::new();
    if let Some(first_run) = runs.first() {
        for (idx, target) in first_run.targets.iter().enumerate() {
            let min_el = runs
                .iter()
                .map(|r| r.targets[idx].total_elapsed_sec)
                .fold(f64::INFINITY, |a, b| a.min(b));
            let max_el = runs
                .iter()
                .map(|r| r.targets[idx].total_elapsed_sec)
                .fold(f64::NEG_INFINITY, |a, b| a.max(b));
            let mean_el = runs
                .iter()
                .map(|r| r.targets[idx].total_elapsed_sec)
                .sum::<f64>()
                / runs.len() as f64;

            let min_rps = runs
                .iter()
                .map(|r| r.targets[idx].records_per_sec)
                .fold(f64::INFINITY, |a, b| a.min(b));
            let max_rps = runs
                .iter()
                .map(|r| r.targets[idx].records_per_sec)
                .fold(f64::NEG_INFINITY, |a, b| a.max(b));
            let mean_rps = runs
                .iter()
                .map(|r| r.targets[idx].records_per_sec)
                .sum::<f64>()
                / runs.len() as f64;

            agg_targets.push(PerfScanAggregateTarget {
                record_type: target.record_type.clone(),
                min_total_elapsed_sec: min_el,
                mean_total_elapsed_sec: mean_el,
                max_total_elapsed_sec: max_el,
                min_records_per_sec: min_rps,
                mean_records_per_sec: mean_rps,
                max_records_per_sec: max_rps,
            });
        }
    }

    let aggregate = PerfRepoScanAggregate {
        min_total_elapsed_sec,
        mean_total_elapsed_sec,
        max_total_elapsed_sec,
        min_overall_records_per_sec,
        mean_overall_records_per_sec,
        max_overall_records_per_sec,
        targets: agg_targets,
    };

    Ok(PerfRepoScanReport {
        endpoint_url,
        repo_id,
        auth_elapsed_sec,
        concurrency,
        total_records: final_total_records,
        warmup_used: warmup,
        runs,
        aggregate,
    })
}

async fn run_perf_ping(
    client: &Client,
    endpoint_url: String,
    auth_elapsed_sec: f64,
) -> PerfPingReport {
    let ping_start = std::time::Instant::now();
    let mut success = false;
    let mut summary = None;

    if let Ok(req) = client.get("version") {
        if let Ok(res) = req.send().await {
            success = res.status().is_success();
            if success {
                summary = res.text().await.ok();
            }
        }
    }
    let ping_elapsed_sec = ping_start.elapsed().as_secs_f64();

    PerfPingReport {
        endpoint_url,
        auth_elapsed_sec,
        ping_elapsed_sec,
        total_elapsed_sec: auth_elapsed_sec + ping_elapsed_sec,
        success,
        summary,
    }
}

fn summarize_endpoint_sample(
    endpoint_label: &str,
    endpoint_path: &str,
    elapsed_ms: &[f64],
    success_count: usize,
    error_count: usize,
) -> PerfEndpointSample {
    let min_elapsed_ms = elapsed_ms
        .iter()
        .copied()
        .fold(f64::INFINITY, |a, b| a.min(b));
    let max_elapsed_ms = elapsed_ms
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, |a, b| a.max(b));
    let mean_elapsed_ms = elapsed_ms.iter().sum::<f64>() / elapsed_ms.len() as f64;

    PerfEndpointSample {
        endpoint_label: endpoint_label.to_string(),
        endpoint_path: endpoint_path.to_string(),
        repeat_count: elapsed_ms.len(),
        success_count,
        error_count,
        min_elapsed_ms,
        mean_elapsed_ms,
        max_elapsed_ms,
    }
}

async fn measure_endpoint(
    client: &Client,
    endpoint_label: &str,
    endpoint_path: &str,
    repeat: usize,
) -> PerfEndpointSample {
    let mut elapsed_ms = Vec::with_capacity(repeat);
    let mut success_count = 0;
    let mut error_count = 0;

    for _ in 0..repeat {
        let start = std::time::Instant::now();
        let success = match client.get(endpoint_path) {
            Ok(req) => req
                .send()
                .await
                .map(|res| res.status().is_success())
                .unwrap_or(false),
            Err(_) => false,
        };
        elapsed_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        if success {
            success_count += 1;
        } else {
            error_count += 1;
        }
    }

    summarize_endpoint_sample(
        endpoint_label,
        endpoint_path,
        &elapsed_ms,
        success_count,
        error_count,
    )
}

async fn run_perf_endpoints(
    client: &Client,
    endpoint_url: String,
    repo_id: u32,
    auth_elapsed_sec: f64,
    repeat: usize,
) -> Result<PerfEndpointsReport> {
    let endpoints_to_measure = [
        ("version", "/version".to_string()),
        ("repositories", "/repositories".to_string()),
        (
            "resource_ids",
            format!("/repositories/{}/resources?all_ids=true", repo_id),
        ),
        (
            "archival_object_ids",
            format!("/repositories/{}/archival_objects?all_ids=true", repo_id),
        ),
        (
            "digital_object_ids",
            format!("/repositories/{}/digital_objects?all_ids=true", repo_id),
        ),
    ];

    let mut endpoints = Vec::with_capacity(endpoints_to_measure.len());
    for (endpoint_label, endpoint_path) in endpoints_to_measure {
        endpoints.push(measure_endpoint(client, endpoint_label, &endpoint_path, repeat).await);
    }

    Ok(PerfEndpointsReport {
        endpoint_url,
        repo_id,
        auth_elapsed_sec,
        endpoints,
    })
}

pub(crate) fn parse_primary_type_facets(json: &serde_json::Value) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    let Some(values) = json
        .pointer("/facets/facet_fields/primary_type")
        .and_then(|v| v.as_array())
    else {
        return counts;
    };

    for pair in values.chunks(2) {
        if let [record_type, count] = pair {
            if let (Some(record_type), Some(count)) = (record_type.as_str(), count.as_u64()) {
                counts.insert(record_type.to_string(), count);
            }
        }
    }

    counts
}

async fn run_perf_record_types(
    client: &Client,
    endpoint_url: String,
    repo_id: u32,
) -> Result<PerfRecordTypesReport> {
    let search_path =
        format!("repositories/{repo_id}/search?q[]=%2A&page=1&page_size=1&facet[]=primary_type");
    let start = std::time::Instant::now();
    let response = client.get(&search_path)?.send().await;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    match response {
        Ok(res) if res.status().is_success() => {
            let json = res.json::<serde_json::Value>().await?;
            let total_hits = json.get("total_hits").and_then(|v| v.as_u64());
            let counts_by_type = parse_primary_type_facets(&json);

            Ok(PerfRecordTypesReport {
                endpoint_url,
                repo_id,
                search_path,
                total_hits,
                counts_by_type,
                elapsed_ms,
                success: true,
            })
        }
        Ok(_) => Ok(PerfRecordTypesReport {
            endpoint_url,
            repo_id,
            search_path,
            total_hits: None,
            counts_by_type: BTreeMap::new(),
            elapsed_ms,
            success: false,
        }),
        Err(_) => Ok(PerfRecordTypesReport {
            endpoint_url,
            repo_id,
            search_path,
            total_hits: None,
            counts_by_type: BTreeMap::new(),
            elapsed_ms,
            success: false,
        }),
    }
}

async fn run_perf_health(
    client: &Client,
    endpoint_url: String,
    repo_id: u32,
    auth_elapsed_sec: f64,
    solr_url: String,
) -> Result<PerfHealthReport> {
    let ping = run_perf_ping(client, endpoint_url.clone(), auth_elapsed_sec).await;
    let record_types = run_perf_record_types(client, endpoint_url.clone(), repo_id).await?;
    let search = run_perf_search_check(client, endpoint_url.clone(), repo_id, 3).await?;
    let solr = run_perf_solr_check(solr_url, 3).await?;
    let overall_success = ping.success
        && record_types.success
        && search.error_count == 0
        && search.success_count == search.repeat_count
        && solr.error_count == 0
        && solr.success_count == solr.repeat_count;

    Ok(PerfHealthReport {
        endpoint_url,
        repo_id,
        overall_success,
        ping,
        record_types,
        search,
        solr,
    })
}

fn make_consistency_check(
    record_type: &str,
    api_count: usize,
    search_count: Option<u64>,
) -> PerfConsistencyCheck {
    let status = match search_count {
        Some(count) if count == api_count as u64 => "match",
        _ => "mismatch",
    };

    PerfConsistencyCheck {
        record_type: record_type.to_string(),
        api_count,
        search_count,
        status: status.to_string(),
    }
}

async fn run_perf_consistency(
    client: &Client,
    endpoint_url: String,
    repo_id: u32,
    solr_url: String,
) -> Result<PerfConsistencyReport> {
    let resource_ids = client.resource_ids(repo_id).await?;
    let archival_object_ids = client.archival_object_ids(repo_id).await?;
    let digital_object_ids = client.digital_object_ids(repo_id).await?;
    let record_types = run_perf_record_types(client, endpoint_url.clone(), repo_id).await?;
    let solr = run_perf_solr_check(solr_url, 3).await?;

    let checks = vec![
        make_consistency_check(
            "resource",
            resource_ids.len(),
            record_types.counts_by_type.get("resource").copied(),
        ),
        make_consistency_check(
            "archival_object",
            archival_object_ids.len(),
            record_types.counts_by_type.get("archival_object").copied(),
        ),
        make_consistency_check(
            "digital_object",
            digital_object_ids.len(),
            record_types.counts_by_type.get("digital_object").copied(),
        ),
    ];

    let overall_status = if checks.iter().all(|check| check.status == "match") {
        "match"
    } else {
        "mismatch"
    };

    let mut notes = Vec::new();
    notes.push(
        "Solr numFound is global to the configured collection and is not expected to equal repository-scoped API counts."
            .to_string(),
    );
    if checks.iter().any(|check| check.status == "mismatch") {
        notes.push(
            "Mismatches may indicate indexing lag, scope differences, or record types absent from the repository search facet."
                .to_string(),
        );
    }

    Ok(PerfConsistencyReport {
        endpoint_url,
        repo_id,
        overall_status: overall_status.to_string(),
        checks,
        search_total_hits: record_types.total_hits,
        solr_num_found: solr.num_found,
        notes,
    })
}

async fn run_perf_search_check(
    client: &Client,
    endpoint_url: String,
    repo_id: u32,
    repeat: usize,
) -> Result<PerfSearchCheckReport> {
    let search_path = format!("repositories/{repo_id}/search?q[]=%2A&page=1&page_size=10");
    let mut elapsed_ms = Vec::with_capacity(repeat);
    let mut success_count = 0;
    let mut error_count = 0;
    let mut total_hits = None;

    for _ in 0..repeat {
        let start = std::time::Instant::now();
        let response = client.get(&search_path)?.send().await;
        elapsed_ms.push(start.elapsed().as_secs_f64() * 1000.0);

        match response {
            Ok(res) if res.status().is_success() => {
                success_count += 1;
                if let Ok(json) = res.json::<serde_json::Value>().await {
                    total_hits = json.get("total_hits").and_then(|v| v.as_u64());
                }
            }
            Ok(_) | Err(_) => {
                error_count += 1;
            }
        }
    }

    let min_elapsed_ms = elapsed_ms
        .iter()
        .copied()
        .fold(f64::INFINITY, |a, b| a.min(b));
    let max_elapsed_ms = elapsed_ms
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, |a, b| a.max(b));
    let mean_elapsed_ms = elapsed_ms.iter().sum::<f64>() / elapsed_ms.len() as f64;

    Ok(PerfSearchCheckReport {
        endpoint_url,
        repo_id,
        search_path,
        repeat_count: repeat,
        success_count,
        error_count,
        min_elapsed_ms,
        mean_elapsed_ms,
        max_elapsed_ms,
        total_hits,
    })
}

fn output_report(
    repo_id: u32,
    summary: ValidationSummary,
    metrics: ValidationMetrics,
    report_issues: Vec<ValidationIssueRow>,
    json: bool,
    csv: bool,
    output: &Option<String>,
) -> Result<()> {
    if json {
        let report = ValidationReport {
            repo_id,
            summary,
            metrics,
            issues: report_issues,
        };
        let json_out = serde_json::to_string_pretty(&report)?;
        println!("{}", json_out);
    } else if csv {
        let mut wtr = match output {
            Some(path) => {
                let file = std::fs::File::create(path)?;
                csv::WriterBuilder::new()
                    .has_headers(false)
                    .from_writer(Box::new(file) as Box<dyn io::Write>)
            }
            None => csv::WriterBuilder::new()
                .has_headers(false)
                .from_writer(Box::new(io::stdout()) as Box<dyn io::Write>),
        };
        wtr.write_record([
            "repo_id",
            "uri",
            "title",
            "issue",
            "severity",
            "parent_resource",
        ])?;
        for issue in report_issues {
            wtr.serialize(issue)?;
        }
        wtr.flush()?;
    } else {
        println!("\n--- Validation Summary (Repo {}) ---", repo_id);
        println!("Total Records: {}", summary.total_records);
        println!("Valid Records: {}", summary.valid_records);
        println!("Missing Title: {}", summary.missing_title);
        println!("Missing Dates: {}", summary.missing_dates);
        println!("Missing Extents: {}", summary.missing_extents);
        if summary.missing_digital_object_id > 0 {
            println!("Missing DO ID: {}", summary.missing_digital_object_id);
        }
        if summary.missing_file_versions > 0 {
            println!("Missing File Versions: {}", summary.missing_file_versions);
        }
        println!("Unpublished (Info): {}", summary.unpublished);

        println!("\n--- Validation Metrics ---");
        println!("Endpoint URL: {}", metrics.endpoint_url);
        println!("Record Type: {}", metrics.record_type);
        println!("Total IDs: {}", metrics.total_ids);
        println!(
            "Batches: {} (Size: {})",
            metrics.total_batches, metrics.batch_size
        );
        println!(
            "Total Elapsed Time: {:.2}s (includes ID list fetching)",
            metrics.elapsed_time_sec
        );
        println!(
            "Batch Validation Time: {:.2}s",
            metrics.batch_elapsed_time_sec
        );
        println!("Records/Second (Overall): {:.2}", metrics.records_per_sec);
        println!("Output Mode: {}", metrics.output_mode);
    }
    Ok(())
}

async fn run_perf_solr_check(solr_url: String, repeat: usize) -> Result<PerfSolrCheckReport> {
    use reqwest::Client;
    use std::time::Instant;

    let client = Client::builder().build()?;
    let url = format!(
        "{}/select?q=*:*&rows=0&wt=json",
        solr_url.trim_end_matches('/')
    );

    let mut success_count = 0;
    let mut error_count = 0;
    let mut elapsed_times = Vec::new();
    let mut last_num_found = None;

    for _ in 0..repeat {
        let start = Instant::now();
        let res = client.get(&url).send().await;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        elapsed_times.push(elapsed);

        match res {
            Ok(resp) => {
                if resp.status().is_success() {
                    success_count += 1;
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(resp_obj) = json.get("response") {
                            if let Some(nf) = resp_obj.get("numFound") {
                                last_num_found = nf.as_u64();
                            }
                        }
                    }
                } else {
                    error_count += 1;
                }
            }
            Err(_) => {
                error_count += 1;
            }
        }
    }

    let min_elapsed_ms = elapsed_times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_elapsed_ms = elapsed_times
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let mean_elapsed_ms = if elapsed_times.is_empty() {
        0.0
    } else {
        elapsed_times.iter().sum::<f64>() / elapsed_times.len() as f64
    };

    Ok(PerfSolrCheckReport {
        solr_url,
        repeat_count: repeat,
        success_count,
        error_count,
        min_elapsed_ms: if min_elapsed_ms.is_infinite() {
            0.0
        } else {
            min_elapsed_ms
        },
        mean_elapsed_ms,
        max_elapsed_ms: if max_elapsed_ms.is_infinite() {
            0.0
        } else {
            max_elapsed_ms
        },
        num_found: last_num_found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    #[tokio::test]
    async fn test_run_perf_ping_success() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_version = server
            .mock("GET", "/version")
            .with_status(200)
            .with_body("ArchivesSpace 3.4.1")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_ping(&client, server.url(), 0.05).await;

        assert!(report.success);
        assert_eq!(report.summary.unwrap(), "ArchivesSpace 3.4.1");
        assert_eq!(report.endpoint_url, server.url());
        assert_eq!(report.auth_elapsed_sec, 0.05);
        assert!(report.ping_elapsed_sec >= 0.0);
        assert!(report.total_elapsed_sec >= 0.05);
    }

    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn staff_ao_probe_cli_parsing() {
        let cli =
            Cli::try_parse_from(["aspace", "staff-ao-probe", "--repo", "2", "--json"]).unwrap();
        match cli.command {
            Commands::StaffAoProbe(args) => {
                assert_eq!(args.repo, 2);
                assert_eq!(args.batch_size, 25);
                assert_eq!(args.concurrency, 4);
                assert!(args.json);
                assert!(args.validate().is_ok());
            }
            _ => panic!("Expected Commands::StaffAoProbe"),
        }

        let cli_alias = Cli::try_parse_from([
            "aspace",
            "staff-ao-hydration-probe",
            "--repo-id",
            "3",
            "--batch-size",
            "10",
            "--concurrency",
            "8",
        ])
        .unwrap();
        match cli_alias.command {
            Commands::StaffAoProbe(args) => {
                assert_eq!(args.repo, 3);
                assert_eq!(args.batch_size, 10);
                assert_eq!(args.concurrency, 8);
                assert!(!args.json);
                assert!(args.validate().is_ok());
            }
            _ => panic!("Expected Commands::StaffAoProbe"),
        }

        let cli_perf =
            Cli::try_parse_from(["aspace", "perf", "staff-ao-probe", "--repo", "2"]).unwrap();
        match cli_perf.command {
            Commands::Perf {
                target: PerfTarget::StaffAoProbe(args),
            } => {
                assert_eq!(args.repo, 2);
                assert_eq!(args.batch_size, 25);
                assert_eq!(args.concurrency, 4);
                assert!(args.validate().is_ok());
            }
            _ => panic!("Expected PerfTarget::StaffAoProbe"),
        }

        let cli_candidate = Cli::try_parse_from([
            "aspace",
            "staff-ao-candidate",
            "--repo",
            "4",
            "--batch-size",
            "25",
            "--concurrency",
            "4",
            "--solr-url",
            "http://127.0.0.1:18983/solr",
            "--candidate-core",
            "stage_staff_20261026_candidate",
            "--arks-enabled",
            "false",
            "--strip-markers",
            "true",
            "--json",
        ])
        .unwrap();
        match cli_candidate.command {
            Commands::StaffAoCandidate(args) => {
                assert_eq!(args.repo, 4);
                assert_eq!(args.batch_size, 25);
                assert_eq!(args.concurrency, 4);
                assert_eq!(args.solr_url, "http://127.0.0.1:18983/solr");
                assert_eq!(args.candidate_core, "stage_staff_20261026_candidate");
                assert!(!args.arks_enabled);
                assert!(args.strip_markers);
                assert!(args.json);
                assert_eq!(args.fail_after_batches, None);
                assert!(args.validate().is_ok());
            }
            _ => panic!("Expected Commands::StaffAoCandidate"),
        }

        let cli_candidate_fail = Cli::try_parse_from([
            "aspace",
            "staff-ao-candidate",
            "--repo",
            "4",
            "--solr-url",
            "http://127.0.0.1:18983/solr",
            "--candidate-core",
            "stage_staff_20261026_candidate",
            "--fail-after-batches",
            "5",
        ])
        .unwrap();
        match cli_candidate_fail.command {
            Commands::StaffAoCandidate(args) => {
                assert_eq!(args.fail_after_batches, Some(5));
                assert!(args.validate().is_ok());
            }
            _ => panic!("Expected Commands::StaffAoCandidate"),
        }

        let cli_builder = Cli::try_parse_from([
            "aspace",
            "staff-ao-builder",
            "--repo",
            "4",
            "--ids",
            "1221,2229",
            "--arks-enabled",
        ])
        .unwrap();
        match cli_builder.command {
            Commands::StaffAoBuilder(args) => {
                assert_eq!(args.repo, 4);
                assert_eq!(args.ids, vec![1221, 2229]);
                assert!(args.arks_enabled);
                assert!(args.strip_markers);
                assert!(args.validate().is_ok());
            }
            _ => panic!("Expected Commands::StaffAoBuilder"),
        }

        let cli_builder_write_output = Cli::try_parse_from([
            "aspace",
            "staff-ao-builder",
            "--repo",
            "4",
            "--ids",
            "1221,2229",
            "--solr-url",
            "http://127.0.0.1:18983/solr",
            "--candidate-core",
            "stage_staff_20261026_candidate",
            "--output",
            "/tmp/should_be_rejected.json",
        ])
        .unwrap();
        match cli_builder_write_output.command {
            Commands::StaffAoBuilder(args) => {
                let err = args.validate().unwrap_err();
                assert!(err.to_string().contains("--output"));
            }
            _ => panic!("Expected Commands::StaffAoBuilder"),
        }

        let cli_build_doc = Cli::try_parse_from([
            "aspace",
            "build-index-document",
            "--input",
            "/tmp/test_input.json",
            "--strip-markers",
        ])
        .unwrap();
        match cli_build_doc.command {
            Commands::BuildIndexDocument(args) => {
                assert_eq!(
                    args.input,
                    Some(std::path::PathBuf::from("/tmp/test_input.json"))
                );
                assert!(args.strip_markers);
            }
            _ => panic!("Expected Commands::BuildIndexDocument"),
        }
    }

    #[test]
    fn output_file_requires_csv() {
        assert!(Cli::try_parse_from([
            "aspace",
            "validate",
            "resources",
            "--repo",
            "2",
            "--json",
            "--output",
            "issues.csv",
        ])
        .is_err());

        assert!(Cli::try_parse_from([
            "aspace",
            "validate",
            "archival-objects",
            "--repo",
            "2",
            "--output",
            "issues.csv",
        ])
        .is_err());
    }

    #[test]
    fn repository_scan_output_requires_csv() {
        assert!(Cli::try_parse_from([
            "aspace",
            "perf",
            "repository-scan",
            "--repo",
            "2",
            "--output",
            "scan.csv",
        ])
        .is_err());
    }

    #[test]
    fn repository_scan_json_conflicts_csv() {
        assert!(Cli::try_parse_from([
            "aspace",
            "perf",
            "repository-scan",
            "--repo",
            "2",
            "--csv",
            "--json",
        ])
        .is_err());
    }

    #[test]
    fn repository_scan_repeat_must_be_positive() {
        assert!(Cli::try_parse_from([
            "aspace",
            "perf",
            "repository-scan",
            "--repo",
            "2",
            "--repeat",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn repository_scan_batch_size_must_be_positive() {
        assert!(Cli::try_parse_from([
            "aspace",
            "perf",
            "repository-scan",
            "--repo",
            "2",
            "--batch-size",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn repository_scan_concurrency_must_be_positive() {
        assert!(Cli::try_parse_from([
            "aspace",
            "perf",
            "repository-scan",
            "--repo",
            "2",
            "--concurrency",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn endpoint_repeat_must_be_positive() {
        assert!(Cli::try_parse_from([
            "aspace",
            "perf",
            "endpoints",
            "--repo",
            "2",
            "--repeat",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn search_check_repeat_must_be_positive() {
        assert!(Cli::try_parse_from([
            "aspace",
            "perf",
            "search-check",
            "--repo",
            "2",
            "--repeat",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn solr_check_repeat_must_be_positive() {
        assert!(Cli::try_parse_from(["aspace", "perf", "solr-check", "--repeat", "0"]).is_err());
    }

    #[test]
    fn shadow_query_descendants_requires_resource_uri() {
        assert!(Cli::try_parse_from(["aspace", "shadow-query", "descendants"]).is_err());
    }

    #[test]
    fn shadow_query_descendants_parses_options() {
        let parsed = Cli::try_parse_from([
            "aspace",
            "shadow-query",
            "descendants",
            "--resource-uri",
            "/repositories/2/resources/18",
            "--date-filter",
            "*1974*",
            "--title-filter",
            "*Example*",
            "--rows",
            "10",
            "--start",
            "20",
            "--json",
        ])
        .unwrap();

        match parsed.command {
            Commands::ShadowQuery {
                target: ShadowQueryTarget::Descendants(args),
            } => {
                assert_eq!(args.resource_uri, "/repositories/2/resources/18");
                assert_eq!(args.date_filter.as_deref(), Some("*1974*"));
                assert_eq!(args.title_filter.as_deref(), Some("*Example*"));
                assert_eq!(args.rows, 10);
                assert_eq!(args.start, 20);
                assert_eq!(args.solr_url, "http://localhost:8985/solr");
                assert_eq!(args.collection, "archivesspace_shadow");
                assert!(args.json);
            }
            _ => panic!("expected shadow-query descendants"),
        }
    }

    #[tokio::test]
    async fn shadow_query_descendants_refuses_live_collection_before_network() {
        let args = ShadowQueryDescendantsArgs {
            resource_uri: "/repositories/2/resources/18".to_string(),
            date_filter: None,
            title_filter: None,
            rows: 0,
            start: 0,
            solr_url: "http://nonexistent.invalid/solr".to_string(),
            collection: "archivesspace".to_string(),
            json: true,
            tree: false,
            include_ancestors: false,
        };

        let err = run_shadow_query_descendants(args).await.unwrap_err();

        assert_eq!(
            err.to_string(),
            "refusing to query live Solr collection 'archivesspace'"
        );
    }

    #[tokio::test]
    async fn shadow_query_descendants_refuses_tree_with_zero_rows() {
        let args = ShadowQueryDescendantsArgs {
            resource_uri: "/repositories/2/resources/18".to_string(),
            date_filter: None,
            title_filter: None,
            rows: 0,
            start: 0,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            json: false,
            tree: true,
            include_ancestors: false,
        };

        let err = run_shadow_query_descendants(args).await.unwrap_err();

        assert_eq!(
            err.to_string(),
            "cannot assemble tree with rows=0 (documents must be fetched)"
        );
    }

    #[test]
    fn shadow_query_descendants_accepts_include_ancestors_with_tree() {
        let parsed = Cli::try_parse_from([
            "aspace",
            "shadow-query",
            "descendants",
            "--resource-uri",
            "/repositories/2/resources/18",
            "--tree",
            "--include-ancestors",
        ])
        .unwrap();

        match parsed.command {
            Commands::ShadowQuery {
                target: ShadowQueryTarget::Descendants(args),
            } => {
                assert!(args.tree);
                assert!(args.include_ancestors);
            }
            _ => panic!("expected shadow-query descendants"),
        }
    }

    #[tokio::test]
    async fn shadow_query_descendants_rejects_include_ancestors_without_tree() {
        let args = ShadowQueryDescendantsArgs {
            resource_uri: "/repositories/2/resources/18".to_string(),
            date_filter: None,
            title_filter: None,
            rows: 50,
            start: 0,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            json: false,
            tree: false,
            include_ancestors: true,
        };

        let err = run_shadow_query_descendants(args).await.unwrap_err();
        assert_eq!(err.to_string(), "--include-ancestors requires --tree");
    }

    #[test]
    fn shadow_query_search_parses_options() {
        let parsed = Cli::try_parse_from([
            "aspace",
            "shadow-query",
            "search",
            "--q",
            "Example",
            "--resource-uri",
            "/repositories/2/resources/18",
            "--date-filter",
            "*1974*",
            "--title-filter",
            "*letter*",
            "--rows",
            "25",
            "--start",
            "10",
            "--solr-url",
            "http://127.0.0.1:8985/solr",
            "--collection",
            "archivesspace_shadow",
            "--json",
        ])
        .unwrap();

        match parsed.command {
            Commands::ShadowQuery {
                target: ShadowQueryTarget::Search(args),
            } => {
                assert_eq!(args.q.as_deref(), Some("Example"));
                assert_eq!(
                    args.resource_uri.as_deref(),
                    Some("/repositories/2/resources/18")
                );
                assert_eq!(args.date_filter.as_deref(), Some("*1974*"));
                assert_eq!(args.title_filter.as_deref(), Some("*letter*"));
                assert_eq!(args.rows, 25);
                assert_eq!(args.start, 10);
                assert_eq!(args.solr_url, "http://127.0.0.1:8985/solr");
                assert_eq!(args.collection, "archivesspace_shadow");
                assert!(args.json);
                assert!(!args.tree);
                assert!(!args.include_ancestors);
            }
            _ => panic!("expected shadow-query search"),
        }
    }

    #[tokio::test]
    async fn shadow_query_search_rejects_include_ancestors_without_tree() {
        let args = ShadowQuerySearchArgs {
            q: Some("test".to_string()),
            resource_uri: Some("/repositories/2/resources/18".to_string()),
            date_filter: None,
            title_filter: None,
            rows: 50,
            start: 0,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            json: false,
            tree: false,
            include_ancestors: true,
        };

        let err = run_shadow_query_search(args).await.unwrap_err();
        assert_eq!(err.to_string(), "--include-ancestors requires --tree");
    }

    #[tokio::test]
    async fn shadow_query_search_rejects_tree_without_resource_uri() {
        let args = ShadowQuerySearchArgs {
            q: Some("test".to_string()),
            resource_uri: None,
            date_filter: None,
            title_filter: None,
            rows: 50,
            start: 0,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            json: false,
            tree: true,
            include_ancestors: false,
        };

        let err = run_shadow_query_search(args).await.unwrap_err();
        assert_eq!(err.to_string(), "--tree requires --resource-uri");
    }

    #[tokio::test]
    async fn shadow_query_search_rejects_tree_with_zero_rows() {
        let args = ShadowQuerySearchArgs {
            q: Some("test".to_string()),
            resource_uri: Some("/repositories/2/resources/18".to_string()),
            date_filter: None,
            title_filter: None,
            rows: 0,
            start: 0,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            json: false,
            tree: true,
            include_ancestors: false,
        };

        let err = run_shadow_query_search(args).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot assemble tree with rows=0 (documents must be fetched)"
        );
    }

    #[tokio::test]
    async fn shadow_query_search_refuses_live_collection_before_network() {
        let args = ShadowQuerySearchArgs {
            q: Some("test".to_string()),
            resource_uri: None,
            date_filter: None,
            title_filter: None,
            rows: 50,
            start: 0,
            solr_url: "http://nonexistent.invalid/solr".to_string(),
            collection: "archivesspace".to_string(),
            json: false,
            tree: false,
            include_ancestors: false,
        };

        let err = run_shadow_query_search(args).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "refusing to query live Solr collection 'archivesspace'"
        );
    }

    #[tokio::test]
    async fn shadow_query_search_mocked_success() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "(title:Example OR display_string:Example OR date_expressions:Example)".into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "50".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 1,
                        "start": 0,
                        "docs": [
                            {
                                "id": "/repositories/2/archival_objects/4031",
                                "title": "Example correspondence",
                                "resource_uri": "/repositories/2/resources/18"
                            }
                        ]
                    }
                }"#,
            )
            .create_async()
            .await;

        let args = ShadowQuerySearchArgs {
            q: Some("Example".to_string()),
            resource_uri: None,
            date_filter: None,
            title_filter: None,
            rows: 50,
            start: 0,
            solr_url: server.url(),
            collection: "archivesspace_shadow".to_string(),
            json: true,
            tree: false,
            include_ancestors: false,
        };

        let res = run_shadow_query_search(args).await;
        mock.assert_async().await;
        assert!(res.is_ok());
    }

    #[test]
    fn summarize_endpoint_sample_calculates_latency_stats() {
        let sample = summarize_endpoint_sample("version", "/version", &[10.0, 25.0, 15.0], 2, 1);

        assert_eq!(sample.endpoint_label, "version");
        assert_eq!(sample.endpoint_path, "/version");
        assert_eq!(sample.repeat_count, 3);
        assert_eq!(sample.success_count, 2);
        assert_eq!(sample.error_count, 1);
        assert_eq!(sample.min_elapsed_ms, 10.0);
        assert_eq!(sample.mean_elapsed_ms, 50.0 / 3.0);
        assert_eq!(sample.max_elapsed_ms, 25.0);
    }

    #[test]
    pub(crate) fn parse_primary_type_facets_reads_alternating_type_count_pairs() {
        let json = serde_json::json!({
            "facets": {
                "facet_fields": {
                    "primary_type": [
                        "archival_object", 5255,
                        "resource", 14,
                        "accession", 4
                    ]
                }
            }
        });

        let counts = parse_primary_type_facets(&json);

        assert_eq!(counts.get("archival_object"), Some(&5255));
        assert_eq!(counts.get("resource"), Some(&14));
        assert_eq!(counts.get("accession"), Some(&4));
    }

    #[tokio::test]
    async fn test_run_perf_ping_failure() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_version = server
            .mock("GET", "/version")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_ping(&client, server.url(), 0.1).await;

        assert!(!report.success);
        assert!(report.summary.is_none());
        assert_eq!(report.auth_elapsed_sec, 0.1);
    }

    #[tokio::test]
    async fn test_run_perf_repository_scan() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_res_ids = server
            .mock("GET", "/repositories/2/resources?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[1, 2]"#)
            .create_async()
            .await;

        let _mock_res_batch = server
            .mock("GET", "/repositories/2/resources?id_set=1,2")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .create_async()
            .await;

        let _mock_ao_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .create_async()
            .await;

        let _mock_do_ids = server
            .mock("GET", "/repositories/2/digital_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_repository_scan(&client, server.url(), 2, 0.1, 1, false, 50, 4)
            .await
            .unwrap();

        assert_eq!(report.repo_id, 2);
        assert_eq!(report.concurrency, 4);
        assert_eq!(report.total_records, 2);
        assert_eq!(report.runs[0].targets.len(), 3);
        assert_eq!(report.runs[0].targets[0].record_type, "resource");
        assert_eq!(report.runs[0].targets[0].total_records, 2);
        assert_eq!(report.runs[0].targets[0].batch_size, 50);
    }

    #[tokio::test]
    async fn test_run_perf_endpoints() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_version = server
            .mock("GET", "/version")
            .with_status(200)
            .with_body("ArchivesSpace 3.4.1")
            .expect(2)
            .create_async()
            .await;

        let _mock_repositories = server
            .mock("GET", "/repositories")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .expect(2)
            .create_async()
            .await;

        let _mock_res_ids = server
            .mock("GET", "/repositories/2/resources?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .expect(2)
            .create_async()
            .await;

        let _mock_ao_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .expect(2)
            .create_async()
            .await;

        let _mock_do_ids = server
            .mock("GET", "/repositories/2/digital_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .expect(2)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_endpoints(&client, server.url(), 2, 0.1, 2)
            .await
            .unwrap();

        assert_eq!(report.endpoint_url, server.url());
        assert_eq!(report.repo_id, 2);
        assert_eq!(report.auth_elapsed_sec, 0.1);
        assert_eq!(report.endpoints.len(), 5);
        assert_eq!(report.endpoints[0].endpoint_label, "version");
        assert_eq!(report.endpoints[0].endpoint_path, "/version");
        assert_eq!(report.endpoints[0].repeat_count, 2);
        assert_eq!(report.endpoints[0].success_count, 2);

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["endpoints"][0]["endpoint_label"], "version");
        assert_eq!(json["endpoints"][0]["endpoint_path"], "/version");
        assert_eq!(json["endpoints"][0]["repeat_count"], 2);
    }

    #[tokio::test]
    async fn test_run_perf_search_check_success() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_search = server
            .mock("GET", "/repositories/2/search")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("q[]".into(), "*".into()),
                Matcher::UrlEncoded("page".into(), "1".into()),
                Matcher::UrlEncoded("page_size".into(), "10".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"total_hits": 42, "results": []}"#)
            .expect(2)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_search_check(&client, server.url(), 2, 2)
            .await
            .unwrap();

        assert_eq!(report.endpoint_url, server.url());
        assert_eq!(report.repo_id, 2);
        assert_eq!(
            report.search_path,
            "repositories/2/search?q[]=%2A&page=1&page_size=10"
        );
        assert_eq!(report.repeat_count, 2);
        assert_eq!(report.success_count, 2);
        assert_eq!(report.error_count, 0);
        assert_eq!(report.total_hits, Some(42));
        assert!(report.min_elapsed_ms >= 0.0);
        assert!(report.max_elapsed_ms >= report.min_elapsed_ms);
    }

    #[tokio::test]
    async fn test_run_perf_record_types_success() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_search = server
            .mock("GET", "/repositories/2/search")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("q[]".into(), "*".into()),
                Matcher::UrlEncoded("page".into(), "1".into()),
                Matcher::UrlEncoded("page_size".into(), "1".into()),
                Matcher::UrlEncoded("facet[]".into(), "primary_type".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "total_hits": 5274,
                    "facets": {
                        "facet_fields": {
                            "primary_type": [
                                "archival_object", 5255,
                                "resource", 14,
                                "accession", 4
                            ]
                        }
                    }
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_record_types(&client, server.url(), 2)
            .await
            .unwrap();

        assert!(report.success);
        assert_eq!(report.endpoint_url, server.url());
        assert_eq!(report.repo_id, 2);
        assert_eq!(
            report.search_path,
            "repositories/2/search?q[]=%2A&page=1&page_size=1&facet[]=primary_type"
        );
        assert_eq!(report.total_hits, Some(5274));
        assert_eq!(report.counts_by_type.get("archival_object"), Some(&5255));
        assert_eq!(report.counts_by_type.get("resource"), Some(&14));
        assert_eq!(report.counts_by_type.get("accession"), Some(&4));
        assert!(report.elapsed_ms >= 0.0);
    }

    #[tokio::test]
    async fn test_run_perf_health_success() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_version = server
            .mock("GET", "/version")
            .with_status(200)
            .with_body("ArchivesSpace 4.2.0")
            .expect(1)
            .create_async()
            .await;

        let _mock_record_types = server
            .mock("GET", "/repositories/2/search")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("q[]".into(), "*".into()),
                Matcher::UrlEncoded("page".into(), "1".into()),
                Matcher::UrlEncoded("page_size".into(), "1".into()),
                Matcher::UrlEncoded("facet[]".into(), "primary_type".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "total_hits": 3,
                    "facets": {
                        "facet_fields": {
                            "primary_type": [
                                "resource", 1,
                                "archival_object", 2
                            ]
                        }
                    }
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        let _mock_search = server
            .mock("GET", "/repositories/2/search")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("q[]".into(), "*".into()),
                Matcher::UrlEncoded("page".into(), "1".into()),
                Matcher::UrlEncoded("page_size".into(), "10".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"total_hits": 3, "results": []}"#)
            .expect(3)
            .create_async()
            .await;

        let _mock_solr = server
            .mock("GET", "/select?q=*:*&rows=0&wt=json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"response": {"numFound": 3}}"#)
            .expect(3)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_health(&client, server.url(), 2, 0.1, server.url())
            .await
            .unwrap();

        assert!(report.overall_success);
        assert!(report.ping.success);
        assert!(report.record_types.success);
        assert_eq!(report.record_types.total_hits, Some(3));
        assert_eq!(report.search.success_count, 3);
        assert_eq!(report.solr.success_count, 3);
    }

    #[test]
    fn make_consistency_check_marks_match_and_mismatch() {
        let matched = make_consistency_check("resource", 14, Some(14));
        assert_eq!(matched.status, "match");

        let mismatched = make_consistency_check("digital_object", 15, None);
        assert_eq!(mismatched.status, "mismatch");
    }

    #[tokio::test]
    async fn test_run_perf_consistency_reports_mismatch() {
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_resource_ids = server
            .mock("GET", "/repositories/2/resources?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[1]"#)
            .expect(1)
            .create_async()
            .await;

        let _mock_ao_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[1,2]"#)
            .expect(1)
            .create_async()
            .await;

        let _mock_do_ids = server
            .mock("GET", "/repositories/2/digital_objects?all_ids=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[1]"#)
            .expect(1)
            .create_async()
            .await;

        let _mock_record_types = server
            .mock("GET", "/repositories/2/search")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("q[]".into(), "*".into()),
                Matcher::UrlEncoded("page".into(), "1".into()),
                Matcher::UrlEncoded("page_size".into(), "1".into()),
                Matcher::UrlEncoded("facet[]".into(), "primary_type".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "total_hits": 3,
                    "facets": {
                        "facet_fields": {
                            "primary_type": [
                                "resource", 1,
                                "archival_object", 2
                            ]
                        }
                    }
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        let _mock_solr = server
            .mock("GET", "/select?q=*:*&rows=0&wt=json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"response": {"numFound": 4}}"#)
            .expect(3)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let report = run_perf_consistency(&client, server.url(), 2, server.url())
            .await
            .unwrap();

        assert_eq!(report.overall_status, "mismatch");
        assert_eq!(report.search_total_hits, Some(3));
        assert_eq!(report.solr_num_found, Some(4));
        assert_eq!(report.checks.len(), 3);
        assert_eq!(report.checks[0].status, "match");
        assert_eq!(report.checks[1].status, "match");
        assert_eq!(report.checks[2].status, "mismatch");
    }

    #[tokio::test]
    async fn test_run_perf_solr_check_success() {
        let mut server = Server::new_async().await;

        let _mock_solr = server
            .mock("GET", "/select?q=*:*&rows=0&wt=json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"response": {"numFound": 12345}}"#)
            .expect(3)
            .create_async()
            .await;

        let report = run_perf_solr_check(server.url(), 3).await.unwrap();

        assert_eq!(report.repeat_count, 3);
        assert_eq!(report.success_count, 3);
        assert_eq!(report.error_count, 0);
        assert_eq!(report.num_found, Some(12345));
        assert!(report.min_elapsed_ms >= 0.0);
        assert!(report.max_elapsed_ms >= report.min_elapsed_ms);
        assert!(
            report.mean_elapsed_ms >= report.min_elapsed_ms
                && report.mean_elapsed_ms <= report.max_elapsed_ms
        );
    }
}

#[cfg(test)]
mod csv_tests {
    use super::*;

    #[test]
    fn test_csv_serialization_schema() {
        let row = PerfRepoScanCsvRow {
            repo_id: 2,
            run_index: 1,
            record_type: "resource".to_string(),
            total_records: 100,
            total_batches: 2,
            batch_size: 50,
            concurrency: 1,
            warmup_used: false,
            elapsed_time_sec: 1.5,
            records_per_sec: 66.67,
            success: true,
        };

        let mut wtr = csv::WriterBuilder::new()
            .has_headers(true)
            .from_writer(vec![]);
        wtr.serialize(row).unwrap();
        let bytes = wtr.into_inner().unwrap();
        let csv_string = String::from_utf8(bytes).unwrap();

        let lines: Vec<&str> = csv_string.lines().collect();
        assert_eq!(lines[0], "repo_id,run_index,record_type,total_records,total_batches,batch_size,concurrency,warmup_used,elapsed_time_sec,records_per_sec,success");
        assert_eq!(lines[1], "2,1,resource,100,2,50,1,false,1.5,66.67,true");
    }
}
