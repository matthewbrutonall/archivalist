//! Isolated Solr STAFF Archival Object (AO) Writer
//!
//! # Purpose & Scope
//! This module provides bounded, isolated Solr writing infrastructure for eventual Rust
//! fetch -> prepare -> write pipelines indexing STAFF Archival Object records into disposable
//! benchmark stage cores.
//!
//! **Important Operational Notes:**
//! - This is infrastructure only; it does NOT claim speedup, parity, or production readiness.
//! - It must NEVER be wired to production indexing flags or live application traffic.
//! - Edits and writes are strictly restricted to designated disposable stage candidate cores.
//!
//! # Defense-in-Depth Safety Architecture & Isolation Notice
//! **CRITICAL SAFETY NOTICE:**
//! Checking for a loopback URL (`127.0.0.1`, `localhost`, `::1`) and candidate stage core naming
//! conventions (`stage_staff_<run_nonce>_candidate`) does **NOT** by itself prove that the Solr
//! instance is an isolated, disposable staging container. In development and profiling setups,
//! the primary ArchivesSpace Solr instance may also run on a loopback interface (e.g. port 8983).
//!
//! Therefore, this library functions strictly as **defense-in-depth**. The caller and benchmark
//! harness **MUST independently verify** that the target `solr_host_url` points to an isolated
//! candidate container by checking:
//! 1. Profiling stack state via `profiling_stack.py verify`
//! 2. Docker container identity and container labels (e.g. `com.archivesspace.profiling=true`)
//! 3. Candidate core ownership and lifecycle before constructing this writer.
//!
//! # Native Child Deletion Semantics & Hook Configuration
//! In ArchivesSpace:
//! - STAFF indexing instantiates `PeriodicIndexer`, which inherits from `IndexerCommon`.
//! - In `IndexerCommon` (`indexer_common.rb:1196`), `record_has_children` is registered ONLY
//!   for `'collection_management'`.
//! - `PUIIndexer` (`pui_indexer.rb:93`) registers `'archival_object'`, but `PUIIndexer` is NOT
//!   instantiated by the STAFF AO indexing baseline.
//! - `IndexBatch#record_info_for_type` checks the record's explicit `primary_type`, never inferring
//!   type from URIs or record IDs.
//! - For fresh unique stage candidate cores (`stage_staff_<run_nonce>_candidate`), stale-child deletion
//!   is typically unnecessary because the stage core starts empty.
//! - This writer makes **no automatic parity claim** and does **not** auto-delete parent archival
//!   objects or infer record types from IDs. If child record deletion is explicitly needed by a
//!   profiling harness, the caller must supply exact configured parent IDs via `queue_child_delete_for_parent`.
//!
//! # Strict Safety Guards
//! - Only constructs for explicit unique stage candidate cores matching the native stage naming pattern
//!   `stage_staff_<run_nonce>_candidate`.
//! - Explicitly refuses live/default core (`archivesspace`), test cores (`test_...` or containing `test`),
//!   shadow primary (`archivesspace_shadow`, `shadow_primary`), fallback/native/ablation roles, PUI cores,
//!   and malformed core names (path traversal, invalid characters).
//! - Base URL must be explicitly supplied and must point to a loopback host (`127.0.0.1`, `localhost`, `::1`).
//!   URLs with credentials, query strings, or fragments are strictly refused.
//! - Creates no cores, deletes no cores, and performs no database mutations.
//! - Issues HTTP POST requests only to the validated `<solr_base>/<stage_candidate_core>/update` endpoint.
//! - Buffers documents in deterministic order, independent of fetch batch sizes (e.g. 25 records),
//!   enforcing configured `max_doc_count` (<= 500) and `max_serialized_bytes` to guarantee bounded memory.
//! - Actual serialized request body size (accounting for array delimiters `[` `]` and commas `,`) is
//!   strictly kept within `max_serialized_bytes` in all cases, including UTF-8 multi-byte characters.
//! - Explicit delete queue is independently bounded by `max_delete_count` and `max_serialized_bytes`.
//! - Performs a hard commit once upon successful completion (`POST /update` with `{"commit":{"softCommit":false}}`).
//! - Never sends a commit after a failed write (fails closed upon any write error).
//! - Parses both HTTP status and Solr JSON response status (`responseHeader.status`), requiring parseable
//!   JSON with `responseHeader.status == 0` for success.
//! - Never includes arbitrary Solr response body or error messages in returned errors or logs, avoiding
//!   any leak of private record content.

use crate::error::{ASpaceError, Result};
use reqwest::Url;
use serde_json::{json, Value};

/// Default maximum number of documents buffered before automatic flush.
pub const DEFAULT_MAX_DOC_COUNT: usize = 500;
/// Hard upper ceiling for `max_doc_count` to prevent unbounded memory growth.
pub const ABSOLUTE_MAX_DOC_COUNT: usize = 500;
/// Default maximum serialized bytes buffered before automatic flush (10 MB).
pub const DEFAULT_MAX_SERIALIZED_BYTES: usize = 10 * 1024 * 1024;
/// Hard upper ceiling for `max_serialized_bytes` (25 MB).
pub const ABSOLUTE_MAX_SERIALIZED_BYTES: usize = 25 * 1024 * 1024;
/// Minimum allowed `max_serialized_bytes` (1 KB).
pub const MIN_SERIALIZED_BYTES: usize = 1024;
/// Default maximum explicit parent delete IDs queued before flush (500).
pub const DEFAULT_MAX_DELETE_COUNT: usize = 500;
/// Hard upper ceiling for explicit delete queue count (500).
pub const ABSOLUTE_MAX_DELETE_COUNT: usize = 500;
/// Default Solr field name for parent ID child deletions.
pub const DEFAULT_PARENT_ID_FIELD: &str = "parent_id";

/// Stage core name prefix for Staff AO indexing.
const STAGE_STAFF_PREFIX: &str = "stage_staff_";
/// Stage core name suffix for Candidate stage cores.
const STAGE_CANDIDATE_SUFFIX: &str = "_candidate";

/// Forbidden core keywords matching `ArchivalistHarnessSafety::FORBIDDEN_KEYWORDS` and safety policies.
const FORBIDDEN_CORE_SUBSTRINGS: &[&str] = &[
    "archivesspace",
    "production",
    "primary",
    "live",
    "test",
    "shadow",
    "native",
    "fallback",
    "ablation",
    "pui",
];

/// Configuration for `IsolatedSolrStaffAoWriter`.
#[derive(Debug, Clone)]
pub struct IsolatedSolrWriterConfig {
    /// Maximum number of documents to buffer before automatic flush (1..=500).
    pub max_doc_count: usize,
    /// Maximum serialized request body bytes to buffer before automatic flush (1024..=26214400).
    pub max_serialized_bytes: usize,
    /// The Solr field holding parent IDs for child deletion (defaults to "parent_id").
    pub parent_id_field: String,
    /// Maximum number of explicit parent IDs queued for child deletion (1..=500).
    pub max_delete_count: usize,
}

impl Default for IsolatedSolrWriterConfig {
    fn default() -> Self {
        Self {
            max_doc_count: DEFAULT_MAX_DOC_COUNT,
            max_serialized_bytes: DEFAULT_MAX_SERIALIZED_BYTES,
            parent_id_field: DEFAULT_PARENT_ID_FIELD.to_string(),
            max_delete_count: DEFAULT_MAX_DELETE_COUNT,
        }
    }
}

impl IsolatedSolrWriterConfig {
    /// Validates the configuration values against safety and memory bounds.
    pub fn validate(&self) -> Result<()> {
        if self.max_doc_count == 0 || self.max_doc_count > ABSOLUTE_MAX_DOC_COUNT {
            return Err(ASpaceError::InvalidRequest(format!(
                "max_doc_count must be between 1 and {}, got {}",
                ABSOLUTE_MAX_DOC_COUNT, self.max_doc_count
            )));
        }
        if self.max_serialized_bytes < MIN_SERIALIZED_BYTES
            || self.max_serialized_bytes > ABSOLUTE_MAX_SERIALIZED_BYTES
        {
            return Err(ASpaceError::InvalidRequest(format!(
                "max_serialized_bytes must be between {} and {}, got {}",
                MIN_SERIALIZED_BYTES, ABSOLUTE_MAX_SERIALIZED_BYTES, self.max_serialized_bytes
            )));
        }
        if self.max_delete_count == 0 || self.max_delete_count > ABSOLUTE_MAX_DELETE_COUNT {
            return Err(ASpaceError::InvalidRequest(format!(
                "max_delete_count must be between 1 and {}, got {}",
                ABSOLUTE_MAX_DELETE_COUNT, self.max_delete_count
            )));
        }
        if self.parent_id_field.trim().is_empty()
            || !self
                .parent_id_field
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(ASpaceError::InvalidRequest(format!(
                "parent_id_field must be a valid non-empty alphanumeric identifier, got '{}'",
                self.parent_id_field
            )));
        }
        Ok(())
    }
}

/// Escapes characters inside a quoted Solr literal query term (`"..."`).
/// In Solr/Lucene, inside a double-quoted phrase, only `"` and `\` require escaping.
pub fn escape_solr_quoted_literal(val: &str) -> String {
    let mut escaped = String::with_capacity(val.len() + 4);
    for c in val.chars() {
        match c {
            '\"' | '\\' => {
                escaped.push('\\');
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Validates a stage candidate core name against native ArchivesSpace harness conventions.
///
/// Native naming convention: `stage_staff_<run_nonce>_candidate`.
/// Rejects forbidden keywords (e.g. `archivesspace`, `production`, `test`, `shadow`, `live`),
/// invalid prefixes/suffixes, path traversal sequences, and non-alphanumeric nonce characters.
pub fn validate_stage_candidate_core(
    core_name: &str,
    expected_run_nonce: Option<&str>,
) -> Result<String> {
    let trimmed = core_name.trim();
    if trimmed.is_empty() {
        return Err(ASpaceError::InvalidRequest(
            "Stage core name cannot be empty".to_string(),
        ));
    }

    let lower = trimmed.to_ascii_lowercase();
    for kw in FORBIDDEN_CORE_SUBSTRINGS {
        if lower.contains(kw) {
            return Err(ASpaceError::InvalidRequest(format!(
                "Refusing stage core name '{}': contains forbidden keyword '{}'",
                trimmed, kw
            )));
        }
    }

    if !trimmed.starts_with(STAGE_STAFF_PREFIX) {
        return Err(ASpaceError::InvalidRequest(format!(
            "Refusing core name '{}': must begin with '{}' for staff AO stage writer",
            trimmed, STAGE_STAFF_PREFIX
        )));
    }

    if !trimmed.ends_with(STAGE_CANDIDATE_SUFFIX) {
        return Err(ASpaceError::InvalidRequest(format!(
            "Refusing core name '{}': must end with '{}' for stage candidate writer",
            trimmed, STAGE_CANDIDATE_SUFFIX
        )));
    }

    let min_len = STAGE_STAFF_PREFIX.len() + STAGE_CANDIDATE_SUFFIX.len();
    if trimmed.len() <= min_len {
        return Err(ASpaceError::InvalidRequest(format!(
            "Refusing core name '{}': missing run nonce",
            trimmed
        )));
    }

    let nonce = &trimmed[STAGE_STAFF_PREFIX.len()..trimmed.len() - STAGE_CANDIDATE_SUFFIX.len()];
    if nonce.is_empty()
        || !nonce.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || nonce.starts_with('_')
        || nonce.ends_with('_')
    {
        return Err(ASpaceError::InvalidRequest(format!(
            "Refusing core name '{}': run nonce '{}' must consist of ASCII alphanumeric characters and underscores, and cannot start or end with an underscore",
            trimmed, nonce
        )));
    }

    if let Some(expected) = expected_run_nonce {
        if nonce != expected {
            return Err(ASpaceError::InvalidRequest(format!(
                "Stage core run nonce '{}' does not match expected run nonce '{}'",
                nonce, expected
            )));
        }
    }

    Ok(nonce.to_string())
}

/// Validates that the provided Solr base URL is an isolated loopback endpoint.
///
/// Refuses:
/// - Non-loopback hostnames (e.g. `solr`, `example.com`, public or LAN IPs).
/// - Credentials (username / password).
/// - Query strings or fragments.
/// - Path traversal (`..`).
pub fn validate_isolated_solr_base_url(base_url: &str) -> Result<Url> {
    let parsed = Url::parse(base_url)
        .map_err(|e| ASpaceError::InvalidRequest(format!("Invalid Solr base URL: {}", e)))?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ASpaceError::InvalidRequest(format!(
            "Solr base URL scheme must be http or https, got '{}'",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ASpaceError::InvalidRequest("Solr base URL missing host".to_string()))?;

    let is_loopback = match host {
        "127.0.0.1" | "localhost" | "::1" | "[::1]" => true,
        ip if ip.starts_with("127.") => ip
            .parse::<std::net::Ipv4Addr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false),
        _ => false,
    };

    if !is_loopback {
        return Err(ASpaceError::InvalidRequest(format!(
            "Refusing non-loopback Solr base URL host '{}'; isolated writer requires loopback",
            host
        )));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ASpaceError::InvalidRequest(
            "Refusing Solr base URL containing user credentials".to_string(),
        ));
    }

    if parsed.query().is_some() {
        return Err(ASpaceError::InvalidRequest(
            "Refusing Solr base URL containing query parameters".to_string(),
        ));
    }

    if parsed.fragment().is_some() {
        return Err(ASpaceError::InvalidRequest(
            "Refusing Solr base URL containing fragment".to_string(),
        ));
    }

    if base_url.contains("..") || parsed.path().contains("..") {
        return Err(ASpaceError::InvalidRequest(
            "Refusing Solr base URL with path traversal".to_string(),
        ));
    }

    Ok(parsed)
}

/// Validates an ArchivesSpace parent URI before generating Solr child delete queries.
///
/// Prevents Solr/Lucene query injection by strictly checking structure:
/// - Must match `/repositories/<digits>/<record_type>/<digits>`.
/// - Strictly disallows spaces, quotes, colons, parentheses, brackets, carets, wildcards, and control characters.
pub fn validate_parent_uri(uri: &str) -> Result<()> {
    if uri.trim().is_empty() {
        return Err(ASpaceError::InvalidRequest(
            "Parent URI cannot be empty".to_string(),
        ));
    }

    if !uri.starts_with('/') {
        return Err(ASpaceError::InvalidRequest(
            "Parent URI must start with '/'".to_string(),
        ));
    }

    // Reject any character that could cause query injection or unexpected Lucene behavior
    for c in uri.chars() {
        match c {
            '/' | '_' | '-' => {}
            c if c.is_ascii_alphanumeric() => {}
            _ => {
                return Err(ASpaceError::InvalidRequest(
                    "Parent URI contains invalid characters; rejected before query generation"
                        .to_string(),
                ));
            }
        }
    }

    // Segments: ["", "repositories", "<repo_id>", "<type>", "<id>"]
    let parts: Vec<&str> = uri.split('/').collect();
    if parts.len() != 5
        || !parts[0].is_empty()
        || parts[1] != "repositories"
        || parts[2].is_empty()
        || !parts[2].chars().all(|c| c.is_ascii_digit())
        || parts[3].is_empty()
        || !parts[3].chars().all(|c| c.is_ascii_lowercase() || c == '_')
        || parts[4].is_empty()
        || !parts[4].chars().all(|c| c.is_ascii_digit())
    {
        return Err(ASpaceError::InvalidRequest(
            "Parent URI does not match canonical ArchivesSpace pattern '/repositories/<digits>/<type>/<digits>'; rejected before query generation".to_string(),
        ));
    }

    Ok(())
}

/// Calculates the exact serialized JSON array request body length for a given count of documents
/// and the sum of their individual serialized byte lengths.
///
/// Solr JSON array syntax: `[` + doc1 + `,` + doc2 + ... + `]`.
/// - 0 docs: `[]` = 2 bytes.
/// - 1 doc: `[` + doc + `]` = doc_len + 2 bytes.
/// - n docs: `[` + doc1 + `,` + ... + `]` = sum_doc_bytes + n + 1 bytes.
pub fn serialized_json_array_body_len(doc_count: usize, sum_doc_bytes: usize) -> usize {
    if doc_count == 0 {
        2
    } else {
        sum_doc_bytes + doc_count + 1
    }
}

/// Isolated Solr STAFF Archival Object (AO) Writer.
///
/// Bounded memory writer with automatic threshold flushing, explicit parent child deletion,
/// and safe hard commit upon completion.
pub struct IsolatedSolrStaffAoWriter {
    base_url: String,
    target_core: String,
    update_url: Url,
    config: IsolatedSolrWriterConfig,
    http: reqwest::Client,
    doc_buffer: Vec<Value>,
    parent_ids_to_delete: Vec<String>,
    buffered_doc_bytes: usize,
    has_error: bool,
    committed: bool,
}

impl IsolatedSolrStaffAoWriter {
    /// Constructs a new writer for the specified Solr base URL and candidate stage core name.
    ///
    /// Requires caller/harness verification of stack state and Docker isolation before invocation.
    pub fn new(base_url: &str, core_name: &str) -> Result<Self> {
        Self::with_config(base_url, core_name, IsolatedSolrWriterConfig::default())
    }

    /// Constructs a new writer with custom buffering configuration.
    pub fn with_config(
        base_url: &str,
        core_name: &str,
        config: IsolatedSolrWriterConfig,
    ) -> Result<Self> {
        config.validate()?;
        validate_stage_candidate_core(core_name, None)?;
        let parsed_base = validate_isolated_solr_base_url(base_url)?;

        let mut update_url = parsed_base.clone();
        let base_path = parsed_base.path().trim_end_matches('/');
        let new_path = format!("{}/{}/update", base_path, core_name);
        update_url.set_path(&new_path);

        // Verify no path traversal and exact update path structure
        if !update_url
            .path()
            .ends_with(&format!("/{}/update", core_name))
        {
            return Err(ASpaceError::InvalidRequest(
                "Failed to construct canonical Solr update endpoint URL without path modification"
                    .to_string(),
            ));
        }

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            target_core: core_name.to_string(),
            update_url,
            config,
            http: reqwest::Client::new(),
            doc_buffer: Vec::new(),
            parent_ids_to_delete: Vec::new(),
            buffered_doc_bytes: 0,
            has_error: false,
            committed: false,
        })
    }

    /// Constructs a writer validating an expected run nonce.
    pub fn with_expected_run_nonce(
        base_url: &str,
        core_name: &str,
        expected_nonce: &str,
    ) -> Result<Self> {
        Self::with_expected_run_nonce_and_config(
            base_url,
            core_name,
            expected_nonce,
            IsolatedSolrWriterConfig::default(),
        )
    }

    /// Constructs a writer validating an expected run nonce and custom configuration.
    pub fn with_expected_run_nonce_and_config(
        base_url: &str,
        core_name: &str,
        expected_nonce: &str,
        config: IsolatedSolrWriterConfig,
    ) -> Result<Self> {
        validate_stage_candidate_core(core_name, Some(expected_nonce))?;
        Self::with_config(base_url, core_name, config)
    }

    /// The designated stage core name.
    pub fn target_core(&self) -> &str {
        &self.target_core
    }

    /// The validated Solr base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The full canonical update URL (`<base_url>/<target_core>/update`).
    pub fn update_url(&self) -> &Url {
        &self.update_url
    }

    /// Current number of documents buffered in memory awaiting flush.
    pub fn buffered_doc_count(&self) -> usize {
        self.doc_buffer.len()
    }

    /// Current number of parent IDs queued for child deletion awaiting flush.
    pub fn pending_delete_count(&self) -> usize {
        self.parent_ids_to_delete.len()
    }

    /// Current serialized byte size of the document array request body in the buffer.
    /// Returns 0 when no documents are buffered.
    pub fn buffered_bytes(&self) -> usize {
        if self.doc_buffer.is_empty() {
            0
        } else {
            serialized_json_array_body_len(self.doc_buffer.len(), self.buffered_doc_bytes)
        }
    }

    /// Returns `true` if the writer encountered an unrecoverable write error.
    pub fn has_error(&self) -> bool {
        self.has_error
    }

    /// Returns `true` if the writer has successfully completed and sent a hard commit.
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// The active writer configuration.
    pub fn config(&self) -> &IsolatedSolrWriterConfig {
        &self.config
    }

    /// Adds a single document to the ordered buffer.
    ///
    /// # Safety and Validation Semantics
    /// - Performs all format, field, and size validation **before** any buffer or queue mutation.
    /// - If the document causes the buffer to reach `max_doc_count` or `max_serialized_bytes`,
    ///   flushing occurs.
    /// - Does **NOT** auto-delete parent records or infer record types from URIs or IDs.
    ///   (Real STAFF indexing baseline uses `PeriodicIndexer`, which only hooks `collection_management`
    ///   at `indexer_common.rb:1196`; `archival_object` is registered by `PUIIndexer` at `pui_indexer.rb:93`,
    ///   which is not instantiated for STAFF AO indexing).
    pub async fn add_document(&mut self, doc: Value) -> Result<()> {
        if self.has_error {
            return Err(ASpaceError::InvalidRequest(
                "Cannot add document: writer is in error state due to previous failed write"
                    .to_string(),
            ));
        }
        if self.committed {
            return Err(ASpaceError::InvalidRequest(
                "Cannot add document: writer has already finished and committed".to_string(),
            ));
        }

        // Step 1: Validate document structure
        let obj = doc.as_object().ok_or_else(|| {
            ASpaceError::InvalidRequest("Solr document must be a JSON object".to_string())
        })?;

        let _id = obj.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
            ASpaceError::InvalidRequest("Solr document missing string 'id' field".to_string())
        })?;

        // Step 2: Serialize to measure exact UTF-8 byte length
        let serialized = serde_json::to_vec(&doc)?;
        let doc_len = serialized.len();

        // Enforce that single doc request body `[<doc>]` fits within max_serialized_bytes cap
        let single_doc_request_len = doc_len + 2;
        if single_doc_request_len > self.config.max_serialized_bytes {
            return Err(ASpaceError::InvalidRequest(format!(
                "Single document serialized request body size ({} bytes) exceeds maximum allowed buffer request size ({} bytes)",
                single_doc_request_len, self.config.max_serialized_bytes
            )));
        }

        // Step 3: Check if adding this document would exceed bounds for the current buffer.
        // If so, PREFLUSH prior buffer before mutating state or enqueuing this document.
        if !self.doc_buffer.is_empty() {
            let next_doc_count = self.doc_buffer.len() + 1;
            let next_body_len =
                serialized_json_array_body_len(next_doc_count, self.buffered_doc_bytes + doc_len);
            if next_doc_count > self.config.max_doc_count
                || next_body_len > self.config.max_serialized_bytes
            {
                self.flush().await?;
            }
        }

        // Step 4: Mutate state by pushing validated document into buffer
        self.doc_buffer.push(doc);
        self.buffered_doc_bytes += doc_len;

        // Step 5: Flush if buffer has now reached thresholds
        let current_body_len =
            serialized_json_array_body_len(self.doc_buffer.len(), self.buffered_doc_bytes);
        if self.doc_buffer.len() >= self.config.max_doc_count
            || current_body_len >= self.config.max_serialized_bytes
        {
            self.flush().await?;
        }

        Ok(())
    }

    /// Adds multiple documents in sequence to the ordered buffer.
    pub async fn add_documents(&mut self, docs: Vec<Value>) -> Result<()> {
        for doc in docs {
            self.add_document(doc).await?;
        }
        Ok(())
    }

    /// Formats the Solr delete query JSON payload for the current delete queue plus an optional parent URI.
    fn format_delete_payload(&self, additional_parent: Option<&str>) -> Value {
        let mut parents = self.parent_ids_to_delete.clone();
        if let Some(p) = additional_parent {
            if !parents.iter().any(|existing| existing == p) {
                parents.push(p.to_string());
            }
        }

        let query = if parents.len() == 1 {
            format!(
                "{}:(\"{}\")",
                self.config.parent_id_field,
                escape_solr_quoted_literal(&parents[0])
            )
        } else {
            let escaped_ids: Vec<String> = parents
                .iter()
                .map(|id| format!("\"{}\"", escape_solr_quoted_literal(id)))
                .collect();
            format!(
                "{}:({})",
                self.config.parent_id_field,
                escaped_ids.join(" OR ")
            )
        };

        json!({
            "delete": {
                "query": query
            }
        })
    }

    /// Explicitly queues a parent URI for child deletion in the next flushed logical batch.
    ///
    /// # Semantics & Bounds
    /// - The caller must supply exact configured parent IDs only. No automatic parity is claimed.
    /// - Enforces validation of parent URI before mutating queue.
    /// - Explicit delete queue is bounded independently by `max_delete_count` and `max_serialized_bytes`.
    pub fn queue_child_delete_for_parent(&mut self, parent_uri: &str) -> Result<()> {
        if self.has_error {
            return Err(ASpaceError::InvalidRequest(
                "Cannot queue child delete: writer is in error state".to_string(),
            ));
        }
        if self.committed {
            return Err(ASpaceError::InvalidRequest(
                "Cannot queue child delete: writer has already committed".to_string(),
            ));
        }

        // Validate parent URI before mutation
        validate_parent_uri(parent_uri)?;

        // Deduplicate: already queued is a no-op
        if self.parent_ids_to_delete.iter().any(|p| p == parent_uri) {
            return Ok(());
        }

        // Check independent count bound
        if self.parent_ids_to_delete.len() >= self.config.max_delete_count {
            return Err(ASpaceError::InvalidRequest(format!(
                "Cannot queue child delete: explicit delete queue capacity limit reached ({})",
                self.config.max_delete_count
            )));
        }

        // Check request byte bound
        let test_payload = self.format_delete_payload(Some(parent_uri));
        let serialized_len = serde_json::to_vec(&test_payload)?.len();
        if serialized_len > self.config.max_serialized_bytes {
            return Err(ASpaceError::InvalidRequest(format!(
                "Cannot queue child delete: serialized delete request body ({} bytes) exceeds max_serialized_bytes limit ({} bytes)",
                serialized_len, self.config.max_serialized_bytes
            )));
        }

        // Mutate state only after all validations pass
        self.parent_ids_to_delete.push(parent_uri.to_string());
        Ok(())
    }

    /// Flushes all pending child deletions and document additions to the target stage core.
    ///
    /// Preserves native execution order: child delete query is sent BEFORE document additions.
    pub async fn flush(&mut self) -> Result<()> {
        if self.has_error {
            return Err(ASpaceError::InvalidRequest(
                "Cannot flush: writer is in error state due to previous failed write".to_string(),
            ));
        }

        if self.doc_buffer.is_empty() && self.parent_ids_to_delete.is_empty() {
            return Ok(());
        }

        // Step 1: Send child deletions if any parent IDs are pending
        if !self.parent_ids_to_delete.is_empty() {
            let delete_payload = self.format_delete_payload(None);
            let delete_bytes = serde_json::to_vec(&delete_payload)?;

            if delete_bytes.len() > self.config.max_serialized_bytes {
                self.has_error = true;
                return Err(ASpaceError::InvalidRequest(format!(
                    "Explicit child delete payload size ({} bytes) exceeds max_serialized_bytes limit ({} bytes)",
                    delete_bytes.len(), self.config.max_serialized_bytes
                )));
            }

            if let Err(err) = self.post_update(&delete_bytes).await {
                self.has_error = true;
                return Err(err);
            }

            self.parent_ids_to_delete.clear();
        }

        // Step 2: Send document additions array
        if !self.doc_buffer.is_empty() {
            let docs: Vec<Value> = std::mem::take(&mut self.doc_buffer);
            self.buffered_doc_bytes = 0;
            let payload_bytes = serde_json::to_vec(&docs)?;

            if payload_bytes.len() > self.config.max_serialized_bytes {
                self.has_error = true;
                return Err(ASpaceError::InvalidRequest(format!(
                    "Serialized document batch request size ({} bytes) exceeds max_serialized_bytes limit ({} bytes)",
                    payload_bytes.len(), self.config.max_serialized_bytes
                )));
            }

            if let Err(err) = self.post_update(&payload_bytes).await {
                self.has_error = true;
                return Err(err);
            }
        }

        Ok(())
    }

    /// Flushes remaining documents and executes a single hard commit.
    ///
    /// Never commits if a previous write failed.
    pub async fn commit(&mut self) -> Result<()> {
        if self.has_error {
            return Err(ASpaceError::InvalidRequest(
                "Cannot commit: writer is in error state due to previous failed write".to_string(),
            ));
        }

        if self.committed {
            return Ok(());
        }

        // Flush remaining buffered documents and queued deletes
        self.flush().await?;

        // Hard commit matching native send_commit(:hard): {"commit": {"softCommit": false}}
        let commit_payload = json!({
            "commit": {
                "softCommit": false
            }
        });
        let commit_bytes = serde_json::to_vec(&commit_payload)?;

        if let Err(err) = self.post_update(&commit_bytes).await {
            self.has_error = true;
            return Err(err);
        }

        self.committed = true;
        Ok(())
    }

    /// Completes writing by flushing buffered records and committing.
    pub async fn finish(&mut self) -> Result<()> {
        self.commit().await
    }

    /// Verifies that the designated stage candidate core exists in Solr and contains zero documents.
    ///
    /// Refuses to write if:
    /// - The Solr core is absent from Solr admin STATUS.
    /// - The Solr core already contains documents (`numFound > 0`).
    /// - The Solr admin or select endpoint fails or returns a non-zero status.
    pub async fn verify_target_core_ready(&self) -> Result<()> {
        let status_url = format!(
            "{}/admin/cores?action=STATUS&core={}&wt=json",
            self.base_url.trim_end_matches('/'),
            self.target_core
        );
        let resp = match self
            .http
            .get(&status_url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Err(ASpaceError::Solr {
                    status: 0,
                    message: format!("HTTP transport failure querying Solr core STATUS: {}", e),
                });
            }
        };

        let http_status = resp.status();
        if !http_status.is_success() {
            return Err(ASpaceError::Solr {
                status: http_status.as_u16(),
                message: format!(
                    "Solr core STATUS request failed with HTTP status {}",
                    http_status.as_u16()
                ),
            });
        }

        let body_text = resp.text().await.unwrap_or_default();
        let json_body: Value = match serde_json::from_str(&body_text) {
            Ok(v) => v,
            Err(_) => {
                return Err(ASpaceError::Solr {
                    status: http_status.as_u16(),
                    message: "Solr core STATUS response is not valid JSON".to_string(),
                });
            }
        };

        if json_body
            .get("responseHeader")
            .and_then(|h| h.get("status"))
            .and_then(Value::as_i64)
            != Some(0)
        {
            return Err(ASpaceError::Solr {
                status: http_status.as_u16(),
                message: "Solr core STATUS returned non-zero responseHeader status".to_string(),
            });
        }

        let core_info = json_body
            .get("status")
            .and_then(|s| s.get(&self.target_core));

        match core_info {
            Some(Value::Object(map)) if !map.is_empty() => {}
            _ => {
                return Err(ASpaceError::InvalidRequest(format!(
                    "Target candidate core '{}' does not exist in Solr. Writer refuses to write to absent core.",
                    self.target_core
                )));
            }
        }

        let select_url = format!(
            "{}/{}/select?q=*:*&rows=0&wt=json",
            self.base_url.trim_end_matches('/'),
            self.target_core
        );
        let sel_resp = match self
            .http
            .get(&select_url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Err(ASpaceError::Solr {
                    status: 0,
                    message: format!("HTTP transport failure querying Solr core select: {}", e),
                });
            }
        };

        let sel_status = sel_resp.status();
        if !sel_status.is_success() {
            return Err(ASpaceError::Solr {
                status: sel_status.as_u16(),
                message: format!(
                    "Solr select request on '{}' failed with HTTP status {}",
                    self.target_core,
                    sel_status.as_u16()
                ),
            });
        }

        let sel_body = sel_resp.text().await.unwrap_or_default();
        let sel_json: Value = match serde_json::from_str(&sel_body) {
            Ok(v) => v,
            Err(_) => {
                return Err(ASpaceError::Solr {
                    status: sel_status.as_u16(),
                    message: "Solr select response is not valid JSON".to_string(),
                });
            }
        };

        let num_found = sel_json
            .get("response")
            .and_then(|r| r.get("numFound"))
            .and_then(Value::as_u64)
            .ok_or_else(|| ASpaceError::Solr {
                status: sel_status.as_u16(),
                message: "Missing or invalid numFound in Solr select response".to_string(),
            })?;

        if num_found > 0 {
            return Err(ASpaceError::InvalidRequest(format!(
                "Target candidate core '{}' is not empty (found {} existing documents). Writer refuses to write to non-empty core.",
                self.target_core, num_found
            )));
        }

        Ok(())
    }

    /// Posts an update payload to the target stage core update endpoint.
    ///
    /// Fails closed unless the HTTP response is 2xx AND the response body is parseable JSON
    /// with `responseHeader.status == 0`.
    ///
    /// To ensure no private record content is ever logged or leaked in error messages,
    /// returns a fixed generic message containing numeric HTTP and Solr status codes only.
    async fn post_update(&self, payload_bytes: &[u8]) -> Result<()> {
        let response = match self
            .http
            .post(self.update_url.clone())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(payload_bytes.to_vec())
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                return Err(ASpaceError::Solr {
                    status: 0,
                    message: format!("HTTP transport failure to Solr update: {}", err),
                });
            }
        };

        let http_status = response.status();
        if !http_status.is_success() {
            return Err(ASpaceError::Solr {
                status: http_status.as_u16(),
                message: format!(
                    "Solr update request failed with HTTP status {}",
                    http_status.as_u16()
                ),
            });
        }

        let body_text = response.text().await.unwrap_or_default();
        let json_body: Value = match serde_json::from_str(&body_text) {
            Ok(v) => v,
            Err(_) => {
                return Err(ASpaceError::Solr {
                    status: http_status.as_u16(),
                    message: format!(
                        "Solr update response malformed (HTTP status {}): response is not valid JSON",
                        http_status.as_u16()
                    ),
                });
            }
        };

        let response_header = match json_body.get("responseHeader") {
            Some(h) if h.is_object() => h,
            _ => {
                return Err(ASpaceError::Solr {
                    status: http_status.as_u16(),
                    message: format!(
                        "Solr update response malformed (HTTP status {}): missing responseHeader object",
                        http_status.as_u16()
                    ),
                });
            }
        };

        let solr_status = match response_header.get("status") {
            Some(s) if s.is_i64() => s.as_i64().unwrap(),
            _ => {
                return Err(ASpaceError::Solr {
                    status: http_status.as_u16(),
                    message: format!(
                        "Solr update response malformed (HTTP status {}): responseHeader.status missing or not integer",
                        http_status.as_u16()
                    ),
                });
            }
        };

        if solr_status != 0 {
            let status_code = if (1..=u16::MAX as i64).contains(&solr_status) {
                solr_status as u16
            } else {
                http_status.as_u16()
            };
            return Err(ASpaceError::Solr {
                status: status_code,
                message: format!(
                    "Solr update request failed with Solr responseHeader status {}",
                    solr_status
                ),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    const VALID_BASE_URL: &str = "http://127.0.0.1:8983/solr";
    const VALID_STAGE_CORE: &str = "stage_staff_20261026_074400_12345_abcd1234_candidate";

    #[test]
    fn test_target_guards_accepts_valid_candidate_core() {
        let nonce = validate_stage_candidate_core(VALID_STAGE_CORE, None).unwrap();
        assert_eq!(nonce, "20261026_074400_12345_abcd1234");

        let writer = IsolatedSolrStaffAoWriter::new(VALID_BASE_URL, VALID_STAGE_CORE).unwrap();
        assert_eq!(writer.target_core(), VALID_STAGE_CORE);
        assert_eq!(writer.base_url(), "http://127.0.0.1:8983/solr");
        assert_eq!(
            writer.update_url().as_str(),
            "http://127.0.0.1:8983/solr/stage_staff_20261026_074400_12345_abcd1234_candidate/update"
        );
    }

    #[test]
    fn test_target_guards_refuses_forbidden_cores() {
        let forbidden = [
            "archivesspace",
            "stage_staff_archivesspace_candidate",
            "stage_staff_shadow_candidate",
            "stage_staff_primary_candidate",
            "stage_staff_live_candidate",
            "stage_staff_production_candidate",
            "stage_staff_test_run_candidate",
            "test_core",
            "archivesspace_shadow",
            "shadow_primary",
        ];
        for core in &forbidden {
            assert!(
                IsolatedSolrStaffAoWriter::new(VALID_BASE_URL, core).is_err(),
                "Expected core '{}' to be refused",
                core
            );
        }
    }

    #[test]
    fn test_target_guards_refuses_non_candidate_roles() {
        let non_candidates = [
            "stage_staff_20261026_074400_native",
            "stage_staff_20261026_074400_fallback",
            "stage_staff_20261026_074400_ablation",
        ];
        for core in &non_candidates {
            assert!(
                IsolatedSolrStaffAoWriter::new(VALID_BASE_URL, core).is_err(),
                "Expected core '{}' to be refused",
                core
            );
        }
    }

    #[test]
    fn test_target_guards_refuses_non_staff_mode() {
        let non_staff = [
            "stage_pui_20261026_074400_candidate",
            "stage_candidate_20261026_candidate",
        ];
        for core in &non_staff {
            assert!(
                IsolatedSolrStaffAoWriter::new(VALID_BASE_URL, core).is_err(),
                "Expected non-staff core '{}' to be refused",
                core
            );
        }
    }

    #[test]
    fn test_target_guards_refuses_malformed_core_names() {
        let malformed = [
            "stage_staff__candidate",
            "stage_staff_../etc/passwd_candidate",
            "stage_staff_candidate",
            "stage_staff_foo/bar_candidate",
            "stage_staff_foo\\bar_candidate",
            "stage_staff_foo?bar_candidate",
            "stage_staff_foo#bar_candidate",
            "stage_staff_foo bar_candidate",
            "stage_staff_foo;drop_candidate",
        ];
        for core in &malformed {
            assert!(
                IsolatedSolrStaffAoWriter::new(VALID_BASE_URL, core).is_err(),
                "Expected malformed core '{}' to be refused",
                core
            );
        }
    }

    #[test]
    fn test_target_guards_expected_nonce_validation() {
        let core = "stage_staff_run_nonce_1234_candidate";
        // Matching nonce succeeds
        assert!(IsolatedSolrStaffAoWriter::with_expected_run_nonce(
            VALID_BASE_URL,
            core,
            "run_nonce_1234"
        )
        .is_ok());

        // Mismatched nonce fails
        assert!(IsolatedSolrStaffAoWriter::with_expected_run_nonce(
            VALID_BASE_URL,
            core,
            "other_nonce_5678"
        )
        .is_err());
    }

    #[test]
    fn test_target_guards_refuses_non_loopback_urls() {
        let non_loopback = [
            "http://solr:8983/solr",
            "http://example.com/solr",
            "http://192.168.1.100:8983/solr",
            "http://10.0.0.1:8983/solr",
            "http://0.0.0.0:8983/solr",
            "ftp://127.0.0.1:8983/solr",
        ];
        for url in &non_loopback {
            assert!(
                IsolatedSolrStaffAoWriter::new(url, VALID_STAGE_CORE).is_err(),
                "Expected non-loopback url '{}' to be refused",
                url
            );
        }
    }

    #[test]
    fn test_target_guards_refuses_credentials_query_fragment_traversal() {
        let invalid_urls = [
            "http://admin:secret@127.0.0.1:8983/solr",
            "http://127.0.0.1:8983/solr?core=foo",
            "http://127.0.0.1:8983/solr#section",
            "http://127.0.0.1:8983/solr/../other",
        ];
        for url in &invalid_urls {
            assert!(
                IsolatedSolrStaffAoWriter::new(url, VALID_STAGE_CORE).is_err(),
                "Expected invalid url '{}' to be refused",
                url
            );
        }
    }

    #[test]
    fn test_url_guard_defense_in_depth_arbitrary_loopback_ports() {
        // Accepts arbitrary loopback ports without hardcoding 18983, supporting dynamic mock servers
        let urls = [
            "http://127.0.0.1:8983/solr",
            "http://127.0.0.1:18983/solr",
            "http://localhost:54321/solr",
            "http://[::1]:8983/solr",
        ];
        for url in &urls {
            assert!(
                validate_isolated_solr_base_url(url).is_ok(),
                "Expected valid loopback url '{}' to pass",
                url
            );
        }
    }

    #[tokio::test]
    async fn test_ao_document_does_not_trigger_auto_deletion() {
        let mut server = Server::new_async().await;
        let base_url = format!("{}/solr", server.url());
        let core = "stage_staff_run_20261026_noautodel_candidate";
        let update_path = format!("/solr/{}/update", core);

        // Document addition mock: expect 1 call
        let add_body = json!([
            {
                "id": "/repositories/2/archival_objects/42",
                "primary_type": "archival_object",
                "title": "Item 42"
            }
        ]);
        let add_mock = server
            .mock("POST", update_path.as_str())
            .match_header("content-type", "application/json")
            .match_body(Matcher::Json(add_body))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader":{"status":0,"QTime":2}}"#)
            .expect(1)
            .create_async()
            .await;

        let commit_mock = server
            .mock("POST", update_path.as_str())
            .match_header("content-type", "application/json")
            .match_body(Matcher::Json(json!({"commit":{"softCommit":false}})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader":{"status":0,"QTime":1}}"#)
            .expect(1)
            .create_async()
            .await;

        // Delete mock: MUST NOT be called!
        let delete_mock = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Regex("delete".to_string()))
            .expect(0)
            .create_async()
            .await;

        let mut writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();

        // Adding an AO document does NOT queue or trigger deletion
        writer
            .add_document(json!({
                "id": "/repositories/2/archival_objects/42",
                "primary_type": "archival_object",
                "title": "Item 42"
            }))
            .await
            .unwrap();

        assert_eq!(writer.pending_delete_count(), 0);
        writer.finish().await.unwrap();

        add_mock.assert_async().await;
        commit_mock.assert_async().await;
        delete_mock.assert_async().await;
        assert!(writer.is_committed());
    }

    #[tokio::test]
    async fn test_explicit_child_delete_queue_triggers_deletion() {
        let mut server = Server::new_async().await;
        let base_url = format!("{}/solr", server.url());
        let core = "stage_staff_run_20261026_explicitdel_candidate";
        let update_path = format!("/solr/{}/update", core);

        let delete_body = json!({
            "delete": {
                "query": "parent_id:(\"/repositories/2/archival_objects/42\")"
            }
        });

        let add_body = json!([
            {
                "id": "/repositories/2/archival_objects/42",
                "primary_type": "archival_object",
                "title": "Item 42"
            }
        ]);

        let delete_mock = server
            .mock("POST", update_path.as_str())
            .match_header("content-type", "application/json")
            .match_body(Matcher::Json(delete_body))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let add_mock = server
            .mock("POST", update_path.as_str())
            .match_header("content-type", "application/json")
            .match_body(Matcher::Json(add_body))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let commit_mock = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Json(json!({"commit":{"softCommit":false}})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let mut writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();

        // Caller explicitly enqueues parent ID for child deletion
        writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/42")
            .unwrap();
        assert_eq!(writer.pending_delete_count(), 1);

        writer
            .add_document(json!({
                "id": "/repositories/2/archival_objects/42",
                "primary_type": "archival_object",
                "title": "Item 42"
            }))
            .await
            .unwrap();

        writer.finish().await.unwrap();

        delete_mock.assert_async().await;
        add_mock.assert_async().await;
        commit_mock.assert_async().await;
        assert_eq!(writer.pending_delete_count(), 0);
    }

    #[tokio::test]
    async fn test_rejected_document_leaves_state_completely_unchanged() {
        let mut server = Server::new_async().await;
        let base_url = format!("{}/solr", server.url());
        let core = "stage_staff_run_20261026_rejecteddoc_candidate";
        let update_path = format!("/solr/{}/update", core);

        let config = IsolatedSolrWriterConfig {
            max_doc_count: 5,
            max_serialized_bytes: 2048,
            parent_id_field: "parent_id".to_string(),
            max_delete_count: 10,
        };
        let mut writer = IsolatedSolrStaffAoWriter::with_config(&base_url, core, config).unwrap();

        // Queue an explicit parent delete
        writer
            .queue_child_delete_for_parent("/repositories/2/resources/1")
            .unwrap();
        assert_eq!(writer.pending_delete_count(), 1);

        // Add 1 valid doc
        writer
            .add_document(json!({
                "id": "/repositories/2/archival_objects/1",
                "title": "Valid Doc 1"
            }))
            .await
            .unwrap();

        let initial_doc_count = writer.buffered_doc_count();
        let initial_bytes = writer.buffered_bytes();
        assert_eq!(initial_doc_count, 1);
        assert!(initial_bytes > 0);

        // Attempt 1: Non-object JSON
        assert!(writer.add_document(json!("not an object")).await.is_err());
        assert_eq!(writer.buffered_doc_count(), initial_doc_count);
        assert_eq!(writer.buffered_bytes(), initial_bytes);
        assert_eq!(writer.pending_delete_count(), 1);
        assert!(!writer.has_error());

        // Attempt 2: Missing 'id'
        assert!(writer
            .add_document(json!({"title": "No ID"}))
            .await
            .is_err());
        assert_eq!(writer.buffered_doc_count(), initial_doc_count);
        assert_eq!(writer.buffered_bytes(), initial_bytes);
        assert_eq!(writer.pending_delete_count(), 1);
        assert!(!writer.has_error());

        // Attempt 3: Oversized doc exceeding max_serialized_bytes (2048)
        let huge_title = "X".repeat(3000);
        let huge_doc = json!({
            "id": "/repositories/2/archival_objects/999",
            "title": huge_title
        });
        assert!(writer.add_document(huge_doc).await.is_err());
        assert_eq!(writer.buffered_doc_count(), initial_doc_count);
        assert_eq!(writer.buffered_bytes(), initial_bytes);
        assert_eq!(writer.pending_delete_count(), 1);
        assert!(!writer.has_error());

        // Commit behavior remains intact: flushes queued delete, valid doc 1, and commits
        let delete_mock = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Regex("delete".to_string()))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let add_mock = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Regex("Valid Doc 1".to_string()))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let commit_mock = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Regex("commit".to_string()))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        writer.finish().await.unwrap();

        delete_mock.assert_async().await;
        add_mock.assert_async().await;
        commit_mock.assert_async().await;
        assert!(writer.is_committed());
    }

    #[tokio::test]
    async fn test_preflush_ordering_on_capacity_boundary() {
        let mut server = Server::new_async().await;
        let base_url = format!("{}/solr", server.url());
        let core = "stage_staff_run_20261026_preflush_candidate";
        let update_path = format!("/solr/{}/update", core);

        // max_doc_count = 2
        let config = IsolatedSolrWriterConfig {
            max_doc_count: 2,
            max_serialized_bytes: DEFAULT_MAX_SERIALIZED_BYTES,
            parent_id_field: "parent_id".to_string(),
            max_delete_count: 10,
        };

        // First flush: docs [1, 2]
        let add_batch1 = json!([
            {"id": "/repositories/2/archival_objects/1", "primary_type": "archival_object"},
            {"id": "/repositories/2/archival_objects/2", "primary_type": "archival_object"}
        ]);
        let m_add1 = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Json(add_batch1))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        // Second flush: doc 3
        let add_batch2 = json!([
            {"id": "/repositories/2/archival_objects/3", "primary_type": "archival_object"}
        ]);
        let m_add2 = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Json(add_batch2))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let m_commit = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Json(json!({"commit":{"softCommit":false}})))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let mut writer = IsolatedSolrStaffAoWriter::with_config(&base_url, core, config).unwrap();

        // Adding doc 1: buffered
        writer
            .add_document(json!({"id": "/repositories/2/archival_objects/1", "primary_type": "archival_object"}))
            .await
            .unwrap();
        assert_eq!(writer.buffered_doc_count(), 1);

        // Adding doc 2: fills buffer and flushes immediately
        writer
            .add_document(json!({"id": "/repositories/2/archival_objects/2", "primary_type": "archival_object"}))
            .await
            .unwrap();
        assert_eq!(writer.buffered_doc_count(), 0);

        // Adding doc 3: buffered
        writer
            .add_document(json!({"id": "/repositories/2/archival_objects/3", "primary_type": "archival_object"}))
            .await
            .unwrap();
        assert_eq!(writer.buffered_doc_count(), 1);

        // Finish flushes doc 3 and commits
        writer.finish().await.unwrap();
        assert_eq!(writer.buffered_doc_count(), 0);

        m_add1.assert_async().await;
        m_add2.assert_async().await;
        m_commit.assert_async().await;
    }

    #[tokio::test]
    async fn test_post_update_fail_closed_on_malformed_solr_responses() {
        let test_cases = vec![
            ("empty_body", 200, ""),
            (
                "html_body",
                200,
                "<html><body>502 Bad Gateway</body></html>",
            ),
            ("missing_response_header", 200, r#"{"status":0}"#),
            (
                "missing_status_in_header",
                200,
                r#"{"responseHeader":{"QTime":1}}"#,
            ),
            (
                "string_status_in_header",
                200,
                r#"{"responseHeader":{"status":"0"}}"#,
            ),
            (
                "boolean_status_in_header",
                200,
                r#"{"responseHeader":{"status":false}}"#,
            ),
            (
                "nonzero_status_400",
                200,
                r#"{"responseHeader":{"status":400}}"#,
            ),
            (
                "nonzero_status_1",
                200,
                r#"{"responseHeader":{"status":1}}"#,
            ),
        ];

        for (name, status_code, body) in test_cases {
            let mut server = Server::new_async().await;
            let base_url = format!("{}/solr", server.url());
            let core = format!("stage_staff_case_{}_candidate", name);
            let update_path = format!("/solr/{}/update", core);

            let _mock = server
                .mock("POST", update_path.as_str())
                .with_status(status_code)
                .with_header("content-type", "application/json")
                .with_body(body)
                .create_async()
                .await;

            let mut writer = IsolatedSolrStaffAoWriter::new(&base_url, &core).unwrap();
            writer
                .add_document(json!({"id": "/repositories/2/archival_objects/1"}))
                .await
                .unwrap();

            let flush_res = writer.flush().await;
            assert!(
                flush_res.is_err(),
                "Case '{}' should have failed closed, but succeeded",
                name
            );
            assert!(
                writer.has_error(),
                "Writer must be in error state after '{}'",
                name
            );

            // Verify no commit is possible after failure
            let commit_res = writer.commit().await;
            assert!(
                commit_res.is_err(),
                "Commit should fail closed after error in '{}'",
                name
            );
            assert!(!writer.is_committed());
        }
    }

    #[tokio::test]
    async fn test_solr_error_message_never_leaks_private_record_content() {
        let secret_markers = [
            "SECRET_PATIENT_RECORDS_DATA_ABC_12345",
            "CLASSIFIED_COLLECTION_SUMMARY_XYZ_67890",
        ];

        for marker in secret_markers {
            // Case A: 500 with Solr error msg containing secret
            {
                let mut server = Server::new_async().await;
                let base_url = format!("{}/solr", server.url());
                let core = "stage_staff_leak_a_candidate";
                let update_path = format!("/solr/{}/update", core);

                let body = format!(
                    r#"{{"error":{{"msg":"Failed to parse query containing record {}"}}}}"#,
                    marker
                );
                let _mock = server
                    .mock("POST", update_path.as_str())
                    .with_status(500)
                    .with_header("content-type", "application/json")
                    .with_body(body)
                    .create_async()
                    .await;

                let mut writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();
                writer
                    .add_document(json!({"id": "/repositories/2/archival_objects/1"}))
                    .await
                    .unwrap();

                let err = writer.flush().await.unwrap_err();
                let display_str = format!("{}", err);
                let debug_str = format!("{:?}", err);

                assert!(
                    !display_str.contains(marker),
                    "Secret marker leaked in Display: {}",
                    display_str
                );
                assert!(
                    !debug_str.contains(marker),
                    "Secret marker leaked in Debug: {}",
                    debug_str
                );
            }

            // Case B: 200 with nonzero status and HTML/text containing secret
            {
                let mut server = Server::new_async().await;
                let base_url = format!("{}/solr", server.url());
                let core = "stage_staff_leak_b_candidate";
                let update_path = format!("/solr/{}/update", core);

                let body = format!(
                    r#"{{"responseHeader":{{"status":400}},"error":{{"msg":"Sensitive text: {}"}}}}"#,
                    marker
                );
                let _mock = server
                    .mock("POST", update_path.as_str())
                    .with_status(200)
                    .with_header("content-type", "application/json")
                    .with_body(body)
                    .create_async()
                    .await;

                let mut writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();
                writer
                    .add_document(json!({"id": "/repositories/2/archival_objects/1"}))
                    .await
                    .unwrap();

                let err = writer.flush().await.unwrap_err();
                let display_str = format!("{}", err);
                let debug_str = format!("{:?}", err);

                assert!(
                    !display_str.contains(marker),
                    "Secret marker leaked in Display: {}",
                    display_str
                );
                assert!(
                    !debug_str.contains(marker),
                    "Secret marker leaked in Debug: {}",
                    debug_str
                );
            }
        }
    }

    #[tokio::test]
    async fn test_serialized_bytes_includes_brackets_commas_and_utf8() {
        let mut server = Server::new_async().await;
        let base_url = format!("{}/solr", server.url());
        let core = "stage_staff_run_20261026_bytescap_candidate";
        let update_path = format!("/solr/{}/update", core);

        // Cap of 1024 bytes
        let cap = 1024;
        let config = IsolatedSolrWriterConfig {
            max_doc_count: 10,
            max_serialized_bytes: cap,
            parent_id_field: "parent_id".to_string(),
            max_delete_count: 10,
        };

        // Construct a doc with UTF-8 multi-byte characters (4-byte emoji, 3-byte CJK, 2-byte accented)
        // Multi-byte string: "é" (2 bytes), "漢" (3 bytes), "🚀" (4 bytes)
        let utf8_prefix = "é漢🚀"; // 2 + 3 + 4 = 9 bytes
        let base_doc = json!({
            "id": "/repositories/2/archival_objects/100",
            "title": utf8_prefix,
            "padding": ""
        });
        let base_len = serde_json::to_vec(&base_doc).unwrap().len();

        // Exactly fill so that [base_doc] takes exactly 1024 bytes:
        // doc_len + 2 = 1024 => doc_len = 1022.
        let needed_padding = 1022 - base_len;
        let exact_doc = json!({
            "id": "/repositories/2/archival_objects/100",
            "title": utf8_prefix,
            "padding": "A".repeat(needed_padding)
        });
        let exact_doc_len = serde_json::to_vec(&exact_doc).unwrap().len();
        assert_eq!(exact_doc_len + 2, cap);

        let _mock = server
            .mock("POST", update_path.as_str())
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .create_async()
            .await;

        let mut writer =
            IsolatedSolrStaffAoWriter::with_config(&base_url, core, config.clone()).unwrap();

        // 1. Doc fitting exactly at cap succeeds
        writer.add_document(exact_doc).await.unwrap();
        // Since current_body_len reached cap (1024 >= 1024), it flushed automatically!
        assert_eq!(writer.buffered_doc_count(), 0);

        // 2. Doc exceeding cap by 1 byte is rejected before network
        let over_doc = json!({
            "id": "/repositories/2/archival_objects/101",
            "title": utf8_prefix,
            "padding": "A".repeat(needed_padding + 1)
        });
        let over_doc_len = serde_json::to_vec(&over_doc).unwrap().len();
        assert_eq!(over_doc_len + 2, cap + 1);

        let err = writer.add_document(over_doc).await.unwrap_err();
        match err {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains("exceeds maximum allowed buffer request size"));
            }
            other => panic!("Expected InvalidRequest, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_explicit_delete_queue_independent_bounds() {
        let config = IsolatedSolrWriterConfig {
            max_doc_count: 50,
            max_serialized_bytes: 10240,
            parent_id_field: "parent_id".to_string(),
            max_delete_count: 3, // Bound queue at 3
        };
        let mut writer =
            IsolatedSolrStaffAoWriter::with_config(VALID_BASE_URL, VALID_STAGE_CORE, config)
                .unwrap();

        // Enqueue 3 parents
        writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/1")
            .unwrap();
        writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/2")
            .unwrap();
        writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/3")
            .unwrap();
        assert_eq!(writer.pending_delete_count(), 3);
        assert_eq!(writer.buffered_doc_count(), 0);

        // Deduplication doesn't increase count
        writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/2")
            .unwrap();
        assert_eq!(writer.pending_delete_count(), 3);

        // Enqueue 4th parent exceeds capacity and fails closed
        let err = writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/4")
            .unwrap_err();
        match err {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains("explicit delete queue capacity limit reached"));
            }
            other => panic!("Expected InvalidRequest, got {:?}", other),
        }
        assert_eq!(writer.pending_delete_count(), 3);
    }

    #[tokio::test]
    async fn test_malformed_uri_rejected_before_network() {
        let mut writer = IsolatedSolrStaffAoWriter::new(VALID_BASE_URL, VALID_STAGE_CORE).unwrap();

        let malicious_uris = [
            "/repositories/2/archival_objects/123\" OR *:*",
            "/repositories/2/archival_objects/123) OR id:(456",
            "/repositories/2/archival_objects/123\nDELETE",
            "/repositories/2/archival_objects/123 456",
            "*:*",
            "/repositories/2/archival_objects/123;DROP TABLE",
            "http://external/repositories/2/archival_objects/123",
            "../repositories/2/archival_objects/123",
        ];

        for bad_uri in &malicious_uris {
            let res = writer.queue_child_delete_for_parent(bad_uri);
            assert!(
                res.is_err(),
                "Expected malicious parent URI '{}' to be rejected",
                bad_uri
            );
        }
    }

    #[test]
    fn test_no_accidental_target_core_change() {
        let writer = IsolatedSolrStaffAoWriter::new(VALID_BASE_URL, VALID_STAGE_CORE).unwrap();
        let expected_endpoint = format!("{}/{}/update", VALID_BASE_URL, VALID_STAGE_CORE);
        assert_eq!(writer.target_core(), VALID_STAGE_CORE);
        assert_eq!(writer.update_url().as_str(), expected_endpoint);
    }

    #[tokio::test]
    async fn test_multiple_parent_delete_or_syntax() {
        let mut server = Server::new_async().await;
        let base_url = format!("{}/solr", server.url());
        let core = "stage_staff_run_20261026_multiparent_candidate";
        let update_path = format!("/solr/{}/update", core);

        let delete_body = json!({
            "delete": {
                "query": "parent_id:(\"/repositories/2/archival_objects/10\" OR \"/repositories/2/archival_objects/20\")"
            }
        });

        let add_body = json!([
            {"id": "/repositories/2/archival_objects/10", "primary_type": "archival_object"},
            {"id": "/repositories/2/archival_objects/20", "primary_type": "archival_object"}
        ]);

        let del_mock = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Json(delete_body))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let add_mock = server
            .mock("POST", update_path.as_str())
            .match_body(Matcher::Json(add_body))
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(1)
            .create_async()
            .await;

        let mut writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();
        writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/10")
            .unwrap();
        writer
            .queue_child_delete_for_parent("/repositories/2/archival_objects/20")
            .unwrap();

        writer
            .add_document(json!({"id": "/repositories/2/archival_objects/10", "primary_type": "archival_object"}))
            .await
            .unwrap();
        writer
            .add_document(json!({"id": "/repositories/2/archival_objects/20", "primary_type": "archival_object"}))
            .await
            .unwrap();

        writer.flush().await.unwrap();

        del_mock.assert_async().await;
        add_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_verify_target_core_ready_refuses_absent_core() {
        let mut server = Server::new_async().await;
        let base_url = server.url();
        let core = "stage_staff_20261026_caseabsent_candidate";

        let _status_mock = server
            .mock(
                "GET",
                "/admin/cores?action=STATUS&core=stage_staff_20261026_caseabsent_candidate&wt=json",
            )
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0},"status":{}}"#)
            .expect(1)
            .create_async()
            .await;

        let writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();
        let err = writer.verify_target_core_ready().await.unwrap_err();
        assert!(err.to_string().contains("does not exist in Solr"));
    }

    #[tokio::test]
    async fn test_verify_target_core_ready_refuses_non_empty_core() {
        let mut server = Server::new_async().await;
        let base_url = server.url();
        let core = "stage_staff_20261026_casenonempty_candidate";

        let _status_mock = server
            .mock("GET", "/admin/cores?action=STATUS&core=stage_staff_20261026_casenonempty_candidate&wt=json")
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0},"status":{"stage_staff_20261026_casenonempty_candidate":{"name":"stage_staff_20261026_casenonempty_candidate"}}}"#)
            .expect(1)
            .create_async()
            .await;

        let _select_mock = server
            .mock(
                "GET",
                "/stage_staff_20261026_casenonempty_candidate/select?q=*:*&rows=0&wt=json",
            )
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0},"response":{"numFound":5,"docs":[]}}"#)
            .expect(1)
            .create_async()
            .await;

        let writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();
        let err = writer.verify_target_core_ready().await.unwrap_err();
        assert!(err
            .to_string()
            .contains("is not empty (found 5 existing documents)"));
    }

    #[tokio::test]
    async fn test_verify_target_core_ready_success() {
        let mut server = Server::new_async().await;
        let base_url = server.url();
        let core = "stage_staff_20261026_casesuccess_candidate";

        let _status_mock = server
            .mock("GET", "/admin/cores?action=STATUS&core=stage_staff_20261026_casesuccess_candidate&wt=json")
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0},"status":{"stage_staff_20261026_casesuccess_candidate":{"name":"stage_staff_20261026_casesuccess_candidate"}}}"#)
            .expect(1)
            .create_async()
            .await;

        let _select_mock = server
            .mock(
                "GET",
                "/stage_staff_20261026_casesuccess_candidate/select?q=*:*&rows=0&wt=json",
            )
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0},"response":{"numFound":0,"docs":[]}}"#)
            .expect(1)
            .create_async()
            .await;

        let writer = IsolatedSolrStaffAoWriter::new(&base_url, core).unwrap();
        writer.verify_target_core_ready().await.unwrap();
    }
}
