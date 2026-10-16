use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Default listen port for the shadow query sidecar.
pub const DEFAULT_SIDECAR_PORT: u16 = 4570;

/// Default Solr core/collection name for shadow queries.
pub const DEFAULT_SHADOW_COLLECTION: &str = "archivesspace_shadow";

/// Default Solr base URL.
pub const DEFAULT_SOLR_URL: &str = "http://localhost:8985/solr";

#[derive(Parser, Debug)]
#[command(name = "aspace-shadow-sidecar")]
#[command(
    about = "Long-running read-only HTTP sidecar prototype for ArchivesSpace shadow Solr queries"
)]
struct SidecarArgs {
    /// Port to listen on.
    #[arg(
        long,
        env = "ARCHIVALIST_SIDECAR_PORT",
        default_value_t = DEFAULT_SIDECAR_PORT
    )]
    port: u16,

    /// Bind host.
    #[arg(long, env = "ARCHIVALIST_SIDECAR_HOST", default_value = "127.0.0.1")]
    host: String,

    /// Default Solr base URL.
    #[arg(long, env = "ARCHIVALIST_SOLR_URL", default_value = DEFAULT_SOLR_URL)]
    solr_url: String,

    /// Default collection name.
    #[arg(
        long,
        env = "ARCHIVALIST_SHADOW_COLLECTION",
        default_value = DEFAULT_SHADOW_COLLECTION
    )]
    collection: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ShadowQueryParams {
    pub resource_uri: String,
    pub date_filter: Option<String>,
    pub title_filter: Option<String>,
    pub rows: usize,
    pub start: usize,
    pub collection: String,
    pub solr_url: String,
    pub tree: bool,
    pub include_ancestors: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ShadowSidecarResponse {
    pub resource_uri: String,
    pub date_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_filter: Option<String>,
    pub solr_url: String,
    pub collection: String,
    pub rows: usize,
    pub start: usize,
    pub num_found: u64,
    pub docs: Vec<aspace_client::ShadowArchivalObjectHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_nodes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<Vec<aspace_client::ShadowTreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orphans: Option<Vec<aspace_client::ShadowTreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub served_by: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ShadowSearchParams {
    pub q: Option<String>,
    pub resource_uri: Option<String>,
    pub date_filter: Option<String>,
    pub title_filter: Option<String>,
    pub rows: usize,
    pub start: usize,
    pub collection: String,
    pub solr_url: String,
    pub tree: bool,
    pub include_ancestors: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ShadowSearchResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    pub date_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_filter: Option<String>,
    pub solr_url: String,
    pub collection: String,
    pub rows: usize,
    pub start: usize,
    pub num_found: u64,
    pub docs: Vec<aspace_client::ShadowArchivalObjectHit>,
    pub query_string: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_nodes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<Vec<aspace_client::ShadowTreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orphans: Option<Vec<aspace_client::ShadowTreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub served_by: Option<String>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct ShadowHealthResponse {
    pub status: String,
    pub served_by: String,
    pub solr_url: String,
    pub collection: String,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct ShadowReadyResponse {
    pub status: String,
    pub served_by: String,
    pub solr_url: String,
    pub collection: String,
    pub num_found: u64,
}

#[derive(Deserialize)]
struct SolrReadyEnvelope {
    response: SolrReadyResult,
}

#[derive(Deserialize)]
struct SolrReadyResult {
    #[serde(rename = "numFound")]
    num_found: u64,
}

pub fn build_health_response(solr_url: &str, collection: &str) -> ShadowHealthResponse {
    ShadowHealthResponse {
        status: "ok".to_string(),
        served_by: "aspace-shadow-sidecar".to_string(),
        solr_url: solr_url.to_string(),
        collection: collection.to_string(),
    }
}

pub async fn check_readiness(
    client: &reqwest::Client,
    solr_url: &str,
    collection: &str,
) -> Result<ShadowReadyResponse> {
    let url = format!("{}/{}/select", solr_url.trim_end_matches('/'), collection);
    let response = client
        .get(&url)
        .query(&[("q", "*:*"), ("rows", "0"), ("wt", "json")])
        .send()
        .await
        .with_context(|| format!("failed to query shadow Solr collection at {url}"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("shadow Solr readiness query returned HTTP {status}");
    }
    let payload: SolrReadyEnvelope = response
        .json()
        .await
        .context("invalid shadow Solr readiness response")?;

    Ok(ShadowReadyResponse {
        status: "ready".to_string(),
        served_by: "aspace-shadow-sidecar".to_string(),
        solr_url: solr_url.to_string(),
        collection: collection.to_string(),
        num_found: payload.response.num_found,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum SidecarRoute<'a> {
    Health,
    Ready,
    Metrics,
    Descendants(&'a str),
    Search(&'a str),
    ResourceTreeWaypoint(&'a str),
    ResourceTreeNodeFromRoot(&'a str),
    NotFound(&'a str),
}

pub fn route_request<'a>(method: &str, full_path: &'a str) -> Result<SidecarRoute<'a>, String> {
    if method != "GET" {
        return Err(format!(
            "Method '{method}' not allowed; only GET is supported"
        ));
    }
    let (path, query_str) = match full_path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (full_path, ""),
    };
    match path {
        "/health" => Ok(SidecarRoute::Health),
        "/ready" => Ok(SidecarRoute::Ready),
        "/metrics" => Ok(SidecarRoute::Metrics),
        "/shadow/descendants" => Ok(SidecarRoute::Descendants(query_str)),
        "/shadow/search" => Ok(SidecarRoute::Search(query_str)),
        "/shadow/resource-tree/waypoint" => Ok(SidecarRoute::ResourceTreeWaypoint(query_str)),
        "/shadow/resource-tree/node-from-root" => {
            Ok(SidecarRoute::ResourceTreeNodeFromRoot(query_str))
        }
        _ => Ok(SidecarRoute::NotFound(path)),
    }
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    error: String,
}

pub fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.as_bytes().iter();
    while let Some(&b) = chars.next() {
        if b == b'%' {
            if let (Some(&h1), Some(&h2)) = (chars.next(), chars.next()) {
                if let Ok(hex_str) = std::str::from_utf8(&[h1, h2]) {
                    if let Ok(byte) = u8::from_str_radix(hex_str, 16) {
                        result.push(byte as char);
                        continue;
                    }
                }
                result.push('%');
                result.push(h1 as char);
                result.push(h2 as char);
            } else {
                result.push('%');
            }
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

pub fn parse_query_string(query_str: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query_str.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let val = parts.next().unwrap_or("");
        if !key.is_empty() {
            map.insert(url_decode(key), url_decode(val));
        }
    }
    map
}

pub fn parse_shadow_query_params(
    query_str: &str,
    default_solr_url: &str,
    default_collection: &str,
) -> Result<ShadowQueryParams> {
    let map = parse_query_string(query_str);

    let resource_uri = match map.get("resource_uri") {
        Some(uri) if !uri.trim().is_empty() => uri.trim().to_string(),
        _ => bail!("missing required query parameter 'resource_uri'"),
    };
    let date_filter = map
        .get("date_filter")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let title_filter = map
        .get("title_filter")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let rows = match map.get("rows") {
        Some(r) => r
            .parse::<usize>()
            .with_context(|| format!("invalid rows parameter: '{r}'"))?,
        None => 50,
    };

    let start = match map.get("start") {
        Some(s) => s
            .parse::<usize>()
            .with_context(|| format!("invalid start parameter: '{s}'"))?,
        None => 0,
    };

    let collection = map
        .get("collection")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| default_collection.to_string());

    if collection == "archivesspace" {
        bail!("refusing to query live Solr collection 'archivesspace'");
    }

    let solr_url = map
        .get("solr_url")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| default_solr_url.to_string());

    let tree = match map.get("tree") {
        Some(t) => {
            let t_lower = t.trim().to_lowercase();
            t_lower == "true" || t_lower == "1"
        }
        None => false,
    };

    let include_ancestors = match map.get("include_ancestors") {
        Some(ia) => {
            let ia_lower = ia.trim().to_lowercase();
            ia_lower == "true" || ia_lower == "1"
        }
        None => false,
    };

    if include_ancestors && !tree {
        bail!("include_ancestors requires tree=true");
    }

    if tree && rows == 0 {
        bail!("cannot assemble tree with rows=0 (documents must be fetched)");
    }

    Ok(ShadowQueryParams {
        resource_uri,
        date_filter,
        title_filter,
        rows,
        start,
        collection,
        solr_url,
        tree,
        include_ancestors,
    })
}

pub fn parse_shadow_search_params(
    query_str: &str,
    default_solr_url: &str,
    default_collection: &str,
) -> Result<ShadowSearchParams> {
    let map = parse_query_string(query_str);

    let collection = map
        .get("collection")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| default_collection.to_string());

    if collection == "archivesspace" {
        bail!("refusing to query live Solr collection 'archivesspace'");
    }

    let tree = match map.get("tree") {
        Some(t) => {
            let t_lower = t.trim().to_lowercase();
            t_lower == "true" || t_lower == "1"
        }
        None => false,
    };

    let include_ancestors = match map.get("include_ancestors") {
        Some(ia) => {
            let ia_lower = ia.trim().to_lowercase();
            ia_lower == "true" || ia_lower == "1"
        }
        None => false,
    };

    if include_ancestors && !tree {
        bail!("include_ancestors requires tree=true");
    }

    let resource_uri = map
        .get("resource_uri")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    if tree && resource_uri.is_none() {
        bail!("tree=true requires resource_uri");
    }

    let rows = match map.get("rows") {
        Some(r) => r
            .parse::<usize>()
            .with_context(|| format!("invalid rows parameter: '{r}'"))?,
        None => 50,
    };

    if tree && rows == 0 {
        bail!("cannot assemble tree with rows=0 (documents must be fetched)");
    }

    let q = map
        .get("q")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let date_filter = map
        .get("date_filter")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let title_filter = map
        .get("title_filter")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let start = match map.get("start") {
        Some(s) => s
            .parse::<usize>()
            .with_context(|| format!("invalid start parameter: '{s}'"))?,
        None => 0,
    };

    let solr_url = map
        .get("solr_url")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| default_solr_url.to_string());

    Ok(ShadowSearchParams {
        q,
        resource_uri,
        date_filter,
        title_filter,
        rows,
        start,
        collection,
        solr_url,
        tree,
        include_ancestors,
    })
}

pub fn parse_shadow_waypoint_params(
    query_str: &str,
    default_solr_url: &str,
    default_collection: &str,
) -> Result<aspace_client::ShadowTreeWaypointParams> {
    let map = parse_query_string(query_str);

    let resource_uri = match map.get("resource_uri") {
        Some(uri) if !uri.trim().is_empty() => uri.trim().to_string(),
        _ => bail!("missing required query parameter 'resource_uri'"),
    };
    let offset = match map.get("offset") {
        Some(o) => o
            .parse::<usize>()
            .with_context(|| format!("invalid offset parameter: '{o}'"))?,
        None => bail!("missing required query parameter 'offset'"),
    };

    let parent_node = map
        .get("parent_node")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let collection = map
        .get("collection")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| default_collection.to_string());

    if collection == "archivesspace" {
        bail!("refusing to query live Solr collection 'archivesspace'");
    }

    let solr_url = map
        .get("solr_url")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| default_solr_url.to_string());

    let mut params = aspace_client::ShadowTreeWaypointParams::new(resource_uri, offset)
        .with_base_url(solr_url)
        .with_collection(collection);

    if let Some(parent) = parent_node {
        params = params.with_parent_node(parent);
    }

    Ok(params)
}

pub fn parse_shadow_node_from_root_params(
    query_str: &str,
    default_solr_url: &str,
    default_collection: &str,
) -> Result<aspace_client::ShadowTreeNodeFromRootParams> {
    let map = parse_query_string(query_str);
    let resource_uri = match map.get("resource_uri") {
        Some(uri) if !uri.trim().is_empty() => uri.trim().to_string(),
        _ => bail!("missing required query parameter 'resource_uri'"),
    };
    let root_title = match map.get("root_title") {
        Some(title) if !title.trim().is_empty() => title.to_string(),
        _ => bail!("missing required query parameter 'root_title'"),
    };
    let node_ids = match map.get("node_ids") {
        Some(ids) => ids
            .split(',')
            .map(|id| {
                id.trim()
                    .parse::<u64>()
                    .with_context(|| format!("invalid node ID: '{id}'"))
            })
            .collect::<Result<Vec<_>>>()?,
        None => bail!("missing required query parameter 'node_ids'"),
    };
    if node_ids.is_empty() || node_ids.len() > 128 || node_ids.contains(&0) {
        bail!("node_ids must contain between 1 and 128 positive IDs");
    }
    let mut unique = std::collections::HashSet::new();
    if !node_ids.iter().all(|id| unique.insert(*id)) {
        bail!("node_ids must not contain duplicates");
    }
    let published_only = match map.get("published_only").map(String::as_str) {
        None | Some("false") | Some("0") => false,
        Some("true") | Some("1") => true,
        Some(value) => bail!("invalid published_only parameter: '{value}'"),
    };
    let collection = map
        .get("collection")
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| default_collection.to_string());
    if collection == "archivesspace" {
        bail!("refusing to query live Solr collection 'archivesspace'");
    }
    let solr_url = map
        .get("solr_url")
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| default_solr_url.to_string());

    Ok(
        aspace_client::ShadowTreeNodeFromRootParams::new(resource_uri, root_title, node_ids)
            .with_base_url(solr_url)
            .with_collection(collection)
            .with_published_only(published_only),
    )
}

struct AppState {
    solr_url: String,
    collection: String,
    client: aspace_client::ShadowSolrClient,
    readiness_client: reqwest::Client,
    metrics: Arc<SidecarMetrics>,
}

struct SidecarMetrics {
    started_at: Instant,
    requests_total: AtomicU64,
    requests_active: AtomicU64,
}

impl SidecarMetrics {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            requests_total: AtomicU64::new(0),
            requests_active: AtomicU64::new(0),
        }
    }

    fn render(&self) -> String {
        format!(
            concat!(
                "# HELP archivalist_sidecar_build_info Static build information.\n",
                "# TYPE archivalist_sidecar_build_info gauge\n",
                "archivalist_sidecar_build_info{{version=\"{}\"}} 1\n",
                "# HELP archivalist_sidecar_uptime_seconds Seconds since the sidecar started.\n",
                "# TYPE archivalist_sidecar_uptime_seconds gauge\n",
                "archivalist_sidecar_uptime_seconds {:.3}\n",
                "# HELP archivalist_sidecar_requests_total HTTP requests accepted by the sidecar.\n",
                "# TYPE archivalist_sidecar_requests_total counter\n",
                "archivalist_sidecar_requests_total {}\n",
                "# HELP archivalist_sidecar_requests_active HTTP requests currently being handled.\n",
                "# TYPE archivalist_sidecar_requests_active gauge\n",
                "archivalist_sidecar_requests_active {}\n"
            ),
            env!("CARGO_PKG_VERSION"),
            self.started_at.elapsed().as_secs_f64(),
            self.requests_total.load(Ordering::Relaxed),
            self.requests_active.load(Ordering::Relaxed)
        )
    }
}

struct ActiveRequestGuard {
    metrics: Arc<SidecarMetrics>,
}

impl ActiveRequestGuard {
    fn begin(metrics: Arc<SidecarMetrics>) -> Self {
        metrics.requests_total.fetch_add(1, Ordering::Relaxed);
        metrics.requests_active.fetch_add(1, Ordering::Relaxed);
        Self { metrics }
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.metrics.requests_active.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle_connection(mut stream: TcpStream, state: Arc<AppState>) -> Result<()> {
    let mut buffer = [0u8; 4096];
    let bytes_read = stream.read(&mut buffer).await?;
    if bytes_read == 0 {
        return Ok(());
    }
    let _request_guard = ActiveRequestGuard::begin(Arc::clone(&state.metrics));

    let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);
    let mut lines = request_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let full_path = parts.next().unwrap_or("");

    let route = match route_request(method, full_path) {
        Ok(r) => r,
        Err(err_msg) => {
            let response_body = serde_json::to_string(&ErrorResponse { error: err_msg })?;
            let response = format!(
                "HTTP/1.1 405 Method Not Allowed\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
    };

    match route {
        SidecarRoute::Health => {
            let health = build_health_response(&state.solr_url, &state.collection);
            let response_body = serde_json::to_string(&health)?;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
        SidecarRoute::Ready => {
            match check_readiness(&state.readiness_client, &state.solr_url, &state.collection).await
            {
                Ok(readiness) => {
                    let response_body = serde_json::to_string(&readiness)?;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
                Err(err) => {
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: format!("Shadow Solr is not ready: {err}"),
                    })?;
                    let response = format!(
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
            }
            return Ok(());
        }
        SidecarRoute::Metrics => {
            let response_body = state.metrics.render();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
        SidecarRoute::NotFound(path) => {
            let response_body = serde_json::to_string(&ErrorResponse {
                error: format!("Not found: '{path}'"),
            })?;
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
        SidecarRoute::ResourceTreeNodeFromRoot(query_str) => {
            let params = match parse_shadow_node_from_root_params(
                query_str,
                &state.solr_url,
                &state.collection,
            ) {
                Ok(params) => params,
                Err(err) => {
                    let status_code = if err.to_string().contains("refusing") {
                        403
                    } else {
                        400
                    };
                    let status_text = if status_code == 403 {
                        "Forbidden"
                    } else {
                        "Bad Request"
                    };
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: err.to_string(),
                    })?;
                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status_code,
                        status_text,
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                    return Ok(());
                }
            };

            let result =
                if params.base_url == state.solr_url && params.collection == state.collection {
                    state.client.tree_node_from_root(&params).await
                } else {
                    let client =
                        aspace_client::ShadowSolrClient::new(&params.base_url, &params.collection);
                    client.tree_node_from_root(&params).await
                };

            match result {
                Ok(paths) => {
                    let response_body = serde_json::to_string(&paths)?;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
                Err(err) => {
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: format!("Solr node-from-root error: {err}"),
                    })?;
                    let response = format!(
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
            }
            return Ok(());
        }
        SidecarRoute::ResourceTreeWaypoint(query_str) => {
            let params = match parse_shadow_waypoint_params(
                query_str,
                &state.solr_url,
                &state.collection,
            ) {
                Ok(p) => p,
                Err(err) => {
                    let status_code = if err.to_string().contains("refusing") {
                        403
                    } else {
                        400
                    };
                    let status_text = if status_code == 403 {
                        "Forbidden"
                    } else {
                        "Bad Request"
                    };
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: err.to_string(),
                    })?;
                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status_code,
                        status_text,
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                    return Ok(());
                }
            };

            let waypoint_res =
                if params.base_url == state.solr_url && params.collection == state.collection {
                    state.client.tree_waypoint(&params).await
                } else {
                    let dynamic_client =
                        aspace_client::ShadowSolrClient::new(&params.base_url, &params.collection);
                    dynamic_client.tree_waypoint(&params).await
                };

            match waypoint_res {
                Ok(nodes) => {
                    let response_body = serde_json::to_string(&nodes)?;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
                Err(err) => {
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: format!("Solr waypoint error: {err}"),
                    })?;
                    let response = format!(
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
            }
            return Ok(());
        }
        SidecarRoute::Descendants(q) => {
            let query_str = q;
            let start_instant = std::time::Instant::now();
            let params = match parse_shadow_query_params(
                query_str,
                &state.solr_url,
                &state.collection,
            ) {
                Ok(p) => p,
                Err(err) => {
                    let status_code = if err.to_string().contains("refusing") {
                        403
                    } else {
                        400
                    };
                    let status_text = if status_code == 403 {
                        "Forbidden"
                    } else {
                        "Bad Request"
                    };
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: err.to_string(),
                    })?;
                    let response = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_code,
                    status_text,
                    response_body.len(),
                    response_body
                );
                    stream.write_all(response.as_bytes()).await?;
                    return Ok(());
                }
            };

            let search_params = aspace_client::ShadowSearchParams {
                base_url: params.solr_url.clone(),
                collection: params.collection.clone(),
                resource_uri: params.resource_uri.clone(),
                date_filter: params.date_filter.clone(),
                title_filter: params.title_filter.clone(),
                rows: params.rows,
                start: params.start,
            };

            // Use warm client if target matches default client, or construct target client
            let (search_res, active_client) =
                if params.solr_url == state.solr_url && params.collection == state.collection {
                    (
                        state.client.search(&search_params).await,
                        state.client.clone(),
                    )
                } else {
                    let dynamic_client =
                        aspace_client::ShadowSolrClient::new(&params.solr_url, &params.collection);
                    (dynamic_client.search(&search_params).await, dynamic_client)
                };

            let elapsed_ms = start_instant.elapsed().as_secs_f64() * 1000.0;

            match search_res {
                Ok(result) => {
                    let (total_nodes, tree, orphans) = if params.tree {
                        let tree_res = if params.include_ancestors {
                            let missing_ancestors = aspace_client::collect_missing_ancestor_uris(
                                &result.docs,
                                &params.resource_uri,
                            );
                            let context_hits = match active_client
                                .docs_by_ids(&missing_ancestors)
                                .await
                            {
                                Ok(hits) => hits,
                                Err(e) => {
                                    let response_body = serde_json::to_string(&ErrorResponse {
                                        error: format!("failed to fetch ancestor context: {e}"),
                                    })?;
                                    let response = format!(
                                    "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    response_body.len(),
                                    response_body
                                );
                                    stream.write_all(response.as_bytes()).await?;
                                    return Ok(());
                                }
                            };
                            aspace_client::assemble_shadow_tree_with_context(
                                result.docs.clone(),
                                context_hits,
                                &params.resource_uri,
                            )
                        } else {
                            aspace_client::assemble_shadow_tree(
                                result.docs.clone(),
                                &params.resource_uri,
                            )
                        };
                        (
                            Some(tree_res.total_nodes),
                            Some(tree_res.tree),
                            Some(tree_res.orphans),
                        )
                    } else {
                        (None, None, None)
                    };

                    let output = ShadowSidecarResponse {
                        resource_uri: params.resource_uri,
                        date_filter: params.date_filter,
                        title_filter: params.title_filter,
                        solr_url: params.solr_url,
                        collection: params.collection,
                        rows: params.rows,
                        start: params.start,
                        num_found: result.num_found,
                        docs: result.docs,
                        total_nodes,
                        tree,
                        orphans,
                        elapsed_ms: Some((elapsed_ms * 100.0).round() / 100.0),
                        served_by: Some("aspace-shadow-sidecar".to_string()),
                    };
                    let response_body = serde_json::to_string(&output)?;
                    let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                    stream.write_all(response.as_bytes()).await?;
                }
                Err(err) => {
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: format!("Solr search error: {err}"),
                    })?;
                    let response = format!(
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                    stream.write_all(response.as_bytes()).await?;
                }
            }
            return Ok(());
        }
        SidecarRoute::Search(query_str) => {
            let start_instant = std::time::Instant::now();
            let params = match parse_shadow_search_params(
                query_str,
                &state.solr_url,
                &state.collection,
            ) {
                Ok(p) => p,
                Err(err) => {
                    let status_code = if err.to_string().contains("refusing") {
                        403
                    } else {
                        400
                    };
                    let status_text = if status_code == 403 {
                        "Forbidden"
                    } else {
                        "Bad Request"
                    };
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: err.to_string(),
                    })?;
                    let response = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_code,
                status_text,
                response_body.len(),
                response_body
            );
                    stream.write_all(response.as_bytes()).await?;
                    return Ok(());
                }
            };

            let keyword_params = aspace_client::ShadowKeywordSearchParams {
                base_url: params.solr_url.clone(),
                collection: params.collection.clone(),
                query: params.q.clone(),
                resource_uri: params.resource_uri.clone(),
                date_filter: params.date_filter.clone(),
                title_filter: params.title_filter.clone(),
                rows: params.rows,
                start: params.start,
            };

            // Use warm client if target matches default client, or construct target client
            let (search_res, active_client) =
                if params.solr_url == state.solr_url && params.collection == state.collection {
                    (
                        state.client.keyword_search(&keyword_params).await,
                        state.client.clone(),
                    )
                } else {
                    let dynamic_client =
                        aspace_client::ShadowSolrClient::new(&params.solr_url, &params.collection);
                    (
                        dynamic_client.keyword_search(&keyword_params).await,
                        dynamic_client,
                    )
                };

            let elapsed_ms = start_instant.elapsed().as_secs_f64() * 1000.0;

            match search_res {
                Ok(result) => {
                    let (total_nodes, tree, orphans) = if params.tree {
                        let root_resource_uri = params
                            .resource_uri
                            .as_deref()
                            .expect("resource_uri validated for tree");

                        let tree_res = if params.include_ancestors {
                            let missing_ancestors = aspace_client::collect_missing_ancestor_uris(
                                &result.docs,
                                root_resource_uri,
                            );
                            let context_hits = match active_client
                                .docs_by_ids(&missing_ancestors)
                                .await
                            {
                                Ok(hits) => hits,
                                Err(e) => {
                                    let response_body = serde_json::to_string(&ErrorResponse {
                                        error: format!("failed to fetch ancestor context: {e}"),
                                    })?;
                                    let response = format!(
                                        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                        response_body.len(),
                                        response_body
                                    );
                                    stream.write_all(response.as_bytes()).await?;
                                    return Ok(());
                                }
                            };
                            aspace_client::assemble_shadow_tree_with_context(
                                result.docs.clone(),
                                context_hits,
                                root_resource_uri,
                            )
                        } else {
                            aspace_client::assemble_shadow_tree(
                                result.docs.clone(),
                                root_resource_uri,
                            )
                        };
                        (
                            Some(tree_res.total_nodes),
                            Some(tree_res.tree),
                            Some(tree_res.orphans),
                        )
                    } else {
                        (None, None, None)
                    };

                    let output = ShadowSearchResponse {
                        q: params.q,
                        resource_uri: params.resource_uri,
                        date_filter: params.date_filter,
                        title_filter: params.title_filter,
                        solr_url: params.solr_url,
                        collection: params.collection,
                        rows: params.rows,
                        start: params.start,
                        num_found: result.num_found,
                        docs: result.docs,
                        query_string: result.query_string,
                        total_nodes,
                        tree,
                        orphans,
                        elapsed_ms: Some((elapsed_ms * 100.0).round() / 100.0),
                        served_by: Some("aspace-shadow-sidecar".to_string()),
                    };
                    let response_body = serde_json::to_string(&output)?;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
                Err(err) => {
                    let response_body = serde_json::to_string(&ErrorResponse {
                        error: format!("Solr search error: {err}"),
                    })?;
                    let response = format!(
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).await?;
                }
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = SidecarArgs::parse();

    if args.collection == "archivesspace" {
        bail!("refusing to serve live Solr collection 'archivesspace'");
    }

    let bind_addr = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("Failed to bind TCP listener to {bind_addr}"))?;

    let warm_client = aspace_client::ShadowSolrClient::new(&args.solr_url, &args.collection);
    let readiness_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .context("Failed to build readiness HTTP client")?;
    let state = Arc::new(AppState {
        solr_url: args.solr_url,
        collection: args.collection,
        client: warm_client,
        readiness_client,
        metrics: Arc::new(SidecarMetrics::new()),
    });

    println!(
        "aspace-shadow-sidecar listening on http://{} (target: {}/{})",
        bind_addr, state.solr_url, state.collection
    );

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Accept error: {e}");
                continue;
            }
        };

        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, state_clone).await {
                eprintln!("Connection handling error: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_decode() {
        assert_eq!(
            url_decode("%2Frepositories%2F2%2Fresources%2F18"),
            "/repositories/2/resources/18"
        );
        assert_eq!(url_decode("hello+world"), "hello world");
        assert_eq!(url_decode("%2A1974%2A"), "*1974*");
    }

    #[test]
    fn test_parse_query_string() {
        let qs = "resource_uri=%2Frepositories%2F2%2Fresources%2F18&date_filter=*1974*&rows=0";
        let parsed = parse_query_string(qs);
        assert_eq!(
            parsed.get("resource_uri").unwrap(),
            "/repositories/2/resources/18"
        );
        assert_eq!(parsed.get("date_filter").unwrap(), "*1974*");
        assert_eq!(parsed.get("rows").unwrap(), "0");
    }

    #[test]
    fn test_parse_shadow_query_params_defaults() {
        let qs = "resource_uri=%2Frepositories%2F2%2Fresources%2F18";
        let params =
            parse_shadow_query_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap();
        assert_eq!(params.resource_uri, "/repositories/2/resources/18");
        assert_eq!(params.date_filter, None);
        assert_eq!(params.title_filter, None);
        assert_eq!(params.rows, 50);
        assert_eq!(params.start, 0);
        assert_eq!(params.collection, "archivesspace_shadow");
        assert_eq!(params.solr_url, "http://localhost:8985/solr");
        assert!(!params.tree);
    }

    #[test]
    fn test_parse_shadow_query_params_custom() {
        let qs = "resource_uri=%2Frepositories%2F2%2Fresources%2F18&date_filter=*1974*&title_filter=*Example*&rows=10&start=5&solr_url=http%3A%2F%2Fexample.com%3A8983%2Fsolr&collection=custom_shadow&tree=true";
        let params =
            parse_shadow_query_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap();
        assert_eq!(params.resource_uri, "/repositories/2/resources/18");
        assert_eq!(params.date_filter.as_deref(), Some("*1974*"));
        assert_eq!(params.title_filter.as_deref(), Some("*Example*"));
        assert_eq!(params.rows, 10);
        assert_eq!(params.start, 5);
        assert_eq!(params.collection, "custom_shadow");
        assert_eq!(params.solr_url, "http://example.com:8983/solr");
        assert!(params.tree);
    }

    #[test]
    fn test_parse_shadow_query_params_tree_with_zero_rows_fails() {
        let qs = "resource_uri=%2Frepositories%2F2%2Fresources%2F18&rows=0&tree=true";
        let err =
            parse_shadow_query_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot assemble tree with rows=0 (documents must be fetched)"
        );
    }

    #[test]
    fn test_parse_shadow_query_params_missing_resource_uri() {
        let qs = "date_filter=*1974*&rows=0";
        let err =
            parse_shadow_query_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap_err();
        assert!(err
            .to_string()
            .contains("missing required query parameter 'resource_uri'"));
    }

    #[test]
    fn test_parse_shadow_query_params_refuses_live_collection() {
        let qs = "resource_uri=%2Frepositories%2F2%2Fresources%2F18&collection=archivesspace";
        let err =
            parse_shadow_query_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap_err();
        assert_eq!(
            err.to_string(),
            "refusing to query live Solr collection 'archivesspace'"
        );
    }

    #[test]
    fn test_parse_shadow_waypoint_params_defaults_and_parent() {
        let qs = "resource_uri=%2Frepositories%2F2%2Fresources%2F18&offset=2&parent_node=%2Frepositories%2F2%2Farchival_objects%2F100";
        let params =
            parse_shadow_waypoint_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap();

        assert_eq!(params.resource_uri, "/repositories/2/resources/18");
        assert_eq!(
            params.parent_node.as_deref(),
            Some("/repositories/2/archival_objects/100")
        );
        assert_eq!(params.offset, 2);
        assert_eq!(params.base_url, DEFAULT_SOLR_URL);
        assert_eq!(params.collection, DEFAULT_SHADOW_COLLECTION);
    }

    #[test]
    fn test_parse_shadow_waypoint_params_rejects_missing_and_invalid_params() {
        let err =
            parse_shadow_waypoint_params("offset=0", DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
                .unwrap_err();
        assert_eq!(
            err.to_string(),
            "missing required query parameter 'resource_uri'"
        );

        let err = parse_shadow_waypoint_params(
            "resource_uri=/repositories/2/resources/18",
            DEFAULT_SOLR_URL,
            DEFAULT_SHADOW_COLLECTION,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "missing required query parameter 'offset'");

        let err = parse_shadow_waypoint_params(
            "resource_uri=/repositories/2/resources/18&offset=abc",
            DEFAULT_SOLR_URL,
            DEFAULT_SHADOW_COLLECTION,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid offset parameter: 'abc'"));
    }

    #[test]
    fn test_parse_shadow_waypoint_params_refuses_live_collection() {
        let err = parse_shadow_waypoint_params(
            "resource_uri=/repositories/2/resources/18&offset=0&collection=archivesspace",
            DEFAULT_SOLR_URL,
            DEFAULT_SHADOW_COLLECTION,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "refusing to query live Solr collection 'archivesspace'"
        );
    }

    #[test]
    fn test_parse_shadow_query_params_accepts_include_ancestors_with_tree() {
        for val in ["true", "1"] {
            let qs = format!(
                "resource_uri=%2Frepositories%2F2%2Fresources%2F18&tree=true&include_ancestors={val}"
            );
            let params =
                parse_shadow_query_params(&qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
                    .unwrap();
            assert!(params.tree);
            assert!(params.include_ancestors);
        }
    }

    #[test]
    fn test_parse_shadow_query_params_rejects_include_ancestors_without_tree() {
        for val in ["true", "1"] {
            let qs = format!(
                "resource_uri=%2Frepositories%2F2%2Fresources%2F18&include_ancestors={val}"
            );
            let err = parse_shadow_query_params(&qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
                .unwrap_err();
            assert_eq!(err.to_string(), "include_ancestors requires tree=true");
        }
    }

    #[test]
    fn test_parse_shadow_search_params_defaults() {
        let qs = "";
        let params =
            parse_shadow_search_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap();
        assert_eq!(params.q, None);
        assert_eq!(params.resource_uri, None);
        assert_eq!(params.date_filter, None);
        assert_eq!(params.title_filter, None);
        assert_eq!(params.rows, 50);
        assert_eq!(params.start, 0);
        assert_eq!(params.collection, "archivesspace_shadow");
        assert_eq!(params.solr_url, "http://localhost:8985/solr");
        assert!(!params.tree);
        assert!(!params.include_ancestors);
    }

    #[test]
    fn test_parse_shadow_search_params_custom() {
        let qs = "q=Example&resource_uri=%2Frepositories%2F2%2Fresources%2F18&date_filter=*1974*&title_filter=*letter*&rows=25&start=10&solr_url=http%3A%2F%2Fexample.com%3A8983%2Fsolr&collection=custom_shadow";
        let params =
            parse_shadow_search_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION).unwrap();
        assert_eq!(params.q.as_deref(), Some("Example"));
        assert_eq!(
            params.resource_uri.as_deref(),
            Some("/repositories/2/resources/18")
        );
        assert_eq!(params.date_filter.as_deref(), Some("*1974*"));
        assert_eq!(params.title_filter.as_deref(), Some("*letter*"));
        assert_eq!(params.rows, 25);
        assert_eq!(params.start, 10);
        assert_eq!(params.collection, "custom_shadow");
        assert_eq!(params.solr_url, "http://example.com:8983/solr");
        assert!(!params.tree);
        assert!(!params.include_ancestors);
    }

    #[test]
    fn test_parse_shadow_search_params_accepts_include_ancestors_with_tree_and_resource_uri() {
        for val in ["true", "1"] {
            let qs = format!(
                "q=Example&resource_uri=%2Frepositories%2F2%2Fresources%2F18&tree=true&include_ancestors={val}"
            );
            let params =
                parse_shadow_search_params(&qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
                    .unwrap();
            assert_eq!(params.q.as_deref(), Some("Example"));
            assert_eq!(
                params.resource_uri.as_deref(),
                Some("/repositories/2/resources/18")
            );
            assert!(params.tree);
            assert!(params.include_ancestors);
        }
    }

    #[test]
    fn test_parse_shadow_search_params_rejects_include_ancestors_without_tree() {
        for val in ["true", "1"] {
            let qs = format!("q=Example&include_ancestors={val}");
            let err = parse_shadow_search_params(&qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
                .unwrap_err();
            assert_eq!(err.to_string(), "include_ancestors requires tree=true");
        }
    }

    #[test]
    fn test_parse_shadow_search_params_rejects_tree_without_resource_uri() {
        let qs = "q=Example&tree=true";
        let err = parse_shadow_search_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
            .unwrap_err();
        assert_eq!(err.to_string(), "tree=true requires resource_uri");
    }

    #[test]
    fn test_parse_shadow_search_params_rejects_tree_with_zero_rows() {
        let qs = "resource_uri=%2Frepositories%2F2%2Fresources%2F18&rows=0&tree=true";
        let err = parse_shadow_search_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot assemble tree with rows=0 (documents must be fetched)"
        );
    }

    #[test]
    fn test_parse_shadow_search_params_refuses_live_collection() {
        let qs = "q=Example&collection=archivesspace";
        let err = parse_shadow_search_params(qs, DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "refusing to query live Solr collection 'archivesspace'"
        );
    }

    #[test]
    fn test_shadow_search_response_non_tree_serialization_contract() {
        let response = ShadowSearchResponse {
            q: Some("Example".to_string()),
            resource_uri: Some("/repositories/2/resources/18".to_string()),
            date_filter: None,
            title_filter: None,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            rows: 5,
            start: 0,
            num_found: 37,
            docs: vec![aspace_client::ShadowArchivalObjectHit {
                id: "/repositories/2/archival_objects/2050".to_string(),
                title: Some("Example correspondence, 1976".to_string()),
                display_string: Some("Example correspondence, 1976".to_string()),
                resource_uri: Some("/repositories/2/resources/18".to_string()),
                parent_uri: Some("/repositories/2/archival_objects/1712".to_string()),
                ancestor_uris: vec![
                    "/repositories/2/archival_objects/1712".to_string(),
                    "/repositories/2/resources/18".to_string(),
                ],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec!["1976 mayo".to_string()],
                position_i: None,
            }],
            query_string: "(title:Example OR display_string:Example OR date_expressions:Example)"
                .to_string(),
            total_nodes: None,
            tree: None,
            orphans: None,
            elapsed_ms: Some(1.23),
            served_by: Some("aspace-shadow-sidecar".to_string()),
        };

        let json_value =
            serde_json::to_value(&response).expect("serialization to Value must succeed");
        let obj = json_value
            .as_object()
            .expect("response must be JSON object");

        // Contract assertions: includes `q` and `query_string`
        assert_eq!(obj.get("q").and_then(|v| v.as_str()), Some("Example"));
        assert_eq!(
            obj.get("query_string").and_then(|v| v.as_str()),
            Some("(title:Example OR display_string:Example OR date_expressions:Example)")
        );
        assert_eq!(
            obj.get("resource_uri").and_then(|v| v.as_str()),
            Some("/repositories/2/resources/18")
        );
        assert_eq!(obj.get("num_found").and_then(|v| v.as_u64()), Some(37));
        assert_eq!(obj.get("rows").and_then(|v| v.as_u64()), Some(5));

        // Contract assertions: omits `tree`, `orphans`, `total_nodes` when None
        assert!(!obj.contains_key("tree"), "tree must be omitted when None");
        assert!(
            !obj.contains_key("orphans"),
            "orphans must be omitted when None"
        );
        assert!(
            !obj.contains_key("total_nodes"),
            "total_nodes must be omitted when None"
        );
        assert!(
            !obj.contains_key("title_filter"),
            "title_filter must be omitted when None"
        );
    }

    #[test]
    fn test_shadow_search_response_tree_serialization_contract() {
        let tree_node = aspace_client::ShadowTreeNode {
            id: "/repositories/2/archival_objects/1712".to_string(),
            title: "Correspondencia, 1919-1991".to_string(),
            resource_uri: "/repositories/2/resources/18".to_string(),
            parent_uri: None,
            date_expressions: vec!["1919-1991".to_string()],
            matched: false,
            children: vec![aspace_client::ShadowTreeNode {
                id: "/repositories/2/archival_objects/2050".to_string(),
                title: "Example correspondence, 1976".to_string(),
                resource_uri: "/repositories/2/resources/18".to_string(),
                parent_uri: Some("/repositories/2/archival_objects/1712".to_string()),
                date_expressions: vec!["1976 mayo".to_string()],
                matched: true,
                children: vec![],
            }],
        };

        let orphan_node = aspace_client::ShadowTreeNode {
            id: "/repositories/2/archival_objects/9999".to_string(),
            title: "Orphan hit".to_string(),
            resource_uri: "/repositories/2/resources/18".to_string(),
            parent_uri: Some("/repositories/2/archival_objects/8888".to_string()),
            date_expressions: vec![],
            matched: true,
            children: vec![],
        };

        let response = ShadowSearchResponse {
            q: Some("Example".to_string()),
            resource_uri: Some("/repositories/2/resources/18".to_string()),
            date_filter: None,
            title_filter: None,
            solr_url: "http://localhost:8985/solr".to_string(),
            collection: "archivesspace_shadow".to_string(),
            rows: 5,
            start: 0,
            num_found: 37,
            docs: vec![],
            query_string: "(title:Example OR display_string:Example OR date_expressions:Example)"
                .to_string(),
            total_nodes: Some(3),
            tree: Some(vec![tree_node]),
            orphans: Some(vec![orphan_node]),
            elapsed_ms: Some(2.45),
            served_by: Some("aspace-shadow-sidecar".to_string()),
        };

        let json_value =
            serde_json::to_value(&response).expect("serialization to Value must succeed");
        let obj = json_value
            .as_object()
            .expect("response must be JSON object");

        // Contract assertions: includes `total_nodes`, `tree`, `orphans`
        assert_eq!(obj.get("total_nodes").and_then(|v| v.as_u64()), Some(3));
        assert!(obj.get("tree").and_then(|v| v.as_array()).is_some());
        assert!(obj.get("orphans").and_then(|v| v.as_array()).is_some());

        // Contract assertions: includes `matched` on tree nodes (both context and matched)
        let tree_roots = obj["tree"].as_array().unwrap();
        assert_eq!(tree_roots.len(), 1);
        let root = &tree_roots[0];
        assert_eq!(root.get("matched").and_then(|v| v.as_bool()), Some(false));

        let root_children = root["children"].as_array().unwrap();
        assert_eq!(root_children.len(), 1);
        let child = &root_children[0];
        assert_eq!(child.get("matched").and_then(|v| v.as_bool()), Some(true));

        let orphans = obj["orphans"].as_array().unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(
            orphans[0].get("matched").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_route_request() {
        assert_eq!(route_request("GET", "/health"), Ok(SidecarRoute::Health));
        assert_eq!(
            route_request("GET", "/health?check=1"),
            Ok(SidecarRoute::Health)
        );
        assert_eq!(route_request("GET", "/ready"), Ok(SidecarRoute::Ready));
        assert_eq!(route_request("GET", "/metrics"), Ok(SidecarRoute::Metrics));
        assert_eq!(
            route_request("GET", "/shadow/descendants?rows=5"),
            Ok(SidecarRoute::Descendants("rows=5"))
        );
        assert_eq!(
            route_request("GET", "/shadow/search?q=Example"),
            Ok(SidecarRoute::Search("q=Example"))
        );
        assert_eq!(
            route_request(
                "GET",
                "/shadow/resource-tree/waypoint?resource_uri=/res/1&offset=0"
            ),
            Ok(SidecarRoute::ResourceTreeWaypoint(
                "resource_uri=/res/1&offset=0"
            ))
        );
        assert_eq!(
            route_request(
                "GET",
                "/shadow/resource-tree/node-from-root?resource_uri=/res/1&root_title=Root&node_ids=1"
            ),
            Ok(SidecarRoute::ResourceTreeNodeFromRoot(
                "resource_uri=/res/1&root_title=Root&node_ids=1"
            ))
        );
        assert_eq!(
            route_request("GET", "/not-a-route"),
            Ok(SidecarRoute::NotFound("/not-a-route"))
        );
        assert_eq!(
            route_request("POST", "/health"),
            Err("Method 'POST' not allowed; only GET is supported".to_string())
        );
    }

    #[test]
    fn test_build_health_response() {
        let resp = build_health_response("http://localhost:8985/solr", "archivesspace_shadow");
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.served_by, "aspace-shadow-sidecar");
        assert_eq!(resp.solr_url, "http://localhost:8985/solr");
        assert_eq!(resp.collection, "archivesspace_shadow");
    }

    #[test]
    fn test_metrics_render_prometheus_contract() {
        let metrics = SidecarMetrics::new();
        metrics.requests_total.store(7, Ordering::Relaxed);
        metrics.requests_active.store(2, Ordering::Relaxed);

        let rendered = metrics.render();
        assert!(rendered.contains("archivalist_sidecar_build_info{version=\"0.1.0\"} 1"));
        assert!(rendered.contains("archivalist_sidecar_requests_total 7"));
        assert!(rendered.contains("archivalist_sidecar_requests_active 2"));
        assert!(rendered.contains("archivalist_sidecar_uptime_seconds "));
    }

    #[tokio::test]
    async fn test_check_readiness_queries_configured_shadow_collection() {
        let mut server = mockito::Server::new_async().await;
        let readiness_mock = server
            .mock("GET", "/solr/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("q".into(), "*:*".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "0".into()),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"response":{"numFound":5255,"start":0,"docs":[]}}"#)
            .create_async()
            .await;

        let response = check_readiness(
            &reqwest::Client::new(),
            &format!("{}/solr", server.url()),
            "archivesspace_shadow",
        )
        .await
        .unwrap();

        readiness_mock.assert_async().await;
        assert_eq!(response.status, "ready");
        assert_eq!(response.served_by, "aspace-shadow-sidecar");
        assert_eq!(response.collection, "archivesspace_shadow");
        assert_eq!(response.num_found, 5255);
    }

    #[tokio::test]
    async fn test_check_readiness_rejects_solr_error() {
        let mut server = mockito::Server::new_async().await;
        let readiness_mock = server
            .mock("GET", "/solr/archivesspace_shadow/select")
            .match_query(mockito::Matcher::Any)
            .with_status(503)
            .create_async()
            .await;

        let error = check_readiness(
            &reqwest::Client::new(),
            &format!("{}/solr", server.url()),
            "archivesspace_shadow",
        )
        .await
        .unwrap_err();

        readiness_mock.assert_async().await;
        assert!(error
            .to_string()
            .contains("shadow Solr readiness query returned HTTP 503"));
    }

    #[test]
    fn test_parse_shadow_node_from_root_params() {
        let params = parse_shadow_node_from_root_params(
            "resource_uri=%2Frepositories%2F2%2Fresources%2F16&root_title=Root+Title&node_ids=1%2C2&published_only=true",
            DEFAULT_SOLR_URL,
            DEFAULT_SHADOW_COLLECTION,
        )
        .unwrap();

        assert_eq!(params.resource_uri, "/repositories/2/resources/16");
        assert_eq!(params.root_title, "Root Title");
        assert_eq!(params.node_ids, vec![1, 2]);
        assert!(params.published_only);
    }

    #[test]
    fn test_parse_shadow_node_from_root_params_rejects_invalid_batches() {
        for query in [
            "resource_uri=%2Fres%2F1&node_ids=1",
            "resource_uri=%2Fres%2F1&root_title=Root&node_ids=1%2C1",
            "resource_uri=%2Fres%2F1&root_title=Root&node_ids=0",
            "resource_uri=%2Fres%2F1&root_title=Root&node_ids=1&collection=archivesspace",
        ] {
            assert!(parse_shadow_node_from_root_params(
                query,
                DEFAULT_SOLR_URL,
                DEFAULT_SHADOW_COLLECTION,
            )
            .is_err());
        }

        let ids = (1..=129)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let query = format!("resource_uri=%2Fres%2F1&root_title=Root&node_ids={ids}");
        assert!(parse_shadow_node_from_root_params(
            &query,
            DEFAULT_SOLR_URL,
            DEFAULT_SHADOW_COLLECTION,
        )
        .is_err());
    }

    #[test]
    fn test_health_response_serialization() {
        let resp = build_health_response("http://localhost:8985/solr", "archivesspace_shadow");
        let json_value = serde_json::to_value(&resp).expect("serialization to Value must succeed");
        let obj = json_value
            .as_object()
            .expect("response must be JSON object");
        assert_eq!(obj.get("status").and_then(|v| v.as_str()), Some("ok"));
        assert_eq!(
            obj.get("served_by").and_then(|v| v.as_str()),
            Some("aspace-shadow-sidecar")
        );
        assert_eq!(
            obj.get("solr_url").and_then(|v| v.as_str()),
            Some("http://localhost:8985/solr")
        );
        assert_eq!(
            obj.get("collection").and_then(|v| v.as_str()),
            Some("archivesspace_shadow")
        );
    }
}
