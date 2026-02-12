//! Server state types, persistence helpers, and re-exports for tool handlers.

// Re-export submodules so `use crate::types::*` in tools.rs keeps working.
pub(crate) use crate::helpers::*;
pub(crate) use crate::params::*;

// Re-export the routing prompt from rpg-encoder (avoids cross-crate include_str! paths).
pub(crate) use rpg_encoder::semantic_lifting::SEMANTIC_ROUTING_PROMPT as ROUTING_PROMPT;

use anyhow::Result;
use rpg_core::graph::RPGraph;
use rpg_core::storage;
use serde::Deserialize;
use serde::Serialize;

/// Cached lifting session — holds raw entities for stable batch indexing.
/// Without this, each `get_entities_for_lifting` call rebuilds the unlifted list,
/// causing batch indices to shift as entities get lifted between calls.
pub(crate) struct LiftingSession {
    pub(crate) scope_key: String,
    pub(crate) raw_entities: Vec<rpg_parser::entities::RawEntity>,
    pub(crate) batch_ranges: Vec<(usize, usize)>,
    /// Number of entities auto-lifted during session setup.
    pub(crate) auto_lifted: usize,
}

/// An entity pending LLM-based semantic routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingRouting {
    pub(crate) entity_id: String,
    pub(crate) original_path: String,
    pub(crate) features: Vec<String>,
    pub(crate) reason: String,
}

/// Persisted pending routing state (crash-safe).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingRoutingState {
    pub(crate) graph_revision: String,
    pub(crate) entries: Vec<PendingRouting>,
}

/// Load pending routing state from disk, if it exists.
pub(crate) fn load_pending_routing(project_root: &std::path::Path) -> Option<PendingRoutingState> {
    let path = storage::pending_routing_file(project_root);
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save pending routing state to disk.
pub(crate) fn save_pending_routing(
    project_root: &std::path::Path,
    state: &PendingRoutingState,
) -> Result<()> {
    let path = storage::pending_routing_file(project_root);
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Clear pending routing state from disk.
pub(crate) fn clear_pending_routing(project_root: &std::path::Path) {
    let path = storage::pending_routing_file(project_root);
    let _ = std::fs::remove_file(&path);
}

/// Get the graph revision string for stale-decision protection.
/// Uses `updated_at` (changes on every save) rather than `base_commit`
/// (which stays constant across lift/routing operations).
pub(crate) fn graph_revision(graph: &RPGraph) -> String {
    graph.updated_at.to_rfc3339()
}
