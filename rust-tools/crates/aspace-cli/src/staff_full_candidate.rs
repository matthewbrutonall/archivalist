use anyhow::{bail, Context, Result};
use aspace_client::index_document::COLLECTION_MANAGEMENT_DOCUMENTS_KEY;
use aspace_client::isolated_solr_writer::{
    validate_isolated_solr_base_url, validate_stage_candidate_core, IsolatedSolrStaffAoWriter,
};
use aspace_client::{
    build_index_document_batch, Client, IndexDocumentBatchRequest, IndexDocumentInput,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::staff_ao_candidate::fetch_and_build_ao_candidate_batch;
use crate::staff_candidate_runner::{partition_ids_into_batches, CandidateBatchOutput};
use crate::staff_combined_candidate::{
    fetch_and_build_digital_object_candidate_batch, fetch_and_build_top_container_candidate_batch,
    process_type_candidate_batches, SharedCandidateState, TypeSliceSummary,
};
use crate::staff_resource_candidate::fetch_and_build_resource_candidate_batch;

#[derive(clap::Args, Clone, Debug)]
pub struct StaffFullCandidateArgs {
    #[arg(long)]
    pub solr_url: String,
    #[arg(long)]
    pub candidate_core: String,
    #[arg(long)]
    pub state: std::path::PathBuf,
    #[arg(long, default_value_t = 25)]
    pub batch_size: usize,
    #[arg(long, default_value_t = 4)]
    pub concurrency: usize,
    #[arg(long, default_value_t = false, num_args = 0..=1, default_missing_value = "true", action = clap::ArgAction::Set)]
    pub arks_enabled: bool,
    #[arg(long, default_value_t = true, num_args = 0..=1, default_missing_value = "true", action = clap::ArgAction::Set)]
    pub strip_markers: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub job_locale_file: Option<PathBuf>,
    #[arg(long)]
    pub job_time_zone: Option<String>,
    #[arg(long, hide = true)]
    pub fail_after_batches: Option<usize>,
}

impl StaffFullCandidateArgs {
    fn validate(&self) -> Result<()> {
        validate_isolated_solr_base_url(&self.solr_url)?;
        validate_stage_candidate_core(&self.candidate_core, None)?;
        if self.state.as_os_str().is_empty() {
            bail!("missing profiling-stack state file");
        }
        if !self.strip_markers {
            bail!("--strip-markers false is forbidden in Solr write mode");
        }
        if !(1..=25).contains(&self.batch_size) || !(1..=32).contains(&self.concurrency) {
            bail!("batch size must be 1..=25 and concurrency must be 1..=32");
        }
        if self.fail_after_batches == Some(0) {
            bail!("fail_after_batches must be positive");
        }
        if self.job_time_zone.as_deref() == Some("") {
            bail!("job_time_zone must not be empty");
        }
        if let Some(path) = &self.job_locale_file {
            if !path.is_file() {
                bail!("job locale file is missing or not a regular file");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct TypeSpec {
    record_type: &'static str,
    endpoint: &'static str,
    global_path: bool,
    first_repository_only: bool,
}

const TYPES: &[TypeSpec] = &[
    TypeSpec {
        record_type: "resource",
        endpoint: "resources",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "digital_object",
        endpoint: "digital_objects",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "accession",
        endpoint: "accessions",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "agent_person",
        endpoint: "agents/people",
        global_path: true,
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "agent_software",
        endpoint: "agents/software",
        global_path: true,
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "agent_family",
        endpoint: "agents/families",
        global_path: true,
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "agent_corporate_entity",
        endpoint: "agents/corporate_entities",
        global_path: true,
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "subject",
        endpoint: "subjects",
        global_path: true,
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "location",
        endpoint: "locations",
        global_path: true,
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "event",
        endpoint: "events",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "top_container",
        endpoint: "top_containers",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "classification",
        endpoint: "classifications",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "container_profile",
        endpoint: "container_profiles",
        global_path: true,
        // The API path is global: a full rebuild needs one final document per URI.
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "location_profile",
        endpoint: "location_profiles",
        global_path: true,
        first_repository_only: true,
    },
    TypeSpec {
        record_type: "archival_object",
        endpoint: "archival_objects",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "digital_object_component",
        endpoint: "digital_object_components",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "classification_term",
        endpoint: "classification_terms",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "assessment",
        endpoint: "assessments",
        global_path: false,
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "job",
        endpoint: "jobs",
        global_path: false,
        first_repository_only: false,
    },
];

impl TypeSpec {
    fn path(self, repo_id: u32) -> String {
        if self.global_path {
            self.endpoint.to_string()
        } else {
            format!("repositories/{repo_id}/{}", self.endpoint)
        }
    }
}

#[derive(Serialize)]
pub struct FullStageSummary {
    pub repository_id: u32,
    pub wall_elapsed_sec: f64,
    #[serde(flatten)]
    pub slice: TypeSliceSummary,
}

#[derive(Serialize)]
pub struct StaffFullCandidateSummary {
    pub full_reindex_started_at_epoch_sec: u64,
    pub repository_ids: Vec<u32>,
    pub stages: Vec<FullStageSummary>,
    pub total_ids: usize,
    pub total_documents: usize,
    pub unique_uris: usize,
    pub batches_completed: usize,
    pub solr_update_requests_sent: usize,
    pub wall_elapsed_sec: f64,
    pub committed: bool,
    pub candidate_core: String,
}

fn repo_id_from_record(record: &Value) -> Result<u32> {
    let uri = record
        .get("uri")
        .and_then(Value::as_str)
        .context("repository missing URI")?;
    let id = uri
        .strip_prefix("/repositories/")
        .context("invalid repository URI")?;
    Ok(id.parse()?)
}

fn append_native_pui_types(doc: &mut Value, record_type: &str) -> Result<()> {
    let obj = doc
        .as_object_mut()
        .context("built document is not an object")?;
    if obj.get("publish").is_none_or(|v| v.is_null() || v == false) {
        return Ok(());
    }
    let types = obj
        .get_mut("types")
        .and_then(Value::as_array_mut)
        .context("built document missing types array")?;
    let mut add = |kind: String| types.push(Value::String(kind));
    if matches!(
        record_type,
        "accession" | "digital_object" | "digital_object_component"
    ) {
        add("pui_record".to_string());
    }
    if matches!(record_type, "agent_person" | "agent_corporate_entity") {
        add("pui_agent".to_string());
    }
    if record_type != "archival_object" {
        let kind = match record_type {
            "resource" => "pui_collection".to_string(),
            "classification" => "pui_record_group".to_string(),
            "agent_person" => "pui_person".to_string(),
            other => format!("pui_{other}"),
        };
        add(kind);
        add("pui".to_string());
    }
    Ok(())
}

fn sanitize_native_index_record(record: &mut Value, record_type: &str) -> Result<()> {
    if matches!(
        record_type,
        "agent_person" | "agent_family" | "agent_software" | "agent_corporate_entity"
    ) {
        let obj = record
            .as_object_mut()
            .context("agent record is not an object")?;
        obj.insert("agent_contacts".to_string(), Value::Array(Vec::new()));
    }
    if record_type == "job" {
        let obj = record
            .as_object_mut()
            .context("job record is not an object")?;
        obj.entry("has_modified_records".to_string())
            .or_insert(Value::Bool(false));
    }
    Ok(())
}

fn clean_document_whitespace(value: &mut Value) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim_matches(|c: char| c.is_ascii_whitespace());
            if trimmed.len() != text.len() {
                *text = trimmed.to_string();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(clean_document_whitespace),
        Value::Object(fields) => fields.values_mut().for_each(clean_document_whitespace),
        _ => {}
    }
}

fn parse_accession_years(records: &[Value]) -> Result<Vec<i32>> {
    let dates = records
        .iter()
        .map(|record| {
            record
                .get("accession_date")
                .and_then(Value::as_str)
                .context("accession missing string accession_date")
        })
        .collect::<Result<Vec<_>>>()?;
    let mut child = Command::new("ruby")
        .args([
            "-rdate",
            "-rjson",
            "-e",
            "STDOUT.write(JSON.generate(JSON.parse(STDIN.read).map { |date| Date.parse(date).year }))",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot launch Ruby Date.parse for accession year parity")?;
    child
        .stdin
        .take()
        .context("Ruby date parser stdin unavailable")?
        .write_all(&serde_json::to_vec(&dates)?)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("Ruby Date.parse rejected an accession date");
    }
    let years: Vec<i32> = serde_json::from_slice(&output.stdout)
        .context("invalid accession year response from Ruby Date.parse")?;
    if years.len() != records.len() {
        bail!("Ruby Date.parse returned the wrong number of accession years");
    }
    Ok(years)
}

fn extract_generic_documents(
    documents: Vec<Value>,
    spec: TypeSpec,
    repo_id: u32,
) -> Result<CandidateBatchOutput> {
    let path = spec.path(repo_id);
    let expected_prefix = format!("/{path}/");
    let mut ordered_documents = Vec::new();
    let mut parent_uris = Vec::new();
    let mut child_uris = Vec::new();
    for mut parent in documents {
        let obj = parent
            .as_object_mut()
            .context("built document is not an object")?;
        let uri = obj
            .get("uri")
            .and_then(Value::as_str)
            .context("built document missing URI")?
            .to_string();
        if !uri.starts_with(&expected_prefix)
            || uri[expected_prefix.len()..].parse::<u32>().is_err()
        {
            bail!(
                "built document has wrong URI for {}: {uri}",
                spec.record_type
            );
        }
        if obj.get("id").and_then(Value::as_str) != Some(uri.as_str()) {
            bail!("built document has mismatched id: {uri}");
        }
        if obj.get("primary_type").and_then(Value::as_str) != Some(spec.record_type) {
            bail!("built document has mismatched primary_type: {uri}");
        }
        let children = obj.remove(COLLECTION_MANAGEMENT_DOCUMENTS_KEY);
        obj.retain(|key, _| !key.starts_with("_archivalist_"));
        append_native_pui_types(&mut parent, spec.record_type)?;
        parent_uris.push(Some(uri.clone()));
        ordered_documents.push(parent);
        if let Some(children) = children {
            let items = children
                .as_array()
                .context("malformed collection management child envelope")?;
            if spec.record_type != "accession" && !items.is_empty() {
                bail!("unhandled child documents for {}", spec.record_type);
            }
            for mut child in items.iter().cloned() {
                let child_obj = child.as_object_mut().context("child is not an object")?;
                let child_uri = child_obj
                    .get("uri")
                    .and_then(Value::as_str)
                    .context("child missing URI")?
                    .to_string();
                let child_prefix = format!("/repositories/{repo_id}/collection_management/");
                if !child_uri.starts_with(&child_prefix)
                    || child_uri[child_prefix.len()..].parse::<u32>().is_err()
                    || child_obj.get("id").and_then(Value::as_str) != Some(child_uri.as_str())
                    || child_obj.get("parent_id").and_then(Value::as_str) != Some(uri.as_str())
                    || child_obj.get("parent_type").and_then(Value::as_str) != Some("accession")
                    || child_obj.get("primary_type").and_then(Value::as_str)
                        != Some("collection_management")
                {
                    bail!("invalid accession child identity");
                }
                child_obj.retain(|key, _| !key.starts_with("_archivalist_"));
                child_uris.push(child_uri);
                ordered_documents.push(child);
            }
        }
    }
    Ok(CandidateBatchOutput {
        parent_count: parent_uris.len(),
        child_count: child_uris.len(),
        ordered_documents,
        parent_uris,
        child_uris,
    })
}

#[derive(Clone)]
struct FullBatchContext {
    repo_id: u32,
    batch_size: usize,
    dynamic_enums: Arc<Vec<String>>,
    arks_enabled: bool,
    job_locale_file: Option<PathBuf>,
    job_time_zone: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobProjection {
    title: String,
    files: Vec<String>,
    job_data: Vec<String>,
    time_submitted: Option<String>,
    time_started: Option<String>,
    time_finished: Option<String>,
}

fn project_job_fields(
    entries: &[(Value, Vec<Value>)],
    locale_file: &PathBuf,
    time_zone: &str,
) -> Result<Vec<JobProjection>> {
    let input = json!({"entries": entries.iter().map(|(record, files)| json!({"record":record,"output_files":files})).collect::<Vec<_>>()});
    let mut child = Command::new("ruby")
        .args([
            "-e",
            include_str!("../../../scripts/staff_job_projection.rb"),
        ])
        .arg(locale_file)
        .env("TZ", time_zone)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot launch Ruby job compatibility projection")?;
    child
        .stdin
        .take()
        .context("job projection stdin unavailable")?
        .write_all(&serde_json::to_vec(&input)?)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("Ruby job compatibility projection failed");
    }
    let projections: Vec<JobProjection> =
        serde_json::from_slice(&output.stdout).context("invalid Ruby job projection response")?;
    if projections.len() != entries.len() {
        bail!("Ruby job projection returned the wrong number of records");
    }
    Ok(projections)
}

async fn fetch_and_build_batch(
    client: Client,
    spec: TypeSpec,
    ids: Vec<u32>,
    batch_index: usize,
    ctx: FullBatchContext,
) -> Result<CandidateBatchOutput> {
    let FullBatchContext {
        repo_id,
        batch_size,
        dynamic_enums,
        arks_enabled,
        job_locale_file,
        job_time_zone,
    } = ctx;
    match spec.record_type {
        "archival_object" => {
            return fetch_and_build_ao_candidate_batch(
                &client,
                repo_id,
                &ids,
                batch_index,
                dynamic_enums,
                arks_enabled,
            )
            .await
        }
        "resource" => {
            return fetch_and_build_resource_candidate_batch(
                &client,
                repo_id,
                &ids,
                batch_index,
                batch_size,
                dynamic_enums,
                arks_enabled,
            )
            .await
        }
        "digital_object" => {
            return fetch_and_build_digital_object_candidate_batch(
                &client,
                repo_id,
                &ids,
                batch_index,
                batch_size,
                dynamic_enums,
                arks_enabled,
            )
            .await
        }
        "top_container" => {
            return fetch_and_build_top_container_candidate_batch(
                &client,
                repo_id,
                &ids,
                batch_index,
                batch_size,
                dynamic_enums,
                arks_enabled,
            )
            .await
        }
        _ => {}
    }
    let mut records = client
        .fetch_staff_records_by_ids_bounded_path(&spec.path(repo_id), &ids, batch_size)
        .await?;
    let location_owner_codes = if spec.record_type == "location" {
        let mut codes = Vec::with_capacity(records.len());
        for record in &records {
            let owner = record.get("owner_repo").and_then(Value::as_object);
            if let Some(owner) = owner {
                let owner_ref = owner
                    .get("ref")
                    .and_then(Value::as_str)
                    .context("location owner missing ref")?;
                let owner_id: u32 = owner_ref
                    .strip_prefix("/repositories/")
                    .context("invalid location owner URI")?
                    .parse()?;
                if owner_id == 0 {
                    bail!("invalid location owner repository ID");
                }
                let response = client
                    .get(&format!("repositories/{owner_id}"))?
                    .send()
                    .await?
                    .error_for_status()?;
                let owner_record: Value = response.json().await?;
                let code = owner_record
                    .get("repo_code")
                    .and_then(Value::as_str)
                    .context("location owner repository missing code")?;
                codes.push(Some(code.to_string()));
            } else {
                codes.push(None);
            }
        }
        Some(codes)
    } else {
        None
    };
    let job_projections = if spec.record_type == "job" {
        let locale_file = job_locale_file
            .as_ref()
            .context("--job-locale-file is required when indexing jobs")?;
        let time_zone = job_time_zone
            .as_deref()
            .context("--job-time-zone is required when indexing jobs")?;
        let mut entries = Vec::with_capacity(records.len());
        for record in &mut records {
            sanitize_native_index_record(record, "job")?;
            let uri = record
                .get("uri")
                .and_then(Value::as_str)
                .context("job missing URI")?;
            let path = uri.strip_prefix('/').context("invalid job URI")?;
            let response = client
                .get(&format!("{path}/output_files"))?
                .send()
                .await?
                .error_for_status()?;
            let files: Vec<Value> = response.json().await?;
            entries.push((record.clone(), files));
        }
        Some(project_job_fields(&entries, locale_file, time_zone)?)
    } else {
        None
    };
    let accession_years = if spec.record_type == "accession" {
        parse_accession_years(&records)?
            .into_iter()
            .map(Some)
            .collect()
    } else {
        vec![None; records.len()]
    };
    let inputs = records
        .into_iter()
        .zip(accession_years)
        .map(|(mut record, accession_year): (Value, Option<i32>)| {
            sanitize_native_index_record(&mut record, spec.record_type)?;
            let uri = record
                .get("uri")
                .and_then(Value::as_str)
                .context("record missing URI")?
                .to_string();
            Ok(IndexDocumentInput {
                uri,
                record_type: spec.record_type.to_string(),
                repository: Value::String(if spec.global_path {
                    "global".to_string()
                } else {
                    format!("/repositories/{repo_id}")
                }),
                record,
                enrich: true,
                dynamic_enum_fields: (*dynamic_enums).clone(),
                arks_enabled,
                parsed_accession_date_year: accession_year.map(|year| serde_json::json!(year)),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut built = build_index_document_batch(IndexDocumentBatchRequest { records: inputs })?;
    if let Some(codes) = location_owner_codes {
        if built.documents.len() != codes.len() {
            bail!("location builder returned the wrong number of documents");
        }
        for (document, code) in built.documents.iter_mut().zip(codes) {
            if let Some(code) = code {
                let fields = document
                    .as_object_mut()
                    .context("location document is not an object")?;
                fields.insert(
                    "owner_repo_display_string_u_ssort".to_string(),
                    Value::String(code),
                );
            }
        }
    }
    if let Some(projections) = job_projections {
        if built.documents.len() != projections.len() {
            bail!("job builder returned the wrong number of documents");
        }
        for (document, projection) in built.documents.iter_mut().zip(projections) {
            let fields = document
                .as_object_mut()
                .context("job document is not an object")?;
            fields.insert("title".to_string(), Value::String(projection.title));
            fields.insert("files".to_string(), json!(projection.files));
            fields.insert("job_data".to_string(), json!(projection.job_data));
            for (key, value) in [
                ("time_submitted", projection.time_submitted),
                ("time_started", projection.time_started),
                ("time_finished", projection.time_finished),
            ] {
                if let Some(value) = value {
                    fields.insert(key.to_string(), Value::String(value));
                }
            }
        }
    }
    extract_generic_documents(built.documents, spec, repo_id)
}

pub async fn execute_staff_full_candidate(
    client: &Client,
    args: &StaffFullCandidateArgs,
) -> Result<StaffFullCandidateSummary> {
    args.validate()?;
    crate::profiling_stack::verify_profiling_stack(&args.state, client.base_url(), &args.solr_url)?;
    let mut writer = IsolatedSolrStaffAoWriter::new(&args.solr_url, &args.candidate_core)?;
    writer.verify_target_core_ready().await?;
    let full_reindex_started_at_epoch_sec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    let dynamic_enums = Arc::new(client.dynamic_enum_fields_from_schemas().await?);
    let mut repositories = client.staff_repositories_raw().await?;
    repositories.sort_by_key(|repo| repo_id_from_record(repo).unwrap_or(u32::MAX));
    if repositories.is_empty() {
        bail!("backend returned no repositories");
    }
    let mut repo_ids = Vec::new();
    let mut seen_repos = HashSet::new();
    for repo in &repositories {
        let id = repo_id_from_record(repo)?;
        if id == 0 || !seen_repos.insert(id) {
            bail!("invalid or duplicate repository ID {id}");
        }
        repo_ids.push(id);
    }

    let started = Instant::now();
    let mut global_unique_uris = HashSet::new();
    let mut cross_type_duplicates = 0;
    let mut completed_batches = 0;
    let mut write_elapsed = 0.0;
    let mut first_error = None;
    let mut stages = Vec::new();
    let mut total_ids = 0;
    let mut total_documents = 0;

    for (repo_index, repo) in repositories.into_iter().enumerate() {
        let repo_id = repo_ids[repo_index];
        let repo_uri = format!("/repositories/{repo_id}");
        let built = build_index_document_batch(IndexDocumentBatchRequest {
            records: vec![IndexDocumentInput {
                uri: repo_uri.clone(),
                record_type: "repository".to_string(),
                repository: Value::String(repo_uri.clone()),
                record: repo,
                enrich: true,
                dynamic_enum_fields: (*dynamic_enums).clone(),
                arks_enabled: args.arks_enabled,
                ..Default::default()
            }],
        })?;
        let mut docs = built.documents;
        if docs.len() != 1 {
            bail!("repository builder returned {} documents", docs.len());
        }
        let mut repo_doc = docs.remove(0);
        let repo_obj = repo_doc
            .as_object_mut()
            .context("repository document is not an object")?;
        if repo_obj.get("uri").and_then(Value::as_str) != Some(repo_uri.as_str()) {
            bail!("repository document URI mismatch");
        }
        repo_obj.retain(|key, _| !key.starts_with("_archivalist_"));
        append_native_pui_types(&mut repo_doc, "repository")?;
        clean_document_whitespace(&mut repo_doc);
        if !global_unique_uris.insert(repo_uri) {
            bail!("duplicate repository document");
        }
        writer.add_documents(vec![repo_doc]).await?;
        total_ids += 1;
        total_documents += 1;

        for spec in TYPES {
            if spec.first_repository_only && repo_index > 0 {
                continue;
            }
            let stage_started = Instant::now();
            let path = spec.path(repo_id);
            let ids = client
                .staff_record_ids_by_path(&path, 0)
                .await
                .with_context(|| format!("ID list for {path}"))?;
            if check_duplicate_ids(&ids) {
                bail!("duplicate ID in {path}");
            }
            let batches = partition_ids_into_batches(&ids, args.batch_size);
            let expected_prefix = format!("/{path}/");
            let client_for_batch = client.clone();
            let batch_context = FullBatchContext {
                repo_id,
                batch_size: args.batch_size,
                dynamic_enums: Arc::clone(&dynamic_enums),
                arks_enabled: args.arks_enabled,
                job_locale_file: args.job_locale_file.clone(),
                job_time_zone: args.job_time_zone.clone(),
            };
            let mut shared = SharedCandidateState {
                writer: &mut writer,
                global_unique_uris: &mut global_unique_uris,
                cross_type_duplicate_uris_count: &mut cross_type_duplicates,
                total_completed_batches: &mut completed_batches,
                solr_write_elapsed_sec: &mut write_elapsed,
                first_error: &mut first_error,
                concurrency: args.concurrency,
                fail_after_batches: args.fail_after_batches,
            };
            let (slice, _) = process_type_candidate_batches(
                spec.record_type,
                &expected_prefix,
                &ids,
                batches,
                &mut shared,
                move |batch_index, chunk| {
                    let client = client_for_batch.clone();
                    let ctx = batch_context.clone();
                    async move {
                        let mut output =
                            fetch_and_build_batch(client, *spec, chunk, batch_index, ctx).await?;
                        for document in &mut output.ordered_documents {
                            clean_document_whitespace(document);
                        }
                        Ok(output)
                    }
                },
            )
            .await;
            total_ids += ids.len();
            total_documents += slice.total_docs;
            let failed = slice.batches_failed > 0
                || slice.missing_ids > 0
                || slice.duplicate_ids > 0
                || slice.unexpected_ids > 0;
            stages.push(FullStageSummary {
                repository_id: repo_id,
                wall_elapsed_sec: stage_started.elapsed().as_secs_f64(),
                slice,
            });
            if failed || first_error.is_some() || cross_type_duplicates > 0 {
                bail!(
                    "full candidate failed in {path}: {}",
                    first_error.as_deref().unwrap_or("record set mismatch")
                );
            }
        }
    }
    if global_unique_uris.len() != total_documents {
        bail!("document URI count mismatch");
    }
    writer.commit().await?;
    Ok(StaffFullCandidateSummary {
        full_reindex_started_at_epoch_sec,
        repository_ids: repo_ids,
        stages,
        total_ids,
        total_documents,
        unique_uris: global_unique_uris.len(),
        batches_completed: completed_batches,
        solr_update_requests_sent: writer.update_requests_sent(),
        wall_elapsed_sec: started.elapsed().as_secs_f64(),
        committed: true,
        candidate_core: args.candidate_core.clone(),
    })
}

fn check_duplicate_ids(ids: &[u32]) -> bool {
    let mut set = HashSet::with_capacity(ids.len());
    ids.iter().any(|id| !set.insert(*id))
}

pub async fn run_staff_full_candidate(
    client: &Client,
    args: &StaffFullCandidateArgs,
) -> Result<()> {
    let summary = execute_staff_full_candidate(client, args).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "Indexed {} STAFF documents in {:.3}s into {}",
            summary.total_documents, summary.wall_elapsed_sec, summary.candidate_core
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoint_table_matches_native_type_order() {
        assert_eq!(TYPES.len(), 19);
        assert_eq!(TYPES.first().unwrap().record_type, "resource");
        assert_eq!(TYPES.last().unwrap().record_type, "job");
        assert_eq!(TYPES[3].path(4), "agents/people");
        assert_eq!(TYPES[14].path(4), "repositories/4/archival_objects");
        assert!(TYPES[12].first_repository_only);
        assert!(TYPES[13].first_repository_only);
    }

    #[test]
    fn accession_years_follow_ruby_date_parse() {
        let records = vec![
            json!({"accession_date":"2026-01-01"}),
            json!({"accession_date":"2026-2-03"}),
            json!({"accession_date":"9999-12-31"}),
        ];
        assert_eq!(
            parse_accession_years(&records).unwrap(),
            vec![2026, 2026, 9999]
        );
        assert!(parse_accession_years(&[json!({"accession_date":"invalid"})]).is_err());
    }

    #[test]
    fn generic_extractor_rejects_wrong_identity_and_strips_markers() {
        let spec = TYPES[7];
        let doc = json!({"uri":"/subjects/7","id":"/subjects/7","primary_type":"subject","types":["subject"],"publish":true,"_archivalist_test":true});
        let output = extract_generic_documents(vec![doc], spec, 2).unwrap();
        assert_eq!(
            output.ordered_documents[0]["types"],
            json!(["subject", "pui_subject", "pui"])
        );
        assert!(output.ordered_documents[0]
            .get("_archivalist_test")
            .is_none());
        assert!(extract_generic_documents(
            vec![json!({"uri":"/subjects/8","id":"/subjects/9","types":[]})],
            spec,
            2
        )
        .is_err());
    }

    #[test]
    fn agent_contacts_are_empty_before_document_building() {
        let mut record = json!({"jsonmodel_type":"agent_corporate_entity","agent_contacts":[{"name":"private"}]});
        sanitize_native_index_record(&mut record, "agent_corporate_entity").unwrap();
        assert_eq!(record["agent_contacts"], json!([]));
        assert!(!serde_json::to_string(&record).unwrap().contains("private"));
        let mut resource = json!({"jsonmodel_type":"resource","agent_contacts":[{"name":"kept"}]});
        sanitize_native_index_record(&mut resource, "resource").unwrap();
        assert_eq!(resource["agent_contacts"], json!([{"name":"kept"}]));
        let mut job = json!({"jsonmodel_type":"job"});
        sanitize_native_index_record(&mut job, "job").unwrap();
        assert_eq!(job["has_modified_records"], false);
    }

    #[test]
    fn final_document_whitespace_matches_native_cleanup() {
        let mut document =
            json!({"identifier_sort":"G  ","dates":["2026 - "],"nested":{"text":"  value  "}});
        clean_document_whitespace(&mut document);
        assert_eq!(
            document,
            json!({"identifier_sort":"G","dates":["2026 -"],"nested":{"text":"value"}})
        );
    }

    #[test]
    fn accession_child_documents_follow_parent_and_validate_identity() {
        let parent_uri = "/repositories/4/accessions/5";
        let child_uri = "/repositories/4/collection_management/9";
        let parent = json!({
            "uri":parent_uri,"id":parent_uri,"primary_type":"accession","types":["accession"],
            "publish":false,
            "_archivalist_collection_management_documents":[{
                "uri":child_uri,"id":child_uri,"parent_id":parent_uri,"parent_type":"accession",
                "primary_type":"collection_management","types":["collection_management"]
            }]
        });
        let output = extract_generic_documents(vec![parent], TYPES[2], 4).unwrap();
        assert_eq!(output.parent_count, 1);
        assert_eq!(output.child_count, 1);
        assert_eq!(output.ordered_documents[0]["uri"], parent_uri);
        assert_eq!(output.ordered_documents[1]["uri"], child_uri);
        assert!(output.ordered_documents[0]
            .get(COLLECTION_MANAGEMENT_DOCUMENTS_KEY)
            .is_none());
        let invalid = json!({
            "uri":parent_uri,"id":parent_uri,"primary_type":"accession","types":["accession"],
            "_archivalist_collection_management_documents":[{
                "uri":child_uri,"id":child_uri,"parent_id":"/repositories/4/accessions/6",
                "parent_type":"accession","primary_type":"collection_management"
            }]
        });
        assert!(extract_generic_documents(vec![invalid], TYPES[2], 4).is_err());
    }
}
