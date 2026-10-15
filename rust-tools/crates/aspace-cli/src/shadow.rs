use anyhow::{Context, Result};
use aspace_client::models::{ArchivalObject, DigitalObject, Resource};
use aspace_client::Client;
use clap::{Args, Subcommand};
use futures::stream::{self, StreamExt};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

#[derive(Args, Clone)]
#[command(subcommand_negates_reqs = true, args_conflicts_with_subcommands = true)]
pub struct ShadowIndexCommand {
    #[command(subcommand)]
    pub action: Option<ShadowIndexAction>,

    #[command(flatten)]
    pub export_args: ShadowIndexExportArgs,
}

#[derive(Subcommand, Clone)]
pub enum ShadowIndexAction {
    /// Compare shadow index counts to search facet counts
    Compare(ShadowIndexCompareArgs),
    /// Compare sample fields between shadow and search
    CompareFields(ShadowIndexCompareFieldsArgs),
    /// Generate Solr-shaped documents
    SolrDocs(ShadowIndexSolrDocsArgs),
    /// Compare sample fields directly against Solr
    SolrCompare(ShadowIndexSolrCompareArgs),
    /// Load shadow docs into a separate Solr collection
    SolrLoadShadow(ShadowIndexSolrLoadShadowArgs),
    /// Compare documents between shadow Solr collection and live Solr collection
    SolrShadowCompare(ShadowIndexSolrShadowCompareArgs),
    /// Reindex specific URIs into the shadow collection
    SolrReindexUris(ShadowIndexSolrReindexUrisArgs),
    /// Emit URIs for API records changed since a timestamp
    ChangedUris(ShadowIndexChangedUrisArgs),
    /// Incrementally reindex changed records into the shadow collection
    IncrementalReindex(ShadowIndexIncrementalReindexArgs),
    /// Compare shadow documents against live Solr and report field parity
    ParityReport(ShadowIndexParityReportArgs),
    /// Ensure shadow Solr collection schema contains richer archival object fields
    SolrEnsureShadowSchema(ShadowIndexSolrEnsureShadowSchemaArgs),
}

#[derive(Args, Clone)]
pub struct ShadowIndexExportArgs {
    #[arg(long, required = true, help = "Repository ID")]
    pub repo: Option<u32>,

    #[arg(
        long,
        required = true,
        help = "Target record type, 'resources', 'archival-objects', and 'digital-objects' are supported"
    )]
    pub target: Option<String>,

    #[arg(long, help = "Output file path (default: stdout)")]
    pub output: Option<PathBuf>,

    #[arg(long, help = "Batch size for fetching records", default_value = "50")]
    pub batch_size: std::num::NonZeroUsize,

    #[arg(
        long,
        help = "Concurrency level. Requests are made concurrently but results are buffered to preserve original order.",
        default_value = "1"
    )]
    pub concurrency: usize,
}

#[derive(Args, Clone)]
pub struct ShadowIndexCompareArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(long, help = "Output comparison result as JSON")]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct ShadowIndexCompareFieldsArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        required = true,
        help = "Target record type, 'resources', 'archival-objects', and 'digital-objects' are supported"
    )]
    pub target: String,

    #[arg(long, help = "Limit sample size", default_value = "10")]
    pub limit: usize,

    #[arg(long, help = "Output comparison result as JSON")]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct ShadowIndexSolrDocsArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, only 'archival-objects' is supported for now"
    )]
    pub target: String,

    #[arg(long, help = "Output file path (default: stdout)")]
    pub output: Option<PathBuf>,

    #[arg(long, help = "Limit sample size for coarse tests")]
    pub limit: Option<usize>,

    #[arg(
        long,
        help = "Compare generated Solr title against authenticated search title by URI"
    )]
    pub compare_sample: bool,

    #[arg(long, help = "Output summary as JSON")]
    pub json_summary: bool,

    #[arg(long, help = "Batch size for fetching records", default_value = "50")]
    pub batch_size: std::num::NonZeroUsize,

    #[arg(long, help = "Concurrency level", default_value = "1")]
    pub concurrency: usize,
}

#[derive(Args, Clone)]
pub struct ShadowIndexSolrCompareArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, only 'archival-objects' is supported"
    )]
    pub target: String,

    #[arg(long, help = "Limit sample size", default_value = "10")]
    pub limit: usize,

    #[arg(
        long,
        help = "Solr URL",
        default_value = "http://localhost:8985/solr/archivesspace"
    )]
    pub solr_url: String,

    #[arg(long, help = "Output comparison result as JSON")]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct ShadowIndexSolrLoadShadowArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, only 'archival-objects' is supported for this milestone"
    )]
    pub target: String,

    #[arg(
        long,
        help = "Limit total number of documents to load",
        default_value = "100"
    )]
    pub limit: usize,

    #[arg(long, help = "Solr URL", default_value = "http://localhost:8985/solr")]
    pub solr_url: String,

    #[arg(long, help = "Collection name", default_value = "archivesspace_shadow")]
    pub collection: String,

    #[arg(long, help = "Create the shadow collection/core if absent")]
    pub create_if_missing: bool,

    #[arg(long, help = "Delete all docs in the shadow collection before loading")]
    pub replace_shadow: bool,

    #[arg(
        long,
        help = "Delete docs for the selected repo and target from the shadow collection before loading"
    )]
    pub replace_target: bool,
}

#[derive(Args, Clone)]
pub struct ShadowIndexSolrReindexUrisArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, 'archival-objects', 'resources', and 'digital-objects' are supported"
    )]
    pub target: String,

    #[arg(long, help = "File containing URIs to reindex")]
    pub uri_file: PathBuf,

    #[arg(long, help = "Solr URL", default_value = "http://localhost:8985/solr")]
    pub solr_url: String,

    #[arg(long, help = "Collection name", default_value = "archivesspace_shadow")]
    pub collection: String,
}

#[derive(Args, Clone)]
pub struct ShadowIndexChangedUrisArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, 'archival-objects', 'resources', and 'digital-objects' are supported"
    )]
    pub target: String,

    #[arg(long, help = "RFC3339 timestamp, e.g. 2026-10-07T10:30:00Z")]
    pub since: String,

    #[arg(long, help = "Output file path (default: stdout)")]
    pub output: Option<PathBuf>,

    #[arg(long, help = "Batch size for fetching records", default_value = "50")]
    pub batch_size: std::num::NonZeroUsize,

    #[arg(long, help = "Concurrency level", default_value = "1")]
    pub concurrency: usize,
}

#[derive(Args, Clone)]
pub struct ShadowIndexIncrementalReindexArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, 'archival-objects', 'resources', and 'digital-objects' are supported"
    )]
    pub target: String,

    #[arg(long, help = "RFC3339 timestamp, e.g. 2026-10-07T10:30:00Z")]
    pub since: String,

    #[arg(long, help = "Solr URL", default_value = "http://localhost:8985/solr")]
    pub solr_url: String,

    #[arg(long, help = "Collection name", default_value = "archivesspace_shadow")]
    pub collection: String,

    #[arg(long, help = "Batch size for fetching records", default_value = "50")]
    pub batch_size: std::num::NonZeroUsize,

    #[arg(long, help = "Concurrency level", default_value = "1")]
    pub concurrency: usize,
}

#[derive(Args, Clone)]
pub struct ShadowIndexSolrShadowCompareArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, only 'archival-objects' is supported for this milestone"
    )]
    pub target: String,

    #[arg(long, help = "Limit sample size", default_value = "25")]
    pub limit: usize,

    #[arg(long, help = "Solr URL", default_value = "http://localhost:8985/solr")]
    pub solr_url: String,

    #[arg(long, help = "Live collection name", default_value = "archivesspace")]
    pub live_collection: String,

    #[arg(
        long,
        help = "Shadow collection name",
        default_value = "archivesspace_shadow"
    )]
    pub shadow_collection: String,

    #[arg(long, help = "Output comparison result as JSON")]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct ShadowIndexParityReportArgs {
    #[arg(long, help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        help = "Target record type, only 'archival-objects' is supported"
    )]
    pub target: String,

    #[arg(long, help = "Solr URL", default_value = "http://localhost:8985/solr")]
    pub solr_url: String,

    #[arg(long, help = "Live collection name", default_value = "archivesspace")]
    pub live_collection: String,

    #[arg(long, help = "Limit sample size")]
    pub limit: Option<usize>,

    #[arg(long, help = "Batch size for fetching records", default_value = "50")]
    pub batch_size: std::num::NonZeroUsize,

    #[arg(long, help = "Output comparison result as JSON")]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct ShadowIndexSolrEnsureShadowSchemaArgs {
    #[arg(long, help = "Solr URL", default_value = "http://localhost:8985/solr")]
    pub solr_url: String,

    #[arg(long, help = "Collection name", default_value = "archivesspace_shadow")]
    pub collection: String,

    #[arg(long, help = "Output report as JSON")]
    pub json: bool,
}

#[derive(Serialize)]
pub struct SolrArchivalObjectDocument {
    pub id: String,
    pub uri: String,
    pub primary_type: String,
    pub title: String,
    pub publish: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jsonmodel_type: Option<String>,
    pub repo_id: u32,
    pub record_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_i: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_string: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub date_expressions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subject_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_roles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_relators: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_titles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_uri: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_titles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_levels: Vec<String>,
}

impl SolrArchivalObjectDocument {
    pub fn from_archival_object(repo_id: u32, id: u32, ao: ArchivalObject) -> Self {
        let json = compact_tree_source_json(&ao);
        let mut base_title = if let Some(t) = ao.title.as_deref().filter(|s| !s.trim().is_empty()) {
            t.to_string()
        } else {
            ao.display_string.clone().unwrap_or_default()
        };

        let expressions: Vec<String> = ao
            .dates
            .iter()
            .filter_map(|d| {
                d.expression
                    .as_deref()
                    .filter(|e| !e.trim().is_empty())
                    .map(|e| {
                        if d.date_type.as_deref() == Some("bulk") {
                            format!("bulk: {}", e)
                        } else {
                            e.to_string()
                        }
                    })
            })
            .collect();

        if !expressions.is_empty() {
            if !base_title.is_empty() {
                base_title.push_str(", ");
            }
            base_title.push_str(&expressions.join(", "));
        }

        let date_expressions: Vec<String> = ao
            .dates
            .iter()
            .filter_map(|d| {
                d.expression
                    .as_deref()
                    .filter(|e| !e.trim().is_empty())
                    .map(|e| e.to_string())
            })
            .collect();

        let subject_uris = non_empty_strings(ao.subjects.iter().map(|r| r.ref_uri.as_str()));
        let linked_agent_uris = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.ref_uri.as_deref()),
        );
        let linked_agent_roles = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.role.as_deref()),
        );
        let linked_agent_relators = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.relator.as_deref()),
        );
        let linked_agent_titles = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.title.as_deref()),
        );
        let resource_uri = ao
            .resource
            .as_ref()
            .map(|r| r.ref_uri.trim().to_string())
            .filter(|s| !s.is_empty());
        let parent_uri = ao
            .parent
            .as_ref()
            .map(|r| r.ref_uri.trim().to_string())
            .filter(|s| !s.is_empty());
        let ancestor_uris =
            non_empty_strings(ao.ancestors.iter().filter_map(|a| a.ref_uri.as_deref()));
        let ancestor_titles =
            non_empty_strings(ao.ancestors.iter().filter_map(|a| a.title.as_deref()));
        let ancestor_levels =
            non_empty_strings(ao.ancestors.iter().filter_map(|a| a.level.as_deref()));

        SolrArchivalObjectDocument {
            id: ao.uri.clone(),
            uri: ao.uri.clone(),
            primary_type: "archival_object".to_string(),
            title: base_title,
            publish: ao.publish,
            json,
            parent_resource: ao.resource.map(|r| r.ref_uri),
            jsonmodel_type: ao.jsonmodel_type,
            repo_id,
            record_id: id,
            level: ao.level,
            ref_id: ao.ref_id,
            component_id: ao.component_id,
            position_i: ao.position,
            display_string: ao.display_string,
            date_expressions,
            subject_uris,
            linked_agent_uris,
            linked_agent_roles,
            linked_agent_relators,
            linked_agent_titles,
            resource_uri,
            parent_uri,
            ancestor_uris,
            ancestor_titles,
            ancestor_levels,
        }
    }
}

fn compact_tree_source_json(ao: &ArchivalObject) -> Option<String> {
    let source = serde_json::to_value(ao).ok()?;
    let mut compact = serde_json::Map::new();

    for key in [
        "title",
        "component_id",
        "level",
        "other_level",
        "suppressed",
        "slug",
        "dates",
    ] {
        if let Some(value) = source.get(key).filter(|value| !value.is_null()) {
            compact.insert(key.to_string(), value.clone());
        }
    }

    let instances = source
        .get("instances")
        .and_then(|value| value.as_array())
        .map(|instances| {
            instances
                .iter()
                .map(|instance| {
                    let mut compact_instance = serde_json::Map::new();
                    for key in ["instance_type", "digital_object"] {
                        if let Some(value) = instance.get(key).filter(|value| !value.is_null()) {
                            compact_instance.insert(key.to_string(), value.clone());
                        }
                    }

                    if let Some(sub_container) = instance.get("sub_container") {
                        let mut compact_sub = serde_json::Map::new();
                        for key in [
                            "type_2",
                            "indicator_2",
                            "barcode_2",
                            "type_3",
                            "indicator_3",
                        ] {
                            if let Some(value) =
                                sub_container.get(key).filter(|value| !value.is_null())
                            {
                                compact_sub.insert(key.to_string(), value.clone());
                            }
                        }

                        if let Some(resolved) = sub_container
                            .get("top_container")
                            .and_then(|value| value.get("_resolved"))
                        {
                            let mut compact_top = serde_json::Map::new();
                            for key in ["type", "indicator", "barcode"] {
                                if let Some(value) =
                                    resolved.get(key).filter(|value| !value.is_null())
                                {
                                    compact_top.insert(key.to_string(), value.clone());
                                }
                            }
                            if !compact_top.is_empty() {
                                compact_sub.insert(
                                    "top_container".to_string(),
                                    serde_json::json!({"_resolved": compact_top}),
                                );
                            }
                        }

                        if !compact_sub.is_empty() {
                            compact_instance.insert(
                                "sub_container".to_string(),
                                serde_json::Value::Object(compact_sub),
                            );
                        }
                    }

                    serde_json::Value::Object(compact_instance)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !instances.is_empty() {
        compact.insert("instances".to_string(), serde_json::Value::Array(instances));
    }

    serde_json::to_string(&serde_json::Value::Object(compact)).ok()
}

#[derive(Serialize)]
pub struct SafeSolrDocument {
    pub id: String,
    pub uri: String,
    pub primary_type: String,
    pub title: String,
    pub publish: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_i: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_string: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub date_expressions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subject_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_roles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_relators: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_titles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_uri: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_titles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_levels: Vec<String>,
}

impl From<SolrArchivalObjectDocument> for SafeSolrDocument {
    fn from(doc: SolrArchivalObjectDocument) -> Self {
        Self {
            id: doc.id,
            uri: doc.uri,
            primary_type: doc.primary_type,
            title: doc.title,
            publish: doc.publish,
            json: doc.json,
            level: doc.level,
            ref_id: doc.ref_id,
            component_id: doc.component_id,
            position_i: doc.position_i,
            display_string: doc.display_string,
            date_expressions: doc.date_expressions,
            subject_uris: doc.subject_uris,
            linked_agent_uris: doc.linked_agent_uris,
            linked_agent_roles: doc.linked_agent_roles,
            linked_agent_relators: doc.linked_agent_relators,
            linked_agent_titles: doc.linked_agent_titles,
            resource_uri: doc.resource_uri,
            parent_uri: doc.parent_uri,
            ancestor_uris: doc.ancestor_uris,
            ancestor_titles: doc.ancestor_titles,
            ancestor_levels: doc.ancestor_levels,
        }
    }
}

impl SafeSolrDocument {
    pub fn from_resource(res: aspace_client::models::Resource) -> Self {
        Self {
            id: res.uri.clone(),
            uri: res.uri.clone(),
            primary_type: "resource".to_string(),
            title: res.title.unwrap_or_default(),
            publish: res.publish,
            json: None,
            level: None,
            ref_id: None,
            component_id: None,
            position_i: None,
            display_string: None,
            date_expressions: Vec::new(),
            subject_uris: Vec::new(),
            linked_agent_uris: Vec::new(),
            linked_agent_roles: Vec::new(),
            linked_agent_relators: Vec::new(),
            linked_agent_titles: Vec::new(),
            resource_uri: None,
            parent_uri: None,
            ancestor_uris: Vec::new(),
            ancestor_titles: Vec::new(),
            ancestor_levels: Vec::new(),
        }
    }

    pub fn from_digital_object(do_obj: aspace_client::models::DigitalObject) -> Self {
        Self {
            id: do_obj.uri.clone(),
            uri: do_obj.uri.clone(),
            primary_type: "digital_object".to_string(),
            title: do_obj.title.unwrap_or_default(),
            publish: do_obj.publish,
            json: None,
            level: None,
            ref_id: None,
            component_id: None,
            position_i: None,
            display_string: None,
            date_expressions: Vec::new(),
            subject_uris: Vec::new(),
            linked_agent_uris: Vec::new(),
            linked_agent_roles: Vec::new(),
            linked_agent_relators: Vec::new(),
            linked_agent_titles: Vec::new(),
            resource_uri: None,
            parent_uri: None,
            ancestor_uris: Vec::new(),
            ancestor_titles: Vec::new(),
            ancestor_levels: Vec::new(),
        }
    }
}

#[derive(Serialize)]
pub struct CompareFieldsReport {
    pub repo_id: u32,
    pub target: String,
    pub sample_size: usize,
    pub overall_status: String,
    pub results: Vec<CompareFieldsResult>,
}

#[derive(Serialize)]
pub struct CompareFieldsResult {
    pub uri: String,
    pub match_status: String,
    pub fields: Vec<FieldComparison>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct FieldComparison {
    pub field: String,
    pub shadow_value: Option<String>,
    pub search_value: Option<String>,
    pub status: String,
}

#[derive(Serialize)]
pub struct ShadowDocument {
    pub repo_id: u32,
    pub id: u32,
    pub uri: String,
    pub primary_type: String,
    pub title: Option<String>,
    pub publish: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jsonmodel_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digital_object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_version_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_i: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_string: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub date_expressions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subject_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_roles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_relators: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_agent_titles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_uri: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_uris: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_titles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestor_levels: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ShadowCompareCheck {
    pub record_type: String,
    pub shadow_count: usize,
    pub search_count: Option<u64>,
    pub status: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ShadowCompareReport {
    pub overall_status: String,
    pub checks: Vec<ShadowCompareCheck>,
}

pub fn make_field_comparison(
    field: &str,
    shadow_val: Option<String>,
    search_val: Option<String>,
) -> FieldComparison {
    let status = match (&shadow_val, &search_val) {
        (Some(sh), Some(se)) if sh == se => "match",
        (None, None) => "match",
        (_, None) => "search_absent",
        _ => "mismatch",
    };
    FieldComparison {
        field: field.to_string(),
        shadow_value: shadow_val,
        search_value: search_val,
        status: status.to_string(),
    }
}

pub fn make_compare_check(
    record_type: &str,
    shadow_count: usize,
    search_count: Option<u64>,
) -> ShadowCompareCheck {
    let status = if Some(shadow_count as u64) == search_count {
        "match"
    } else {
        "mismatch"
    };
    ShadowCompareCheck {
        record_type: record_type.to_string(),
        shadow_count,
        search_count,
        status: status.to_string(),
    }
}

fn non_empty_strings<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn sorted_vec(values: &[String]) -> Vec<String> {
    let mut sorted = values.to_vec();
    sorted.sort();
    sorted
}

fn search_value_as_vec(value: &serde_json::Value) -> Vec<String> {
    let mut values = if let Some(s) = value.as_str() {
        vec![s.to_string()]
    } else if let Some(arr) = value.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(ToString::to_string))
            .collect()
    } else if value.is_null() {
        Vec::new()
    } else {
        vec![value.to_string()]
    };
    values.retain(|value| !value.trim().is_empty());
    values.sort();
    values
}

fn make_vec_field_comparison(
    field: &str,
    shadow_values: &[String],
    search_value: &serde_json::Value,
) -> FieldComparison {
    let shadow_values = sorted_vec(shadow_values);
    let search_values = search_value_as_vec(search_value);
    let status = if shadow_values == search_values {
        "match"
    } else {
        "mismatch"
    };
    FieldComparison {
        field: field.to_string(),
        shadow_value: Some(shadow_values.join("|")),
        search_value: Some(search_values.join("|")),
        status: status.to_string(),
    }
}

pub fn compare_archival_object_search_fields(
    doc: &ShadowDocument,
    sdoc: &serde_json::Value,
    comparisons: &mut Vec<FieldComparison>,
) {
    for key in [
        "level",
        "ref_id",
        "component_id",
        "position_i",
        "display_string",
        "resource_uri",
        "parent_uri",
    ] {
        if let Some(val) = sdoc.get(key) {
            let search_val = if let Some(s) = val.as_str() {
                Some(s.to_string())
            } else if let Some(arr) = val.as_array() {
                arr.first().and_then(|v| v.as_str()).map(|s| s.to_string())
            } else if val.is_null() {
                None
            } else {
                Some(val.to_string())
            };
            let shadow_val = match key {
                "level" => doc.level.clone(),
                "ref_id" => doc.ref_id.clone(),
                "component_id" => doc.component_id.clone(),
                "position_i" => doc.position_i.map(|v| v.to_string()),
                "display_string" => doc.display_string.clone(),
                "resource_uri" => doc.resource_uri.clone(),
                "parent_uri" => doc.parent_uri.clone(),
                _ => None,
            };
            comparisons.push(make_field_comparison(key, shadow_val, search_val));
        }
    }

    for key in [
        "subject_uris",
        "date_expressions",
        "linked_agent_uris",
        "linked_agent_roles",
        "linked_agent_relators",
        "linked_agent_titles",
        "ancestor_uris",
        "ancestor_titles",
        "ancestor_levels",
    ] {
        if let Some(search_value) = sdoc.get(key) {
            let shadow_values = match key {
                "subject_uris" => &doc.subject_uris,
                "date_expressions" => &doc.date_expressions,
                "linked_agent_uris" => &doc.linked_agent_uris,
                "linked_agent_roles" => &doc.linked_agent_roles,
                "linked_agent_relators" => &doc.linked_agent_relators,
                "linked_agent_titles" => &doc.linked_agent_titles,
                "ancestor_uris" => &doc.ancestor_uris,
                "ancestor_titles" => &doc.ancestor_titles,
                "ancestor_levels" => &doc.ancestor_levels,
                _ => unreachable!(),
            };
            comparisons.push(make_vec_field_comparison(key, shadow_values, search_value));
        }
    }
}

impl ShadowDocument {
    pub fn from_resource(repo_id: u32, id: u32, resource: Resource) -> Self {
        ShadowDocument {
            repo_id,
            id,
            uri: resource.uri.clone(),
            primary_type: "resource".to_string(),
            title: resource.title,
            publish: resource.publish,
            parent_resource: None,
            jsonmodel_type: resource.jsonmodel_type,
            digital_object_id: None,
            file_version_count: None,
            level: None,
            ref_id: None,
            component_id: None,
            position_i: None,
            display_string: None,
            date_expressions: Vec::new(),
            subject_uris: Vec::new(),
            linked_agent_uris: Vec::new(),
            linked_agent_roles: Vec::new(),
            linked_agent_relators: Vec::new(),
            linked_agent_titles: Vec::new(),
            resource_uri: None,
            parent_uri: None,
            ancestor_uris: Vec::new(),
            ancestor_titles: Vec::new(),
            ancestor_levels: Vec::new(),
        }
    }

    pub fn from_archival_object(repo_id: u32, id: u32, ao: ArchivalObject) -> Self {
        let title = if let Some(t) = ao.title.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(t.to_string())
        } else {
            ao.display_string.clone()
        };
        let subject_uris = non_empty_strings(ao.subjects.iter().map(|r| r.ref_uri.as_str()));
        let date_expressions = non_empty_strings(
            ao.dates
                .iter()
                .filter_map(|date| date.expression.as_deref()),
        );
        let linked_agent_uris = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.ref_uri.as_deref()),
        );
        let linked_agent_roles = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.role.as_deref()),
        );
        let linked_agent_relators = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.relator.as_deref()),
        );
        let linked_agent_titles = non_empty_strings(
            ao.linked_agents
                .iter()
                .filter_map(|agent| agent.title.as_deref()),
        );
        let resource_uri = ao
            .resource
            .as_ref()
            .map(|r| r.ref_uri.trim().to_string())
            .filter(|s| !s.is_empty());
        let parent_uri = ao
            .parent
            .as_ref()
            .map(|r| r.ref_uri.trim().to_string())
            .filter(|s| !s.is_empty());
        let ancestor_uris =
            non_empty_strings(ao.ancestors.iter().filter_map(|a| a.ref_uri.as_deref()));
        let ancestor_titles =
            non_empty_strings(ao.ancestors.iter().filter_map(|a| a.title.as_deref()));
        let ancestor_levels =
            non_empty_strings(ao.ancestors.iter().filter_map(|a| a.level.as_deref()));

        ShadowDocument {
            repo_id,
            id,
            uri: ao.uri.clone(),
            primary_type: "archival_object".to_string(),
            title,
            publish: ao.publish,
            parent_resource: ao.resource.map(|r| r.ref_uri),
            jsonmodel_type: ao.jsonmodel_type,
            digital_object_id: None,
            file_version_count: None,
            level: ao.level,
            ref_id: ao.ref_id,
            component_id: ao.component_id,
            position_i: ao.position,
            display_string: ao.display_string,
            date_expressions,
            subject_uris,
            linked_agent_uris,
            linked_agent_roles,
            linked_agent_relators,
            linked_agent_titles,
            resource_uri,
            parent_uri,
            ancestor_uris,
            ancestor_titles,
            ancestor_levels,
        }
    }

    pub fn from_digital_object(
        repo_id: u32,
        id: u32,
        do_obj: aspace_client::models::DigitalObject,
    ) -> Self {
        ShadowDocument {
            repo_id,
            id,
            uri: do_obj.uri.clone(),
            primary_type: "digital_object".to_string(),
            title: do_obj.title,
            publish: do_obj.publish,
            parent_resource: None,
            jsonmodel_type: do_obj.jsonmodel_type,
            digital_object_id: do_obj.digital_object_id,
            file_version_count: Some(do_obj.file_versions.len()),
            level: None,
            ref_id: None,
            component_id: None,
            position_i: None,
            display_string: None,
            date_expressions: Vec::new(),
            subject_uris: Vec::new(),
            linked_agent_uris: Vec::new(),
            linked_agent_roles: Vec::new(),
            linked_agent_relators: Vec::new(),
            linked_agent_titles: Vec::new(),
            resource_uri: None,
            parent_uri: None,
            ancestor_uris: Vec::new(),
            ancestor_titles: Vec::new(),
            ancestor_levels: Vec::new(),
        }
    }
}

pub async fn run_shadow_index(client: &Client, command: ShadowIndexCommand) -> Result<()> {
    if let Some(action) = command.action {
        match action {
            ShadowIndexAction::Compare(args) => run_shadow_index_compare(client, args).await,
            ShadowIndexAction::CompareFields(args) => {
                run_shadow_index_compare_fields(client, args).await
            }
            ShadowIndexAction::SolrDocs(args) => run_shadow_index_solr_docs(client, args).await,
            ShadowIndexAction::SolrCompare(args) => {
                run_shadow_index_solr_compare(client, args).await
            }
            ShadowIndexAction::SolrLoadShadow(args) => {
                run_shadow_index_solr_load_shadow(client, args).await
            }
            ShadowIndexAction::SolrShadowCompare(args) => {
                run_shadow_index_solr_shadow_compare(client, args).await
            }
            ShadowIndexAction::SolrReindexUris(args) => {
                run_shadow_index_solr_reindex_uris(client, args).await
            }
            ShadowIndexAction::ChangedUris(args) => {
                run_shadow_index_changed_uris(client, args).await
            }
            ShadowIndexAction::IncrementalReindex(args) => {
                run_shadow_index_incremental_reindex(client, args).await
            }
            ShadowIndexAction::ParityReport(args) => {
                run_shadow_index_parity_report(client, args).await
            }
            ShadowIndexAction::SolrEnsureShadowSchema(args) => {
                run_shadow_index_solr_ensure_shadow_schema(args).await
            }
        }
    } else {
        run_shadow_index_export(client, command.export_args).await
    }
}

pub async fn run_shadow_index_compare(client: &Client, args: ShadowIndexCompareArgs) -> Result<()> {
    let repo_id = args.repo;
    let resource_ids = client.resource_ids(repo_id).await?;
    let ao_ids = client.archival_object_ids(repo_id).await?;
    let do_ids = client.digital_object_ids(repo_id).await?;

    let search_path =
        format!("repositories/{repo_id}/search?q[]=%2A&page=1&page_size=1&facet[]=primary_type");
    let response = client.get(&search_path)?.send().await?;
    let counts_by_type = if response.status().is_success() {
        let json = response.json::<serde_json::Value>().await?;
        crate::parse_primary_type_facets(&json)
    } else {
        std::collections::BTreeMap::new()
    };

    let mut checks = Vec::new();
    let types = [
        ("resource", resource_ids.len()),
        ("archival_object", ao_ids.len()),
        ("digital_object", do_ids.len()),
    ];

    for (record_type, shadow_count) in types {
        let search_count = counts_by_type.get(record_type).copied();
        checks.push(make_compare_check(record_type, shadow_count, search_count));
    }

    let overall_status = if checks.iter().all(|c| c.status == "match") {
        "match"
    } else {
        "mismatch"
    };

    let report = ShadowCompareReport {
        overall_status: overall_status.to_string(),
        checks,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Overall Status: {}", overall_status);
        println!(
            "{:<20} | {:<15} | {:<15} | Status",
            "Record Type", "Shadow Count", "Search Count"
        );
        println!("{}", "-".repeat(65));
        for check in &report.checks {
            let search_count_str = check
                .search_count
                .map(|c| c.to_string())
                .unwrap_or_else(|| "None".to_string());
            println!(
                "{:<20} | {:<15} | {:<15} | {}",
                check.record_type, check.shadow_count, search_count_str, check.status
            );
        }
    }

    Ok(())
}

pub async fn run_shadow_index_compare_fields(
    client: &Client,
    args: ShadowIndexCompareFieldsArgs,
) -> Result<()> {
    let repo_id = args.repo;
    let target = args.target.as_str();

    if target != "resources" && target != "archival-objects" && target != "digital-objects" {
        anyhow::bail!(
            "Unsupported target '{}'. Only 'resources', 'archival-objects', and 'digital-objects' are supported.",
            target
        );
    }

    let mut ids = match target {
        "resources" => client.resource_ids(repo_id).await?,
        "archival-objects" => client.archival_object_ids(repo_id).await?,
        "digital-objects" => client.digital_object_ids(repo_id).await?,
        _ => unreachable!(),
    };

    ids.truncate(args.limit);
    let sample_size = ids.len();

    let docs: Vec<ShadowDocument> = match target {
        "resources" => {
            let records = client.resources_by_ids(repo_id, &ids).await?;
            records
                .into_iter()
                .map(|r| {
                    let id = r
                        .uri
                        .split('/')
                        .next_back()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    ShadowDocument::from_resource(repo_id, id, r)
                })
                .collect()
        }
        "archival-objects" => {
            let records = client.archival_objects_by_ids(repo_id, &ids).await?;
            records
                .into_iter()
                .map(|r| {
                    let id = r
                        .uri
                        .split('/')
                        .next_back()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    ShadowDocument::from_archival_object(repo_id, id, r)
                })
                .collect()
        }
        "digital-objects" => {
            let records = client.digital_objects_by_ids(repo_id, &ids).await?;
            records
                .into_iter()
                .map(|r| {
                    let id = r
                        .uri
                        .split('/')
                        .next_back()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    ShadowDocument::from_digital_object(repo_id, id, r)
                })
                .collect()
        }
        _ => unreachable!(),
    };

    let mut results = Vec::new();
    let mut overall_match = true;

    for doc in docs {
        let q = format!("id:\"{}\"", doc.uri)
            .replace('"', "%22")
            .replace('/', "%2F")
            .replace(':', "%3A");
        let search_path = format!("repositories/{}/search?q={}&page=1&page_size=1", repo_id, q);
        let response = client.get(&search_path)?.send().await?;

        let search_doc = if response.status().is_success() {
            let json = response.json::<serde_json::Value>().await?;
            if let Some(hits) = json.get("total_hits").and_then(|h| h.as_u64()) {
                if hits > 0 {
                    json.get("results")
                        .and_then(|arr| arr.as_array())
                        .and_then(|arr| arr.first())
                        .cloned()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut field_comparisons = Vec::new();

        if let Some(sdoc) = search_doc {
            field_comparisons.push(make_field_comparison(
                "uri",
                Some(doc.uri.clone()),
                sdoc.get("uri")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ));
            field_comparisons.push(make_field_comparison(
                "primary_type",
                Some(doc.primary_type.clone()),
                sdoc.get("primary_type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ));
            field_comparisons.push(make_field_comparison(
                "title",
                doc.title.clone(),
                sdoc.get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ));
            field_comparisons.push(make_field_comparison(
                "publish",
                Some(doc.publish.to_string()),
                sdoc.get("publish")
                    .and_then(|v| v.as_bool())
                    .map(|b| b.to_string()),
            ));

            if target == "archival-objects" {
                compare_archival_object_search_fields(&doc, &sdoc, &mut field_comparisons);
            }

            let doc_match = field_comparisons.iter().all(|f| f.status == "match");
            if !doc_match {
                overall_match = false;
            }
            results.push(CompareFieldsResult {
                uri: doc.uri.clone(),
                match_status: if doc_match {
                    "match".to_string()
                } else {
                    "mismatch".to_string()
                },
                fields: field_comparisons,
            });
        } else {
            overall_match = false;
            results.push(CompareFieldsResult {
                uri: doc.uri.clone(),
                match_status: "missing_in_search".to_string(),
                fields: vec![],
            });
        }
    }

    let report = CompareFieldsReport {
        repo_id,
        target: target.to_string(),
        sample_size,
        overall_status: if overall_match {
            "match".to_string()
        } else {
            "mismatch".to_string()
        },
        results,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Target: {}", report.target);
        println!("Sample Size: {}", report.sample_size);
        println!("Overall Status: {}", report.overall_status);
        println!("{}", "-".repeat(65));
        for res in &report.results {
            println!("URI: {} [{}]", res.uri, res.match_status);
            for field in &res.fields {
                if field.status != "match" {
                    let sh = field.shadow_value.as_deref().unwrap_or("None");
                    let se = field.search_value.as_deref().unwrap_or("None");
                    println!(
                        "  - {}: shadow='{}', search='{}' ({})",
                        field.field, sh, se, field.status
                    );
                }
            }
        }
    }

    Ok(())
}

pub async fn run_shadow_index_export(client: &Client, args: ShadowIndexExportArgs) -> Result<()> {
    let target = args.target.as_deref().unwrap();
    if target != "resources" && target != "archival-objects" && target != "digital-objects" {
        anyhow::bail!(
            "Unsupported target '{}'. Only 'resources', 'archival-objects', and 'digital-objects' are supported.",
            target
        );
    }

    let repo_id = args.repo.unwrap();

    let mut writer: Box<dyn Write> = if let Some(path) = args.output {
        Box::new(BufWriter::new(File::create(path)?))
    } else {
        Box::new(BufWriter::new(io::stdout()))
    };

    let concurrency = args.concurrency.max(1);

    if target == "resources" {
        let ids = client.resource_ids(repo_id).await?;
        let chunks: Vec<Vec<u32>> = ids
            .chunks(args.batch_size.get())
            .map(|c| c.to_vec())
            .collect();

        let mut stream = stream::iter(chunks)
            .map(|batch_ids| {
                let client = client.clone();
                async move {
                    let records = client.resources_by_ids(repo_id, &batch_ids).await;
                    (batch_ids, records)
                }
            })
            .buffered(concurrency);

        while let Some((_batch_ids, records_res)) = stream.next().await {
            let records = records_res?;
            for record in records {
                let parts: Vec<&str> = record.uri.split('/').collect();
                let id = parts
                    .last()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                let doc = ShadowDocument::from_resource(repo_id, id, record);
                let json = serde_json::to_string(&doc)?;
                writeln!(writer, "{}", json)?;
            }
        }
    } else if target == "archival-objects" {
        let ids = client.archival_object_ids(repo_id).await?;
        let chunks: Vec<Vec<u32>> = ids
            .chunks(args.batch_size.get())
            .map(|c| c.to_vec())
            .collect();

        let mut stream = stream::iter(chunks)
            .map(|batch_ids| {
                let client = client.clone();
                async move {
                    let records = client.archival_objects_by_ids(repo_id, &batch_ids).await;
                    (batch_ids, records)
                }
            })
            .buffered(concurrency);

        while let Some((_batch_ids, records_res)) = stream.next().await {
            let records = records_res?;
            for record in records {
                let parts: Vec<&str> = record.uri.split('/').collect();
                let id = parts
                    .last()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                let doc = ShadowDocument::from_archival_object(repo_id, id, record);
                let json = serde_json::to_string(&doc)?;
                writeln!(writer, "{}", json)?;
            }
        }
    } else if target == "digital-objects" {
        let ids = client.digital_object_ids(repo_id).await?;
        let chunks: Vec<Vec<u32>> = ids
            .chunks(args.batch_size.get())
            .map(|c| c.to_vec())
            .collect();

        let mut stream = stream::iter(chunks)
            .map(|batch_ids| {
                let client = client.clone();
                async move {
                    let records = client.digital_objects_by_ids(repo_id, &batch_ids).await;
                    (batch_ids, records)
                }
            })
            .buffered(concurrency);

        while let Some((_batch_ids, records_res)) = stream.next().await {
            let records = records_res?;
            for record in records {
                let parts: Vec<&str> = record.uri.split('/').collect();
                let id = parts
                    .last()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                let doc = ShadowDocument::from_digital_object(repo_id, id, record);
                let json = serde_json::to_string(&doc)?;
                writeln!(writer, "{}", json)?;
            }
        }
    }

    Ok(())
}

pub async fn run_shadow_index_solr_docs(
    client: &Client,
    args: ShadowIndexSolrDocsArgs,
) -> Result<()> {
    let target = args.target.as_str();
    if target != "archival-objects" {
        anyhow::bail!(
            "Unsupported target '{}'. Only 'archival-objects' is supported for now.",
            target
        );
    }

    let repo_id = args.repo;
    let mut ids = client.archival_object_ids(repo_id).await?;

    if args.compare_sample {
        let limit = args.limit.unwrap_or(10);
        ids.truncate(limit);
    } else if let Some(limit) = args.limit {
        ids.truncate(limit);
    }

    if args.compare_sample {
        let mut match_count = 0;
        let mut mismatch_count = 0;

        let records = client.archival_objects_by_ids(repo_id, &ids).await?;

        for record in records {
            let parts: Vec<&str> = record.uri.split('/').collect();
            let id = parts
                .last()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);

            let doc = SolrArchivalObjectDocument::from_archival_object(repo_id, id, record);

            let q = format!("id:\"{}\"", doc.uri)
                .replace('"', "%22")
                .replace('/', "%2F")
                .replace(':', "%3A");
            let search_path = format!("repositories/{}/search?q={}&page=1&page_size=1", repo_id, q);
            let response = client.get(&search_path)?.send().await?;

            let search_doc = if response.status().is_success() {
                let json = response.json::<serde_json::Value>().await?;
                if let Some(hits) = json.get("total_hits").and_then(|h| h.as_u64()) {
                    if hits > 0 {
                        json.get("results")
                            .and_then(|arr| arr.as_array())
                            .and_then(|arr| arr.first())
                            .cloned()
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let search_title = search_doc
                .as_ref()
                .and_then(|d| d.get("title"))
                .and_then(|t| t.as_str());

            let shadow_title = doc.title.as_str();

            let is_match = search_title.map(|s| s == shadow_title).unwrap_or(false);

            if is_match {
                match_count += 1;
            } else {
                mismatch_count += 1;
                if !args.json_summary {
                    println!("Mismatch for URI {}:", doc.uri);
                    println!("  Shadow: {}", shadow_title);
                    println!("  Search: {}", search_title.unwrap_or("None"));
                }
            }
        }

        if args.json_summary {
            let summary = serde_json::json!({
                "target": target,
                "sample_size": ids.len(),
                "match_count": match_count,
                "mismatch_count": mismatch_count,
            });
            println!("{}", serde_json::to_string_pretty(&summary)?);
        } else {
            println!("Target: {}", target);
            println!("Sample Size: {}", ids.len());
            println!("Matches: {}", match_count);
            println!("Mismatches: {}", mismatch_count);
        }

        return Ok(());
    }

    let mut writer: Box<dyn Write> = if let Some(path) = args.output {
        Box::new(BufWriter::new(File::create(path)?))
    } else {
        Box::new(BufWriter::new(io::stdout()))
    };

    let concurrency = args.concurrency.max(1);
    let chunks: Vec<Vec<u32>> = ids
        .chunks(args.batch_size.get())
        .map(|c| c.to_vec())
        .collect();

    let mut stream = stream::iter(chunks)
        .map(|batch_ids| {
            let client = client.clone();
            async move {
                let records = client.archival_objects_by_ids(repo_id, &batch_ids).await;
                (batch_ids, records)
            }
        })
        .buffered(concurrency);

    while let Some((_batch_ids, records_res)) = stream.next().await {
        let records = records_res?;
        for record in records {
            let parts: Vec<&str> = record.uri.split('/').collect();
            let id = parts
                .last()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let doc = SolrArchivalObjectDocument::from_archival_object(repo_id, id, record);
            let json = serde_json::to_string(&doc)?;
            writeln!(writer, "{}", json)?;
        }
    }

    Ok(())
}

pub fn make_solr_field_comparison(
    field: &str,
    shadow_val: Option<String>,
    solr_val: Option<String>,
) -> FieldComparison {
    let status = match (&shadow_val, &solr_val) {
        (Some(sh), Some(so)) if sh == so => "match",
        (None, None) => "match",
        (Some(_), None) => "solr_absent",
        _ => "mismatch",
    };
    FieldComparison {
        field: field.to_string(),
        shadow_value: shadow_val,
        search_value: solr_val,
        status: status.to_string(),
    }
}

pub async fn run_shadow_index_solr_compare(
    client: &Client,
    args: ShadowIndexSolrCompareArgs,
) -> Result<()> {
    let repo_id = args.repo;
    let target = args.target.as_str();

    if target != "archival-objects" {
        anyhow::bail!(
            "Unsupported target '{}'. Only 'archival-objects' is supported for this milestone.",
            target
        );
    }

    let mut ids = client.archival_object_ids(repo_id).await?;
    ids.truncate(args.limit);
    let sample_size = ids.len();

    let records = client.archival_objects_by_ids(repo_id, &ids).await?;

    let docs: Vec<SolrArchivalObjectDocument> = records
        .into_iter()
        .map(|r| {
            let id = r
                .uri
                .split('/')
                .next_back()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            SolrArchivalObjectDocument::from_archival_object(repo_id, id, r)
        })
        .collect();

    let reqwest_client = reqwest::Client::new();
    let mut results = Vec::new();
    let mut overall_match = true;

    for doc in docs {
        let q = format!("id:\"{}\"", doc.uri)
            .replace('\"', "%22")
            .replace('/', "%2F")
            .replace(':', "%3A");
        let solr_query_url = format!("{}/select?q={}&wt=json", args.solr_url, q);
        let response = reqwest_client.get(&solr_query_url).send().await?;

        let solr_doc = if response.status().is_success() {
            let json = response.json::<serde_json::Value>().await?;
            if let Some(hits) = json
                .get("response")
                .and_then(|r| r.get("numFound"))
                .and_then(|h| h.as_u64())
            {
                if hits > 0 {
                    json.get("response")
                        .and_then(|r| r.get("docs"))
                        .and_then(|arr| arr.as_array())
                        .and_then(|arr| arr.first())
                        .cloned()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut field_comparisons = Vec::new();

        if let Some(sdoc) = solr_doc {
            field_comparisons.push(make_solr_field_comparison(
                "id",
                Some(doc.id.clone()),
                sdoc.get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ));
            field_comparisons.push(make_solr_field_comparison(
                "uri",
                Some(doc.uri.clone()),
                sdoc.get("uri")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ));
            field_comparisons.push(make_solr_field_comparison(
                "primary_type",
                Some(doc.primary_type.clone()),
                sdoc.get("primary_type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ));
            field_comparisons.push(make_solr_field_comparison(
                "title",
                Some(doc.title.clone()),
                sdoc.get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ));
            field_comparisons.push(make_solr_field_comparison(
                "publish",
                Some(doc.publish.to_string()),
                sdoc.get("publish")
                    .and_then(|v| v.as_bool())
                    .map(|b| b.to_string()),
            ));

            if doc.parent_resource.is_some() || sdoc.get("parent_resource").is_some() {
                field_comparisons.push(make_solr_field_comparison(
                    "parent_resource",
                    doc.parent_resource.clone(),
                    sdoc.get("parent_resource")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                ));
            }

            let doc_match = field_comparisons.iter().all(|f| f.status == "match");
            if !doc_match {
                overall_match = false;
            }
            results.push(CompareFieldsResult {
                uri: doc.uri.clone(),
                match_status: if doc_match {
                    "match".to_string()
                } else {
                    "mismatch".to_string()
                },
                fields: field_comparisons,
            });
        } else {
            overall_match = false;
            results.push(CompareFieldsResult {
                uri: doc.uri.clone(),
                match_status: "missing_in_solr".to_string(),
                fields: vec![],
            });
        }
    }

    let report = CompareFieldsReport {
        repo_id,
        target: target.to_string(),
        sample_size,
        overall_status: if overall_match {
            "match".to_string()
        } else {
            "mismatch".to_string()
        },
        results,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Target: {}", report.target);
        println!("Sample Size: {}", report.sample_size);
        println!("Overall Status: {}", report.overall_status);
        println!("{}", "-".repeat(65));
        for res in &report.results {
            println!("URI: {} [{}]", res.uri, res.match_status);
            for field in &res.fields {
                if field.status != "match" {
                    let sh = field.shadow_value.as_deref().unwrap_or("None");
                    let se = field.search_value.as_deref().unwrap_or("None");
                    println!(
                        "  - {}: shadow='{}', solr='{}' ({})",
                        field.field, sh, se, field.status
                    );
                }
            }
        }
    }

    Ok(())
}

async fn fetch_solr_docs_batch(
    client: &reqwest::Client,
    url_base: &str,
    collection: &str,
    uris: &[String],
) -> Result<std::collections::BTreeMap<String, serde_json::Value>> {
    let mut map = std::collections::BTreeMap::new();
    if uris.is_empty() {
        return Ok(map);
    }

    let id_list = uris
        .iter()
        .map(|u| format!("\"{}\"", u))
        .collect::<Vec<_>>()
        .join(" ");
    let q = format!("id:({})", id_list);
    let rows = uris.len().to_string();

    let query_url = format!("{}/{}/select", url_base, collection);
    let params = [("q", q.as_str()), ("wt", "json"), ("rows", rows.as_str())];

    let response = client.post(&query_url).form(&params).send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("Solr query failed with status {}: {}", status, query_url);
    }

    let json = response.json::<serde_json::Value>().await?;
    if let Some(docs) = json
        .get("response")
        .and_then(|r| r.get("docs"))
        .and_then(|arr| arr.as_array())
    {
        for doc in docs {
            if let Some(id) = doc.get("id").and_then(|v| v.as_str()) {
                map.insert(id.to_string(), doc.clone());
            }
        }
    } else {
        anyhow::bail!("Invalid Solr response format from {}", query_url);
    }

    Ok(map)
}

pub async fn run_shadow_index_solr_shadow_compare(
    client: &Client,
    args: ShadowIndexSolrShadowCompareArgs,
) -> Result<()> {
    let repo_id = args.repo;
    let target = args.target.as_str();

    validate_target(target)?;

    if args.live_collection == args.shadow_collection {
        anyhow::bail!("Live collection and shadow collection cannot be the same.");
    }

    let mut ids = match target {
        "archival-objects" => client.archival_object_ids(repo_id).await?,
        "resources" => client.resource_ids(repo_id).await?,
        "digital-objects" => client.digital_object_ids(repo_id).await?,
        _ => unreachable!(),
    };
    ids.truncate(args.limit);
    let sample_size = ids.len();

    let uris: Vec<String> = ids
        .into_iter()
        .map(|id| {
            let type_segment = solr_target_segment(target);
            format!("/repositories/{}/{}/{}", repo_id, type_segment, id)
        })
        .collect();

    let reqwest_client = reqwest::Client::new();
    let mut results = Vec::new();
    let mut overall_match = true;

    let solr_url_base = args.solr_url.trim_end_matches('/');
    let mut processed = 0;

    if !args.json {
        eprintln!("Comparing {} {}...", uris.len(), target);
    }

    for chunk in uris.chunks(500) {
        let live_map =
            fetch_solr_docs_batch(&reqwest_client, solr_url_base, &args.live_collection, chunk)
                .await?;
        let shadow_map = fetch_solr_docs_batch(
            &reqwest_client,
            solr_url_base,
            &args.shadow_collection,
            chunk,
        )
        .await?;

        for uri in chunk {
            let live_doc = live_map.get(uri).cloned();
            let shadow_doc = shadow_map.get(uri).cloned();

            let mut field_comparisons = Vec::new();
            let mut doc_match = true;
            let match_status: String;

            match (live_doc, shadow_doc) {
                (None, None) => {
                    match_status = "missing_in_both".to_string();
                    doc_match = false;
                }
                (None, Some(_)) => {
                    match_status = "missing_in_live".to_string();
                    doc_match = false;
                }
                (Some(_), None) => {
                    match_status = "missing_in_shadow".to_string();
                    doc_match = false;
                }
                (Some(ldoc), Some(sdoc)) => {
                    let fields_to_check = ["id", "uri", "primary_type", "title", "publish"];

                    for field in fields_to_check {
                        let lval = ldoc.get(field).and_then(|v| {
                            if let Some(s) = v.as_str() {
                                Some(s.to_string())
                            } else {
                                v.as_bool().map(|b| b.to_string())
                            }
                        });
                        let sval = sdoc.get(field).and_then(|v| {
                            if let Some(s) = v.as_str() {
                                Some(s.to_string())
                            } else {
                                v.as_bool().map(|b| b.to_string())
                            }
                        });

                        let comp = make_solr_field_comparison(field, sval.clone(), lval.clone());
                        if comp.status != "match" {
                            doc_match = false;
                        }
                        field_comparisons.push(comp);
                    }
                    match_status = if doc_match {
                        "match".to_string()
                    } else {
                        "mismatch".to_string()
                    };
                }
            }

            if !doc_match {
                overall_match = false;
            }

            results.push(CompareFieldsResult {
                uri: uri.clone(),
                match_status,
                fields: field_comparisons,
            });
        }

        processed += chunk.len();
        if !args.json {
            eprintln!("Compared {}/{} documents...", processed, uris.len());
        }
    }

    let report = CompareFieldsReport {
        repo_id,
        target: target.to_string(),
        sample_size,
        overall_status: if overall_match {
            "match".to_string()
        } else {
            "mismatch".to_string()
        },
        results,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Target: {}", report.target);
        println!("Sample Size: {}", report.sample_size);
        println!("Overall Status: {}", report.overall_status);
        println!("{}", "-".repeat(65));
        let mut match_count = 0;
        let mut document_mismatch_count = 0;
        let mut missing_live = 0;
        let mut missing_shadow = 0;
        let mut missing_both = 0;
        let mut field_mismatches: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();

        for res in &report.results {
            if res.match_status == "match" {
                match_count += 1;
            } else {
                if res.match_status == "mismatch" {
                    document_mismatch_count += 1;
                }
                if res.match_status == "missing_in_live" {
                    missing_live += 1;
                }
                if res.match_status == "missing_in_shadow" {
                    missing_shadow += 1;
                }
                if res.match_status == "missing_in_both" {
                    missing_both += 1;
                }

                println!("URI: {} [{}]", res.uri, res.match_status);
                for field in &res.fields {
                    if field.status != "match" {
                        *field_mismatches.entry(field.field.as_str()).or_insert(0) += 1;
                        let sv = field.shadow_value.as_deref().unwrap_or("None");
                        let lv = field.search_value.as_deref().unwrap_or("None");
                        println!(
                            "  - {}: live='{}', shadow='{}' ({})",
                            field.field, lv, sv, field.status
                        );
                    }
                }
            }
        }
        println!("{}", "-".repeat(65));
        println!(
            "Summary: {} matches, {} documents with field mismatches, {} missing in live, {} missing in shadow, {} missing in both",
            match_count, document_mismatch_count, missing_live, missing_shadow, missing_both
        );
        if !field_mismatches.is_empty() {
            let field_summary = field_mismatches
                .iter()
                .map(|(field, count)| format!("{}={}", field, count))
                .collect::<Vec<_>>()
                .join(", ");
            println!("Field mismatches by field: {}", field_summary);
        }
    }

    Ok(())
}

pub fn validate_target(target: &str) -> Result<()> {
    if target != "archival-objects" && target != "resources" && target != "digital-objects" {
        anyhow::bail!(
            "Unsupported target '{}'. Only 'archival-objects', 'resources', and 'digital-objects' are supported.",
            target
        );
    }
    Ok(())
}

pub fn solr_target_segment(target: &str) -> &'static str {
    match target {
        "archival-objects" => "archival_objects",
        "resources" => "resources",
        "digital-objects" => "digital_objects",
        _ => unreachable!(),
    }
}

pub fn solr_configset_for_shadow_collection(collection: &str) -> &'static str {
    if collection == "archivesspace_shadow" {
        "archivesspace_shadow"
    } else {
        "archivesspace"
    }
}

pub fn extract_ids_from_uris(uris: &[String], repo_id: u32, target: &str) -> Result<Vec<u32>> {
    let target_segment = solr_target_segment(target);
    let expected_prefix = format!("/repositories/{}/{}/", repo_id, target_segment);

    let mut ids = Vec::with_capacity(uris.len());
    for uri in uris {
        let trimmed = uri.trim();
        if !trimmed.starts_with(&expected_prefix) {
            anyhow::bail!(
                "URI '{}' does not belong to the selected repo {} and target '{}' (expected prefix: {})",
                trimmed, repo_id, target, expected_prefix
            );
        }

        let id_part = trimmed.split('/').next_back().unwrap_or("0");
        let id: u32 = id_part
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid URI format, ID not numeric: {}", trimmed))?;
        ids.push(id);
    }
    Ok(ids)
}

pub fn parse_and_validate_uris(
    content: &str,
    repo_id: u32,
    target: &str,
) -> Result<(Vec<String>, Vec<u32>)> {
    let raw_uris: Vec<String> = content
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let ids = extract_ids_from_uris(&raw_uris, repo_id, target)?;
    Ok((raw_uris, ids))
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) as i64
}

fn parse_two_digits(value: &str, start: usize, label: &str) -> Result<u32> {
    value
        .get(start..start + 2)
        .ok_or_else(|| anyhow::anyhow!("Invalid RFC3339 timestamp: missing {}", label))?
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("Invalid RFC3339 timestamp: bad {}", label))
}

fn parse_rfc3339_epoch_seconds(value: &str) -> Result<i64> {
    if value.len() < 20 {
        anyhow::bail!("Invalid RFC3339 timestamp '{}'", value);
    }
    if value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || value.as_bytes().get(10) != Some(&b'T')
        || value.as_bytes().get(13) != Some(&b':')
        || value.as_bytes().get(16) != Some(&b':')
    {
        anyhow::bail!("Invalid RFC3339 timestamp '{}'", value);
    }

    let year = value[0..4]
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("Invalid RFC3339 timestamp: bad year"))?;
    let month = parse_two_digits(value, 5, "month")?;
    let day = parse_two_digits(value, 8, "day")?;
    let hour = parse_two_digits(value, 11, "hour")?;
    let minute = parse_two_digits(value, 14, "minute")?;
    let second = parse_two_digits(value, 17, "second")?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        anyhow::bail!("Invalid RFC3339 timestamp '{}'", value);
    }

    let mut tz_start = 19;
    if value.as_bytes().get(tz_start) == Some(&b'.') {
        tz_start += 1;
        while value
            .as_bytes()
            .get(tz_start)
            .is_some_and(|b| b.is_ascii_digit())
        {
            tz_start += 1;
        }
    }

    let tz = value
        .get(tz_start..)
        .ok_or_else(|| anyhow::anyhow!("Invalid RFC3339 timestamp: missing timezone"))?;
    let offset_seconds = if tz == "Z" {
        0
    } else if tz.len() == 6
        && (tz.as_bytes()[0] == b'+' || tz.as_bytes()[0] == b'-')
        && tz.as_bytes()[3] == b':'
    {
        let sign = if tz.as_bytes()[0] == b'+' { 1 } else { -1 };
        let offset_hour = tz[1..3]
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("Invalid RFC3339 timestamp: bad offset hour"))?;
        let offset_minute = tz[4..6]
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("Invalid RFC3339 timestamp: bad offset minute"))?;
        if offset_hour > 23 || offset_minute > 59 {
            anyhow::bail!("Invalid RFC3339 timestamp '{}'", value);
        }
        sign * (offset_hour * 3600 + offset_minute * 60)
    } else {
        anyhow::bail!("Invalid RFC3339 timestamp: missing timezone");
    };

    let local_seconds = days_from_civil(year, month, day) * 86_400
        + i64::from(hour) * 3600
        + i64::from(minute) * 60
        + i64::from(second);
    Ok(local_seconds - offset_seconds)
}

fn is_changed_since(
    uri: &str,
    system_mtime: Option<&str>,
    user_mtime: Option<&str>,
    since_epoch: i64,
) -> Result<bool> {
    for mtime in [system_mtime, user_mtime].into_iter().flatten() {
        if parse_rfc3339_epoch_seconds(mtime)
            .with_context(|| format!("Failed to parse mtime for {}", uri))?
            > since_epoch
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn resource_changed_uri(record: &Resource, since_epoch: i64) -> Result<Option<String>> {
    if is_changed_since(
        &record.uri,
        record.system_mtime.as_deref(),
        record.user_mtime.as_deref(),
        since_epoch,
    )? {
        Ok(Some(record.uri.clone()))
    } else {
        Ok(None)
    }
}

fn archival_object_changed_uri(
    record: &ArchivalObject,
    since_epoch: i64,
) -> Result<Option<String>> {
    if is_changed_since(
        &record.uri,
        record.system_mtime.as_deref(),
        record.user_mtime.as_deref(),
        since_epoch,
    )? {
        Ok(Some(record.uri.clone()))
    } else {
        Ok(None)
    }
}

fn digital_object_changed_uri(record: &DigitalObject, since_epoch: i64) -> Result<Option<String>> {
    if is_changed_since(
        &record.uri,
        record.system_mtime.as_deref(),
        record.user_mtime.as_deref(),
        since_epoch,
    )? {
        Ok(Some(record.uri.clone()))
    } else {
        Ok(None)
    }
}

pub async fn fetch_changed_uris(
    client: &Client,
    repo_id: u32,
    target: &str,
    since_epoch: i64,
    batch_size: usize,
    concurrency: usize,
) -> Result<Vec<String>> {
    let ids = match target {
        "resources" => client.resource_ids(repo_id).await?,
        "archival-objects" => client.archival_object_ids(repo_id).await?,
        "digital-objects" => client.digital_object_ids(repo_id).await?,
        _ => unreachable!(),
    };
    let chunks: Vec<Vec<u32>> = ids.chunks(batch_size.max(1)).map(|c| c.to_vec()).collect();
    let concurrency = concurrency.max(1);

    let mut changed_uris = Vec::new();
    match target {
        "resources" => {
            let mut stream = stream::iter(chunks)
                .map(|batch_ids| {
                    let client = client.clone();
                    async move { client.resources_by_ids(repo_id, &batch_ids).await }
                })
                .buffered(concurrency);
            while let Some(records) = stream.next().await {
                for record in records? {
                    if let Some(uri) = resource_changed_uri(&record, since_epoch)? {
                        changed_uris.push(uri);
                    }
                }
            }
        }
        "archival-objects" => {
            let mut stream = stream::iter(chunks)
                .map(|batch_ids| {
                    let client = client.clone();
                    async move { client.archival_objects_by_ids(repo_id, &batch_ids).await }
                })
                .buffered(concurrency);
            while let Some(records) = stream.next().await {
                for record in records? {
                    if let Some(uri) = archival_object_changed_uri(&record, since_epoch)? {
                        changed_uris.push(uri);
                    }
                }
            }
        }
        "digital-objects" => {
            let mut stream = stream::iter(chunks)
                .map(|batch_ids| {
                    let client = client.clone();
                    async move { client.digital_objects_by_ids(repo_id, &batch_ids).await }
                })
                .buffered(concurrency);
            while let Some(records) = stream.next().await {
                for record in records? {
                    if let Some(uri) = digital_object_changed_uri(&record, since_epoch)? {
                        changed_uris.push(uri);
                    }
                }
            }
        }
        _ => unreachable!(),
    }

    changed_uris.sort();
    changed_uris.dedup();
    Ok(changed_uris)
}

pub async fn run_shadow_index_changed_uris(
    client: &Client,
    args: ShadowIndexChangedUrisArgs,
) -> Result<()> {
    validate_target(&args.target)?;

    let since_epoch = parse_rfc3339_epoch_seconds(&args.since)?;
    let changed_uris = fetch_changed_uris(
        client,
        args.repo,
        &args.target,
        since_epoch,
        args.batch_size.get(),
        args.concurrency,
    )
    .await?;

    let mut writer: Box<dyn Write> = if let Some(path) = args.output {
        Box::new(BufWriter::new(File::create(path)?))
    } else {
        Box::new(BufWriter::new(io::stdout()))
    };
    for uri in changed_uris {
        writeln!(writer, "{}", uri)?;
    }
    writer.flush()?;
    Ok(())
}

pub async fn run_shadow_index_solr_load_shadow(
    client: &Client,
    args: ShadowIndexSolrLoadShadowArgs,
) -> Result<()> {
    // 1. Check requirements
    validate_target(&args.target)?;
    if args.collection == "archivesspace" {
        anyhow::bail!("Refusing to operate on the live 'archivesspace' collection.");
    }

    let repo_id = args.repo;
    let solr_url = args.solr_url.trim_end_matches('/');
    let collection = &args.collection;
    let reqwest_client = reqwest::Client::new();

    // 2. Create if missing
    if args.create_if_missing {
        // Simple heuristic: try to ping the collection via admin ping or just select
        let select_url = format!("{}/{}/select?q=*:*&rows=0", solr_url, collection);
        let resp = reqwest_client.get(&select_url).send().await;
        if resp.is_err() || !resp.unwrap().status().is_success() {
            println!(
                "Collection '{}' might be missing, attempting to create...",
                collection
            );
            // Solr create core command. The default shadow collection has its own
            // configset so richer field experiments stay isolated from live Solr.
            let config_set = solr_configset_for_shadow_collection(collection);
            let create_url = format!(
                "{}/admin/cores?action=CREATE&name={}&instanceDir={}&configSet={}",
                solr_url, collection, collection, config_set
            );
            let create_resp = reqwest_client.get(&create_url).send().await?;
            if !create_resp.status().is_success() {
                let status = create_resp.status();
                let txt = create_resp.text().await.unwrap_or_default();
                anyhow::bail!("Failed to create collection/core '{}'. It may require manual setup. Status: {}, Response: {}", collection, status, txt);
            }
            println!("Successfully created collection '{}'.", collection);
        } else {
            println!("Collection '{}' already exists.", collection);
        }
    }

    // 3. Replace shadow
    if args.replace_shadow && args.replace_target {
        anyhow::bail!("Cannot supply both --replace-shadow and --replace-target. Choose one.");
    }

    if args.replace_shadow {
        println!("Deleting all documents from '{}'...", collection);
        let update_url = format!("{}/{}/update?commit=true", solr_url, collection);
        let delete_payload = serde_json::json!({"delete": {"query": "*:*"}});
        let resp = reqwest_client
            .post(&update_url)
            .json(&delete_payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to clear collection. Status: {}, Response: {}",
                status,
                txt
            );
        }
        println!("Collection cleared.");
    } else if args.replace_target {
        let segment = solr_target_segment(&args.target);
        println!(
            "Deleting documents for repo {} target '{}' from '{}'...",
            repo_id, args.target, collection
        );
        let update_url = format!("{}/{}/update?commit=true", solr_url, collection);
        let query = format!("id:\\/repositories\\/{}\\/{}\\/*", repo_id, segment);
        let delete_payload = serde_json::json!({"delete": {"query": query}});
        let resp = reqwest_client
            .post(&update_url)
            .json(&delete_payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to clear target docs. Status: {}, Response: {}",
                status,
                txt
            );
        }
        println!("Target docs cleared.");
    }

    // 4. Load docs
    let mut ids = match args.target.as_str() {
        "archival-objects" => client.archival_object_ids(repo_id).await?,
        "resources" => client.resource_ids(repo_id).await?,
        "digital-objects" => client.digital_object_ids(repo_id).await?,
        _ => unreachable!(),
    };

    ids.truncate(args.limit);
    println!("Fetching {} {}...", ids.len(), args.target);

    let mut total_loaded = 0;
    for chunk in ids.chunks(250) {
        let docs: Vec<SafeSolrDocument> = match args.target.as_str() {
            "archival-objects" => {
                let records = client
                    .archival_objects_by_ids_with_top_containers(repo_id, chunk)
                    .await?;
                records
                    .into_iter()
                    .map(|r| {
                        let id = r
                            .uri
                            .split('/')
                            .next_back()
                            .unwrap_or("0")
                            .parse()
                            .unwrap_or(0);
                        SafeSolrDocument::from(SolrArchivalObjectDocument::from_archival_object(
                            repo_id, id, r,
                        ))
                    })
                    .collect()
            }
            "resources" => {
                let records = client.resources_by_ids(repo_id, chunk).await?;
                records
                    .into_iter()
                    .map(SafeSolrDocument::from_resource)
                    .collect()
            }
            "digital-objects" => {
                let records = client.digital_objects_by_ids(repo_id, chunk).await?;
                records
                    .into_iter()
                    .map(SafeSolrDocument::from_digital_object)
                    .collect()
            }
            _ => unreachable!(),
        };

        let update_url = format!("{}/{}/update?commit=true", solr_url, collection);
        let resp = reqwest_client.post(&update_url).json(&docs).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            anyhow::bail!("Solr update failed. Status: {}, Error: {}", status, txt);
        }
        total_loaded += docs.len();
        println!("Loaded {}/{} documents...", total_loaded, ids.len());
    }

    println!("Successfully loaded documents.");

    // 5. Query shadow collection count
    let select_url = format!("{}/{}/select?q=*:*&rows=0&wt=json", solr_url, collection);
    let resp = reqwest_client.get(&select_url).send().await?;
    if resp.status().is_success() {
        let json: serde_json::Value = resp.json().await.unwrap();
        if let Some(num_found) = json
            .get("response")
            .and_then(|r| r.get("numFound"))
            .and_then(|n| n.as_u64())
        {
            println!(
                "Shadow collection '{}' now contains {} documents (loaded_count={}).",
                collection, num_found, total_loaded
            );
        } else {
            println!("Could not read numFound from Solr response.");
        }
    } else {
        println!(
            "Could not query collection count after load. Status: {}",
            resp.status()
        );
    }

    Ok(())
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SolrSchemaFieldSpec {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub field_type: &'static str,
    pub stored: bool,
    pub indexed: bool,
    #[serde(rename = "multiValued")]
    pub multi_valued: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SolrSchemaEnsureReport {
    pub collection: String,
    pub existing_fields: Vec<String>,
    pub added_fields: Vec<String>,
}

pub fn rich_archival_object_schema_fields() -> Vec<SolrSchemaFieldSpec> {
    vec![
        SolrSchemaFieldSpec {
            name: "display_string",
            field_type: "text_general",
            stored: true,
            indexed: true,
            multi_valued: false,
        },
        SolrSchemaFieldSpec {
            name: "date_expressions",
            field_type: "string",
            stored: true,
            indexed: true,
            multi_valued: true,
        },
        SolrSchemaFieldSpec {
            name: "resource_uri",
            field_type: "string",
            stored: true,
            indexed: true,
            multi_valued: false,
        },
        SolrSchemaFieldSpec {
            name: "parent_uri",
            field_type: "string",
            stored: true,
            indexed: true,
            multi_valued: false,
        },
        SolrSchemaFieldSpec {
            name: "ancestor_uris",
            field_type: "string",
            stored: true,
            indexed: true,
            multi_valued: true,
        },
        SolrSchemaFieldSpec {
            name: "ancestor_levels",
            field_type: "string",
            stored: true,
            indexed: true,
            multi_valued: true,
        },
        SolrSchemaFieldSpec {
            name: "position_i",
            field_type: "pint",
            stored: true,
            indexed: true,
            multi_valued: false,
        },
    ]
}

pub fn solr_add_field_payload(field: &SolrSchemaFieldSpec) -> serde_json::Value {
    serde_json::json!({ "add-field": field })
}

async fn solr_schema_field_exists(
    reqwest_client: &reqwest::Client,
    solr_url: &str,
    collection: &str,
    field_name: &str,
) -> Result<bool> {
    let field_url = format!(
        "{}/{}/schema/fields/{}",
        solr_url.trim_end_matches('/'),
        collection,
        field_name
    );
    let resp = reqwest_client.get(&field_url).send().await?;
    if resp.status().is_success() {
        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        Ok(json
            .get("field")
            .and_then(|field| field.get("name"))
            .and_then(|name| name.as_str())
            == Some(field_name))
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
        Ok(false)
    } else {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Failed to inspect Solr schema field '{}'. Status: {}, Response: {}",
            field_name,
            status,
            txt
        );
    }
}

pub async fn run_shadow_index_solr_ensure_shadow_schema(
    args: ShadowIndexSolrEnsureShadowSchemaArgs,
) -> Result<()> {
    if args.collection == "archivesspace" {
        anyhow::bail!("Refusing to operate on the live 'archivesspace' collection.");
    }

    let solr_url = args.solr_url.trim_end_matches('/');
    let collection = &args.collection;
    let reqwest_client = reqwest::Client::new();
    let mut existing_fields = Vec::new();
    let mut added_fields = Vec::new();

    for field in rich_archival_object_schema_fields() {
        if solr_schema_field_exists(&reqwest_client, solr_url, collection, field.name).await? {
            existing_fields.push(field.name.to_string());
            continue;
        }

        let schema_url = format!("{}/{}/schema", solr_url, collection);
        let payload = solr_add_field_payload(&field);
        let resp = reqwest_client
            .post(&schema_url)
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to add Solr schema field '{}'. Status: {}, Response: {}",
                field.name,
                status,
                txt
            );
        }
        added_fields.push(field.name.to_string());
    }

    let report = SolrSchemaEnsureReport {
        collection: collection.to_string(),
        existing_fields,
        added_fields,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Collection: {}", report.collection);
        println!("Existing fields: {}", report.existing_fields.join(", "));
        println!("Added fields: {}", report.added_fields.join(", "));
    }

    Ok(())
}

pub async fn delete_solr_exact_uris(
    reqwest_client: &reqwest::Client,
    solr_url: &str,
    collection: &str,
    uris: &[String],
) -> Result<()> {
    if uris.is_empty() {
        return Ok(());
    }

    let id_terms: Vec<String> = uris.iter().map(|uri| format!("\"{}\"", uri)).collect();
    let query = format!("id:({})", id_terms.join(" OR "));
    let delete_payload = serde_json::json!({"delete": {"query": query}});

    let update_url = format!(
        "{}/{}/update?commit=true",
        solr_url.trim_end_matches('/'),
        collection
    );
    let resp = reqwest_client
        .post(&update_url)
        .json(&delete_payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Failed to delete exact URIs. Status: {}, Response: {}",
            status,
            txt
        );
    }
    Ok(())
}

pub async fn load_records_to_solr_by_ids(
    client: &Client,
    reqwest_client: &reqwest::Client,
    solr_url: &str,
    collection: &str,
    repo_id: u32,
    target: &str,
    ids: &[u32],
) -> Result<usize> {
    let update_url = format!(
        "{}/{}/update?commit=true",
        solr_url.trim_end_matches('/'),
        collection
    );
    let mut total_loaded = 0;
    for chunk in ids.chunks(250) {
        let docs: Vec<SafeSolrDocument> = match target {
            "archival-objects" => {
                let records = client
                    .archival_objects_by_ids_with_top_containers(repo_id, chunk)
                    .await?;
                records
                    .into_iter()
                    .map(|r| {
                        let id = r
                            .uri
                            .split('/')
                            .next_back()
                            .unwrap_or("0")
                            .parse()
                            .unwrap_or(0);
                        SafeSolrDocument::from(SolrArchivalObjectDocument::from_archival_object(
                            repo_id, id, r,
                        ))
                    })
                    .collect()
            }
            "resources" => {
                let records = client.resources_by_ids(repo_id, chunk).await?;
                records
                    .into_iter()
                    .map(SafeSolrDocument::from_resource)
                    .collect()
            }
            "digital-objects" => {
                let records = client.digital_objects_by_ids(repo_id, chunk).await?;
                records
                    .into_iter()
                    .map(SafeSolrDocument::from_digital_object)
                    .collect()
            }
            _ => unreachable!(),
        };

        if docs.is_empty() {
            continue;
        }

        let chunk_resp = reqwest_client.post(&update_url).json(&docs).send().await?;

        if !chunk_resp.status().is_success() {
            let status = chunk_resp.status();
            let txt = chunk_resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to load chunk into Solr. Status: {}, Response: {}",
                status,
                txt
            );
        }

        total_loaded += docs.len();
    }
    Ok(total_loaded)
}

pub async fn run_shadow_index_solr_reindex_uris(
    client: &Client,
    args: ShadowIndexSolrReindexUrisArgs,
) -> Result<()> {
    validate_target(&args.target)?;
    if args.collection == "archivesspace" {
        anyhow::bail!("Refusing to operate on the live 'archivesspace' collection.");
    }

    let repo_id = args.repo;
    let content = std::fs::read_to_string(&args.uri_file).map_err(|e| {
        anyhow::anyhow!("Failed to read URI file {}: {}", args.uri_file.display(), e)
    })?;

    let (uris, ids) = parse_and_validate_uris(&content, repo_id, &args.target)?;

    if uris.is_empty() {
        println!("URI file is empty. Nothing to do.");
        return Ok(());
    }

    let solr_url = args.solr_url.trim_end_matches('/');
    let collection = &args.collection;
    let reqwest_client = reqwest::Client::new();

    println!(
        "Deleting exactly {} documents from '{}'...",
        uris.len(),
        collection
    );
    delete_solr_exact_uris(&reqwest_client, solr_url, collection, &uris).await?;
    println!("Deleted exact URIs.");

    println!("Fetching {} {}...", ids.len(), args.target);
    let total_loaded = load_records_to_solr_by_ids(
        client,
        &reqwest_client,
        solr_url,
        collection,
        repo_id,
        &args.target,
        &ids,
    )
    .await?;
    println!("Successfully loaded {} documents.", total_loaded);

    Ok(())
}

pub async fn run_shadow_index_incremental_reindex(
    client: &Client,
    args: ShadowIndexIncrementalReindexArgs,
) -> Result<()> {
    validate_target(&args.target)?;
    if args.collection == "archivesspace" {
        anyhow::bail!("Refusing to operate on the live 'archivesspace' collection.");
    }
    let since_epoch = parse_rfc3339_epoch_seconds(&args.since)?;

    let start = std::time::Instant::now();

    let changed_uris = fetch_changed_uris(
        client,
        args.repo,
        &args.target,
        since_epoch,
        args.batch_size.get(),
        args.concurrency,
    )
    .await?;

    if changed_uris.is_empty() {
        let elapsed_time_sec = start.elapsed().as_secs_f64();
        println!("No changed URIs found since {}.", args.since);
        println!(
            "Summary: changed_count=0, loaded_count=0, elapsed_time={:.2}s",
            elapsed_time_sec
        );
        return Ok(());
    }

    let mut ids = extract_ids_from_uris(&changed_uris, args.repo, &args.target)?;
    ids.sort_unstable();
    ids.dedup();

    let solr_url = args.solr_url.trim_end_matches('/');
    let collection = &args.collection;
    let reqwest_client = reqwest::Client::new();

    println!(
        "Deleting exactly {} documents from '{}'...",
        changed_uris.len(),
        collection
    );
    delete_solr_exact_uris(&reqwest_client, solr_url, collection, &changed_uris).await?;
    println!("Deleted exact URIs.");

    println!("Fetching {} {}...", ids.len(), args.target);
    let loaded_count = load_records_to_solr_by_ids(
        client,
        &reqwest_client,
        solr_url,
        collection,
        args.repo,
        &args.target,
        &ids,
    )
    .await?;
    println!("Successfully loaded {} documents.", loaded_count);

    let elapsed_time_sec = start.elapsed().as_secs_f64();
    println!(
        "Summary: changed_count={}, loaded_count={}, elapsed_time={:.2}s",
        changed_uris.len(),
        loaded_count,
        elapsed_time_sec
    );

    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldMismatchSample {
    pub uri: String,
    pub shadow_value: Option<String>,
    pub solr_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct FieldMismatchSummary {
    pub count: usize,
    pub samples: Vec<FieldMismatchSample>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShadowIndexParityReport {
    pub repo_id: u32,
    pub target: String,
    pub total_records: usize,
    pub matched_records: usize,
    pub mismatched_records: usize,
    pub missing_in_solr: usize,
    pub field_mismatches: BTreeMap<String, FieldMismatchSummary>,
}

pub fn record_field_mismatch(
    field_mismatches: &mut BTreeMap<String, FieldMismatchSummary>,
    field: &str,
    uri: &str,
    shadow_val: Option<String>,
    solr_val: Option<String>,
    max_samples: usize,
) {
    let entry = field_mismatches.entry(field.to_string()).or_default();
    entry.count += 1;
    if entry.samples.len() < max_samples {
        entry.samples.push(FieldMismatchSample {
            uri: uri.to_string(),
            shadow_value: shadow_val,
            solr_value: solr_val,
        });
    }
}

pub fn solr_value_as_string(val: Option<&serde_json::Value>) -> Option<String> {
    match val {
        Some(serde_json::Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(serde_json::Value::Array(arr)) => {
            arr.first().and_then(|v| solr_value_as_string(Some(v)))
        }
        Some(serde_json::Value::Bool(b)) => Some(b.to_string()),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

pub fn compare_vec_field(
    field: &str,
    shadow_values: &[String],
    solr_json: Option<&serde_json::Value>,
) -> Option<(String, Option<String>, Option<String>)> {
    let shadow_sorted = sorted_vec(shadow_values);
    let solr_sorted = solr_json.map(search_value_as_vec).unwrap_or_default();

    if shadow_sorted == solr_sorted {
        None
    } else {
        let sh_str = if shadow_sorted.is_empty() {
            None
        } else {
            Some(shadow_sorted.join("|"))
        };
        let so_str = if solr_sorted.is_empty() {
            None
        } else {
            Some(solr_sorted.join("|"))
        };
        Some((field.to_string(), sh_str, so_str))
    }
}

pub fn compare_shadow_doc_with_solr(
    doc: &ShadowDocument,
    sdoc: &serde_json::Value,
) -> Vec<(String, Option<String>, Option<String>)> {
    let mut mismatches = Vec::new();

    let scalar_fields: &[(&str, Option<String>, Option<&serde_json::Value>)] = &[
        ("uri", Some(doc.uri.clone()), sdoc.get("uri")),
        (
            "primary_type",
            Some(doc.primary_type.clone()),
            sdoc.get("primary_type"),
        ),
        (
            "title",
            doc.title
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("title"),
        ),
        (
            "publish",
            Some(doc.publish.to_string()),
            sdoc.get("publish"),
        ),
        (
            "parent_resource",
            doc.parent_resource
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("parent_resource"),
        ),
        (
            "level",
            doc.level
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("level"),
        ),
        (
            "ref_id",
            doc.ref_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("ref_id"),
        ),
        (
            "component_id",
            doc.component_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("component_id"),
        ),
        (
            "position_i",
            doc.position_i.map(|v| v.to_string()),
            sdoc.get("position_i"),
        ),
        (
            "display_string",
            doc.display_string
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("display_string"),
        ),
        (
            "resource_uri",
            doc.resource_uri
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("resource_uri"),
        ),
        (
            "parent_uri",
            doc.parent_uri
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("parent_uri"),
        ),
    ];

    for (field, shadow_val, solr_json) in scalar_fields {
        let solr_val = solr_value_as_string(*solr_json);
        if *shadow_val != solr_val {
            mismatches.push((field.to_string(), shadow_val.clone(), solr_val));
        }
    }

    let vec_fields: &[(&str, &[String], Option<&serde_json::Value>)] = &[
        (
            "date_expressions",
            &doc.date_expressions,
            sdoc.get("date_expressions"),
        ),
        ("subject_uris", &doc.subject_uris, sdoc.get("subject_uris")),
        (
            "linked_agent_uris",
            &doc.linked_agent_uris,
            sdoc.get("linked_agent_uris"),
        ),
        (
            "linked_agent_roles",
            &doc.linked_agent_roles,
            sdoc.get("linked_agent_roles"),
        ),
        (
            "linked_agent_relators",
            &doc.linked_agent_relators,
            sdoc.get("linked_agent_relators"),
        ),
        (
            "linked_agent_titles",
            &doc.linked_agent_titles,
            sdoc.get("linked_agent_titles"),
        ),
        (
            "ancestor_uris",
            &doc.ancestor_uris,
            sdoc.get("ancestor_uris"),
        ),
        (
            "ancestor_titles",
            &doc.ancestor_titles,
            sdoc.get("ancestor_titles"),
        ),
        (
            "ancestor_levels",
            &doc.ancestor_levels,
            sdoc.get("ancestor_levels"),
        ),
    ];

    for (field, shadow_vals, solr_json) in vec_fields {
        if let Some(mismatch) = compare_vec_field(field, shadow_vals, *solr_json) {
            mismatches.push(mismatch);
        }
    }

    mismatches
}

pub fn compare_safe_solr_doc_with_solr(
    doc: &SafeSolrDocument,
    sdoc: &serde_json::Value,
) -> Vec<(String, Option<String>, Option<String>)> {
    let mut mismatches = Vec::new();

    let scalar_fields: &[(&str, Option<String>, Option<&serde_json::Value>)] = &[
        ("uri", Some(doc.uri.clone()), sdoc.get("uri")),
        (
            "primary_type",
            Some(doc.primary_type.clone()),
            sdoc.get("primary_type"),
        ),
        (
            "title",
            Some(doc.title.trim().to_string()).filter(|s| !s.is_empty()),
            sdoc.get("title"),
        ),
        (
            "publish",
            Some(doc.publish.to_string()),
            sdoc.get("publish"),
        ),
        (
            "level",
            doc.level
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("level"),
        ),
        (
            "ref_id",
            doc.ref_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("ref_id"),
        ),
        (
            "component_id",
            doc.component_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("component_id"),
        ),
        (
            "position_i",
            doc.position_i.map(|v| v.to_string()),
            sdoc.get("position_i"),
        ),
        (
            "display_string",
            doc.display_string
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("display_string"),
        ),
        (
            "resource_uri",
            doc.resource_uri
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("resource_uri"),
        ),
        (
            "parent_uri",
            doc.parent_uri
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sdoc.get("parent_uri"),
        ),
    ];

    for (field, shadow_val, solr_json) in scalar_fields {
        let solr_val = solr_value_as_string(*solr_json);
        if *shadow_val != solr_val {
            mismatches.push((field.to_string(), shadow_val.clone(), solr_val));
        }
    }

    let vec_fields: &[(&str, &[String], Option<&serde_json::Value>)] = &[
        (
            "date_expressions",
            &doc.date_expressions,
            sdoc.get("date_expressions"),
        ),
        ("subject_uris", &doc.subject_uris, sdoc.get("subject_uris")),
        (
            "linked_agent_uris",
            &doc.linked_agent_uris,
            sdoc.get("linked_agent_uris"),
        ),
        (
            "linked_agent_roles",
            &doc.linked_agent_roles,
            sdoc.get("linked_agent_roles"),
        ),
        (
            "linked_agent_relators",
            &doc.linked_agent_relators,
            sdoc.get("linked_agent_relators"),
        ),
        (
            "linked_agent_titles",
            &doc.linked_agent_titles,
            sdoc.get("linked_agent_titles"),
        ),
        (
            "ancestor_uris",
            &doc.ancestor_uris,
            sdoc.get("ancestor_uris"),
        ),
        (
            "ancestor_titles",
            &doc.ancestor_titles,
            sdoc.get("ancestor_titles"),
        ),
        (
            "ancestor_levels",
            &doc.ancestor_levels,
            sdoc.get("ancestor_levels"),
        ),
    ];

    for (field, shadow_vals, solr_json) in vec_fields {
        if let Some(mismatch) = compare_vec_field(field, shadow_vals, *solr_json) {
            mismatches.push(mismatch);
        }
    }

    mismatches
}

async fn fetch_live_solr_doc(
    client: &reqwest::Client,
    solr_url: &str,
    live_collection: &str,
    uri: &str,
) -> Result<Option<serde_json::Value>> {
    let select_url = format!(
        "{}/{}/select",
        solr_url.trim_end_matches('/'),
        live_collection
    );
    let q = format!("id:\"{}\"", uri);
    let response = client
        .get(&select_url)
        .query(&[("q", q.as_str()), ("wt", "json"), ("rows", "1")])
        .send()
        .await
        .with_context(|| format!("Failed to query Solr at {}", select_url))?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Solr query failed with status {}: {}",
            response.status(),
            select_url
        );
    }

    let json: serde_json::Value = response.json().await?;
    let doc = json
        .get("response")
        .and_then(|r| r.get("docs"))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.first())
        .cloned();

    Ok(doc)
}

pub async fn run_shadow_index_parity_report(
    client: &Client,
    args: ShadowIndexParityReportArgs,
) -> Result<()> {
    if args.target != "archival-objects" {
        anyhow::bail!(
            "Unsupported target '{}'. Only 'archival-objects' is supported.",
            args.target
        );
    }

    let repo_id = args.repo;
    let mut ids = client.archival_object_ids(repo_id).await?;
    if let Some(limit) = args.limit {
        ids.truncate(limit);
    }

    let batch_size = args.batch_size.get();
    let reqwest_client = reqwest::Client::new();
    let solr_url = args.solr_url.trim_end_matches('/');
    let live_collection = &args.live_collection;

    let total_records = ids.len();
    let mut matched_records = 0;
    let mut mismatched_records = 0;
    let mut missing_in_solr = 0;
    let mut field_mismatches: BTreeMap<String, FieldMismatchSummary> = BTreeMap::new();

    for chunk in ids.chunks(batch_size) {
        let records = client.archival_objects_by_ids(repo_id, chunk).await?;
        for record in records {
            let parts: Vec<&str> = record.uri.split('/').collect();
            let id = parts
                .last()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let doc = SafeSolrDocument::from(SolrArchivalObjectDocument::from_archival_object(
                repo_id, id, record,
            ));

            let sdoc =
                fetch_live_solr_doc(&reqwest_client, solr_url, live_collection, &doc.uri).await?;
            match sdoc {
                Some(ref sdoc_json) => {
                    let doc_mismatches = compare_safe_solr_doc_with_solr(&doc, sdoc_json);
                    if doc_mismatches.is_empty() {
                        matched_records += 1;
                    } else {
                        mismatched_records += 1;
                        for (field, sh_val, so_val) in doc_mismatches {
                            record_field_mismatch(
                                &mut field_mismatches,
                                &field,
                                &doc.uri,
                                sh_val,
                                so_val,
                                10,
                            );
                        }
                    }
                }
                None => {
                    missing_in_solr += 1;
                    mismatched_records += 1;
                    record_field_mismatch(
                        &mut field_mismatches,
                        "missing_in_solr",
                        &doc.uri,
                        Some(doc.uri.clone()),
                        None,
                        10,
                    );
                }
            }
        }
    }

    let report = ShadowIndexParityReport {
        repo_id,
        target: args.target,
        total_records,
        matched_records,
        mismatched_records,
        missing_in_solr,
        field_mismatches,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Target: {}", report.target);
        println!("Total Records: {}", report.total_records);
        println!("Matched Records: {}", report.matched_records);
        println!("Mismatched Records: {}", report.mismatched_records);
        println!("Missing in Solr: {}", report.missing_in_solr);
        println!("{}", "-".repeat(65));
        if report.field_mismatches.is_empty() {
            println!("No field mismatches found.");
        } else {
            println!("Field Mismatches (by field):");
            for (field, summary) in &report.field_mismatches {
                println!("  - {}: {} mismatch(es)", field, summary.count);
                for sample in &summary.samples {
                    let sh = sample.shadow_value.as_deref().unwrap_or("None");
                    let so = sample.solr_value.as_deref().unwrap_or("None");
                    println!("      [{}] shadow='{}' vs solr='{}'", sample.uri, sh, so);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aspace_client::models::Ref;

    #[test]
    fn test_parse_and_validate_uris_trims_and_ignores_blank_lines() {
        let content = "
            /repositories/2/resources/1

            /repositories/2/resources/2
        ";
        let (uris, ids) = parse_and_validate_uris(content, 2, "resources").unwrap();
        assert_eq!(
            uris,
            vec!["/repositories/2/resources/1", "/repositories/2/resources/2"]
        );
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn test_parse_and_validate_uris_rejects_wrong_target() {
        let content = "/repositories/2/resources_bad/1";
        let err = parse_and_validate_uris(content, 2, "resources").unwrap_err();
        assert!(err
            .to_string()
            .contains("does not belong to the selected repo"));
    }

    #[test]
    fn test_parse_and_validate_uris_rejects_nonnumeric_id() {
        let content = "/repositories/2/resources/abc";
        let err = parse_and_validate_uris(content, 2, "resources").unwrap_err();
        assert!(err.to_string().contains("ID not numeric"));
    }

    #[test]
    fn test_parse_rfc3339_epoch_seconds_handles_offsets() {
        let zulu = parse_rfc3339_epoch_seconds("2026-10-07T10:00:00Z").unwrap();
        let plus_one = parse_rfc3339_epoch_seconds("2026-10-07T11:00:00+01:00").unwrap();
        let fractional = parse_rfc3339_epoch_seconds("2026-10-07T10:00:00.123Z").unwrap();
        assert_eq!(zulu, plus_one);
        assert_eq!(zulu, fractional);
    }

    #[test]
    fn test_resource_changed_uri_uses_system_or_user_mtime() {
        let since = parse_rfc3339_epoch_seconds("2026-10-07T10:00:00Z").unwrap();
        let record = Resource {
            uri: "/repositories/2/resources/1".to_string(),
            title: Some("Changed".to_string()),
            system_mtime: Some("2026-10-07T09:59:59Z".to_string()),
            user_mtime: Some("2026-10-07T10:00:01Z".to_string()),
            ..Default::default()
        };

        assert_eq!(
            resource_changed_uri(&record, since).unwrap().as_deref(),
            Some("/repositories/2/resources/1")
        );
    }

    #[test]
    fn test_missing_mtimes_are_not_changed() {
        let since = parse_rfc3339_epoch_seconds("2026-10-07T10:00:00Z").unwrap();
        let record = Resource {
            uri: "/repositories/2/resources/1".to_string(),
            title: Some("No mtime".to_string()),
            ..Default::default()
        };

        assert!(resource_changed_uri(&record, since).unwrap().is_none());
    }

    #[test]
    fn test_archival_and_digital_changed_uri_helpers() {
        let since = parse_rfc3339_epoch_seconds("2026-10-07T10:00:00Z").unwrap();
        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/1".to_string(),
            system_mtime: Some("2026-10-07T10:00:01Z".to_string()),
            ..Default::default()
        };
        let digital_object = DigitalObject {
            uri: "/repositories/2/digital_objects/1".to_string(),
            user_mtime: Some("2026-10-07T10:00:01Z".to_string()),
            ..Default::default()
        };

        assert_eq!(
            archival_object_changed_uri(&ao, since).unwrap().as_deref(),
            Some("/repositories/2/archival_objects/1")
        );
        assert_eq!(
            digital_object_changed_uri(&digital_object, since)
                .unwrap()
                .as_deref(),
            Some("/repositories/2/digital_objects/1")
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_changed_uris_rejects_unsupported_target_before_network() {
        let client = Client::new("http://localhost:1");
        let args = ShadowIndexChangedUrisArgs {
            repo: 2,
            target: "accessions".to_string(),
            since: "2026-10-07T10:00:00Z".to_string(),
            output: None,
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            concurrency: 1,
        };

        let result = run_shadow_index_changed_uris(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Unsupported target 'accessions'. Only 'archival-objects', 'resources', and 'digital-objects' are supported."
        );
    }

    #[tokio::test]
    async fn test_fetch_solr_docs_batch_errors_on_http_failure() {
        use mockito::Server;
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/bad/select")
            .with_status(500)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let result =
            fetch_solr_docs_batch(&client, &server.url(), "bad", &["/test".to_string()]).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Solr query failed with status 500"));
    }

    #[test]
    fn test_make_field_comparison() {
        let m1 = make_field_comparison("title", Some("A".to_string()), Some("A".to_string()));
        assert_eq!(m1.status, "match");

        let m2 = make_field_comparison("title", Some("A".to_string()), Some("B".to_string()));
        assert_eq!(m2.status, "mismatch");

        let m3 = make_field_comparison("title", Some("A".to_string()), None);
        assert_eq!(m3.status, "search_absent");

        let m4 = make_field_comparison("title", None, Some("B".to_string()));
        assert_eq!(m4.status, "mismatch");

        let m5 = make_field_comparison("title", None, None);
        assert_eq!(m5.status, "match");
    }

    #[test]
    fn test_make_solr_field_comparison() {
        let m1 = make_solr_field_comparison("title", Some("A".to_string()), Some("A".to_string()));
        assert_eq!(m1.status, "match");

        let m2 = make_solr_field_comparison("title", Some("A".to_string()), Some("B".to_string()));
        assert_eq!(m2.status, "mismatch");

        let m3 = make_solr_field_comparison("title", Some("A".to_string()), None);
        assert_eq!(m3.status, "solr_absent");

        let m4 = make_solr_field_comparison("title", None, Some("B".to_string()));
        assert_eq!(m4.status, "mismatch");

        let m5 = make_solr_field_comparison("title", None, None);
        assert_eq!(m5.status, "match");
    }

    #[test]
    fn test_make_compare_check() {
        let matched = make_compare_check("resource", 14, Some(14));
        assert_eq!(matched.status, "match");

        let mismatched = make_compare_check("digital_object", 15, None);
        assert_eq!(mismatched.status, "mismatch");

        let mismatched2 = make_compare_check("archival_object", 100, Some(99));
        assert_eq!(mismatched2.status, "mismatch");
    }

    #[test]
    fn test_shadow_document_from_resource() {
        let resource = Resource {
            uri: "/repositories/2/resources/123".to_string(),
            title: Some("Test Resource".to_string()),
            publish: true,
            jsonmodel_type: Some("resource".to_string()),
            ..Default::default()
        };

        let doc = ShadowDocument::from_resource(2, 123, resource);

        assert_eq!(doc.repo_id, 2);
        assert_eq!(doc.id, 123);
        assert_eq!(doc.uri, "/repositories/2/resources/123");
        assert_eq!(doc.primary_type, "resource");
        assert_eq!(doc.title, Some("Test Resource".to_string()));
        assert!(doc.publish);
        assert_eq!(doc.parent_resource, None);
        assert_eq!(doc.jsonmodel_type, Some("resource".to_string()));
    }

    #[test]
    fn test_shadow_document_from_archival_object() {
        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/456".to_string(),
            title: Some("Test AO".to_string()),
            display_string: Some("Display String".to_string()),
            publish: false,
            resource: Some(Ref {
                ref_uri: "/repositories/2/resources/123".to_string(),
            }),
            jsonmodel_type: Some("archival_object".to_string()),
            ..Default::default()
        };

        let doc = ShadowDocument::from_archival_object(2, 456, ao);

        assert_eq!(doc.repo_id, 2);
        assert_eq!(doc.id, 456);
        assert_eq!(doc.uri, "/repositories/2/archival_objects/456");
        assert_eq!(doc.primary_type, "archival_object");
        assert_eq!(doc.title, Some("Test AO".to_string()));
        assert!(!doc.publish);
        assert_eq!(
            doc.parent_resource,
            Some("/repositories/2/resources/123".to_string())
        );
        assert_eq!(doc.jsonmodel_type, Some("archival_object".to_string()));
    }

    #[test]
    fn test_shadow_document_from_digital_object() {
        let do_obj = aspace_client::models::DigitalObject {
            uri: "/repositories/2/digital_objects/789".to_string(),
            title: Some("Test DO".to_string()),
            digital_object_id: Some("do_123".to_string()),
            system_mtime: None,
            user_mtime: None,
            publish: true,
            file_versions: vec![
                aspace_client::models::FileVersion {
                    file_uri: Some("file1.pdf".to_string()),
                },
                aspace_client::models::FileVersion {
                    file_uri: Some("file2.pdf".to_string()),
                },
            ],
            jsonmodel_type: Some("digital_object".to_string()),
        };

        let doc = ShadowDocument::from_digital_object(2, 789, do_obj);

        assert_eq!(doc.repo_id, 2);
        assert_eq!(doc.id, 789);
        assert_eq!(doc.uri, "/repositories/2/digital_objects/789");
        assert_eq!(doc.primary_type, "digital_object");
        assert_eq!(doc.title, Some("Test DO".to_string()));
        assert!(doc.publish);
        assert_eq!(doc.parent_resource, None);
        assert_eq!(doc.jsonmodel_type, Some("digital_object".to_string()));
        assert_eq!(doc.digital_object_id, Some("do_123".to_string()));
        assert_eq!(doc.file_version_count, Some(2));
    }

    #[test]
    fn test_shadow_document_ao_title_fallback() {
        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/456".to_string(),
            title: Some("   ".to_string()),
            display_string: Some("Display String".to_string()),
            ..Default::default()
        };

        let doc = ShadowDocument::from_archival_object(2, 456, ao);
        assert_eq!(doc.title, Some("Display String".to_string()));

        let ao2 = ArchivalObject {
            uri: "/repositories/2/archival_objects/457".to_string(),
            title: None,
            display_string: Some("Display String 2".to_string()),
            ..Default::default()
        };

        let doc2 = ShadowDocument::from_archival_object(2, 457, ao2);
        assert_eq!(doc2.title, Some("Display String 2".to_string()));
    }

    #[tokio::test]
    async fn test_unsupported_target_fails() {
        let client = Client::new("http://localhost:4567");
        let args = ShadowIndexExportArgs {
            repo: Some(2),
            target: Some("accessions".to_string()),
            output: None,
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            concurrency: 1,
        };
        let result = run_shadow_index_export(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Unsupported target 'accessions'. Only 'resources', 'archival-objects', and 'digital-objects' are supported."
        );
    }

    #[test]
    fn test_solr_ao_title_mapping() {
        use aspace_client::models::Date;

        // title plus one date expression
        let ao1 = ArchivalObject {
            uri: "/repositories/2/archival_objects/1".to_string(),
            title: Some("Title".to_string()),
            dates: vec![Date {
                expression: Some("1990".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let doc1 = SolrArchivalObjectDocument::from_archival_object(2, 1, ao1);
        assert_eq!(doc1.title, "Title, 1990");

        // title plus multiple date expressions
        let ao2 = ArchivalObject {
            uri: "/repositories/2/archival_objects/2".to_string(),
            title: Some("Title".to_string()),
            dates: vec![
                Date {
                    expression: Some("1990".to_string()),
                    ..Default::default()
                },
                Date {
                    expression: Some("1991-1992".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let doc2 = SolrArchivalObjectDocument::from_archival_object(2, 2, ao2);
        assert_eq!(doc2.title, "Title, 1990, 1991-1992");

        // title with bulk date
        let ao_bulk = ArchivalObject {
            uri: "/repositories/2/archival_objects/bulk".to_string(),
            title: Some("Bulk Title".to_string()),
            dates: vec![
                Date {
                    expression: Some("1912-2018".to_string()),
                    ..Default::default()
                },
                Date {
                    expression: Some("1952-1980".to_string()),
                    date_type: Some("bulk".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let doc_bulk = SolrArchivalObjectDocument::from_archival_object(2, 5, ao_bulk);
        assert_eq!(doc_bulk.title, "Bulk Title, 1912-2018, bulk: 1952-1980");

        // blank title falling back to display_string with date
        let ao3 = ArchivalObject {
            uri: "/repositories/2/archival_objects/3".to_string(),
            title: Some("   ".to_string()),
            display_string: Some("Display Title".to_string()),
            dates: vec![Date {
                expression: Some("1995".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let doc3 = SolrArchivalObjectDocument::from_archival_object(2, 3, ao3);
        assert_eq!(doc3.title, "Display Title, 1995");

        // no dates leaves title unchanged
        let ao4 = ArchivalObject {
            uri: "/repositories/2/archival_objects/4".to_string(),
            title: Some("Title Only".to_string()),
            ..Default::default()
        };
        let doc4 = SolrArchivalObjectDocument::from_archival_object(2, 4, ao4);
        assert_eq!(doc4.title, "Title Only");
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_shadow_compare_same_collection() {
        let client = Client::new("http://localhost:8089");
        let args = ShadowIndexSolrShadowCompareArgs {
            repo: 2,
            target: "archival-objects".to_string(),
            limit: 25,
            solr_url: "http://localhost:8985/solr".to_string(),
            live_collection: "archivesspace".to_string(),
            shadow_collection: "archivesspace".to_string(),
            json: false,
        };
        let result = run_shadow_index_solr_shadow_compare(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Live collection and shadow collection cannot be the same."
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_docs_default_limit() {
        use mockito::{Matcher, Server};
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let _mock_ao_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_body(r#"[1,2,3,4,5,6,7,8,9,10,11,12]"#)
            .create_async()
            .await;

        let _mock_ao_by_ids = server
            .mock(
                "GET",
                Matcher::Regex(r#"^/repositories/2/archival_objects\?id_set="#.into()),
            )
            .with_status(200)
            .with_body(r#"[]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let args = ShadowIndexSolrDocsArgs {
            repo: 2,
            target: "archival-objects".to_string(),
            output: None,
            limit: None,
            compare_sample: true,
            json_summary: true,
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            concurrency: 1,
        };

        let result = run_shadow_index_solr_docs(&client, args).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_safe_solr_document_omits_fields() {
        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/456".to_string(),
            title: Some("Test Safe AO".to_string()),
            publish: true,
            resource: Some(Ref {
                ref_uri: "/repositories/2/resources/123".to_string(),
            }),
            jsonmodel_type: Some("archival_object".to_string()),
            ..Default::default()
        };

        let full_doc = SolrArchivalObjectDocument::from_archival_object(2, 456, ao);
        assert_eq!(
            full_doc.parent_resource,
            Some("/repositories/2/resources/123".to_string())
        );
        assert_eq!(full_doc.repo_id, 2);
        assert_eq!(full_doc.record_id, 456);

        let safe_doc = SafeSolrDocument::from(full_doc);

        let json = serde_json::to_string(&safe_doc).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let object = value.as_object().unwrap();
        assert!(!object.contains_key("parent_resource"));
        assert!(!object.contains_key("repo_id"));
        assert!(!object.contains_key("record_id"));
        assert!(object.contains_key("id"));
        assert!(object.contains_key("uri"));
        assert!(object.contains_key("primary_type"));
        assert!(object.contains_key("title"));
        assert!(object.contains_key("publish"));
        assert!(object.contains_key("json"));
        assert!(!object.contains_key("jsonmodel_type"));
    }

    #[test]
    fn test_safe_solr_document_resource_mapping() {
        use aspace_client::models::Resource;
        let res = Resource {
            uri: "/repositories/2/resources/789".to_string(),
            title: Some("Test Resource Title".to_string()),
            publish: false,
            ..Default::default()
        };
        let doc = SafeSolrDocument::from_resource(res);
        assert_eq!(doc.id, "/repositories/2/resources/789");
        assert_eq!(doc.uri, "/repositories/2/resources/789");
        assert_eq!(doc.primary_type, "resource");
        assert_eq!(doc.title, "Test Resource Title");
        assert!(!doc.publish);
    }

    #[test]
    fn test_safe_solr_document_digital_object_mapping() {
        use aspace_client::models::DigitalObject;
        let do_obj = DigitalObject {
            uri: "/repositories/2/digital_objects/101".to_string(),
            title: Some("Test DO Title".to_string()),
            publish: true,
            ..Default::default()
        };
        let doc = SafeSolrDocument::from_digital_object(do_obj);
        assert_eq!(doc.id, "/repositories/2/digital_objects/101");
        assert_eq!(doc.uri, "/repositories/2/digital_objects/101");
        assert_eq!(doc.primary_type, "digital_object");
        assert_eq!(doc.title, "Test DO Title");
        assert!(doc.publish);
    }

    #[test]
    fn test_safe_solr_document_archival_object_serialization_and_title_dates() {
        use aspace_client::models::{Date, LinkedAgentRef, Ref};

        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/456".to_string(),
            title: Some("Folder of Documents".to_string()),
            display_string: Some("Folder of Documents, 1990-1995".to_string()),
            publish: true,
            level: Some("item".to_string()),
            ref_id: Some("ref_456".to_string()),
            component_id: Some("COMP-789".to_string()),
            position: Some(12),
            dates: vec![
                Date {
                    expression: Some("1990-1995".to_string()),
                    ..Default::default()
                },
                Date {
                    expression: Some("1992".to_string()),
                    date_type: Some("bulk".to_string()),
                    ..Default::default()
                },
            ],
            subjects: vec![
                Ref {
                    ref_uri: "/subjects/10".to_string(),
                },
                Ref {
                    ref_uri: "/subjects/20".to_string(),
                },
            ],
            linked_agents: vec![
                LinkedAgentRef {
                    ref_uri: Some("/agents/people/1".to_string()),
                    role: Some("creator".to_string()),
                    relator: Some("aut".to_string()),
                    title: Some("Author Person".to_string()),
                    terms: Vec::new(),
                },
                LinkedAgentRef {
                    ref_uri: Some("/agents/corporate_entities/2".to_string()),
                    role: Some("subject".to_string()),
                    relator: None,
                    title: Some("Corporate Body".to_string()),
                    terms: Vec::new(),
                },
            ],
            ..Default::default()
        };

        let solr_ao_doc = SolrArchivalObjectDocument::from_archival_object(2, 456, ao);
        // Verify existing title-with-date behavior remains unchanged (bulk prefix in title)
        assert_eq!(
            solr_ao_doc.title,
            "Folder of Documents, 1990-1995, bulk: 1992"
        );
        assert_eq!(solr_ao_doc.level.as_deref(), Some("item"));
        assert_eq!(solr_ao_doc.ref_id.as_deref(), Some("ref_456"));
        assert_eq!(solr_ao_doc.component_id.as_deref(), Some("COMP-789"));
        assert_eq!(solr_ao_doc.position_i, Some(12));
        assert_eq!(
            solr_ao_doc.display_string.as_deref(),
            Some("Folder of Documents, 1990-1995")
        );
        assert_eq!(
            solr_ao_doc.date_expressions,
            vec!["1990-1995".to_string(), "1992".to_string()]
        );
        assert_eq!(
            solr_ao_doc.subject_uris,
            vec!["/subjects/10".to_string(), "/subjects/20".to_string()]
        );
        assert_eq!(
            solr_ao_doc.linked_agent_uris,
            vec![
                "/agents/people/1".to_string(),
                "/agents/corporate_entities/2".to_string()
            ]
        );
        assert_eq!(
            solr_ao_doc.linked_agent_roles,
            vec!["creator".to_string(), "subject".to_string()]
        );
        assert_eq!(solr_ao_doc.linked_agent_relators, vec!["aut".to_string()]);
        assert_eq!(
            solr_ao_doc.linked_agent_titles,
            vec!["Author Person".to_string(), "Corporate Body".to_string()]
        );

        let safe_doc = SafeSolrDocument::from(solr_ao_doc);
        assert_eq!(safe_doc.id, "/repositories/2/archival_objects/456");
        assert_eq!(safe_doc.uri, "/repositories/2/archival_objects/456");
        assert_eq!(safe_doc.primary_type, "archival_object");
        assert_eq!(safe_doc.title, "Folder of Documents, 1990-1995, bulk: 1992");
        assert!(safe_doc.publish);
        assert_eq!(safe_doc.level.as_deref(), Some("item"));
        assert_eq!(safe_doc.ref_id.as_deref(), Some("ref_456"));
        assert_eq!(safe_doc.component_id.as_deref(), Some("COMP-789"));
        assert_eq!(safe_doc.position_i, Some(12));
        assert_eq!(
            safe_doc.display_string.as_deref(),
            Some("Folder of Documents, 1990-1995")
        );
        assert_eq!(
            safe_doc.date_expressions,
            vec!["1990-1995".to_string(), "1992".to_string()]
        );
        assert_eq!(
            safe_doc.subject_uris,
            vec!["/subjects/10".to_string(), "/subjects/20".to_string()]
        );
        assert_eq!(
            safe_doc.linked_agent_uris,
            vec![
                "/agents/people/1".to_string(),
                "/agents/corporate_entities/2".to_string()
            ]
        );
        assert_eq!(
            safe_doc.linked_agent_roles,
            vec!["creator".to_string(), "subject".to_string()]
        );
        assert_eq!(safe_doc.linked_agent_relators, vec!["aut".to_string()]);
        assert_eq!(
            safe_doc.linked_agent_titles,
            vec!["Author Person".to_string(), "Corporate Body".to_string()]
        );

        let json = serde_json::to_string(&safe_doc).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["level"], "item");
        assert_eq!(value["ref_id"], "ref_456");
        assert_eq!(value["component_id"], "COMP-789");
        assert_eq!(value["position_i"], 12);
        assert_eq!(value["display_string"], "Folder of Documents, 1990-1995");
        assert_eq!(
            value["date_expressions"],
            serde_json::json!(["1990-1995", "1992"])
        );
        assert_eq!(
            value["subject_uris"],
            serde_json::json!(["/subjects/10", "/subjects/20"])
        );
        assert_eq!(
            value["linked_agent_uris"],
            serde_json::json!(["/agents/people/1", "/agents/corporate_entities/2"])
        );
        assert_eq!(
            value["linked_agent_roles"],
            serde_json::json!(["creator", "subject"])
        );
        assert_eq!(value["linked_agent_relators"], serde_json::json!(["aut"]));
        assert_eq!(
            value["linked_agent_titles"],
            serde_json::json!(["Author Person", "Corporate Body"])
        );
        assert_eq!(value["title"], "Folder of Documents, 1990-1995, bulk: 1992");
    }

    #[test]
    fn test_compact_tree_source_json_keeps_tree_fields_and_drops_resolved_graphs() {
        let ao: ArchivalObject = serde_json::from_value(serde_json::json!({
            "uri": "/repositories/2/archival_objects/1",
            "title": "A <title>mixed</title> title",
            "component_id": "C-1",
            "level": "file",
            "suppressed": false,
            "dates": [{"date_type":"inclusive","label":"creation","expression":"1900"}],
            "instances": [{
                "instance_type": "text",
                "sub_container": {
                    "top_container": {
                        "ref": "/repositories/2/top_containers/1",
                        "_resolved": {
                            "type": "Box",
                            "indicator": "1",
                            "series": [{"ref":"/repositories/2/archival_objects/999"}],
                            "collection": [{"ref":"/repositories/2/resources/1"}]
                        }
                    }
                }
            }]
        }))
        .unwrap();

        let compact: serde_json::Value =
            serde_json::from_str(&compact_tree_source_json(&ao).unwrap()).unwrap();
        assert_eq!(compact["component_id"], "C-1");
        assert_eq!(compact["dates"][0]["label"], "creation");
        assert_eq!(
            compact["instances"][0]["sub_container"]["top_container"]["_resolved"]["indicator"],
            "1"
        );
        assert!(compact.to_string().len() < 500);
        assert!(
            compact["instances"][0]["sub_container"]["top_container"]["_resolved"]
                .get("series")
                .is_none()
        );
    }

    #[test]
    fn test_safe_solr_document_resource_and_digital_behavior_unchanged() {
        use aspace_client::models::{DigitalObject, Resource};

        let res = Resource {
            uri: "/repositories/2/resources/999".to_string(),
            title: Some("Resource Unchanged".to_string()),
            publish: true,
            ..Default::default()
        };
        let safe_res = SafeSolrDocument::from_resource(res);
        assert_eq!(safe_res.level, None);
        assert_eq!(safe_res.ref_id, None);
        assert_eq!(safe_res.component_id, None);
        assert_eq!(safe_res.position_i, None);
        assert_eq!(safe_res.display_string, None);
        assert!(safe_res.date_expressions.is_empty());
        assert!(safe_res.subject_uris.is_empty());
        assert!(safe_res.linked_agent_uris.is_empty());
        assert!(safe_res.linked_agent_roles.is_empty());
        assert!(safe_res.linked_agent_relators.is_empty());
        assert!(safe_res.linked_agent_titles.is_empty());
        assert_eq!(safe_res.resource_uri, None);
        assert_eq!(safe_res.parent_uri, None);
        assert!(safe_res.ancestor_uris.is_empty());
        assert!(safe_res.ancestor_titles.is_empty());
        assert!(safe_res.ancestor_levels.is_empty());

        let res_json = serde_json::to_string(&safe_res).unwrap();
        assert!(!res_json.contains("level"));
        assert!(!res_json.contains("ref_id"));
        assert!(!res_json.contains("component_id"));
        assert!(!res_json.contains("position_i"));
        assert!(!res_json.contains("display_string"));
        assert!(!res_json.contains("date_expressions"));
        assert!(!res_json.contains("subject_uris"));
        assert!(!res_json.contains("linked_agent_"));
        assert!(!res_json.contains("resource_uri"));
        assert!(!res_json.contains("parent_uri"));
        assert!(!res_json.contains("ancestor_"));

        let do_obj = DigitalObject {
            uri: "/repositories/2/digital_objects/888".to_string(),
            title: Some("DO Unchanged".to_string()),
            publish: false,
            ..Default::default()
        };
        let safe_do = SafeSolrDocument::from_digital_object(do_obj);
        assert_eq!(safe_do.level, None);
        assert_eq!(safe_do.ref_id, None);
        assert_eq!(safe_do.component_id, None);
        assert_eq!(safe_do.position_i, None);
        assert_eq!(safe_do.display_string, None);
        assert!(safe_do.date_expressions.is_empty());
        assert!(safe_do.subject_uris.is_empty());
        assert!(safe_do.linked_agent_uris.is_empty());
        assert!(safe_do.linked_agent_roles.is_empty());
        assert!(safe_do.linked_agent_relators.is_empty());
        assert!(safe_do.linked_agent_titles.is_empty());
        assert_eq!(safe_do.resource_uri, None);
        assert_eq!(safe_do.parent_uri, None);
        assert!(safe_do.ancestor_uris.is_empty());
        assert!(safe_do.ancestor_titles.is_empty());
        assert!(safe_do.ancestor_levels.is_empty());

        let do_json = serde_json::to_string(&safe_do).unwrap();
        assert!(!do_json.contains("level"));
        assert!(!do_json.contains("ref_id"));
        assert!(!do_json.contains("component_id"));
        assert!(!do_json.contains("position_i"));
        assert!(!do_json.contains("display_string"));
        assert!(!do_json.contains("date_expressions"));
        assert!(!do_json.contains("subject_uris"));
        assert!(!do_json.contains("linked_agent_"));
        assert!(!do_json.contains("resource_uri"));
        assert!(!do_json.contains("parent_uri"));
        assert!(!do_json.contains("ancestor_"));
    }

    #[test]
    fn test_safe_solr_document_hierarchy_serialization() {
        use aspace_client::models::{AncestorRef, Ref};

        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/456".to_string(),
            title: Some("Child Folder".to_string()),
            publish: true,
            resource: Some(Ref {
                ref_uri: "/repositories/2/resources/100".to_string(),
            }),
            parent: Some(Ref {
                ref_uri: "/repositories/2/archival_objects/200".to_string(),
            }),
            ancestors: vec![
                AncestorRef {
                    ref_uri: Some("/repositories/2/resources/100".to_string()),
                    level: Some("collection".to_string()),
                    title: Some("Top Resource".to_string()),
                },
                AncestorRef {
                    ref_uri: Some("/repositories/2/archival_objects/200".to_string()),
                    level: Some("series".to_string()),
                    title: Some("Series 1".to_string()),
                },
                AncestorRef {
                    ref_uri: Some("   ".to_string()),
                    level: Some("".to_string()),
                    title: None,
                },
            ],
            ..Default::default()
        };

        let solr_ao = SolrArchivalObjectDocument::from_archival_object(2, 456, ao);
        assert_eq!(
            solr_ao.resource_uri.as_deref(),
            Some("/repositories/2/resources/100")
        );
        assert_eq!(
            solr_ao.parent_uri.as_deref(),
            Some("/repositories/2/archival_objects/200")
        );
        assert_eq!(
            solr_ao.ancestor_uris,
            vec![
                "/repositories/2/resources/100".to_string(),
                "/repositories/2/archival_objects/200".to_string()
            ]
        );
        assert_eq!(
            solr_ao.ancestor_titles,
            vec!["Top Resource".to_string(), "Series 1".to_string()]
        );
        assert_eq!(
            solr_ao.ancestor_levels,
            vec!["collection".to_string(), "series".to_string()]
        );

        let safe_doc = SafeSolrDocument::from(solr_ao);
        assert_eq!(
            safe_doc.resource_uri.as_deref(),
            Some("/repositories/2/resources/100")
        );
        assert_eq!(
            safe_doc.parent_uri.as_deref(),
            Some("/repositories/2/archival_objects/200")
        );
        assert_eq!(
            safe_doc.ancestor_uris,
            vec![
                "/repositories/2/resources/100".to_string(),
                "/repositories/2/archival_objects/200".to_string()
            ]
        );
        assert_eq!(
            safe_doc.ancestor_titles,
            vec!["Top Resource".to_string(), "Series 1".to_string()]
        );
        assert_eq!(
            safe_doc.ancestor_levels,
            vec!["collection".to_string(), "series".to_string()]
        );

        let json = serde_json::to_string(&safe_doc).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["resource_uri"], "/repositories/2/resources/100");
        assert_eq!(value["parent_uri"], "/repositories/2/archival_objects/200");
        assert_eq!(
            value["ancestor_uris"],
            serde_json::json!([
                "/repositories/2/resources/100",
                "/repositories/2/archival_objects/200"
            ])
        );
        assert_eq!(
            value["ancestor_titles"],
            serde_json::json!(["Top Resource", "Series 1"])
        );
        assert_eq!(
            value["ancestor_levels"],
            serde_json::json!(["collection", "series"])
        );
    }

    #[test]
    fn test_compare_archival_object_search_fields() {
        use aspace_client::models::{LinkedAgentRef, Ref};

        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/1".to_string(),
            title: Some("AO 1".to_string()),
            level: Some("item".to_string()),
            ref_id: Some("ref_1".to_string()),
            component_id: Some("c_1".to_string()),
            position: Some(3),
            display_string: Some("AO 1 Disp".to_string()),
            subjects: vec![Ref {
                ref_uri: "/subjects/2".to_string(),
            }],
            linked_agents: vec![LinkedAgentRef {
                ref_uri: Some("/agents/people/1".to_string()),
                role: Some("creator".to_string()),
                relator: Some("aut".to_string()),
                title: Some("Author Person".to_string()),
                terms: Vec::new(),
            }],
            ..Default::default()
        };
        let shadow_doc = ShadowDocument::from_archival_object(2, 1, ao);

        // Case 1: Search doc contains all keys matching
        let sdoc_matching = serde_json::json!({
            "level": "item",
            "ref_id": "ref_1",
            "component_id": "c_1",
            "position_i": 3,
            "display_string": "AO 1 Disp",
            "subject_uris": ["/subjects/2"],
            "linked_agent_uris": ["/agents/people/1"],
            "linked_agent_roles": "creator",
            "linked_agent_relators": ["aut"],
            "linked_agent_titles": ["Author Person"]
        });
        let mut comparisons = Vec::new();
        compare_archival_object_search_fields(&shadow_doc, &sdoc_matching, &mut comparisons);
        assert_eq!(comparisons.len(), 10);
        assert!(comparisons.iter().all(|c| c.status == "match"));

        // Case 2: Search doc contains mismatched ref_id
        let sdoc_mismatched = serde_json::json!({
            "level": "item",
            "ref_id": "ref_diff",
            "subject_uris": ["/subjects/999"]
        });
        let mut comparisons2 = Vec::new();
        compare_archival_object_search_fields(&shadow_doc, &sdoc_mismatched, &mut comparisons2);
        assert_eq!(comparisons2.len(), 3);
        let level_cmp = comparisons2.iter().find(|c| c.field == "level").unwrap();
        assert_eq!(level_cmp.status, "match");
        let ref_cmp = comparisons2.iter().find(|c| c.field == "ref_id").unwrap();
        assert_eq!(ref_cmp.status, "mismatch");
        let subject_cmp = comparisons2
            .iter()
            .find(|c| c.field == "subject_uris")
            .unwrap();
        assert_eq!(subject_cmp.status, "mismatch");

        // Case 3: Search doc contains none of those keys
        let sdoc_empty = serde_json::json!({
            "uri": "/repositories/2/archival_objects/1",
            "primary_type": "archival_object"
        });
        let mut comparisons3 = Vec::new();
        compare_archival_object_search_fields(&shadow_doc, &sdoc_empty, &mut comparisons3);
        assert!(comparisons3.is_empty());
    }

    #[test]
    fn test_compare_archival_object_search_fields_hierarchy() {
        use aspace_client::models::{AncestorRef, Ref};

        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/1".to_string(),
            title: Some("Child AO".to_string()),
            resource: Some(Ref {
                ref_uri: "/repositories/2/resources/5".to_string(),
            }),
            parent: Some(Ref {
                ref_uri: "/repositories/2/archival_objects/2".to_string(),
            }),
            ancestors: vec![
                AncestorRef {
                    ref_uri: Some("/repositories/2/resources/5".to_string()),
                    title: Some("Res 5".to_string()),
                    level: Some("collection".to_string()),
                },
                AncestorRef {
                    ref_uri: Some("/repositories/2/archival_objects/2".to_string()),
                    title: Some("Series 2".to_string()),
                    level: Some("series".to_string()),
                },
            ],
            ..Default::default()
        };
        let shadow_doc = ShadowDocument::from_archival_object(2, 1, ao);

        // Case 1: Search doc matches all hierarchy fields
        let sdoc_matching = serde_json::json!({
            "resource_uri": "/repositories/2/resources/5",
            "parent_uri": "/repositories/2/archival_objects/2",
            "ancestor_uris": ["/repositories/2/resources/5", "/repositories/2/archival_objects/2"],
            "ancestor_titles": ["Series 2", "Res 5"], // order normalized by helper
            "ancestor_levels": ["collection", "series"]
        });
        let mut comparisons = Vec::new();
        compare_archival_object_search_fields(&shadow_doc, &sdoc_matching, &mut comparisons);
        assert_eq!(comparisons.len(), 5);
        assert!(comparisons.iter().all(|c| c.status == "match"));

        // Case 2: Hierarchy fields mismatched
        let sdoc_mismatched = serde_json::json!({
            "resource_uri": "/repositories/2/resources/999",
            "parent_uri": "/repositories/2/archival_objects/2",
            "ancestor_uris": ["/repositories/2/resources/wrong"]
        });
        let mut comparisons2 = Vec::new();
        compare_archival_object_search_fields(&shadow_doc, &sdoc_mismatched, &mut comparisons2);
        assert_eq!(comparisons2.len(), 3);
        let res_cmp = comparisons2
            .iter()
            .find(|c| c.field == "resource_uri")
            .unwrap();
        assert_eq!(res_cmp.status, "mismatch");
        let parent_cmp = comparisons2
            .iter()
            .find(|c| c.field == "parent_uri")
            .unwrap();
        assert_eq!(parent_cmp.status, "match");
        let anc_cmp = comparisons2
            .iter()
            .find(|c| c.field == "ancestor_uris")
            .unwrap();
        assert_eq!(anc_cmp.status, "mismatch");

        // Case 3: Hierarchy keys omitted in search doc -> not compared
        let sdoc_other = serde_json::json!({
            "display_string": "Something else"
        });
        let mut comparisons3 = Vec::new();
        compare_archival_object_search_fields(&shadow_doc, &sdoc_other, &mut comparisons3);
        assert_eq!(comparisons3.len(), 1);
        assert_eq!(comparisons3[0].field, "display_string");
    }

    #[test]
    fn test_solr_target_segment() {
        assert_eq!(
            super::solr_target_segment("archival-objects"),
            "archival_objects"
        );
        assert_eq!(super::solr_target_segment("resources"), "resources");
        assert_eq!(
            super::solr_target_segment("digital-objects"),
            "digital_objects"
        );
    }

    #[test]
    fn test_solr_configset_for_shadow_collection() {
        assert_eq!(
            super::solr_configset_for_shadow_collection("archivesspace_shadow"),
            "archivesspace_shadow"
        );
        assert_eq!(
            super::solr_configset_for_shadow_collection("other_shadow"),
            "archivesspace"
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_load_shadow_refuses_live_collection() {
        use crate::shadow::ShadowIndexSolrLoadShadowArgs;
        let server = mockito::Server::new_async().await;
        let client = Client::new(server.url());
        let args = ShadowIndexSolrLoadShadowArgs {
            repo: 2,
            target: "archival-objects".to_string(),
            limit: 10,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace".to_string(),
            create_if_missing: false,
            replace_shadow: true,
            replace_target: true,
        };

        let result = super::run_shadow_index_solr_load_shadow(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Refusing to operate on the live 'archivesspace' collection."
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_load_shadow_refuses_conflict() {
        use crate::shadow::ShadowIndexSolrLoadShadowArgs;
        let server = mockito::Server::new_async().await;
        let client = Client::new(server.url());
        let args = ShadowIndexSolrLoadShadowArgs {
            repo: 2,
            target: "archival-objects".to_string(),
            limit: 10,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            create_if_missing: false,
            replace_shadow: true,
            replace_target: true,
        };

        let result = super::run_shadow_index_solr_load_shadow(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot supply both --replace-shadow and --replace-target. Choose one."
        );
    }

    #[test]
    fn test_rich_archival_object_schema_fields() {
        let fields = rich_archival_object_schema_fields();
        let names: Vec<&str> = fields.iter().map(|field| field.name).collect();

        assert_eq!(
            names,
            vec![
                "display_string",
                "date_expressions",
                "resource_uri",
                "parent_uri",
                "ancestor_uris",
                "ancestor_levels",
                "position_i"
            ]
        );
        assert!(
            fields
                .iter()
                .find(|field| field.name == "date_expressions")
                .unwrap()
                .multi_valued
        );
        assert!(
            !fields
                .iter()
                .find(|field| field.name == "resource_uri")
                .unwrap()
                .multi_valued
        );
        let position = fields
            .iter()
            .find(|field| field.name == "position_i")
            .unwrap();
        assert_eq!(position.field_type, "pint");
        assert!(position.stored);
        assert!(position.indexed);
        assert!(!position.multi_valued);
    }

    #[test]
    fn test_solr_add_field_payload() {
        let field = SolrSchemaFieldSpec {
            name: "ancestor_uris",
            field_type: "string",
            stored: true,
            indexed: true,
            multi_valued: true,
        };

        let payload = solr_add_field_payload(&field);
        assert_eq!(
            payload,
            serde_json::json!({
                "add-field": {
                    "name": "ancestor_uris",
                    "type": "string",
                    "stored": true,
                    "indexed": true,
                    "multiValued": true
                }
            })
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_ensure_shadow_schema_refuses_live_collection() {
        let args = ShadowIndexSolrEnsureShadowSchemaArgs {
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace".to_string(),
            json: false,
        };

        let result = run_shadow_index_solr_ensure_shadow_schema(args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Refusing to operate on the live 'archivesspace' collection."
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_load_shadow_replace_target_sends_query() {
        use crate::shadow::ShadowIndexSolrLoadShadowArgs;
        let mut server = mockito::Server::new_async().await;

        // Mock ArchivesSpace login
        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "test-session"}"#)
            .create_async()
            .await;

        // Mock ArchivesSpace AO ids
        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_body("[1]")
            .create_async()
            .await;

        // Mock ArchivesSpace AO records
        let _mock_records = server
            .mock("GET", mockito::Matcher::Regex(r#"^/repositories/2/archival_objects\?id_set="#.to_string()))
            .with_status(200)
            .with_body(r#"[{"uri": "/repositories/2/archival_objects/1", "title": "Test AO", "publish": true}]"#)
            .create_async()
            .await;

        // Mock Solr delete by query and commit
        let _mock_solr_delete = server
            .mock("POST", "/solr/archivesspace_shadow/update?commit=true")
            .match_body(mockito::Matcher::JsonString(
                r#"{"delete": {"query": "id:\\/repositories\\/2\\/archival_objects\\/*"}}"#
                    .to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .create_async()
            .await;

        // Mock Solr load update
        let _mock_solr_load = server
            .mock("POST", "/solr/archivesspace_shadow/update?commit=true")
            .match_body(mockito::Matcher::Regex(
                r#"^\[\{"id":"/repositories/2/archival_objects/1""#.to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let solr_url = format!("{}/solr", server.url());

        let args = ShadowIndexSolrLoadShadowArgs {
            repo: 2,
            target: "archival-objects".to_string(),
            limit: 10,
            solr_url,
            collection: "archivesspace_shadow".to_string(),
            create_if_missing: false,
            replace_shadow: false,
            replace_target: true,
        };

        let result = super::run_shadow_index_solr_load_shadow(&client, args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_reindex_uris_sends_exact_delete_query() {
        use crate::shadow::ShadowIndexSolrReindexUrisArgs;
        let mut server = mockito::Server::new_async().await;

        let uri_file = std::env::temp_dir().join(format!(
            "aspace-reindex-uris-{}-{}.txt",
            std::process::id(),
            "resources"
        ));
        std::fs::write(
            &uri_file,
            "/repositories/2/resources/1\n/repositories/2/resources/2\n",
        )
        .unwrap();

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "test-session"}"#)
            .create_async()
            .await;

        let _mock_records = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/repositories/2/resources\?id_set=1,2"#.to_string()),
            )
            .with_status(200)
            .with_body(
                r#"[{"uri":"/repositories/2/resources/1","title":"One","publish":true},{"uri":"/repositories/2/resources/2","title":"Two","publish":true}]"#,
            )
            .create_async()
            .await;

        let _mock_solr_delete = server
            .mock("POST", "/solr/archivesspace_shadow/update?commit=true")
            .match_body(mockito::Matcher::JsonString(
                r#"{"delete":{"query":"id:(\"/repositories/2/resources/1\" OR \"/repositories/2/resources/2\")"}}"#.to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .create_async()
            .await;

        let _mock_solr_load = server
            .mock("POST", "/solr/archivesspace_shadow/update?commit=true")
            .match_body(mockito::Matcher::Regex(
                r#"^\[\{"id":"/repositories/2/resources/1""#.to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let args = ShadowIndexSolrReindexUrisArgs {
            repo: 2,
            target: "resources".to_string(),
            uri_file: uri_file.clone(),
            solr_url: format!("{}/solr", server.url()),
            collection: "archivesspace_shadow".to_string(),
        };

        let result = super::run_shadow_index_solr_reindex_uris(&client, args).await;
        let _ = std::fs::remove_file(uri_file);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_shadow_index_solr_reindex_uris_refuses_live_collection_before_file_read() {
        use crate::shadow::ShadowIndexSolrReindexUrisArgs;

        let client = Client::new("http://example.invalid".to_string());
        let args = ShadowIndexSolrReindexUrisArgs {
            repo: 2,
            target: "resources".to_string(),
            uri_file: PathBuf::from("/definitely/not/a/real/file.txt"),
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace".to_string(),
        };

        let result = super::run_shadow_index_solr_reindex_uris(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Refusing to operate on the live 'archivesspace' collection."
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_incremental_reindex_refuses_live_collection_before_network() {
        use crate::shadow::ShadowIndexIncrementalReindexArgs;

        let client = Client::new("http://example.invalid".to_string());
        let args = ShadowIndexIncrementalReindexArgs {
            repo: 2,
            target: "resources".to_string(),
            since: "2026-10-07T10:00:00Z".to_string(),
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace".to_string(),
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            concurrency: 1,
        };

        let result = super::run_shadow_index_incremental_reindex(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Refusing to operate on the live 'archivesspace' collection."
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_incremental_reindex_rejects_unsupported_target_before_network() {
        use crate::shadow::ShadowIndexIncrementalReindexArgs;

        let client = Client::new("http://example.invalid".to_string());
        let args = ShadowIndexIncrementalReindexArgs {
            repo: 2,
            target: "accessions".to_string(),
            since: "2026-10-07T10:00:00Z".to_string(),
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            concurrency: 1,
        };

        let result = super::run_shadow_index_incremental_reindex(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Unsupported target 'accessions'. Only 'archival-objects', 'resources', and 'digital-objects' are supported."
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_incremental_reindex_no_changes_makes_no_solr_requests() {
        use crate::shadow::ShadowIndexIncrementalReindexArgs;
        let mut server = mockito::Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "test-session"}"#)
            .create_async()
            .await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/resources?all_ids=true")
            .with_status(200)
            .with_body("[1,2]")
            .create_async()
            .await;

        let _mock_records = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/repositories/2/resources\?id_set=1,2"#.to_string()),
            )
            .with_status(200)
            .with_body(
                r#"[
                    {"uri":"/repositories/2/resources/1","title":"One","system_mtime":"2026-10-07T08:00:00Z","user_mtime":"2026-10-07T08:00:00Z","publish":true},
                    {"uri":"/repositories/2/resources/2","title":"Two","system_mtime":"2026-10-07T09:00:00Z","user_mtime":"2026-10-07T09:00:00Z","publish":true}
                ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let args = ShadowIndexIncrementalReindexArgs {
            repo: 2,
            target: "resources".to_string(),
            since: "2026-10-07T10:00:00Z".to_string(),
            solr_url: format!("{}/solr", server.url()),
            collection: "archivesspace_shadow".to_string(),
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            concurrency: 1,
        };

        let result = super::run_shadow_index_incremental_reindex(&client, args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_shadow_index_incremental_reindex_exact_delete_and_load() {
        use crate::shadow::ShadowIndexIncrementalReindexArgs;
        let mut server = mockito::Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "test-session"}"#)
            .create_async()
            .await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/resources?all_ids=true")
            .with_status(200)
            .with_body("[1,2]")
            .create_async()
            .await;

        let _mock_records_fetch = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/repositories/2/resources\?id_set=1,2"#.to_string()),
            )
            .with_status(200)
            .with_body(
                r#"[
                    {"uri":"/repositories/2/resources/1","title":"One","system_mtime":"2026-10-07T10:30:00Z","user_mtime":"2026-10-07T10:30:00Z","publish":true},
                    {"uri":"/repositories/2/resources/2","title":"Two","system_mtime":"2026-10-07T09:00:00Z","user_mtime":"2026-10-07T09:00:00Z","publish":true}
                ]"#,
            )
            .create_async()
            .await;

        let mock_solr_delete = server
            .mock("POST", "/solr/archivesspace_shadow/update?commit=true")
            .match_body(mockito::Matcher::JsonString(
                r#"{"delete":{"query":"id:(\"/repositories/2/resources/1\")"}}"#.to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .expect(1)
            .create_async()
            .await;

        let _mock_records_load = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/repositories/2/resources\?id_set=1"#.to_string()),
            )
            .with_status(200)
            .with_body(r#"[{"uri":"/repositories/2/resources/1","title":"One","publish":true}]"#)
            .create_async()
            .await;

        let mock_solr_load = server
            .mock("POST", "/solr/archivesspace_shadow/update?commit=true")
            .match_body(mockito::Matcher::Regex(
                r#"^\[\{"id":"/repositories/2/resources/1""#.to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .expect(1)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let args = ShadowIndexIncrementalReindexArgs {
            repo: 2,
            target: "resources".to_string(),
            since: "2026-10-07T10:00:00Z".to_string(),
            solr_url: format!("{}/solr", server.url()),
            collection: "archivesspace_shadow".to_string(),
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            concurrency: 1,
        };

        let result = super::run_shadow_index_incremental_reindex(&client, args).await;
        assert!(result.is_ok());

        mock_solr_delete.assert_async().await;
        mock_solr_load.assert_async().await;
    }

    #[test]
    fn test_record_field_mismatch_aggregation_and_capping() {
        let mut mismatches: BTreeMap<String, FieldMismatchSummary> = BTreeMap::new();

        record_field_mismatch(
            &mut mismatches,
            "title",
            "/repositories/2/archival_objects/1",
            Some("Shadow 1".to_string()),
            Some("Solr 1".to_string()),
            2,
        );
        record_field_mismatch(
            &mut mismatches,
            "title",
            "/repositories/2/archival_objects/2",
            Some("Shadow 2".to_string()),
            Some("Solr 2".to_string()),
            2,
        );
        record_field_mismatch(
            &mut mismatches,
            "title",
            "/repositories/2/archival_objects/3",
            Some("Shadow 3".to_string()),
            Some("Solr 3".to_string()),
            2,
        );

        record_field_mismatch(
            &mut mismatches,
            "publish",
            "/repositories/2/archival_objects/1",
            Some("true".to_string()),
            Some("false".to_string()),
            2,
        );

        assert_eq!(mismatches.len(), 2);

        let title_summary = mismatches.get("title").unwrap();
        assert_eq!(title_summary.count, 3);
        assert_eq!(title_summary.samples.len(), 2);
        assert_eq!(
            title_summary.samples[0].uri,
            "/repositories/2/archival_objects/1"
        );
        assert_eq!(
            title_summary.samples[0].shadow_value.as_deref(),
            Some("Shadow 1")
        );
        assert_eq!(
            title_summary.samples[0].solr_value.as_deref(),
            Some("Solr 1")
        );
        assert_eq!(
            title_summary.samples[1].uri,
            "/repositories/2/archival_objects/2"
        );

        let publish_summary = mismatches.get("publish").unwrap();
        assert_eq!(publish_summary.count, 1);
        assert_eq!(publish_summary.samples.len(), 1);
        assert_eq!(
            publish_summary.samples[0].uri,
            "/repositories/2/archival_objects/1"
        );
        assert_eq!(
            publish_summary.samples[0].shadow_value.as_deref(),
            Some("true")
        );
        assert_eq!(
            publish_summary.samples[0].solr_value.as_deref(),
            Some("false")
        );
    }

    #[tokio::test]
    async fn test_run_shadow_index_parity_report_rejects_unsupported_target_before_network() {
        let client = Client::new("http://example.invalid");
        let args = ShadowIndexParityReportArgs {
            repo: 2,
            target: "resources".to_string(),
            solr_url: "http://localhost:8985/solr".to_string(),
            live_collection: "archivesspace".to_string(),
            limit: None,
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            json: false,
        };

        let result = run_shadow_index_parity_report(&client, args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Unsupported target 'resources'. Only 'archival-objects' is supported."
        );
    }

    #[test]
    fn test_compare_shadow_doc_with_solr_exact_match() {
        use aspace_client::models::{AncestorRef, Date, LinkedAgentRef, Ref};

        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/100".to_string(),
            title: Some("Sample Title".to_string()),
            display_string: Some("Sample Title Disp".to_string()),
            publish: true,
            level: Some("item".to_string()),
            ref_id: Some("ref_100".to_string()),
            component_id: Some("comp_100".to_string()),
            position: Some(8),
            dates: vec![Date {
                expression: Some("1901-1902".to_string()),
                ..Default::default()
            }],
            resource: Some(Ref {
                ref_uri: "/repositories/2/resources/5".to_string(),
            }),
            parent: Some(Ref {
                ref_uri: "/repositories/2/archival_objects/50".to_string(),
            }),
            subjects: vec![Ref {
                ref_uri: "/subjects/1".to_string(),
            }],
            linked_agents: vec![LinkedAgentRef {
                ref_uri: Some("/agents/people/1".to_string()),
                role: Some("creator".to_string()),
                relator: Some("aut".to_string()),
                title: Some("Author".to_string()),
                terms: Vec::new(),
            }],
            ancestors: vec![AncestorRef {
                ref_uri: Some("/repositories/2/resources/5".to_string()),
                title: Some("Resource 5".to_string()),
                level: Some("collection".to_string()),
            }],
            ..Default::default()
        };

        let doc = ShadowDocument::from_archival_object(2, 100, ao);

        let solr_json = serde_json::json!({
            "uri": "/repositories/2/archival_objects/100",
            "primary_type": "archival_object",
            "title": "Sample Title",
            "display_string": "Sample Title Disp",
            "publish": true,
            "level": "item",
            "ref_id": "ref_100",
            "component_id": "comp_100",
            "position_i": 8,
            "parent_resource": "/repositories/2/resources/5",
            "resource_uri": "/repositories/2/resources/5",
            "parent_uri": "/repositories/2/archival_objects/50",
            "date_expressions": ["1901-1902"],
            "subject_uris": ["/subjects/1"],
            "linked_agent_uris": ["/agents/people/1"],
            "linked_agent_roles": ["creator"],
            "linked_agent_relators": ["aut"],
            "linked_agent_titles": ["Author"],
            "ancestor_uris": ["/repositories/2/resources/5"],
            "ancestor_titles": ["Resource 5"],
            "ancestor_levels": ["collection"]
        });

        let mismatches = compare_shadow_doc_with_solr(&doc, &solr_json);
        assert!(
            mismatches.is_empty(),
            "Expected 0 mismatches, found: {:?}",
            mismatches
        );
    }

    #[test]
    fn test_compare_shadow_doc_with_solr_detects_mismatches() {
        let ao = ArchivalObject {
            uri: "/repositories/2/archival_objects/101".to_string(),
            title: Some("Shadow Title".to_string()),
            publish: true,
            level: Some("item".to_string()),
            ..Default::default()
        };
        let doc = ShadowDocument::from_archival_object(2, 101, ao);

        let solr_json = serde_json::json!({
            "uri": "/repositories/2/archival_objects/101",
            "primary_type": "archival_object",
            "title": "Solr Title",
            "publish": false,
            "level": "file",
            "component_id": "comp_extra"
        });

        let mismatches = compare_shadow_doc_with_solr(&doc, &solr_json);
        let fields: Vec<String> = mismatches.into_iter().map(|(f, _, _)| f).collect();
        assert!(fields.contains(&"title".to_string()));
        assert!(fields.contains(&"publish".to_string()));
        assert!(fields.contains(&"level".to_string()));
        assert!(fields.contains(&"component_id".to_string()));
    }

    #[tokio::test]
    async fn test_run_shadow_index_parity_report_mocked_success() {
        use mockito::Server;
        let mut server = Server::new_async().await;

        let _mock_login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "test-session"}"#)
            .create_async()
            .await;

        let _mock_ids = server
            .mock("GET", "/repositories/2/archival_objects?all_ids=true")
            .with_status(200)
            .with_body("[1]")
            .create_async()
            .await;

        let _mock_records = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/repositories/2/archival_objects\?id_set="#.to_string()),
            )
            .with_status(200)
            .with_body(r#"[{"uri": "/repositories/2/archival_objects/1", "title": "AO 1", "publish": true}]"#)
            .create_async()
            .await;

        let _mock_solr = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/solr/archivesspace/select\?"#.to_string()),
            )
            .with_status(200)
            .with_body(
                r#"{
                "response": {
                    "numFound": 1,
                    "docs": [
                        {
                            "id": "/repositories/2/archival_objects/1",
                            "uri": "/repositories/2/archival_objects/1",
                            "primary_type": "archival_object",
                            "title": "AO 1",
                            "publish": true
                        }
                    ]
                }
            }"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "admin").await.unwrap();

        let args = ShadowIndexParityReportArgs {
            repo: 2,
            target: "archival-objects".to_string(),
            solr_url: format!("{}/solr", server.url()),
            live_collection: "archivesspace".to_string(),
            limit: Some(1),
            batch_size: std::num::NonZeroUsize::new(50).unwrap(),
            json: true,
        };

        let result = run_shadow_index_parity_report(&client, args).await;
        if let Err(e) = &result {
            eprintln!("TEST ERROR: {:?}", e);
        }
        assert!(result.is_ok());
    }
}
