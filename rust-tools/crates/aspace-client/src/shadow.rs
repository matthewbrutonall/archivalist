use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error::{ASpaceError, Result};

/// Default Solr core/collection name for shadow index queries.
pub const DEFAULT_SHADOW_COLLECTION: &str = "archivesspace_shadow";

/// Default base URL for Solr queries.
pub const DEFAULT_SOLR_URL: &str = "http://localhost:8985/solr";

/// Parameters for querying the shadow Solr index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowSearchParams {
    pub base_url: String,
    pub collection: String,
    pub resource_uri: String,
    pub date_filter: Option<String>,
    pub title_filter: Option<String>,
    pub rows: usize,
    pub start: usize,
}

impl ShadowSearchParams {
    pub fn new(resource_uri: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_SOLR_URL.to_string(),
            collection: DEFAULT_SHADOW_COLLECTION.to_string(),
            resource_uri: resource_uri.into(),
            date_filter: None,
            title_filter: None,
            rows: 50,
            start: 0,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }

    pub fn with_date_filter(mut self, date_filter: impl Into<String>) -> Self {
        self.date_filter = Some(date_filter.into());
        self
    }

    pub fn with_title_filter(mut self, title_filter: impl Into<String>) -> Self {
        self.title_filter = Some(title_filter.into());
        self
    }

    pub fn with_rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn with_start(mut self, start: usize) -> Self {
        self.start = start;
        self
    }
}

/// Parameters for keyword searches across shadow Solr documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowKeywordSearchParams {
    pub base_url: String,
    pub collection: String,
    pub query: Option<String>,
    pub resource_uri: Option<String>,
    pub date_filter: Option<String>,
    pub title_filter: Option<String>,
    pub rows: usize,
    pub start: usize,
}

impl Default for ShadowKeywordSearchParams {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_SOLR_URL.to_string(),
            collection: DEFAULT_SHADOW_COLLECTION.to_string(),
            query: None,
            resource_uri: None,
            date_filter: None,
            title_filter: None,
            rows: 50,
            start: 0,
        }
    }
}

impl ShadowKeywordSearchParams {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }

    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.query = Some(query.into());
        self
    }

    pub fn with_resource_uri(mut self, resource_uri: impl Into<String>) -> Self {
        self.resource_uri = Some(resource_uri.into());
        self
    }

    pub fn with_date_filter(mut self, date_filter: impl Into<String>) -> Self {
        self.date_filter = Some(date_filter.into());
        self
    }

    pub fn with_title_filter(mut self, title_filter: impl Into<String>) -> Self {
        self.title_filter = Some(title_filter.into());
        self
    }

    pub fn with_rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn with_start(mut self, start: usize) -> Self {
        self.start = start;
        self
    }
}

/// Constant waypoint batch size for shadow resource tree navigation.
pub const WAYPOINT_SIZE: usize = 200;

/// Parameters for querying the shadow resource tree waypoint endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowTreeWaypointParams {
    pub base_url: String,
    pub collection: String,
    pub resource_uri: String,
    pub parent_node: Option<String>,
    pub offset: usize,
}

/// Parameters for retrieving paths from a resource root to archival objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowTreeNodeFromRootParams {
    pub base_url: String,
    pub collection: String,
    pub resource_uri: String,
    pub root_title: String,
    pub node_ids: Vec<u64>,
    pub published_only: bool,
}

impl ShadowTreeNodeFromRootParams {
    pub fn new(
        resource_uri: impl Into<String>,
        root_title: impl Into<String>,
        node_ids: Vec<u64>,
    ) -> Self {
        Self {
            base_url: DEFAULT_SOLR_URL.to_string(),
            collection: DEFAULT_SHADOW_COLLECTION.to_string(),
            resource_uri: resource_uri.into(),
            root_title: root_title.into(),
            node_ids,
            published_only: false,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }

    pub fn with_published_only(mut self, published_only: bool) -> Self {
        self.published_only = published_only;
        self
    }
}

impl ShadowTreeWaypointParams {
    pub fn new(resource_uri: impl Into<String>, offset: usize) -> Self {
        Self {
            base_url: DEFAULT_SOLR_URL.to_string(),
            collection: DEFAULT_SHADOW_COLLECTION.to_string(),
            resource_uri: resource_uri.into(),
            parent_node: None,
            offset,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }

    pub fn with_parent_node(mut self, parent_node: impl Into<String>) -> Self {
        let p = parent_node.into();
        self.parent_node = if p.trim().is_empty() {
            None
        } else {
            Some(p.trim().to_string())
        };
        self
    }
}

/// Calculate waypoints as ceil(child_count / 200).
pub fn calculate_waypoints(child_count: usize) -> usize {
    if child_count == 0 {
        0
    } else {
        child_count.div_ceil(WAYPOINT_SIZE)
    }
}

/// Builds the Solr query string for a waypoint query given resource_uri and optional parent_node.
pub fn build_waypoint_query(resource_uri: &str, parent_node: Option<&str>) -> String {
    let resource_clause = format!("resource_uri:{}", escape_solr_literal(resource_uri.trim()));
    match parent_node {
        Some(parent) if !parent.trim().is_empty() => format!(
            "{} AND parent_uri:{}",
            resource_clause,
            escape_solr_literal(parent.trim())
        ),
        _ => format!("{} AND -parent_uri:[* TO *]", resource_clause),
    }
}

fn mixed_content_render_value(tag: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let start = lower.find("render")?;
    let after_name = &tag[start + "render".len()..];
    let equals = after_name.find('=')?;
    let value = after_name[equals + 1..].trim_start();
    let quote = value.chars().next()?;
    if quote == '\'' || quote == '"' {
        let rest = &value[quote.len_utf8()..];
        let end = rest.find(quote)?;
        Some(rest[..end].to_string())
    } else {
        Some(
            value
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
        )
    }
}

fn escape_mixed_content_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'&' => {
                let tail = &text[index + 1..];
                let entity_end = tail.find(';').filter(|end| *end <= 12);
                let is_entity = entity_end.is_some_and(|end| {
                    let body = &tail[..end];
                    !body.is_empty()
                        && (body.starts_with('#')
                            || body
                                .chars()
                                .all(|character| character.is_ascii_alphanumeric()))
                });
                if is_entity {
                    let end = entity_end.unwrap();
                    escaped.push_str(&text[index..index + end + 2]);
                    index += end + 2;
                } else {
                    escaped.push_str("&amp;");
                    index += 1;
                }
            }
            b'>' => {
                escaped.push_str("&gt;");
                index += 1;
            }
            _ => {
                let character = text[index..].chars().next().unwrap();
                escaped.push(character);
                index += character.len_utf8();
            }
        }
    }
    escaped
}

/// Render the EAD mixed-content subset accepted in ArchivesSpace titles into the
/// HTML shape used by `MixedContentParser` for LargeTree responses.
pub fn parse_mixed_content_title(content: &str) -> String {
    let content = content.trim();
    let mut output = String::with_capacity(content.len());
    let mut open_tags: Vec<&'static str> = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = content[cursor..].find('<') {
        let tag_start = cursor + relative_start;
        output.push_str(&escape_mixed_content_text(&content[cursor..tag_start]));
        let Some(relative_end) = content[tag_start..].find('>') else {
            output.push_str(&escape_mixed_content_text(&content[tag_start..]));
            return output;
        };
        let tag_end = tag_start + relative_end;
        let raw_tag = content[tag_start + 1..tag_end].trim();
        let closing = raw_tag.starts_with('/');
        let tag_body = raw_tag.trim_start_matches('/').trim();
        let tag_name = tag_body
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        match tag_name.as_str() {
            "emph" | "title" | "unitdate" if closing => {
                output.push_str(open_tags.pop().unwrap_or("</span>"));
            }
            "emph" | "title" | "unitdate" => {
                let render = mixed_content_render_value(tag_body);
                if render.as_deref() == Some("nonproport") {
                    output.push_str("<code class=\"emph render-nonproport\">");
                    open_tags.push("</code>");
                } else {
                    let render = render
                        .as_deref()
                        .filter(|value| !value.is_empty())
                        .unwrap_or("none");
                    output.push_str(&format!("<span class=\"emph render-{render}\">"));
                    open_tags.push("</span>");
                }
            }
            "lb" if !closing => output.push_str("<br>"),
            _ => {}
        }

        cursor = tag_end + 1;
    }

    output.push_str(&escape_mixed_content_text(&content[cursor..]));
    while let Some(closing_tag) = open_tags.pop() {
        output.push_str(closing_tag);
    }
    output
}

fn tree_container_summaries(source: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
    let Some(instances) = source
        .and_then(|value| value.get("instances"))
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };

    instances
        .iter()
        .filter_map(|instance| {
            let mut summary = serde_json::Map::new();
            if let Some(value) = instance
                .get("instance_type")
                .filter(|value| !value.is_null())
            {
                summary.insert("instance_type".to_string(), value.clone());
            }

            if let Some(sub_container) = instance.get("sub_container") {
                if let Some(top_container) = sub_container
                    .get("top_container")
                    .and_then(|value| value.get("_resolved"))
                {
                    for (source_key, output_key) in [
                        ("type", "top_container_type"),
                        ("indicator", "top_container_indicator"),
                        ("barcode", "top_container_barcode"),
                    ] {
                        if let Some(value) = top_container
                            .get(source_key)
                            .filter(|value| !value.is_null())
                        {
                            summary.insert(output_key.to_string(), value.clone());
                        }
                    }
                }
                for key in [
                    "type_2",
                    "indicator_2",
                    "barcode_2",
                    "type_3",
                    "indicator_3",
                ] {
                    if let Some(value) = sub_container.get(key).filter(|value| !value.is_null()) {
                        summary.insert(key.to_string(), value.clone());
                    }
                }
            }

            if summary.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(summary))
            }
        })
        .collect()
}

/// A node returned from the shadow resource tree waypoint endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowWaypointNode {
    pub child_count: usize,
    pub waypoints: usize,
    pub waypoint_size: usize,
    pub title: String,
    pub slugged_url: String,
    pub parsed_title: String,
    pub uri: String,
    pub position: i64,
    pub parent_id: Option<u64>,
    pub suppressed: u8,
    pub jsonmodel_type: String,
    pub has_digital_instance: bool,
    pub identifier: Option<String>,
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dates: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<serde_json::Value>,
}

/// One expansion step in an ArchivesSpace `tree/node_from_root` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowNodeFromRootEntry {
    pub node: Option<String>,
    pub root_record_uri: String,
    pub offset: usize,
    pub jsonmodel_type: String,
    pub title: String,
    pub parsed_title: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ShadowPathDoc {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    display_string: Option<String>,
    #[serde(default)]
    parent_uri: Option<String>,
    #[serde(default)]
    ancestor_uris: Vec<String>,
    #[serde(default)]
    position_i: Option<i64>,
    #[serde(default)]
    publish: bool,
    #[serde(default)]
    json: Option<String>,
}

impl ShadowPathDoc {
    fn suppressed(&self) -> bool {
        self.json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.get("suppressed").and_then(|flag| flag.as_bool()))
            .unwrap_or(false)
    }

    fn visible(&self, published_only: bool) -> bool {
        !published_only || (self.publish && !self.suppressed())
    }

    fn display_title(&self) -> String {
        self.display_string
            .clone()
            .or_else(|| self.title.clone())
            .unwrap_or_else(|| "<untitled>".to_string())
    }
}

/// Builds the Solr query string for keyword searches across shadow Solr documents.
///
/// Rules:
/// - Blank or None query => `*:*`
/// - Present query => safely searches `title`, `display_string`, and `date_expressions`
/// - Optional `resource_uri` (descendant or exact resource scoping via `ancestor_uris`),
///   `date_filter`, and `title_filter` append as AND clauses when supplied
/// - Uses existing escaping helpers to prevent dangerous Lucene query injections.
pub fn build_keyword_query(params: &ShadowKeywordSearchParams) -> String {
    let mut clauses: Vec<String> = Vec::new();

    let query_clause = match &params.query {
        Some(q) if !q.trim().is_empty() => {
            let escaped_q = escape_solr_wildcard_filter(q.trim());
            format!(
                "(title:{} OR display_string:{} OR date_expressions:{})",
                escaped_q, escaped_q, escaped_q
            )
        }
        _ => "*:*".to_string(),
    };
    clauses.push(query_clause);

    if let Some(resource_uri) = &params.resource_uri {
        let res_trimmed = resource_uri.trim();
        if !res_trimmed.is_empty() {
            let escaped_res = escape_solr_literal(res_trimmed);
            clauses.push(format!("ancestor_uris:{}", escaped_res));
        }
    }

    if let Some(date_filter) = &params.date_filter {
        let date_trimmed = date_filter.trim();
        if !date_trimmed.is_empty() {
            let escaped_date = escape_solr_wildcard_filter(date_trimmed);
            clauses.push(format!("date_expressions:{}", escaped_date));
        }
    }

    if let Some(title_filter) = &params.title_filter {
        let title_trimmed = title_filter.trim();
        if !title_trimmed.is_empty() {
            let escaped_title = escape_solr_wildcard_filter(title_trimmed);
            clauses.push(format!("title:{}", escaped_title));
        }
    }

    clauses.join(" AND ")
}

/// A hit returned from querying the shadow Solr hierarchy index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ShadowArchivalObjectHit {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub display_string: Option<String>,
    #[serde(default)]
    pub resource_uri: Option<String>,
    #[serde(default)]
    pub parent_uri: Option<String>,
    #[serde(default)]
    pub ancestor_uris: Vec<String>,
    #[serde(default)]
    pub ancestor_levels: Vec<String>,
    #[serde(default)]
    pub date_expressions: Vec<String>,
    #[serde(default)]
    pub position_i: Option<i64>,
}

/// Typed result envelope for a shadow Solr query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowSearchResult {
    pub num_found: u64,
    pub start: usize,
    pub docs: Vec<ShadowArchivalObjectHit>,
    pub query_string: String,
}

/// A node in a nested hierarchical tree assembled from shadow Solr hits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowTreeNode {
    pub id: String,
    pub title: String,
    pub resource_uri: String,
    pub parent_uri: Option<String>,
    pub date_expressions: Vec<String>,
    #[serde(default = "default_matched")]
    pub matched: bool,
    pub children: Vec<ShadowTreeNode>,
}

fn default_matched() -> bool {
    true
}

impl ShadowTreeNode {
    pub fn from_hit(hit: ShadowArchivalObjectHit, default_resource_uri: &str) -> Self {
        Self::from_hit_with_matched(hit, default_resource_uri, true)
    }

    pub fn from_hit_with_matched(
        hit: ShadowArchivalObjectHit,
        default_resource_uri: &str,
        matched: bool,
    ) -> Self {
        let title = hit
            .title
            .or(hit.display_string)
            .unwrap_or_else(|| "<untitled>".to_string());
        let resource_uri = hit
            .resource_uri
            .unwrap_or_else(|| default_resource_uri.to_string());

        Self {
            id: hit.id,
            title,
            resource_uri,
            parent_uri: hit.parent_uri,
            date_expressions: hit.date_expressions,
            matched,
            children: Vec::new(),
        }
    }
}

/// A tree-shaped hierarchy response representing a resource tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowTreeResponse {
    pub root_resource_uri: String,
    pub total_nodes: usize,
    pub tree: Vec<ShadowTreeNode>,
    pub orphans: Vec<ShadowTreeNode>,
}

/// Assembles a list of `ShadowArchivalObjectHit`s into a nested tree.
///
/// All provided hits are marked with `matched = true`.
///
/// Children whose `parent_uri` matches `root_resource_uri` (or whose parent is absent
/// while `ancestor_uris` contains `root_resource_uri` at top level) are placed in `tree`.
/// Nodes whose `parent_uri` points to an archival object present in the hit set are nested
/// under that parent's `children`. Nodes whose `parent_uri` is missing from the hit set (and
/// not pointing to `root_resource_uri`) are collected deterministically in `orphans`.
///
/// Deterministic ordering: children and root/orphan lists preserve the stable order
/// established by the input hit sequence.
pub fn assemble_shadow_tree(
    hits: Vec<ShadowArchivalObjectHit>,
    root_resource_uri: &str,
) -> ShadowTreeResponse {
    assemble_shadow_tree_with_context(hits, Vec::new(), root_resource_uri)
}

/// Assembles a list of search matches (`matches`) along with ancestor/context hits (`context_hits`)
/// into a nested tree.
///
/// In this function:
/// - Hits from `matches` are marked with `matched = true`.
/// - Hits from `context_hits` are marked with `matched = false`.
/// - If a hit appears in both sets, the match status (`matched = true`) takes precedence.
///
/// Tree assembly nests hits under their parent archival objects. If context contains the missing
/// parent of matched hits, matched hits are nested under that parent instead of classifying them as
/// orphans. If context is still missing a parent, orphan behavior remains deterministic.
pub fn assemble_shadow_tree_with_context(
    matches: Vec<ShadowArchivalObjectHit>,
    context_hits: Vec<ShadowArchivalObjectHit>,
    root_resource_uri: &str,
) -> ShadowTreeResponse {
    if matches.is_empty() && context_hits.is_empty() {
        return ShadowTreeResponse {
            root_resource_uri: root_resource_uri.to_string(),
            total_nodes: 0,
            tree: Vec::new(),
            orphans: Vec::new(),
        };
    }

    let mut nodes: Vec<ShadowTreeNode> = Vec::with_capacity(matches.len() + context_hits.len());
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for hit in matches {
        seen_ids.insert(hit.id.clone());
        nodes.push(ShadowTreeNode::from_hit_with_matched(
            hit,
            root_resource_uri,
            true,
        ));
    }

    for hit in context_hits {
        if seen_ids.insert(hit.id.clone()) {
            nodes.push(ShadowTreeNode::from_hit_with_matched(
                hit,
                root_resource_uri,
                false,
            ));
        }
    }

    let node_ids: std::collections::HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

    // Map parent_id -> list of child nodes
    let mut children_by_parent: std::collections::HashMap<String, Vec<ShadowTreeNode>> =
        std::collections::HashMap::new();
    let mut root_nodes: Vec<ShadowTreeNode> = Vec::new();
    let mut orphan_nodes: Vec<ShadowTreeNode> = Vec::new();

    for node in nodes {
        match &node.parent_uri {
            Some(parent) if parent == root_resource_uri => {
                root_nodes.push(node);
            }
            Some(parent) if node_ids.contains(parent) => {
                children_by_parent
                    .entry(parent.clone())
                    .or_default()
                    .push(node);
            }
            Some(_) => {
                orphan_nodes.push(node);
            }
            None => {
                // If parent_uri is None, treat as direct child of root if matching root
                root_nodes.push(node);
            }
        }
    }

    fn attach_children(
        node: &mut ShadowTreeNode,
        children_map: &mut std::collections::HashMap<String, Vec<ShadowTreeNode>>,
    ) {
        if let Some(mut kids) = children_map.remove(&node.id) {
            for kid in &mut kids {
                attach_children(kid, children_map);
            }
            node.children = kids;
        }
    }

    for root in &mut root_nodes {
        attach_children(root, &mut children_by_parent);
    }

    for orphan in &mut orphan_nodes {
        attach_children(orphan, &mut children_by_parent);
    }

    let root_count = count_tree_nodes(&root_nodes);
    let orphan_count = count_tree_nodes(&orphan_nodes);

    ShadowTreeResponse {
        root_resource_uri: root_resource_uri.to_string(),
        total_nodes: root_count + orphan_count,
        tree: root_nodes,
        orphans: orphan_nodes,
    }
}

fn count_tree_nodes(nodes: &[ShadowTreeNode]) -> usize {
    let mut count = 0;
    for node in nodes {
        count += 1 + count_tree_nodes(&node.children);
    }
    count
}

/// Escapes Solr special characters in a phrase/value that is intended to be matched literally.
///
/// Lucene/Solr special characters: + - && || ! ( ) { } [ ] ^ " ~ * ? : \ /
pub fn escape_solr_literal(val: &str) -> String {
    let mut escaped = String::with_capacity(val.len() * 2);
    for c in val.chars() {
        match c {
            '\\' | '+' | '-' | '!' | '(' | ')' | ':' | '^' | '[' | ']' | '\"' | '{' | '}' | '~'
            | '*' | '?' | '|' | '&' | '/' => {
                escaped.push('\\');
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Escapes Solr special characters while preserving `*` and `?` wildcards.
///
/// Also escapes whitespace and Lucene boolean operators (`AND`, `OR`, `NOT`)
/// so arbitrary user input cannot inject raw boolean operator clauses.
pub fn escape_solr_wildcard_filter(val: &str) -> String {
    let mut escaped = String::with_capacity(val.len() * 2);
    for c in val.chars() {
        match c {
            '\\' | '+' | '-' | '!' | '(' | ')' | ':' | '^' | '[' | ']' | '\"' | '{' | '}' | '~'
            | '|' | '&' | '/' | ' ' | '\t' | '\r' | '\n' => {
                escaped.push('\\');
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Builds the Solr query string for hierarchy descendants, with optional date and title filters.
pub fn build_descendants_query(resource_uri: &str, date_filter: Option<&str>) -> String {
    build_descendants_query_with_filters(resource_uri, date_filter, None)
}

/// Builds the Solr query string for hierarchy descendants, with optional date and title filters.
pub fn build_descendants_query_with_filters(
    resource_uri: &str,
    date_filter: Option<&str>,
    title_filter: Option<&str>,
) -> String {
    let escaped_resource = escape_solr_literal(resource_uri);
    let mut query = format!("ancestor_uris:{}", escaped_resource);

    if let Some(date) = date_filter {
        let date_trimmed = date.trim();
        if !date_trimmed.is_empty() {
            let escaped_date = escape_solr_wildcard_filter(date_trimmed);
            query.push_str(&format!(" AND date_expressions:{}", escaped_date));
        }
    }

    if let Some(title) = title_filter {
        let title_trimmed = title.trim();
        if !title_trimmed.is_empty() {
            let escaped_title = escape_solr_wildcard_filter(title_trimmed);
            query.push_str(&format!(" AND title:{}", escaped_title));
        }
    }

    query
}

/// Client for querying the shadow Solr index.
#[derive(Clone, Debug)]
pub struct ShadowSolrClient {
    base_url: String,
    collection: String,
    http: Arc<HttpClient>,
}

#[derive(Deserialize)]
struct SolrResponseEnvelope {
    response: SolrResponseHeader,
}

#[derive(Deserialize)]
struct SolrResponseHeader {
    #[serde(rename = "numFound")]
    num_found: u64,
    start: usize,
    docs: Vec<ShadowArchivalObjectHit>,
}

impl ShadowSolrClient {
    pub fn new(base_url: impl Into<String>, collection: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            collection: collection.into(),
            http: Arc::new(HttpClient::new()),
        }
    }

    pub fn with_default_url(collection: impl Into<String>) -> Self {
        Self::new(DEFAULT_SOLR_URL, collection)
    }

    pub fn default_shadow() -> Self {
        Self::new(DEFAULT_SOLR_URL, DEFAULT_SHADOW_COLLECTION)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    /// Executes a search against the Solr core using the provided search params.
    pub async fn search(&self, params: &ShadowSearchParams) -> Result<ShadowSearchResult> {
        let select_url = format!(
            "{}/{}/select",
            params.base_url.trim_end_matches('/'),
            params.collection
        );
        let q = build_descendants_query_with_filters(
            &params.resource_uri,
            params.date_filter.as_deref(),
            params.title_filter.as_deref(),
        );
        let fl = "id,title,display_string,resource_uri,parent_uri,ancestor_uris,ancestor_levels,date_expressions,position_i";

        let response = self
            .http
            .get(&select_url)
            .query(&[
                ("q", q.as_str()),
                ("wt", "json"),
                ("fl", fl),
                ("rows", &params.rows.to_string()),
                ("start", &params.start.to_string()),
            ])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown Solr error".to_string());
            return Err(ASpaceError::Solr {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let envelope: SolrResponseEnvelope = response.json().await?;
        Ok(ShadowSearchResult {
            num_found: envelope.response.num_found,
            start: envelope.response.start,
            docs: envelope.response.docs,
            query_string: q,
        })
    }

    /// Executes a keyword search against the shadow Solr core.
    ///
    /// Refuses collection `archivesspace` before making any network request to
    /// ensure queries cannot be directed at the live Solr index.
    pub async fn keyword_search(
        &self,
        params: &ShadowKeywordSearchParams,
    ) -> Result<ShadowSearchResult> {
        if params.collection == "archivesspace" {
            return Err(ASpaceError::Solr {
                status: 403,
                message: "refusing to query live Solr collection 'archivesspace'".to_string(),
            });
        }

        let select_url = format!(
            "{}/{}/select",
            params.base_url.trim_end_matches('/'),
            params.collection
        );
        let q = build_keyword_query(params);
        let fl = "id,title,display_string,resource_uri,parent_uri,ancestor_uris,ancestor_levels,date_expressions,position_i";

        let response = self
            .http
            .get(&select_url)
            .query(&[
                ("q", q.as_str()),
                ("wt", "json"),
                ("fl", fl),
                ("rows", &params.rows.to_string()),
                ("start", &params.start.to_string()),
            ])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown Solr error".to_string());
            return Err(ASpaceError::Solr {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let envelope: SolrResponseEnvelope = response.json().await?;
        Ok(ShadowSearchResult {
            num_found: envelope.response.num_found,
            start: envelope.response.start,
            docs: envelope.response.docs,
            query_string: q,
        })
    }

    /// Retrieve descendant archival objects for a given resource URI with pagination.
    pub async fn descendants(
        &self,
        resource_uri: &str,
        rows: usize,
        start: usize,
    ) -> Result<ShadowSearchResult> {
        let params = ShadowSearchParams {
            base_url: self.base_url.clone(),
            collection: self.collection.clone(),
            resource_uri: resource_uri.to_string(),
            date_filter: None,
            title_filter: None,
            rows,
            start,
        };
        self.search(&params).await
    }

    /// Retrieve descendant archival objects filtered by a date expression with pagination.
    pub async fn descendants_with_date_expression(
        &self,
        resource_uri: &str,
        date_filter: &str,
        rows: usize,
        start: usize,
    ) -> Result<ShadowSearchResult> {
        let params = ShadowSearchParams {
            base_url: self.base_url.clone(),
            collection: self.collection.clone(),
            resource_uri: resource_uri.to_string(),
            date_filter: Some(date_filter.to_string()),
            title_filter: None,
            rows,
            start,
        };
        self.search(&params).await
    }

    /// Retrieve descendant archival objects filtered by date and title expressions with pagination.
    pub async fn descendants_with_date_and_title(
        &self,
        resource_uri: &str,
        date_filter: Option<&str>,
        title_filter: Option<&str>,
        rows: usize,
        start: usize,
    ) -> Result<ShadowSearchResult> {
        let params = ShadowSearchParams {
            base_url: self.base_url.clone(),
            collection: self.collection.clone(),
            resource_uri: resource_uri.to_string(),
            date_filter: date_filter.map(ToString::to_string),
            title_filter: title_filter.map(ToString::to_string),
            rows,
            start,
        };
        self.search(&params).await
    }

    /// Count descendant archival objects for a given resource URI without fetching documents.
    pub async fn count_descendants(&self, resource_uri: &str) -> Result<u64> {
        let params = ShadowSearchParams {
            base_url: self.base_url.clone(),
            collection: self.collection.clone(),
            resource_uri: resource_uri.to_string(),
            date_filter: None,
            title_filter: None,
            rows: 0,
            start: 0,
        };
        let result = self.search(&params).await?;
        Ok(result.num_found)
    }

    /// Count descendant archival objects matching a date expression without fetching documents.
    pub async fn count_descendants_with_date_expression(
        &self,
        resource_uri: &str,
        date_filter: &str,
    ) -> Result<u64> {
        let params = ShadowSearchParams {
            base_url: self.base_url.clone(),
            collection: self.collection.clone(),
            resource_uri: resource_uri.to_string(),
            date_filter: Some(date_filter.to_string()),
            title_filter: None,
            rows: 0,
            start: 0,
        };
        let result = self.search(&params).await?;
        Ok(result.num_found)
    }

    /// Count descendant archival objects matching date and title expressions without fetching documents.
    pub async fn count_descendants_with_date_and_title(
        &self,
        resource_uri: &str,
        date_filter: Option<&str>,
        title_filter: Option<&str>,
    ) -> Result<u64> {
        let params = ShadowSearchParams {
            base_url: self.base_url.clone(),
            collection: self.collection.clone(),
            resource_uri: resource_uri.to_string(),
            date_filter: date_filter.map(ToString::to_string),
            title_filter: title_filter.map(ToString::to_string),
            rows: 0,
            start: 0,
        };
        let result = self.search(&params).await?;
        Ok(result.num_found)
    }

    /// Retrieve documents by their exact Solr IDs from the shadow index.
    ///
    /// Preserves strict safety by using the configured client collection and URL.
    /// If `ids` is empty, immediately returns an empty vector without issuing network requests.
    pub async fn docs_by_ids(&self, ids: &[String]) -> Result<Vec<ShadowArchivalObjectHit>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let select_url = format!("{}/{}/select", self.base_url, self.collection);
        let q_clauses: Vec<String> = ids
            .iter()
            .map(|id| format!("id:{}", escape_solr_literal(id)))
            .collect();
        let q = q_clauses.join(" OR ");
        let fl = "id,title,display_string,resource_uri,parent_uri,ancestor_uris,ancestor_levels,date_expressions,position_i";

        let response = self
            .http
            .get(&select_url)
            .query(&[
                ("q", q.as_str()),
                ("wt", "json"),
                ("fl", fl),
                ("rows", &ids.len().to_string()),
                ("start", "0"),
            ])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown Solr error".to_string());
            return Err(ASpaceError::Solr {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let envelope: SolrResponseEnvelope = response.json().await?;
        Ok(envelope.response.docs)
    }

    /// Retrieve exact ArchivesSpace-compatible paths from a resource root to archival objects.
    pub async fn tree_node_from_root(
        &self,
        params: &ShadowTreeNodeFromRootParams,
    ) -> Result<std::collections::BTreeMap<u64, Vec<ShadowNodeFromRootEntry>>> {
        if params.collection == "archivesspace" {
            return Err(ASpaceError::Solr {
                status: 403,
                message: "refusing to query live Solr collection 'archivesspace'".to_string(),
            });
        }
        if params.node_ids.is_empty() || params.node_ids.len() > 128 {
            return Err(ASpaceError::Solr {
                status: 400,
                message: "node_ids must contain between 1 and 128 IDs".to_string(),
            });
        }

        let select_url = format!(
            "{}/{}/select",
            params.base_url.trim_end_matches('/'),
            params.collection
        );
        let node_uris: Vec<String> = params
            .node_ids
            .iter()
            .map(|id| {
                let prefix = params
                    .resource_uri
                    .split("/resources/")
                    .next()
                    .unwrap_or("");
                format!("{prefix}/archival_objects/{id}")
            })
            .collect();
        let target_q = node_uris
            .iter()
            .map(|uri| format!("id:{}", escape_solr_literal(uri)))
            .collect::<Vec<_>>()
            .join(" OR ");

        #[derive(Deserialize)]
        struct PathEnvelope {
            response: PathResponse,
        }
        #[derive(Deserialize)]
        struct PathResponse {
            #[serde(rename = "numFound")]
            num_found: usize,
            #[serde(default)]
            docs: Vec<ShadowPathDoc>,
        }

        let target_rows = params.node_ids.len().to_string();
        let target_response = self
            .http
            .post(&select_url)
            .form(&[
                ("q", target_q.as_str()),
                ("wt", "json"),
                ("fl", "id,parent_uri,ancestor_uris"),
                ("rows", target_rows.as_str()),
                ("start", "0"),
            ])
            .send()
            .await?;
        if !target_response.status().is_success() {
            let status = target_response.status().as_u16();
            let message = target_response.text().await.unwrap_or_default();
            return Err(ASpaceError::Solr { status, message });
        }
        let targets: PathEnvelope = target_response.json().await?;
        if targets.response.num_found != params.node_ids.len() {
            return Err(ASpaceError::Solr {
                status: 404,
                message: "one or more requested tree nodes are absent from the shadow index"
                    .to_string(),
            });
        }

        let mut parent_uris = std::collections::BTreeSet::new();
        for doc in &targets.response.docs {
            for uri in &doc.ancestor_uris {
                if uri.contains("/archival_objects/") {
                    parent_uris.insert(uri.clone());
                }
            }
        }
        let parent_clauses = parent_uris
            .iter()
            .map(|uri| format!("parent_uri:{}", escape_solr_literal(uri)))
            .collect::<Vec<_>>()
            .join(" OR ");
        let resource_clause = format!(
            "resource_uri:{}",
            escape_solr_literal(params.resource_uri.trim())
        );
        let sibling_clause = if parent_clauses.is_empty() {
            "NOT parent_uri:[* TO *]".to_string()
        } else {
            format!("({parent_clauses} OR (NOT parent_uri:[* TO *]))")
        };
        let path_q = format!("{resource_clause} AND {sibling_clause}");

        let count_response = self
            .http
            .post(&select_url)
            .form(&[("q", path_q.as_str()), ("wt", "json"), ("rows", "0")])
            .send()
            .await?;
        if !count_response.status().is_success() {
            let status = count_response.status().as_u16();
            let message = count_response.text().await.unwrap_or_default();
            return Err(ASpaceError::Solr { status, message });
        }
        let count: PathEnvelope = count_response.json().await?;

        let path_rows = count.response.num_found.to_string();
        let docs_response = self
            .http
            .post(&select_url)
            .form(&[
                ("q", path_q.as_str()),
                ("wt", "json"),
                (
                    "fl",
                    "id,title,display_string,parent_uri,ancestor_uris,position_i,publish,json",
                ),
                ("rows", path_rows.as_str()),
                ("start", "0"),
            ])
            .send()
            .await?;
        if !docs_response.status().is_success() {
            let status = docs_response.status().as_u16();
            let message = docs_response.text().await.unwrap_or_default();
            return Err(ASpaceError::Solr { status, message });
        }
        let docs: PathEnvelope = docs_response.json().await?;

        assemble_node_from_root_paths(params, &node_uris, docs.response.docs)
    }

    /// Retrieve tree waypoint nodes for a resource tree.
    ///
    /// Refuses collection `archivesspace` before making any network request.
    pub async fn tree_waypoint(
        &self,
        params: &ShadowTreeWaypointParams,
    ) -> Result<Vec<ShadowWaypointNode>> {
        if params.collection == "archivesspace" {
            return Err(ASpaceError::Solr {
                status: 403,
                message: "refusing to query live Solr collection 'archivesspace'".to_string(),
            });
        }

        let select_url = format!(
            "{}/{}/select",
            params.base_url.trim_end_matches('/'),
            params.collection
        );
        let q = build_waypoint_query(&params.resource_uri, params.parent_node.as_deref());
        let fl = "id,title,display_string,resource_uri,parent_uri,position_i,json";
        let start = params.offset * WAYPOINT_SIZE;

        let response = self
            .http
            .get(&select_url)
            .query(&[
                ("q", q.as_str()),
                ("wt", "json"),
                ("fl", fl),
                ("sort", "position_i asc, id asc"),
                ("rows", &WAYPOINT_SIZE.to_string()),
                ("start", &start.to_string()),
            ])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown Solr error".to_string());
            return Err(ASpaceError::Solr {
                status: status.as_u16(),
                message: error_text,
            });
        }

        #[derive(Deserialize)]
        struct WaypointEnvelope {
            response: WaypointResponse,
        }

        #[derive(Deserialize)]
        struct WaypointResponse {
            #[serde(default)]
            docs: Vec<WaypointDoc>,
        }

        #[derive(Deserialize)]
        struct WaypointDoc {
            id: String,
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            display_string: Option<String>,
            #[serde(default)]
            parent_uri: Option<String>,
            #[serde(default)]
            position_i: Option<i64>,
            #[serde(default)]
            json: Option<String>,
        }

        let envelope: WaypointEnvelope = response.json().await?;
        let docs = envelope.response.docs;

        if docs.is_empty() {
            return Ok(Vec::new());
        }

        let mut direct_child_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::with_capacity(docs.len());

        let escaped_resource = escape_solr_literal(params.resource_uri.trim());

        // Fetch all direct-child counts for this resource in one field-facet request.
        // This keeps waypoint retrieval at two Solr requests regardless of waypoint width.
        let count_q = format!("resource_uri:{}", escaped_resource);
        let count_resp = self
            .http
            .get(&select_url)
            .query(&[
                ("q", count_q.as_str()),
                ("wt", "json"),
                ("rows", "0"),
                ("facet", "true"),
                ("facet.field", "parent_uri"),
                ("facet.limit", "-1"),
                ("facet.mincount", "1"),
            ])
            .send()
            .await?;
        if count_resp.status().is_success() {
            #[derive(Deserialize)]
            struct FacetEnvelope {
                #[serde(default)]
                facet_counts: Option<FacetCounts>,
            }
            #[derive(Deserialize, Default)]
            struct FacetCounts {
                #[serde(default)]
                facet_fields: std::collections::HashMap<String, Vec<serde_json::Value>>,
            }

            if let Ok(facet_data) = count_resp.json::<FacetEnvelope>().await {
                if let Some(fc) = facet_data.facet_counts {
                    if let Some(values) = fc.facet_fields.get("parent_uri") {
                        for pair in values.as_chunks::<2>().0 {
                            if let (Some(uri), Some(count)) = (pair[0].as_str(), pair[1].as_u64()) {
                                direct_child_counts.insert(uri.to_string(), count as usize);
                            }
                        }
                    }
                }
            }
        }

        let mut result_nodes = Vec::with_capacity(docs.len());
        for (idx, doc) in docs.into_iter().enumerate() {
            let child_cnt = direct_child_counts.get(&doc.id).copied().unwrap_or(0);
            let waypoints = calculate_waypoints(child_cnt);
            let source = doc
                .json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let source_string = |key: &str| {
                source
                    .as_ref()
                    .and_then(|value| value.get(key))
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string)
            };
            let title = source_string("title")
                .or(doc.title)
                .or(doc.display_string)
                .unwrap_or_else(|| "<untitled>".to_string());
            let position = doc.position_i.unwrap_or((start + idx) as i64);
            let parent_id = doc
                .parent_uri
                .as_deref()
                .filter(|uri| uri.contains("/archival_objects/"))
                .and_then(|uri| uri.rsplit('/').next())
                .and_then(|id| id.parse::<u64>().ok());
            let suppressed = source
                .as_ref()
                .and_then(|value| value.get("suppressed"))
                .and_then(|value| value.as_bool())
                .map(u8::from)
                .unwrap_or(0);
            let has_digital_instance = source
                .as_ref()
                .and_then(|value| value.get("instances"))
                .and_then(|value| value.as_array())
                .map(|instances| {
                    instances.iter().any(|instance| {
                        instance
                            .get("digital_object")
                            .is_some_and(|value| !value.is_null())
                    })
                })
                .unwrap_or(false);
            let dates = source
                .as_ref()
                .and_then(|value| value.get("dates"))
                .and_then(|value| value.as_array())
                .map(|dates| {
                    dates
                        .iter()
                        .map(|date| {
                            let mut compact = serde_json::Map::new();
                            for key in ["date_type", "label", "expression", "begin", "end"] {
                                if let Some(value) = date.get(key).filter(|value| !value.is_null())
                                {
                                    let output_key = if key == "date_type" { "type" } else { key };
                                    compact.insert(output_key.to_string(), value.clone());
                                }
                            }
                            serde_json::Value::Object(compact)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let slugged_url = source_string("slug")
                .filter(|slug| !slug.trim().is_empty())
                .map(|slug| format!("{}-{}", doc.id, slug))
                .unwrap_or_default();
            let level = source_string("other_level").or_else(|| source_string("level"));
            let parsed_title = parse_mixed_content_title(&title);
            let containers = tree_container_summaries(source.as_ref());

            result_nodes.push(ShadowWaypointNode {
                child_count: child_cnt,
                waypoints,
                waypoint_size: WAYPOINT_SIZE,
                parsed_title,
                title,
                slugged_url,
                uri: doc.id,
                position,
                parent_id,
                suppressed,
                jsonmodel_type: "archival_object".to_string(),
                has_digital_instance,
                identifier: source_string("component_id"),
                level,
                dates,
                containers,
            });
        }

        Ok(result_nodes)
    }
}

fn assemble_node_from_root_paths(
    params: &ShadowTreeNodeFromRootParams,
    node_uris: &[String],
    docs: Vec<ShadowPathDoc>,
) -> Result<std::collections::BTreeMap<u64, Vec<ShadowNodeFromRootEntry>>> {
    let visible_docs: Vec<&ShadowPathDoc> = docs
        .iter()
        .filter(|doc| {
            doc.id == params.resource_uri
                || (doc.id.contains("/archival_objects/") && doc.visible(params.published_only))
        })
        .collect();
    let by_id: std::collections::HashMap<&str, &ShadowPathDoc> = visible_docs
        .iter()
        .map(|doc| (doc.id.as_str(), *doc))
        .collect();
    let root_title = params.root_title.clone();

    let waypoint_for = |node: &ShadowPathDoc| -> Result<usize> {
        let position = node.position_i.ok_or_else(|| ASpaceError::Solr {
            status: 422,
            message: format!("tree node {} has no position_i", node.id),
        })?;
        let sibling_count = visible_docs
            .iter()
            .filter(|candidate| {
                candidate.id.contains("/archival_objects/")
                    && candidate.parent_uri == node.parent_uri
                    && candidate.position_i.is_some_and(|value| value <= position)
            })
            .count();
        Ok(sibling_count / WAYPOINT_SIZE)
    };

    let mut result = std::collections::BTreeMap::new();
    for (node_id, node_uri) in params.node_ids.iter().zip(node_uris) {
        let mut current =
            by_id
                .get(node_uri.as_str())
                .copied()
                .ok_or_else(|| ASpaceError::Solr {
                    status: 404,
                    message: format!(
                        "requested tree node {node_uri} is not visible in the shadow index"
                    ),
                })?;
        let mut path = Vec::new();

        while let Some(parent_uri) = current.parent_uri.as_deref() {
            let parent = by_id
                .get(parent_uri)
                .copied()
                .ok_or_else(|| ASpaceError::Solr {
                    status: 422,
                    message: format!("tree parent {parent_uri} is absent from the shadow path set"),
                })?;
            let title = parent.display_title();
            path.push(ShadowNodeFromRootEntry {
                node: Some(parent.id.clone()),
                root_record_uri: params.resource_uri.clone(),
                offset: waypoint_for(current)?,
                jsonmodel_type: "archival_object".to_string(),
                parsed_title: parse_mixed_content_title(&title),
                title,
            });
            current = parent;
        }

        path.push(ShadowNodeFromRootEntry {
            node: None,
            root_record_uri: params.resource_uri.clone(),
            offset: waypoint_for(current)?,
            jsonmodel_type: "resource".to_string(),
            parsed_title: parse_mixed_content_title(&root_title),
            title: root_title.clone(),
        });
        path.reverse();
        result.insert(*node_id, path);
    }

    Ok(result)
}

/// Collects missing ancestor/context archival object URIs needed to connect matched hits
/// to the root resource URI.
///
/// Rules:
/// - Excludes `root_resource_uri` (since the root resource itself is not an archival object in the hits).
/// - Excludes URIs already present in `matches` (matching hits win and should not be re-fetched as context).
/// - Excludes duplicate URIs (returns deduplicated URIs in deterministic sorted order).
/// - Inspects `parent_uri` and `ancestor_uris` of matches, collecting URIs that refer to
///   archival objects (i.e. not the root resource) needed to trace up to the root.
pub fn collect_missing_ancestor_uris(
    matches: &[ShadowArchivalObjectHit],
    root_resource_uri: &str,
) -> Vec<String> {
    let mut known_ids: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(matches.len());
    for hit in matches {
        known_ids.insert(hit.id.as_str());
    }

    let mut needed_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for hit in matches {
        if let Some(parent) = &hit.parent_uri {
            let parent_str = parent.as_str();
            if parent_str != root_resource_uri && !known_ids.contains(parent_str) {
                needed_ids.insert(parent.clone());
            }
        }

        for anc in &hit.ancestor_uris {
            let anc_str = anc.as_str();
            if anc_str != root_resource_uri && !known_ids.contains(anc_str) {
                needed_ids.insert(anc.clone());
            }
        }
    }

    needed_ids.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[test]
    fn test_escape_solr_literal() {
        assert_eq!(
            escape_solr_literal("/repositories/2/resources/18"),
            "\\/repositories\\/2\\/resources\\/18"
        );
        assert_eq!(escape_solr_literal("foo:bar"), "foo\\:bar");
        assert_eq!(escape_solr_literal("foo\"bar"), "foo\\\"bar");
        assert_eq!(escape_solr_literal("a && b || c"), "a \\&\\& b \\|\\| c");
        assert_eq!(escape_solr_literal("a*b?c"), "a\\*b\\?c");
    }

    #[test]
    fn test_escape_solr_wildcard_filter() {
        assert_eq!(escape_solr_wildcard_filter("*1974*"), "*1974*");
        assert_eq!(escape_solr_wildcard_filter("195?"), "195?");
        assert_eq!(escape_solr_wildcard_filter("\"1950\""), "\\\"1950\\\"");
        assert_eq!(
            escape_solr_wildcard_filter("/repositories/2/resources/18"),
            "\\/repositories\\/2\\/resources\\/18"
        );
        assert_eq!(
            escape_solr_wildcard_filter("foo:bar AND (baz)*"),
            "foo\\:bar\\ AND\\ \\(baz\\)*"
        );
    }

    #[test]
    fn test_build_descendants_query() {
        let q1 = build_descendants_query("/repositories/2/resources/18", None);
        assert_eq!(q1, "ancestor_uris:\\/repositories\\/2\\/resources\\/18");

        let q2 = build_descendants_query("/repositories/2/resources/18", Some("*1974*"));
        assert_eq!(
            q2,
            "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:*1974*"
        );

        let q3 = build_descendants_query("/repositories/2/resources/18", Some("1950"));
        assert_eq!(
            q3,
            "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:1950"
        );

        let q4 = build_descendants_query("/repositories/2/resources/18", Some("   "));
        assert_eq!(q4, "ancestor_uris:\\/repositories\\/2\\/resources\\/18");
    }

    #[test]
    fn test_build_descendants_query_with_filters() {
        // Resource only
        let q1 = build_descendants_query_with_filters("/repositories/2/resources/18", None, None);
        assert_eq!(q1, "ancestor_uris:\\/repositories\\/2\\/resources\\/18");

        // Resource + Title filter with wildcard
        let q2 = build_descendants_query_with_filters(
            "/repositories/2/resources/18",
            None,
            Some("*Example*"),
        );
        assert_eq!(
            q2,
            "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND title:*Example*"
        );

        // Resource + Date + Title filter
        let q3 = build_descendants_query_with_filters(
            "/repositories/2/resources/18",
            Some("*1974*"),
            Some("*Sample*"),
        );
        assert_eq!(
            q3,
            "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:*1974* AND title:*Sample*"
        );

        // Blank filters ignored
        let q4 = build_descendants_query_with_filters(
            "/repositories/2/resources/18",
            Some("  "),
            Some("   "),
        );
        assert_eq!(q4, "ancestor_uris:\\/repositories\\/2\\/resources\\/18");

        // Escaping in title filter preserves wildcards but escapes Lucene injection
        let q5 = build_descendants_query_with_filters(
            "/repositories/2/resources/18",
            Some("*1974*"),
            Some("foo:bar AND (baz)*"),
        );
        assert_eq!(
            q5,
            "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:*1974* AND title:foo\\:bar\\ AND\\ \\(baz\\)*"
        );
    }

    #[tokio::test]
    async fn test_descendants_query_success() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("q".into(), "ancestor_uris:\\/repositories\\/2\\/resources\\/18".into()),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "50".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 2181,
                        "start": 0,
                        "docs": [
                            {
                                "id": "/repositories/2/archival_objects/4031",
                                "title": "Damaso Alonso: ejemplo y acicate (550), 1974 enero 4",
                                "display_string": "Damaso Alonso: ejemplo y acicate (550), 1974 enero 4",
                                "resource_uri": "/repositories/2/resources/18",
                                "parent_uri": "/repositories/2/archival_objects/4028",
                                "ancestor_uris": [
                                    "/repositories/2/resources/18",
                                    "/repositories/2/archival_objects/4028"
                                ],
                                "ancestor_levels": ["collection", "series"],
                                "date_expressions": ["1974 enero 4", "1974"]
                            }
                        ]
                    }
                }"#,
            )
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let result = client
            .descendants("/repositories/2/resources/18", 50, 0)
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(result.num_found, 2181);
        assert_eq!(result.start, 0);
        assert_eq!(result.docs.len(), 1);
        let doc = &result.docs[0];
        assert_eq!(doc.id, "/repositories/2/archival_objects/4031");
        assert_eq!(
            doc.title.as_deref(),
            Some("Damaso Alonso: ejemplo y acicate (550), 1974 enero 4")
        );
        assert_eq!(
            doc.resource_uri.as_deref(),
            Some("/repositories/2/resources/18")
        );
        assert_eq!(
            doc.parent_uri.as_deref(),
            Some("/repositories/2/archival_objects/4028")
        );
        assert_eq!(doc.ancestor_uris.len(), 2);
        assert_eq!(doc.ancestor_levels.len(), 2);
        assert_eq!(doc.date_expressions, vec!["1974 enero 4", "1974"]);
    }

    #[tokio::test]
    async fn test_descendants_with_date_expression_and_pagination() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:*1974*".into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "10".into()),
                mockito::Matcher::UrlEncoded("start".into(), "20".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 106,
                        "start": 20,
                        "docs": [
                            {
                                "id": "/repositories/2/archival_objects/5001",
                                "title": "Letter 1974",
                                "display_string": "Letter 1974",
                                "resource_uri": "/repositories/2/resources/18",
                                "date_expressions": ["1974"]
                            }
                        ]
                    }
                }"#,
            )
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let result = client
            .descendants_with_date_expression("/repositories/2/resources/18", "*1974*", 10, 20)
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(result.num_found, 106);
        assert_eq!(result.start, 20);
        assert_eq!(result.docs.len(), 1);
        assert_eq!(result.docs[0].id, "/repositories/2/archival_objects/5001");
    }

    #[tokio::test]
    async fn test_count_descendants_success() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "ancestor_uris:\\/repositories\\/2\\/resources\\/3".into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "0".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 24,
                        "start": 0,
                        "docs": []
                    }
                }"#,
            )
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let count = client
            .count_descendants("/repositories/2/resources/3")
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(count, 24);
    }

    #[tokio::test]
    async fn test_count_descendants_with_date_expression_success() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:1950"
                        .into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "0".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 15,
                        "start": 0,
                        "docs": []
                    }
                }"#,
            )
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let count = client
            .count_descendants_with_date_expression("/repositories/2/resources/18", "1950")
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(count, 15);
    }

    #[tokio::test]
    async fn test_descendants_with_date_and_title_and_count_success() {
        let mut server = Server::new_async().await;

        let mock_descendants = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:*1974* AND title:*Example*"
                        .into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "5".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 5,
                        "start": 0,
                        "docs": [
                            {
                                "id": "/repositories/2/archival_objects/2032",
                                "title": "Example, 1974",
                                "display_string": "Example, 1974",
                                "resource_uri": "/repositories/2/resources/18",
                                "date_expressions": ["1974"]
                            }
                        ]
                    }
                }"#,
            )
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let result = client
            .descendants_with_date_and_title(
                "/repositories/2/resources/18",
                Some("*1974*"),
                Some("*Example*"),
                5,
                0,
            )
            .await
            .unwrap();

        mock_descendants.assert_async().await;
        assert_eq!(result.num_found, 5);
        assert_eq!(result.docs.len(), 1);
        assert_eq!(result.docs[0].id, "/repositories/2/archival_objects/2032");

        let mock_count = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:*1974* AND title:*Sample*"
                        .into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "0".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 1,
                        "start": 0,
                        "docs": []
                    }
                }"#,
            )
            .create_async()
            .await;

        let count = client
            .count_descendants_with_date_and_title(
                "/repositories/2/resources/18",
                Some("*1974*"),
                Some("*Sample*"),
            )
            .await
            .unwrap();

        mock_count.assert_async().await;
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_solr_error_propagation() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::Any)
            .with_status(400)
            .with_body("Bad Solr Query")
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let err = client
            .count_descendants("/repositories/2/resources/18")
            .await
            .unwrap_err();

        mock.assert_async().await;
        match err {
            ASpaceError::Solr { status, message } => {
                assert_eq!(status, 400);
                assert_eq!(message, "Bad Solr Query");
            }
            other => panic!("Unexpected error variant: {:?}", other),
        }
    }

    #[test]
    fn test_tree_assembly_two_level() {
        let hits = vec![
            ShadowArchivalObjectHit {
                id: "/repositories/2/archival_objects/1".to_string(),
                title: Some("Series 1".to_string()),
                display_string: None,
                resource_uri: Some("/repositories/2/resources/10".to_string()),
                parent_uri: Some("/repositories/2/resources/10".to_string()),
                ancestor_uris: vec!["/repositories/2/resources/10".to_string()],
                ancestor_levels: vec!["collection".to_string()],
                date_expressions: vec!["1970".to_string()],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/repositories/2/archival_objects/2".to_string(),
                title: Some("File 1.1".to_string()),
                display_string: None,
                resource_uri: Some("/repositories/2/resources/10".to_string()),
                parent_uri: Some("/repositories/2/archival_objects/1".to_string()),
                ancestor_uris: vec![
                    "/repositories/2/archival_objects/1".to_string(),
                    "/repositories/2/resources/10".to_string(),
                ],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec!["1971".to_string()],
                position_i: None,
            },
        ];

        let res = assemble_shadow_tree(hits, "/repositories/2/resources/10");
        assert_eq!(res.root_resource_uri, "/repositories/2/resources/10");
        assert_eq!(res.total_nodes, 2);
        assert_eq!(res.orphans.len(), 0);
        assert_eq!(res.tree.len(), 1);

        let series = &res.tree[0];
        assert_eq!(series.id, "/repositories/2/archival_objects/1");
        assert_eq!(series.title, "Series 1");
        assert_eq!(
            series.parent_uri.as_deref(),
            Some("/repositories/2/resources/10")
        );
        assert_eq!(series.children.len(), 1);

        let file = &series.children[0];
        assert_eq!(file.id, "/repositories/2/archival_objects/2");
        assert_eq!(file.title, "File 1.1");
        assert_eq!(
            file.parent_uri.as_deref(),
            Some("/repositories/2/archival_objects/1")
        );
        assert_eq!(file.children.len(), 0);
    }

    #[test]
    fn test_tree_assembly_deep_nested() {
        let hits = vec![
            ShadowArchivalObjectHit {
                id: "/ao/1".to_string(),
                title: Some("Series".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/res/1".to_string()),
                ancestor_uris: vec!["/res/1".to_string()],
                ancestor_levels: vec!["collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/2".to_string(),
                title: Some("Subseries".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/1".to_string()),
                ancestor_uris: vec!["/ao/1".to_string(), "/res/1".to_string()],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/3".to_string(),
                title: Some("File".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/2".to_string()),
                ancestor_uris: vec![
                    "/ao/2".to_string(),
                    "/ao/1".to_string(),
                    "/res/1".to_string(),
                ],
                ancestor_levels: vec![
                    "subseries".to_string(),
                    "series".to_string(),
                    "collection".to_string(),
                ],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/4".to_string(),
                title: Some("Item".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/3".to_string()),
                ancestor_uris: vec![
                    "/ao/3".to_string(),
                    "/ao/2".to_string(),
                    "/ao/1".to_string(),
                    "/res/1".to_string(),
                ],
                ancestor_levels: vec![
                    "file".to_string(),
                    "subseries".to_string(),
                    "series".to_string(),
                    "collection".to_string(),
                ],
                date_expressions: vec!["1999".to_string()],
                position_i: None,
            },
        ];

        let res = assemble_shadow_tree(hits, "/res/1");
        assert_eq!(res.total_nodes, 4);
        assert_eq!(res.orphans.len(), 0);
        assert_eq!(res.tree.len(), 1);
        assert_eq!(res.tree[0].id, "/ao/1");
        assert_eq!(res.tree[0].children.len(), 1);
        assert_eq!(res.tree[0].children[0].id, "/ao/2");
        assert_eq!(res.tree[0].children[0].children.len(), 1);
        assert_eq!(res.tree[0].children[0].children[0].id, "/ao/3");
        assert_eq!(res.tree[0].children[0].children[0].children.len(), 1);
        assert_eq!(res.tree[0].children[0].children[0].children[0].id, "/ao/4");
    }

    #[test]
    fn test_tree_assembly_orphan_handling() {
        let hits = vec![
            ShadowArchivalObjectHit {
                id: "/ao/child_missing_parent".to_string(),
                title: Some("Orphan File".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/missing_series".to_string()),
                ancestor_uris: vec!["/ao/missing_series".to_string(), "/res/1".to_string()],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec!["1980".to_string()],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/child_of_orphan".to_string(),
                title: Some("Child of Orphan".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/child_missing_parent".to_string()),
                ancestor_uris: vec![
                    "/ao/child_missing_parent".to_string(),
                    "/ao/missing_series".to_string(),
                    "/res/1".to_string(),
                ],
                ancestor_levels: vec![
                    "file".to_string(),
                    "series".to_string(),
                    "collection".to_string(),
                ],
                date_expressions: vec!["1981".to_string()],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/root_item".to_string(),
                title: Some("Direct Child".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/res/1".to_string()),
                ancestor_uris: vec!["/res/1".to_string()],
                ancestor_levels: vec!["collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
        ];

        let res = assemble_shadow_tree(hits, "/res/1");
        assert_eq!(res.total_nodes, 3);
        assert_eq!(res.tree.len(), 1);
        assert_eq!(res.tree[0].id, "/ao/root_item");

        assert_eq!(res.orphans.len(), 1);
        let orphan = &res.orphans[0];
        assert_eq!(orphan.id, "/ao/child_missing_parent");
        assert_eq!(orphan.parent_uri.as_deref(), Some("/ao/missing_series"));
        assert_eq!(orphan.children.len(), 1);
        assert_eq!(orphan.children[0].id, "/ao/child_of_orphan");
    }

    #[test]
    fn test_tree_assembly_deterministic_ordering() {
        let hits = vec![
            ShadowArchivalObjectHit {
                id: "/ao/1".to_string(),
                title: Some("Parent 1".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/res/1".to_string()),
                ancestor_uris: vec!["/res/1".to_string()],
                ancestor_levels: vec!["collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/1_child_b".to_string(),
                title: Some("Child B".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/1".to_string()),
                ancestor_uris: vec!["/ao/1".to_string(), "/res/1".to_string()],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/1_child_a".to_string(),
                title: Some("Child A".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/1".to_string()),
                ancestor_uris: vec!["/ao/1".to_string(), "/res/1".to_string()],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/2".to_string(),
                title: Some("Parent 2".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/res/1".to_string()),
                ancestor_uris: vec!["/res/1".to_string()],
                ancestor_levels: vec!["collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
        ];

        let res = assemble_shadow_tree(hits, "/res/1");
        assert_eq!(res.tree.len(), 2);
        assert_eq!(res.tree[0].id, "/ao/1");
        assert_eq!(res.tree[1].id, "/ao/2");

        // Children preserve the exact encounter order from input hits
        assert_eq!(res.tree[0].children.len(), 2);
        assert_eq!(res.tree[0].children[0].id, "/ao/1_child_b");
        assert_eq!(res.tree[0].children[1].id, "/ao/1_child_a");
    }

    #[test]
    fn test_tree_assembly_example_parent_context() {
        // Parent context node matched=false, 5 children matched=true, orphans empty.
        let parent_context = ShadowArchivalObjectHit {
            id: "/repositories/2/archival_objects/1712".to_string(),
            title: Some("Example parent record".to_string()),
            display_string: None,
            resource_uri: Some("/repositories/2/resources/18".to_string()),
            parent_uri: Some("/repositories/2/resources/18".to_string()),
            ancestor_uris: vec!["/repositories/2/resources/18".to_string()],
            ancestor_levels: vec!["collection".to_string()],
            date_expressions: vec!["1960-1975".to_string()],
            position_i: None,
        };

        let matches: Vec<ShadowArchivalObjectHit> = (1..=5)
            .map(|i| ShadowArchivalObjectHit {
                id: format!("/repositories/2/archival_objects/{}", 2030 + i),
                title: Some(format!("Example letter {}", i)),
                display_string: None,
                resource_uri: Some("/repositories/2/resources/18".to_string()),
                parent_uri: Some("/repositories/2/archival_objects/1712".to_string()),
                ancestor_uris: vec![
                    "/repositories/2/archival_objects/1712".to_string(),
                    "/repositories/2/resources/18".to_string(),
                ],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec!["1974".to_string()],
                position_i: None,
            })
            .collect();

        let res = assemble_shadow_tree_with_context(
            matches,
            vec![parent_context],
            "/repositories/2/resources/18",
        );

        assert_eq!(res.total_nodes, 6);
        assert_eq!(res.orphans.len(), 0);
        assert_eq!(res.tree.len(), 1);

        let parent_node = &res.tree[0];
        assert_eq!(parent_node.id, "/repositories/2/archival_objects/1712");
        assert!(!parent_node.matched);
        assert_eq!(parent_node.children.len(), 5);

        for (i, child) in parent_node.children.iter().enumerate() {
            assert_eq!(
                child.id,
                format!("/repositories/2/archival_objects/{}", 2031 + i)
            );
            assert!(child.matched);
            assert_eq!(
                child.parent_uri.as_deref(),
                Some("/repositories/2/archival_objects/1712")
            );
            assert_eq!(child.children.len(), 0);
        }
    }

    #[test]
    fn test_tree_assembly_existing_marks_all_matched() {
        let hits = vec![
            ShadowArchivalObjectHit {
                id: "/ao/1".to_string(),
                title: Some("Parent Series".to_string()),
                display_string: None,
                resource_uri: Some("/res/10".to_string()),
                parent_uri: Some("/res/10".to_string()),
                ancestor_uris: vec!["/res/10".to_string()],
                ancestor_levels: vec!["collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/2".to_string(),
                title: Some("Child File".to_string()),
                display_string: None,
                resource_uri: Some("/res/10".to_string()),
                parent_uri: Some("/ao/1".to_string()),
                ancestor_uris: vec!["/ao/1".to_string(), "/res/10".to_string()],
                ancestor_levels: vec!["series".to_string(), "collection".to_string()],
                date_expressions: vec![],
                position_i: None,
            },
        ];

        let res = assemble_shadow_tree(hits, "/res/10");
        assert_eq!(res.total_nodes, 2);
        assert_eq!(res.orphans.len(), 0);
        assert_eq!(res.tree.len(), 1);
        assert!(res.tree[0].matched);
        assert_eq!(res.tree[0].children.len(), 1);
        assert!(res.tree[0].children[0].matched);
    }

    #[test]
    fn test_tree_assembly_missing_context_still_creates_orphans() {
        let matches = vec![ShadowArchivalObjectHit {
            id: "/ao/child_missing".to_string(),
            title: Some("Child with Missing Parent".to_string()),
            display_string: None,
            resource_uri: Some("/res/1".to_string()),
            parent_uri: Some("/ao/unprovided_parent".to_string()),
            ancestor_uris: vec!["/ao/unprovided_parent".to_string(), "/res/1".to_string()],
            ancestor_levels: vec!["series".to_string(), "collection".to_string()],
            date_expressions: vec![],
            position_i: None,
        }];

        let context_hits = vec![ShadowArchivalObjectHit {
            id: "/ao/unrelated_context".to_string(),
            title: Some("Unrelated Context Series".to_string()),
            display_string: None,
            resource_uri: Some("/res/1".to_string()),
            parent_uri: Some("/res/1".to_string()),
            ancestor_uris: vec!["/res/1".to_string()],
            ancestor_levels: vec!["collection".to_string()],
            date_expressions: vec![],
            position_i: None,
        }];

        let res = assemble_shadow_tree_with_context(matches, context_hits, "/res/1");
        assert_eq!(res.total_nodes, 2);
        assert_eq!(res.tree.len(), 1);
        assert_eq!(res.tree[0].id, "/ao/unrelated_context");
        assert!(!res.tree[0].matched);

        assert_eq!(res.orphans.len(), 1);
        let orphan = &res.orphans[0];
        assert_eq!(orphan.id, "/ao/child_missing");
        assert!(orphan.matched);
        assert_eq!(orphan.parent_uri.as_deref(), Some("/ao/unprovided_parent"));
    }

    #[test]
    fn test_tree_assembly_with_context_deterministic_ordering() {
        let matches = vec![
            ShadowArchivalObjectHit {
                id: "/ao/m1".to_string(),
                title: Some("Match 1".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/p1".to_string()),
                ancestor_uris: vec!["/ao/p1".to_string(), "/res/1".to_string()],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/m2".to_string(),
                title: Some("Match 2".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/p1".to_string()),
                ancestor_uris: vec!["/ao/p1".to_string(), "/res/1".to_string()],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
        ];

        let context_hits = vec![
            ShadowArchivalObjectHit {
                id: "/ao/p1".to_string(),
                title: Some("Parent Context 1".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/res/1".to_string()),
                ancestor_uris: vec!["/res/1".to_string()],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/p2".to_string(),
                title: Some("Parent Context 2".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/res/1".to_string()),
                ancestor_uris: vec!["/res/1".to_string()],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
        ];

        let res = assemble_shadow_tree_with_context(matches, context_hits, "/res/1");
        assert_eq!(res.tree.len(), 2);
        assert_eq!(res.tree[0].id, "/ao/p1");
        assert_eq!(res.tree[1].id, "/ao/p2");

        assert_eq!(res.tree[0].children.len(), 2);
        assert_eq!(res.tree[0].children[0].id, "/ao/m1");
        assert_eq!(res.tree[0].children[1].id, "/ao/m2");
    }

    #[test]
    fn test_collect_missing_ancestor_uris_exact_and_deduplicated() {
        let matches = vec![
            ShadowArchivalObjectHit {
                id: "/repositories/2/archival_objects/2031".to_string(),
                title: Some("Letter 1".to_string()),
                display_string: None,
                resource_uri: Some("/repositories/2/resources/18".to_string()),
                parent_uri: Some("/repositories/2/archival_objects/1712".to_string()),
                ancestor_uris: vec![
                    "/repositories/2/archival_objects/1712".to_string(),
                    "/repositories/2/archival_objects/1000".to_string(),
                    "/repositories/2/resources/18".to_string(),
                ],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/repositories/2/archival_objects/2032".to_string(),
                title: Some("Letter 2".to_string()),
                display_string: None,
                resource_uri: Some("/repositories/2/resources/18".to_string()),
                parent_uri: Some("/repositories/2/archival_objects/1712".to_string()),
                ancestor_uris: vec![
                    "/repositories/2/archival_objects/1712".to_string(),
                    "/repositories/2/archival_objects/1000".to_string(),
                    "/repositories/2/resources/18".to_string(),
                ],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
        ];

        let missing = collect_missing_ancestor_uris(&matches, "/repositories/2/resources/18");
        // Should only contain /ao/1000 and /ao/1712, sorted, deduplicated, and excluding root /res/18
        assert_eq!(
            missing,
            vec![
                "/repositories/2/archival_objects/1000".to_string(),
                "/repositories/2/archival_objects/1712".to_string(),
            ]
        );
    }

    #[test]
    fn test_collect_missing_ancestor_uris_suppresses_matches() {
        // If an ancestor is already in matches, it must not be in missing
        let matches = vec![
            ShadowArchivalObjectHit {
                id: "/ao/parent".to_string(),
                title: Some("Parent Series".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/res/1".to_string()),
                ancestor_uris: vec!["/res/1".to_string()],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
            ShadowArchivalObjectHit {
                id: "/ao/child".to_string(),
                title: Some("Child File".to_string()),
                display_string: None,
                resource_uri: Some("/res/1".to_string()),
                parent_uri: Some("/ao/parent".to_string()),
                ancestor_uris: vec!["/ao/parent".to_string(), "/res/1".to_string()],
                ancestor_levels: vec![],
                date_expressions: vec![],
                position_i: None,
            },
        ];

        let missing = collect_missing_ancestor_uris(&matches, "/res/1");
        assert!(missing.is_empty());
    }

    #[tokio::test]
    async fn test_docs_by_ids_mocked() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("q".into(), "id:\\/ao\\/1 OR id:\\/ao\\/2".into()),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "2".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 2,
                        "start": 0,
                        "docs": [
                            {
                                "id": "/ao/1",
                                "title": "Context 1",
                                "resource_uri": "/res/1",
                                "parent_uri": "/res/1"
                            },
                            {
                                "id": "/ao/2",
                                "title": "Context 2",
                                "resource_uri": "/res/1",
                                "parent_uri": "/res/1"
                            }
                        ]
                    }
                }"#,
            )
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let docs = client
            .docs_by_ids(&["/ao/1".to_string(), "/ao/2".to_string()])
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].id, "/ao/1");
        assert_eq!(docs[1].id, "/ao/2");

        // Empty IDs should short-circuit and not call Solr
        let empty_docs = client.docs_by_ids(&[]).await.unwrap();
        assert!(empty_docs.is_empty());
    }

    #[test]
    fn test_build_keyword_query_none_or_blank() {
        let params_none = ShadowKeywordSearchParams::new();
        assert_eq!(build_keyword_query(&params_none), "*:*");

        let params_blank = ShadowKeywordSearchParams::new().with_query("   ");
        assert_eq!(build_keyword_query(&params_blank), "*:*");
    }

    #[test]
    fn test_build_keyword_query_searches_fields_and_escapes() {
        let params = ShadowKeywordSearchParams::new().with_query("Example");
        assert_eq!(
            build_keyword_query(&params),
            "(title:Example OR display_string:Example OR date_expressions:Example)"
        );

        // Dangerous syntax escaping (colons, parentheses, wildcards preserved)
        let params_dangerous =
            ShadowKeywordSearchParams::new().with_query("title:hack AND (test)*");
        assert_eq!(
            build_keyword_query(&params_dangerous),
            "(title:title\\:hack\\ AND\\ \\(test\\)* OR display_string:title\\:hack\\ AND\\ \\(test\\)* OR date_expressions:title\\:hack\\ AND\\ \\(test\\)*)"
        );
    }

    #[test]
    fn test_build_keyword_query_filters_combine_with_and() {
        let params = ShadowKeywordSearchParams::new()
            .with_query("Example")
            .with_resource_uri("/repositories/2/resources/18")
            .with_date_filter("*1974*")
            .with_title_filter("*Sample*");

        assert_eq!(
            build_keyword_query(&params),
            "(title:Example OR display_string:Example OR date_expressions:Example) AND ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:*1974* AND title:*Sample*"
        );

        // With blank query but filters present
        let params_no_q = ShadowKeywordSearchParams::new()
            .with_resource_uri("/repositories/2/resources/18")
            .with_date_filter("1970");

        assert_eq!(
            build_keyword_query(&params_no_q),
            "*:* AND ancestor_uris:\\/repositories\\/2\\/resources\\/18 AND date_expressions:1970"
        );
    }

    #[test]
    fn test_build_waypoint_query_root_and_nested() {
        assert_eq!(
            build_waypoint_query("/repositories/2/resources/18", None),
            "resource_uri:\\/repositories\\/2\\/resources\\/18 AND -parent_uri:[* TO *]"
        );
        assert_eq!(
            build_waypoint_query(
                "/repositories/2/resources/18",
                Some("/repositories/2/archival_objects/100")
            ),
            "resource_uri:\\/repositories\\/2\\/resources\\/18 AND parent_uri:\\/repositories\\/2\\/archival_objects\\/100"
        );
    }

    #[test]
    fn test_parse_mixed_content_title_matches_large_tree_shapes() {
        assert_eq!(
            parse_mixed_content_title("What the & 'heck' <emph>ok</emph>?"),
            "What the &amp; 'heck' <span class=\"emph render-none\">ok</span>?"
        );
        assert_eq!(
            parse_mixed_content_title("<title render='italic'>Title</title>"),
            "<span class=\"emph render-italic\">Title</span>"
        );
        assert_eq!(
            parse_mixed_content_title("<emph render=\"nonproport\">Code</emph>"),
            "<code class=\"emph render-nonproport\">Code</code>"
        );
    }

    #[test]
    fn test_assemble_node_from_root_paths_matches_large_tree_contract() {
        let params = ShadowTreeNodeFromRootParams::new(
            "/repositories/2/resources/16",
            "Root <emph>title</emph>",
            vec![30],
        )
        .with_published_only(true);
        let node_uris = vec!["/repositories/2/archival_objects/30".to_string()];
        let docs = vec![
            ShadowPathDoc {
                id: "/repositories/2/archival_objects/10".to_string(),
                title: Some("Top".to_string()),
                display_string: None,
                parent_uri: None,
                ancestor_uris: vec!["/repositories/2/resources/16".to_string()],
                position_i: Some(0),
                publish: true,
                json: None,
            },
            ShadowPathDoc {
                id: "/repositories/2/archival_objects/20".to_string(),
                title: Some("Series".to_string()),
                display_string: Some("Series display".to_string()),
                parent_uri: Some("/repositories/2/archival_objects/10".to_string()),
                ancestor_uris: vec![],
                position_i: Some(0),
                publish: true,
                json: None,
            },
            ShadowPathDoc {
                id: "/repositories/2/archival_objects/30".to_string(),
                title: Some("Requested leaf".to_string()),
                display_string: None,
                parent_uri: Some("/repositories/2/archival_objects/20".to_string()),
                ancestor_uris: vec![],
                position_i: Some(0),
                publish: true,
                json: None,
            },
            ShadowPathDoc {
                id: "/repositories/2/archival_objects/31".to_string(),
                title: Some("Hidden sibling".to_string()),
                display_string: None,
                parent_uri: Some("/repositories/2/archival_objects/20".to_string()),
                ancestor_uris: vec![],
                position_i: Some(0),
                publish: false,
                json: None,
            },
        ];

        let paths = assemble_node_from_root_paths(&params, &node_uris, docs).unwrap();
        assert_eq!(
            paths[&30],
            vec![
                ShadowNodeFromRootEntry {
                    node: None,
                    root_record_uri: "/repositories/2/resources/16".to_string(),
                    offset: 0,
                    jsonmodel_type: "resource".to_string(),
                    title: "Root <emph>title</emph>".to_string(),
                    parsed_title: "Root <span class=\"emph render-none\">title</span>".to_string(),
                },
                ShadowNodeFromRootEntry {
                    node: Some("/repositories/2/archival_objects/10".to_string()),
                    root_record_uri: "/repositories/2/resources/16".to_string(),
                    offset: 0,
                    jsonmodel_type: "archival_object".to_string(),
                    title: "Top".to_string(),
                    parsed_title: "Top".to_string(),
                },
                ShadowNodeFromRootEntry {
                    node: Some("/repositories/2/archival_objects/20".to_string()),
                    root_record_uri: "/repositories/2/resources/16".to_string(),
                    offset: 0,
                    jsonmodel_type: "archival_object".to_string(),
                    title: "Series display".to_string(),
                    parsed_title: "Series display".to_string(),
                },
            ]
        );
    }

    #[test]
    fn test_tree_container_summaries_from_resolved_api_json() {
        let source = serde_json::json!({
            "instances": [{
                "instance_type": "text",
                "sub_container": {
                    "type_2": "Folder",
                    "indicator_2": "3",
                    "top_container": {
                        "_resolved": {
                            "type": "Box",
                            "indicator": "1",
                            "barcode": "12345"
                        }
                    }
                }
            }]
        });
        assert_eq!(
            tree_container_summaries(Some(&source)),
            vec![serde_json::json!({
                "instance_type": "text",
                "top_container_type": "Box",
                "top_container_indicator": "1",
                "top_container_barcode": "12345",
                "type_2": "Folder",
                "indicator_2": "3"
            })]
        );
    }

    #[test]
    fn test_calculate_waypoints_uses_ceiling() {
        assert_eq!(calculate_waypoints(0), 0);
        assert_eq!(calculate_waypoints(1), 1);
        assert_eq!(calculate_waypoints(200), 1);
        assert_eq!(calculate_waypoints(201), 2);
        assert_eq!(calculate_waypoints(400), 2);
    }

    #[tokio::test]
    async fn test_keyword_search_refuses_live_collection() {
        let client = ShadowSolrClient::new("http://localhost:8985/solr", "archivesspace_shadow");
        let params = ShadowKeywordSearchParams::new().with_collection("archivesspace");

        let res = client.keyword_search(&params).await;
        match res {
            Err(ASpaceError::Solr { status, message }) => {
                assert_eq!(status, 403);
                assert!(
                    message.contains("refusing to query live Solr collection 'archivesspace'"),
                    "unexpected message: {}",
                    message
                );
            }
            other => panic!("expected Solr error with status 403, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_tree_waypoint_root_mocked_success() {
        let mut server = Server::new_async().await;

        let children_mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "resource_uri:\\/res\\/1 AND -parent_uri:[* TO *]".into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded(
                    "fl".into(),
                    "id,title,display_string,resource_uri,parent_uri,position_i,json".into(),
                ),
                mockito::Matcher::UrlEncoded("sort".into(), "position_i asc, id asc".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "200".into()),
                mockito::Matcher::UrlEncoded("start".into(), "0".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 2,
                        "start": 0,
                        "docs": [
                            {
                                "id": "/ao/1",
                                "display_string": "Series Display",
                                "resource_uri": "/res/1",
                                "parent_uri": "/res/1",
                                "position_i": 2,
                                "json": "{\"title\":\"Series Source Title\",\"component_id\":\"SER-1\",\"level\":\"series\",\"suppressed\":true,\"slug\":\"series-source-title\",\"dates\":[{\"date_type\":\"inclusive\",\"label\":\"creation\",\"expression\":\"1900-1950\",\"begin\":\"1900\",\"end\":\"1950\"}],\"instances\":[{\"digital_object\":{\"ref\":\"/repositories/2/digital_objects/1\"}}]}"
                            },
                            {
                                "id": "/ao/2",
                                "title": "Series Two",
                                "resource_uri": "/res/1",
                                "parent_uri": "/res/1",
                                "position_i": 3
                            }
                        ]
                    }
                }"#,
            )
            .create_async()
            .await;

        let counts_mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "facet_counts": {
                        "facet_fields": {
                            "parent_uri": ["/ao/1", 201]
                        }
                    }
                }"#,
            )
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let params = ShadowTreeWaypointParams::new("/res/1", 0)
            .with_base_url(server.url())
            .with_collection("archivesspace_shadow");

        let nodes = client.tree_waypoint(&params).await.unwrap();
        children_mock.assert_async().await;
        counts_mock.assert_async().await;

        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].uri, "/ao/1");
        assert_eq!(nodes[0].title, "Series Source Title");
        assert_eq!(nodes[0].parsed_title, "Series Source Title");
        assert_eq!(nodes[0].slugged_url, "/ao/1-series-source-title");
        assert_eq!(nodes[0].position, 2);
        assert_eq!(nodes[0].child_count, 201);
        assert_eq!(nodes[0].waypoints, 2);
        assert_eq!(nodes[0].waypoint_size, WAYPOINT_SIZE);
        assert_eq!(nodes[0].parent_id, None);
        assert_eq!(nodes[0].jsonmodel_type, "archival_object");
        assert_eq!(nodes[0].suppressed, 1);
        assert!(nodes[0].has_digital_instance);
        assert_eq!(nodes[0].identifier.as_deref(), Some("SER-1"));
        assert_eq!(nodes[0].level.as_deref(), Some("series"));
        assert_eq!(
            nodes[0].dates,
            vec![serde_json::json!({
                "type": "inclusive",
                "label": "creation",
                "expression": "1900-1950",
                "begin": "1900",
                "end": "1950"
            })]
        );
    }

    #[tokio::test]
    async fn test_tree_waypoint_nested_parent_and_position_fallback() {
        let mut server = Server::new_async().await;

        let children_mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "resource_uri:\\/res\\/1 AND parent_uri:\\/repositories\\/2\\/archival_objects\\/77".into(),
                ),
                mockito::Matcher::UrlEncoded("sort".into(), "position_i asc, id asc".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "200".into()),
                mockito::Matcher::UrlEncoded("start".into(), "200".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "response": {
                        "numFound": 1,
                        "start": 200,
                        "docs": [
                            {
                                "id": "/ao/child",
                                "resource_uri": "/res/1",
                                "parent_uri": "/repositories/2/archival_objects/77"
                            }
                        ]
                    }
                }"#,
            )
            .create_async()
            .await;

        let counts_mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("q".into(), "resource_uri:\\/res\\/1".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "0".into()),
                mockito::Matcher::UrlEncoded("facet.field".into(), "parent_uri".into()),
                mockito::Matcher::UrlEncoded("facet.limit".into(), "-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"facet_counts":{"facet_fields":{"parent_uri":[]}}}"#)
            .create_async()
            .await;

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let params = ShadowTreeWaypointParams::new("/res/1", 1)
            .with_base_url(server.url())
            .with_collection("archivesspace_shadow")
            .with_parent_node("/repositories/2/archival_objects/77");

        let nodes = client.tree_waypoint(&params).await.unwrap();
        children_mock.assert_async().await;
        counts_mock.assert_async().await;

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].uri, "/ao/child");
        assert_eq!(nodes[0].title, "<untitled>");
        assert_eq!(nodes[0].position, 200);
        assert_eq!(nodes[0].child_count, 0);
        assert_eq!(nodes[0].waypoints, 0);
        assert_eq!(nodes[0].parent_id, Some(77));
        assert_eq!(nodes[0].suppressed, 0);
    }

    #[tokio::test]
    async fn test_keyword_search_mocked_success() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/archivesspace_shadow/select")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded(
                    "q".into(),
                    "(title:Example OR display_string:Example OR date_expressions:Example)".into(),
                ),
                mockito::Matcher::UrlEncoded("wt".into(), "json".into()),
                mockito::Matcher::UrlEncoded("rows".into(), "10".into()),
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

        let client = ShadowSolrClient::new(server.url(), "archivesspace_shadow");
        let params = ShadowKeywordSearchParams::new()
            .with_base_url(server.url())
            .with_collection("archivesspace_shadow")
            .with_query("Example")
            .with_rows(10);

        let result = client.keyword_search(&params).await.unwrap();
        mock.assert_async().await;
        assert_eq!(result.num_found, 1);
        assert_eq!(result.docs.len(), 1);
        assert_eq!(result.docs[0].id, "/repositories/2/archival_objects/4031");
    }
}
