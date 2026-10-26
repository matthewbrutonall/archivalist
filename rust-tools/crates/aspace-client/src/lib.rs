pub mod client;
pub mod error;
pub mod index_document;
pub mod isolated_solr_writer;
pub mod models;
pub mod schema_enums;
pub mod shadow;
pub mod validation;

pub use client::{
    Client, DEFAULT_STAFF_ARCHIVAL_OBJECT_BATCH_SIZE, STAFF_INDEXER_RESOLVED_ATTRIBUTES,
};
pub use error::{ASpaceError, Result};
pub use index_document::{
    build_index_document, build_index_document_batch, IndexDocumentBatchRequest,
    IndexDocumentBatchResponse, IndexDocumentError, IndexDocumentInput, MAX_INDEX_DOCUMENT_BATCH,
};
pub use isolated_solr_writer::{
    IsolatedSolrStaffAoWriter, IsolatedSolrWriterConfig, ABSOLUTE_MAX_DELETE_COUNT,
    ABSOLUTE_MAX_DOC_COUNT, ABSOLUTE_MAX_SERIALIZED_BYTES, DEFAULT_MAX_DELETE_COUNT,
    DEFAULT_MAX_DOC_COUNT, DEFAULT_MAX_SERIALIZED_BYTES, DEFAULT_PARENT_ID_FIELD,
    MIN_SERIALIZED_BYTES,
};
pub use schema_enums::extract_dynamic_enum_fields_from_schemas;
pub use shadow::{
    assemble_shadow_tree, assemble_shadow_tree_with_context, build_keyword_query,
    build_waypoint_query, calculate_waypoints, collect_missing_ancestor_uris, escape_solr_literal,
    escape_solr_wildcard_filter, ShadowArchivalObjectHit, ShadowKeywordSearchParams,
    ShadowNodeFromRootEntry, ShadowSearchParams, ShadowSearchResult, ShadowSolrClient,
    ShadowTreeNode, ShadowTreeNodeFromRootParams, ShadowTreeResponse, ShadowTreeWaypointParams,
    ShadowWaypointNode, WAYPOINT_SIZE,
};
