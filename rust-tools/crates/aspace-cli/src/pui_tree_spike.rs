use anyhow::{bail, Context, Result};
use aspace_client::{
    build_index_document_batch, Client, IndexDocumentBatchRequest, IndexDocumentInput,
};
use futures::{stream, StreamExt, TryStreamExt};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::Instant;

#[derive(clap::Args, Clone, Debug)]
pub struct PuiTreeSpikeArgs {
    #[arg(long)]
    pub resource_uri: String,
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Serialize)]
struct PuiTreeSpikeReport {
    resource_uri: String,
    elapsed_seconds: f64,
    ordered_fetch_seconds: f64,
    root_fetch_seconds: f64,
    walk_seconds: f64,
    path_fetch_seconds: f64,
    document_count: usize,
    delete_candidates: Vec<String>,
    documents: Vec<Value>,
}

fn validate_resource_uri(uri: &str) -> Result<()> {
    let parts: Vec<&str> = uri.split('/').collect();
    if parts.len() != 5
        || !parts[0].is_empty()
        || parts[1] != "repositories"
        || parts[3] != "resources"
        || parts[2].parse::<u32>().ok().is_none_or(|v| v == 0)
        || parts[4].parse::<u32>().ok().is_none_or(|v| v == 0)
    {
        bail!("resource URI must be /repositories/<id>/resources/<id>");
    }
    Ok(())
}

async fn get_json(client: &Client, path: &str, query: &[(&str, String)]) -> Result<Value> {
    let response = client.get(path)?.query(query).send().await?;
    if !response.status().is_success() {
        bail!("PUI tree API {} returned HTTP {}", path, response.status());
    }
    Ok(response.json().await?)
}

fn nonnegative_count(record: &Value, key: &str) -> Result<usize> {
    let count = record
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("tree record missing integer {key}"))?;
    usize::try_from(count).context("tree count overflows usize")
}

fn auxiliary_document(
    uri: String,
    record_type: &str,
    parent: &str,
    record: Value,
) -> Result<Value> {
    let authoritative_json = serde_json::to_string(&record)?;
    let built = build_index_document_batch(IndexDocumentBatchRequest {
        records: vec![IndexDocumentInput {
            uri,
            record_type: record_type.to_string(),
            repository: Value::String("global".to_string()),
            record,
            prepare_pui_auxiliary: true,
            pui_parent_id: Some(parent.to_string()),
            pui_auxiliary_json: Some(authoritative_json),
            ..Default::default()
        }],
    })?;
    if built.documents.len() != 1 {
        bail!("auxiliary builder returned the wrong number of documents");
    }
    let mut doc = built
        .documents
        .into_iter()
        .next()
        .context("auxiliary document missing")?;
    doc.as_object_mut()
        .context("auxiliary document is not an object")?
        .retain(|key, _| !key.starts_with("_archivalist_"));
    Ok(doc)
}

struct TreeCollector<'a> {
    client: &'a Client,
    root_uri: &'a str,
    documents: Vec<Value>,
    node_uris: Vec<String>,
    delete_candidates: Vec<String>,
    seen_nodes: HashSet<String>,
}

impl TreeCollector<'_> {
    async fn walk(&mut self, root: Value) -> Result<()> {
        let mut pending = vec![(root, None::<String>)];
        let client = self.client;
        let root_uri = self.root_uri;
        while !pending.is_empty() {
            let mut waypoint_jobs = Vec::new();
            for (parent, parent_uri) in pending.drain(..) {
                let count = nonnegative_count(&parent, "waypoints")?;
                if count > 100_000 || waypoint_jobs.len().saturating_add(count) > 100_000 {
                    bail!("tree waypoint count exceeds spike bound");
                }
                for offset in 0..count {
                    waypoint_jobs.push((parent_uri.clone(), offset));
                }
            }
            let waypoints: Vec<(Option<String>, usize, Value)> = stream::iter(
                waypoint_jobs
                    .into_iter()
                    .map(|(parent_uri, offset)| async move {
                        let mut query = vec![
                            ("offset", offset.to_string()),
                            ("published_only", "true".to_string()),
                        ];
                        if let Some(ref uri) = parent_uri {
                            query.push(("parent_node", uri.clone()));
                        }
                        let waypoint =
                            get_json(client, &format!("{root_uri}/tree/waypoint"), &query).await?;
                        Ok::<_, anyhow::Error>((parent_uri, offset, waypoint))
                    }),
            )
            .buffered(32)
            .try_collect()
            .await?;

            let mut nonleaf_uris = Vec::new();
            for (parent_uri, offset, waypoint) in waypoints {
                let records = waypoint
                    .as_array()
                    .context("tree waypoint is not an array")?;
                let waypoint_uri = format!(
                    "{root_uri}/tree/waypoint_{}_{}",
                    parent_uri.as_deref().unwrap_or(""),
                    offset
                );
                self.documents.push(auxiliary_document(
                    waypoint_uri,
                    "tree_waypoint",
                    parent_uri.as_deref().unwrap_or(root_uri),
                    waypoint.clone(),
                )?);
                for record in records {
                    let node_uri = record
                        .get("uri")
                        .and_then(Value::as_str)
                        .context("waypoint record missing URI")?;
                    if !node_uri.starts_with("/repositories/")
                        || !self.seen_nodes.insert(node_uri.to_string())
                    {
                        bail!("invalid or duplicate tree node URI");
                    }
                    self.node_uris.push(node_uri.to_string());
                    if nonnegative_count(record, "child_count")? > 0 {
                        nonleaf_uris.push(node_uri.to_string());
                    } else {
                        self.delete_candidates
                            .push(format!("{root_uri}/tree/node_{node_uri}"));
                    }
                }
            }
            let children: Vec<(String, Value)> =
                stream::iter(nonleaf_uris.into_iter().map(|node_uri| async move {
                    let query = [
                        ("node_uri", node_uri.clone()),
                        ("published_only", "true".to_string()),
                    ];
                    let node = get_json(client, &format!("{root_uri}/tree/node"), &query).await?;
                    if node.get("uri").and_then(Value::as_str) != Some(node_uri.as_str()) {
                        bail!("tree node URI mismatch");
                    }
                    Ok((node_uri, node))
                }))
                .buffered(32)
                .try_collect()
                .await?;
            for (node_uri, node) in children {
                self.documents.push(auxiliary_document(
                    format!("{root_uri}/tree/node_{node_uri}"),
                    "tree_node",
                    &node_uri,
                    node.clone(),
                )?);
                pending.push((node, Some(node_uri)));
            }
        }
        Ok(())
    }

    async fn paths_to_root(&mut self) -> Result<()> {
        let uri_by_id: HashMap<u32, &str> = self
            .node_uris
            .iter()
            .map(|uri| {
                let id = uri
                    .rsplit('/')
                    .next()
                    .and_then(|s| s.parse::<u32>().ok())
                    .context("invalid node ID")?;
                Ok((id, uri.as_str()))
            })
            .collect::<Result<_>>()?;
        let chunks: Vec<Vec<String>> = self
            .node_uris
            .chunks(128)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|uri| {
                        uri.rsplit('/')
                            .next()
                            .map(str::to_string)
                            .context("missing node ID")
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<_>>()?;
        let client = self.client;
        let root_uri = self.root_uri;
        let results: Vec<(Vec<String>, Value)> =
            stream::iter(chunks.into_iter().map(|ids| async move {
                let mut query: Vec<(&str, String)> =
                    ids.iter().map(|id| ("node_ids[]", id.clone())).collect();
                query.push(("published_only", "true".to_string()));
                let paths =
                    get_json(client, &format!("{root_uri}/tree/node_from_root"), &query).await?;
                Ok::<_, anyhow::Error>((ids, paths))
            }))
            .buffered(8)
            .try_collect()
            .await?;
        for (ids, paths) in results {
            let entries = paths
                .as_object()
                .context("node_from_root response is not an object")?;
            if entries.len() != ids.len() {
                bail!("node_from_root response count mismatch");
            }
            let expected: HashSet<&str> = ids.iter().map(String::as_str).collect();
            for (id, path) in entries {
                if !expected.contains(id.as_str()) {
                    bail!("unexpected node_from_root ID in batch");
                }
                let node_id = id.parse::<u32>().context("invalid node_from_root ID")?;
                let node_uri = uri_by_id
                    .get(&node_id)
                    .context("unexpected node_from_root ID")?;
                let mut payload = serde_json::Map::new();
                payload.insert(id.clone(), path.clone());
                self.documents.push(auxiliary_document(
                    format!("{}/tree/node_from_root_{}", self.root_uri, id),
                    "tree_node_from_root",
                    node_uri,
                    Value::Object(payload),
                )?);
            }
        }
        Ok(())
    }
}

pub async fn run_pui_tree_spike(client: &Client, args: &PuiTreeSpikeArgs) -> Result<()> {
    validate_resource_uri(&args.resource_uri)?;
    let name = args
        .output
        .file_name()
        .and_then(|v| v.to_str())
        .context("invalid output path")?;
    if !name.starts_with("pui_tree_rust_") || !name.ends_with(".json") {
        bail!("output must be named pui_tree_rust_*.json");
    }
    let started = Instant::now();
    let ordered = get_json(
        client,
        &format!("{}/ordered_records", args.resource_uri),
        &[],
    )
    .await?;
    let ordered_done = Instant::now();
    let root = get_json(
        client,
        &format!("{}/tree/root", args.resource_uri),
        &[("published_only", "true".to_string())],
    )
    .await?;
    let root_done = Instant::now();
    let mut collector = TreeCollector {
        client,
        root_uri: &args.resource_uri,
        documents: vec![
            auxiliary_document(
                format!("{}/ordered_records", args.resource_uri),
                "resource_ordered_records",
                &args.resource_uri,
                ordered,
            )?,
            auxiliary_document(
                format!("{}/tree/root", args.resource_uri),
                "tree_root",
                &args.resource_uri,
                root.clone(),
            )?,
        ],
        node_uris: Vec::new(),
        delete_candidates: Vec::new(),
        seen_nodes: HashSet::new(),
    };
    collector.walk(root).await?;
    let walk_done = Instant::now();
    collector.paths_to_root().await?;
    let paths_done = Instant::now();
    let mut seen_ids = HashSet::new();
    for doc in &collector.documents {
        let id = doc
            .get("id")
            .and_then(Value::as_str)
            .context("auxiliary document missing ID")?;
        if !seen_ids.insert(id) {
            bail!("duplicate auxiliary document ID");
        }
    }
    let report = PuiTreeSpikeReport {
        resource_uri: args.resource_uri.clone(),
        elapsed_seconds: started.elapsed().as_secs_f64(),
        ordered_fetch_seconds: ordered_done.duration_since(started).as_secs_f64(),
        root_fetch_seconds: root_done.duration_since(ordered_done).as_secs_f64(),
        walk_seconds: walk_done.duration_since(root_done).as_secs_f64(),
        path_fetch_seconds: paths_done.duration_since(walk_done).as_secs_f64(),
        document_count: collector.documents.len(),
        delete_candidates: collector.delete_candidates,
        documents: collector.documents,
    };
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&args.output)?;
    file.write_all(&serde_json::to_vec(&report)?)?;
    file.sync_all()?;
    println!(
        "PUI tree spike: {} auxiliary documents in {:.3}s; report {}",
        report.document_count,
        report.elapsed_seconds,
        args.output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_uri_is_strict() {
        assert!(validate_resource_uri("/repositories/4/resources/6").is_ok());
        for uri in [
            "/repositories/4/resources/6/tree",
            "/repositories/0/resources/6",
            "https://example.org/repositories/4/resources/6",
        ] {
            assert!(validate_resource_uri(uri).is_err());
        }
    }

    #[test]
    fn auxiliary_output_has_no_marker() {
        let doc = auxiliary_document(
            "/repositories/4/resources/6/tree/root".into(),
            "tree_root",
            "/repositories/4/resources/6",
            serde_json::json!({"waypoints":0}),
        )
        .unwrap();
        assert!(doc
            .as_object()
            .unwrap()
            .keys()
            .all(|key| !key.starts_with("_archivalist_")));
    }
}
