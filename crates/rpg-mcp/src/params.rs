//! MCP tool parameter structs — one per tool handler, deserialized from JSON-RPC calls.

use schemars::JsonSchema;
use serde::Deserialize;

/// Parameters for the `search_node` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchNodeParams {
    /// The search query describing what you're looking for
    pub(crate) query: String,
    /// Search mode: 'features', 'snippets', or 'auto' (default: 'auto')
    pub(crate) mode: Option<String>,
    /// Optional hierarchy scope to restrict search (e.g., 'Security/auth'). Comma-separated for multiple scopes.
    pub(crate) scope: Option<String>,
    /// Filter to entities within a line range [start, end]
    pub(crate) line_nums: Option<Vec<usize>>,
    /// Glob pattern to filter entities by file path (e.g., "src/**/*.rs")
    pub(crate) file_pattern: Option<String>,
    /// Comma-separated entity type filter (e.g., "function,class,method"). Valid: function, class, method, file, module.
    pub(crate) entity_type_filter: Option<String>,
}

/// Parameters for the `fetch_node` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct FetchNodeParams {
    /// The entity ID to fetch (e.g., 'src/auth.rs:validate_token')
    pub(crate) entity_id: String,
    /// Multiple entity IDs to fetch in batch (overrides entity_id when provided)
    pub(crate) entity_ids: Option<Vec<String>>,
}

/// Parameters for the `explore_rpg` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ExploreRpgParams {
    /// The entity ID to start exploration from
    pub(crate) entity_id: String,
    /// Multiple entity IDs to explore from in batch (overrides entity_id when provided)
    pub(crate) entity_ids: Option<Vec<String>>,
    /// Traversal direction: 'upstream', 'downstream', or 'both'
    pub(crate) direction: Option<String>,
    /// Maximum traversal depth (default: 2). Use -1 for unlimited depth.
    pub(crate) depth: Option<i64>,
    /// Filter edges by kind: 'imports', 'invokes', 'inherits', 'composes', 'contains', 'renders', 'reads_state', 'writes_state', or 'dispatches'
    pub(crate) edge_filter: Option<String>,
    /// Comma-separated entity type filter (e.g., "function,class,method"). Valid: function, class, method, file, module, page, layout, component, hook, store.
    pub(crate) entity_type_filter: Option<String>,
}

/// Parameters for the `build_rpg` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct BuildRpgParams {
    /// Primary language override (auto-detected if not specified)
    pub(crate) language: Option<String>,
    /// Glob pattern to include files (e.g., "src/**/*.rs")
    pub(crate) include: Option<String>,
    /// Glob pattern to exclude files (e.g., "tests/**")
    pub(crate) exclude: Option<String>,
}

/// Parameters for the `update_rpg` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct UpdateRpgParams {
    /// Base commit SHA to diff from (defaults to RPG's stored base_commit)
    pub(crate) since: Option<String>,
}

/// Parameters for the `get_entities_for_lifting` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetEntitiesForLiftingParams {
    /// Scope specifier: file glob ("src/auth/**"), hierarchy path, entity IDs, or "*"/"all".
    pub(crate) scope: String,
    /// Batch index to retrieve (0-based). Omit or 0 for first batch.
    pub(crate) batch_index: Option<usize>,
}

/// Parameters for the `submit_lift_results` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SubmitLiftResultsParams {
    /// JSON object mapping function names to feature arrays.
    /// Example: {"my_func": ["validate input", "return result"], "other": ["compute hash"]}
    pub(crate) features: String,
}

/// Parameters for the `submit_hierarchy` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SubmitHierarchyParams {
    /// JSON object mapping file paths to 3-level hierarchy paths.
    /// Example: {"src/auth/login.rs": "Authentication/manage sessions/handle login"}
    pub(crate) assignments: String,
}

/// Parameters for the `get_files_for_synthesis` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetFilesForSynthesisParams {
    /// Batch index to retrieve (0-based). Omit or 0 for first batch.
    pub(crate) batch_index: Option<usize>,
}

/// Parameters for the `submit_file_syntheses` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SubmitFileSynthesesParams {
    /// JSON object mapping file paths to comma-separated feature strings.
    /// Example: {"src/auth/login.rs": "handle user authentication, manage session tokens",
    ///           "src/db/query.rs": "build SQL queries, execute database operations"}
    pub(crate) syntheses: String,
}

/// Parameters for the `get_routing_candidates` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetRoutingCandidatesParams {
    /// Batch index to retrieve (0-based). For large sets, returns paginated candidates.
    #[serde(default)]
    pub(crate) batch_index: Option<usize>,
}

/// Parameters for the `reconstruct_plan` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ReconstructPlanParams {
    /// Maximum number of entities per execution batch (default: 8).
    pub(crate) max_batch_size: Option<usize>,
    /// Include file-level Module entities in the schedule (default: false).
    pub(crate) include_modules: Option<bool>,
}

/// Parameters for the `submit_routing_decisions` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SubmitRoutingDecisionsParams {
    /// JSON object mapping entity IDs to routing action.
    /// Value is a hierarchy path to route there, or "keep" to confirm current position.
    /// Example: {"src/auth.rs:validate_token": "Security/auth/validate", "src/db.rs:query": "keep"}
    /// Entities not included remain pending (NOT auto-kept).
    pub(crate) decisions: String,
    /// Graph revision from get_routing_candidates response. Required. Rejects stale decisions.
    pub(crate) graph_revision: String,
}
