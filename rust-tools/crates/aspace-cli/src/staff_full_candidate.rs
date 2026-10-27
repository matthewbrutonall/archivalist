use anyhow::{bail, Context, Result};
use aspace_client::index_document::COLLECTION_MANAGEMENT_DOCUMENTS_KEY;
use aspace_client::isolated_solr_writer::{
    validate_isolated_solr_base_url, validate_stage_candidate_core, IsolatedSolrStaffAoWriter,
};
use aspace_client::{
    build_index_document_batch, Client, IndexDocumentBatchRequest, IndexDocumentInput,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

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
        first_repository_only: false,
    },
    TypeSpec {
        record_type: "location_profile",
        endpoint: "location_profiles",
        global_path: true,
        first_repository_only: false,
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
    Ok(())
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
    let records = client
        .fetch_staff_records_by_ids_bounded_path(&spec.path(repo_id), &ids, batch_size)
        .await?;
    let inputs = records
        .into_iter()
        .map(|mut record| {
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
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let built = build_index_document_batch(IndexDocumentBatchRequest { records: inputs })?;
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
            if spec.global_path && repo_index > 0 && !ids.is_empty() {
                bail!("repeated global profile writes across repositories are not yet supported: {path}");
            }
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
            let (slice, _) =
                process_type_candidate_batches(
                    spec.record_type,
                    &expected_prefix,
                    &ids,
                    batches,
                    &mut shared,
                    move |batch_index, chunk| {
                        let client = client_for_batch.clone();
                        let ctx = batch_context.clone();
                        async move {
                            fetch_and_build_batch(client, *spec, chunk, batch_index, ctx).await
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
