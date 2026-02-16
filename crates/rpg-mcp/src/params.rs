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
    /// Git commit to diff from for proximity-based ranking (e.g., "HEAD~10", "abc123"). Boosts entities in changed files and their dependencies.
    pub(crate) since_commit: Option<String>,
}

/// Parameters for the `fetch_node` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct FetchNodeParams {
    /// The entity ID to fetch (e.g., 'src/auth.rs:validate_token')
    pub(crate) entity_id: String,
    /// Multiple entity IDs to fetch in batch (overrides entity_id when provided)
    pub(crate) entity_ids: Option<Vec<String>>,
    /// Comma-separated fields to include: "features", "source", "deps", "hierarchy". Omit for all fields.
    pub(crate) fields: Option<String>,
    /// Maximum lines of source code to return (default: unlimited). Only applies when "source" is included.
    pub(crate) source_max_lines: Option<usize>,
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
    /// Output format: "tree" (default, indented tree) or "compact" (pipe-delimited rows with entity_ids)
    pub(crate) format: Option<String>,
    /// Maximum number of nodes to return (default: unlimited). Truncates output for large traversals.
    pub(crate) max_results: Option<usize>,
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

/// Parameters for the `context_pack` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ContextPackParams {
    /// The search query describing what context you need
    pub(crate) query: String,
    /// Optional hierarchy scope to restrict search (e.g., 'Security/auth')
    pub(crate) scope: Option<String>,
    /// Target token budget for the packed context (default: 4000)
    pub(crate) token_budget: Option<usize>,
    /// Include source code for primary entities (default: true)
    pub(crate) include_source: Option<bool>,
    /// Neighborhood expansion depth: 0 = primary only, 1 = include 1-hop neighbors (default: 1)
    pub(crate) depth: Option<usize>,
}

/// Parameters for the `impact_radius` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ImpactRadiusParams {
    /// The entity ID to compute impact from
    pub(crate) entity_id: String,
    /// Traversal direction: 'upstream' (what depends on this), 'downstream' (what this depends on), or 'both'
    pub(crate) direction: Option<String>,
    /// Maximum traversal depth (default: 3). Use -1 for unlimited.
    pub(crate) max_depth: Option<i64>,
    /// Filter edges by kind: 'imports', 'invokes', 'inherits', 'composes', 'renders', 'reads_state', 'writes_state', 'dispatches'
    pub(crate) edge_filter: Option<String>,
    /// Maximum number of reachable entities to return (default: 100). Prevents overwhelming output on highly-connected nodes.
    pub(crate) max_results: Option<usize>,
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

/// Parameters for the `plan_change` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PlanChangeParams {
    /// The goal or intent of the change (e.g., "add rate limiting to API endpoints")
    pub(crate) goal: String,
    /// Optional hierarchy scope to restrict search (e.g., 'Security/auth')
    pub(crate) scope: Option<String>,
    /// Maximum number of relevant entities to include (default: 15)
    pub(crate) max_entities: Option<usize>,
}

/// Parameters for the `find_paths` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct FindPathsParams {
    /// Source entity ID
    pub(crate) source: String,
    /// Target entity ID
    pub(crate) target: String,
    /// Maximum path length (default: 5). Use -1 for unlimited.
    pub(crate) max_hops: Option<i64>,
    /// Maximum number of paths to return (default: 3)
    pub(crate) max_paths: Option<usize>,
    /// Filter edges by kind: 'imports', 'invokes', 'inherits', 'composes', 'contains', 'renders', 'reads_state', 'writes_state', or 'dispatches'
    pub(crate) edge_filter: Option<String>,
}

/// Parameters for the `slice_between` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SliceBetweenParams {
    /// Entity IDs to connect (minimum 2)
    pub(crate) entity_ids: Vec<String>,
    /// Maximum path length when searching for connections (default: 3)
    pub(crate) max_depth: Option<usize>,
    /// Include entity metadata (name, file, features) in output
    pub(crate) include_metadata: Option<bool>,
}

// =============================================================================
// Generation Tools
// =============================================================================

/// Parameters for the `init_generation` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct InitGenerationParams {
    /// Natural language specification of the code to generate
    pub(crate) spec: String,
    /// Target programming language (rust, python, typescript, etc.)
    pub(crate) language: String,
    /// Optional path to a reference repository for pattern retrieval
    pub(crate) reference_repo: Option<String>,
}

/// Parameters for the `submit_feature_tree` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SubmitFeatureTreeParams {
    /// JSON object containing the feature tree decomposition.
    /// See spec_decomposition.md prompt for expected format.
    pub(crate) features: String,
    /// Plan revision from init_generation or generation_status. Required to prevent stale submissions.
    pub(crate) revision: Option<String>,
}

/// Parameters for the `get_interfaces_for_design` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetInterfacesForDesignParams {
    /// Batch index to retrieve (0-based). Omit or 0 for first batch.
    pub(crate) batch_index: Option<usize>,
}

/// Parameters for the `submit_interface_design` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SubmitInterfaceDesignParams {
    /// JSON object containing the interface design.
    /// See interface_design.md prompt for expected format.
    pub(crate) interfaces: String,
    /// Plan revision from generation_status. Required to prevent stale submissions.
    pub(crate) revision: Option<String>,
}

/// Parameters for the `get_tasks_for_generation` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetTasksForGenerationParams {
    /// Batch index to retrieve (0-based). Omit or 0 for first batch.
    pub(crate) batch_index: Option<usize>,
}

/// Parameters for the `submit_generated_code` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SubmitGeneratedCodeParams {
    /// JSON object mapping planned_id to file path where code was written.
    /// Example: {"auth.login": "src/auth/login.rs", "auth.logout": "src/auth/logout.rs"}
    pub(crate) completions: String,
    /// Plan revision from generation_status. Required to prevent stale submissions.
    pub(crate) revision: Option<String>,
}

/// Parameters for the `validate_generation` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ValidateGenerationParams {
    /// Optional list of specific task IDs to validate. If omitted, validates all completed tasks.
    pub(crate) task_ids: Option<Vec<String>>,
}

/// Parameters for the `report_task_outcome` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ReportTaskOutcomeParams {
    /// Planned entity ID (task ID from get_tasks_for_generation)
    pub(crate) task_id: String,
    /// JSON describing outcome: {"kind":"pass"} or {"kind":"test_failure","failing_count":3,"summary":"..."}
    pub(crate) outcome: String,
    /// Optional JSON: {"total":5,"passed":3,"failed":2,"test_file":"tests/test_auth.rs"}
    pub(crate) test_results: Option<String>,
    /// Optional JSON telemetry payload (sandbox/latency/token/cost metadata)
    pub(crate) telemetry: Option<String>,
    /// File path where code was written (for completed tasks)
    pub(crate) file_path: Option<String>,
    /// Plan revision for staleness check
    pub(crate) revision: Option<String>,
}

/// Parameters for `run_task_test_loop`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RunTaskTestLoopParams {
    /// Planned entity ID (task ID from get_tasks_for_generation)
    pub(crate) task_id: String,
    /// Command that runs tests for this task (e.g. "cargo test -p app auth::tests::login")
    pub(crate) test_command: String,
    /// File path where the generated code lives (required when task passes)
    pub(crate) file_path: Option<String>,
    /// Optional working directory relative to project root.
    pub(crate) working_dir: Option<String>,
    /// Sandbox mode: local, docker, or none.
    pub(crate) sandbox_mode: Option<String>,
    /// Docker image when sandbox_mode=docker. If omitted, resolved from
    /// `.rpg/config.toml` `[generation.docker_images]` using plan language.
    pub(crate) docker_image: Option<String>,
    /// Maximum automatic command adaptations before giving up (default: 1).
    pub(crate) max_auto_adapt: Option<usize>,
    /// Optional model name for cost estimation.
    pub(crate) model: Option<String>,
    /// Optional backbone family name for aggregated efficiency curves.
    pub(crate) backbone: Option<String>,
    /// Optional prompt token count for this loop iteration.
    pub(crate) prompt_tokens: Option<usize>,
    /// Optional completion token count for this loop iteration.
    pub(crate) completion_tokens: Option<usize>,
    /// Plan revision for staleness check.
    pub(crate) revision: Option<String>,
}

/// Parameters for `generation_efficiency_report`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GenerationEfficiencyReportParams {}

/// Parameters for `seed_ontology_features`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SeedOntologyFeaturesParams {
    /// Maximum ontology-seeded features to add per entity (default: 2)
    pub(crate) max_per_entity: Option<usize>,
}

/// Parameters for `assess_representation_quality`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AssessRepresentationQualityParams {
    /// Drift threshold for alerting significant semantic changes (default: 0.5)
    pub(crate) drift_threshold: Option<f64>,
    /// Persist current features as new drift baseline (default: true)
    pub(crate) write_baseline: Option<bool>,
    /// Max entities to show in low-confidence and high-drift examples (default: 20)
    pub(crate) max_examples: Option<usize>,
}

/// Parameters for `run_representation_ablation`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RunRepresentationAblationParams {
    /// Maximum query-target pairs to evaluate (default: 200)
    pub(crate) max_queries: Option<usize>,
    /// Evaluate Acc@k and MRR@k (default: 5)
    pub(crate) k: Option<usize>,
}

/// Parameters for `export_external_validation_bundle`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ExportExternalValidationBundleParams {
    /// Number of blinded tasks to export (default: 200)
    pub(crate) sample_size: Option<usize>,
    /// Retrieval cutoff for leaderboard reporting template (default: 5)
    pub(crate) k: Option<usize>,
}
