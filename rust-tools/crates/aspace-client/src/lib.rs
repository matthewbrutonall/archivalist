pub mod client;
pub mod error;
pub mod models;
pub mod shadow;
pub mod validation;

pub use client::Client;
pub use error::{ASpaceError, Result};
pub use shadow::{
    assemble_shadow_tree, assemble_shadow_tree_with_context, build_keyword_query,
    build_waypoint_query, calculate_waypoints, collect_missing_ancestor_uris, escape_solr_literal,
    escape_solr_wildcard_filter, ShadowArchivalObjectHit, ShadowKeywordSearchParams,
    ShadowNodeFromRootEntry, ShadowSearchParams, ShadowSearchResult, ShadowSolrClient,
    ShadowTreeNode, ShadowTreeNodeFromRootParams, ShadowTreeResponse, ShadowTreeWaypointParams,
    ShadowWaypointNode, WAYPOINT_SIZE,
};
