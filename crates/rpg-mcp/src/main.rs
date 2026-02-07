//! RPG-Encoder MCP Server
//! Exposes SearchNode, FetchNode, ExploreRPG, BuildRPG, UpdateRPG as MCP tools over stdio.
//! Gives any connected LLM full semantic understanding of a codebase.

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::ServerInfo, tool,
    tool_handler, tool_router,
};
use rpg_core::config::RpgConfig;
use rpg_core::graph::RPGraph;
use rpg_core::storage;
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Cached lifting session — holds raw entities for stable batch indexing.
/// Without this, each `get_entities_for_lifting` call rebuilds the unlifted list,
/// causing batch indices to shift as entities get lifted between calls.
struct LiftingSession {
    scope_key: String,
    raw_entities: Vec<rpg_parser::entities::RawEntity>,
    batch_ranges: Vec<(usize, usize)>,
}

/// The RPG MCP server state.
#[derive(Clone)]
struct RpgServer {
    project_root: PathBuf,
    graph: Arc<RwLock<Option<RPGraph>>>,
    config: Arc<RwLock<RpgConfig>>,
    lifting_session: Arc<RwLock<Option<LiftingSession>>>,
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

impl std::fmt::Debug for RpgServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpgServer")
            .field("project_root", &self.project_root)
            .field("lifting_session", &"...")
            .finish()
    }
}

impl RpgServer {
    fn new(project_root: PathBuf) -> Self {
        let graph = storage::load(&project_root).ok();
        let config = RpgConfig::load(&project_root).unwrap_or_default();
        Self {
            project_root,
            graph: Arc::new(RwLock::new(graph)),
            config: Arc::new(RwLock::new(config)),
            lifting_session: Arc::new(RwLock::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    /// Check if the loaded graph is stale (behind git HEAD) and return a notice string.
    async fn staleness_notice(&self) -> String {
        let guard = self.graph.read().await;
        let Some(graph) = guard.as_ref() else {
            return String::new();
        };
        // Detect workdir changes (committed + staged + unstaged)
        let Ok(changes) = rpg_encoder::evolution::detect_workdir_changes(&self.project_root, graph)
        else {
            return String::new();
        };
        let languages = Self::resolve_languages(&graph.metadata);
        let source_changes = if languages.is_empty() {
            changes
        } else {
            rpg_encoder::evolution::filter_source_changes(changes, &languages)
        };
        if source_changes.is_empty() {
            return String::new();
        }
        format!(
            "[stale: {} source file(s) changed since graph was built — call update_rpg to sync]\n\n",
            source_changes.len(),
        )
    }

    /// Resolve all indexed languages from graph metadata (multi-language support).
    fn resolve_languages(
        metadata: &rpg_core::graph::GraphMetadata,
    ) -> Vec<rpg_parser::languages::Language> {
        use rpg_parser::languages::Language;
        if !metadata.languages.is_empty() {
            metadata
                .languages
                .iter()
                .filter_map(|n| Language::from_name(n))
                .collect()
        } else {
            // Backward compat: single-language graph
            Language::from_name(&metadata.language)
                .into_iter()
                .collect()
        }
    }

    async fn ensure_graph(&self) -> Result<(), String> {
        let read = self.graph.read().await;
        if read.is_some() {
            return Ok(());
        }
        drop(read);

        match storage::load(&self.project_root) {
            Ok(g) => {
                *self.graph.write().await = Some(g);
                Ok(())
            }
            Err(_) => {
                Err("No RPG found. Use the build_rpg tool to index this repository first.".into())
            }
        }
    }

    /// Detailed staleness info: which source files changed (committed + staged + unstaged).
    fn staleness_detail(&self, graph: &RPGraph) -> Option<String> {
        let changes =
            rpg_encoder::evolution::detect_workdir_changes(&self.project_root, graph).ok()?;
        let languages = Self::resolve_languages(&graph.metadata);
        let changes = rpg_encoder::evolution::filter_source_changes(changes, &languages);

        if changes.is_empty() {
            return None;
        }

        let graph_commit = graph.base_commit.as_deref().unwrap_or("unknown");
        let mut out = format!(
            "STALE ({} source file(s) changed since {})\n",
            changes.len(),
            &graph_commit[..8.min(graph_commit.len())],
        );
        for change in changes.iter().take(10) {
            let (label, path) = match change {
                rpg_encoder::evolution::FileChange::Added(p) => ("added", p.display().to_string()),
                rpg_encoder::evolution::FileChange::Modified(p) => {
                    ("modified", p.display().to_string())
                }
                rpg_encoder::evolution::FileChange::Deleted(p) => {
                    ("deleted", p.display().to_string())
                }
                rpg_encoder::evolution::FileChange::Renamed { from, to } => {
                    ("renamed", format!("{} -> {}", from.display(), to.display()))
                }
            };
            out.push_str(&format!("  {}: {}\n", label, path));
        }
        if changes.len() > 10 {
            out.push_str(&format!("  ... and {} more\n", changes.len() - 10));
        }
        Some(out)
    }

    async fn format_lifting_status(&self, graph: &RPGraph) -> Result<String, String> {
        let (lifted, total) = graph.lifting_coverage();
        let coverage_pct = if total > 0 {
            lifted as f64 / total as f64 * 100.0
        } else {
            0.0
        };

        let hierarchy_type = if graph.metadata.semantic_hierarchy {
            format!("semantic ({} areas)", graph.metadata.functional_areas)
        } else {
            "structural (file-path based)".to_string()
        };

        // Check staleness
        let stale_detail = self.staleness_detail(graph);
        let graph_line = match &stale_detail {
            Some(detail) => format!(
                "graph: {} ({} entities, {} files)",
                detail.trim(),
                graph.metadata.total_entities,
                graph.metadata.total_files
            ),
            None => format!(
                "graph: loaded ({} entities, {} files)",
                graph.metadata.total_entities, graph.metadata.total_files
            ),
        };

        let mut out = format!(
            "=== RPG Lifting Status ===\n\
             {}\n\
             coverage: {}/{} ({:.0}%)\n\
             hierarchy: {}\n",
            graph_line, lifted, total, coverage_pct, hierarchy_type,
        );

        // Per-area coverage
        let area_cov = graph.area_coverage();
        if !area_cov.is_empty() {
            out.push_str("\nPer-area coverage:\n");
            for (name, area_lifted, area_total) in &area_cov {
                let pct = if *area_total > 0 {
                    *area_lifted as f64 / *area_total as f64 * 100.0
                } else {
                    100.0
                };
                let marker = if *area_lifted < *area_total {
                    "  <- needs lifting"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "  {:<25} {}/{} ({:.0}%){}\n",
                    name, area_lifted, area_total, pct, marker,
                ));
            }
        }

        // Lifting session info
        let session = self.lifting_session.read().await;
        match session.as_ref() {
            Some(s) => {
                out.push_str(&format!(
                    "\nLifting session: active (scope=\"{}\", {} batches, {} entities cached)\n",
                    s.scope_key,
                    s.batch_ranges.len(),
                    s.raw_entities.len(),
                ));
            }
            None => {
                out.push_str("\nLifting session: inactive\n");
            }
        }
        drop(session);

        // Unlifted file breakdown (if any)
        if lifted < total {
            let unlifted = graph.unlifted_by_file();
            if !unlifted.is_empty() {
                let show = unlifted.len().min(8);
                out.push_str(&format!("\nUnlifted files ({} total):\n", unlifted.len()));
                for (file, ids) in unlifted.iter().take(show) {
                    let names: Vec<&str> = ids
                        .iter()
                        .take(3)
                        .map(|id| {
                            // Show qualified name (Class::method) not just method name
                            id.split_once(':').map_or(id.as_str(), |(_, n)| n)
                        })
                        .collect();
                    let suffix = if ids.len() > 3 {
                        format!(", ... +{}", ids.len() - 3)
                    } else {
                        String::new()
                    };
                    out.push_str(&format!(
                        "  {} ({} entities: {}{})\n",
                        file,
                        ids.len(),
                        names.join(", "),
                        suffix,
                    ));
                }
                if unlifted.len() > show {
                    out.push_str(&format!("  ... and {} more files\n", unlifted.len() - show));
                }
            }
        }

        // NEXT STEP — state machine guidance, staleness takes priority
        out.push('\n');
        if stale_detail.is_some() {
            out.push_str("NEXT STEP: Graph is stale. Call update_rpg to sync with code changes, then lift any new entities.\n");
        } else if lifted == 0 {
            out.push_str(
                "NEXT STEP: Call get_entities_for_lifting(scope=\"*\") to start lifting.\n",
            );
        } else if lifted < total {
            out.push_str(&format!(
                "NEXT STEP: {} entities remaining. Call get_entities_for_lifting(scope=\"*\") to continue lifting.\n",
                total - lifted,
            ));
        } else if !graph.metadata.semantic_hierarchy {
            out.push_str(
                "NEXT STEP: All entities lifted. Call finalize_lifting, then get_files_for_synthesis + submit_file_syntheses for holistic file-level features, then build_semantic_hierarchy + submit_hierarchy.\n",
            );
        } else {
            out.push_str(
                "NEXT STEP: All entities lifted. Graph is complete. Use search_node, fetch_node, explore_rpg to navigate.\n",
            );
        }

        Ok(out)
    }

    fn get_config_blocking(&self) -> RpgConfig {
        // For sync contexts where we need config but can't await
        RpgConfig::load(&self.project_root).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchNodeParams {
    /// The search query describing what you're looking for
    query: String,
    /// Search mode: 'features', 'snippets', or 'auto' (default: 'auto')
    mode: Option<String>,
    /// Optional hierarchy scope to restrict search (e.g., 'Security/auth'). Comma-separated for multiple scopes.
    scope: Option<String>,
    /// Filter to entities within a line range [start, end]
    line_nums: Option<Vec<usize>>,
    /// Glob pattern to filter entities by file path (e.g., "src/**/*.rs")
    file_pattern: Option<String>,
    /// Comma-separated entity type filter (e.g., "function,class,method"). Valid: function, class, method, file, module.
    entity_type_filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FetchNodeParams {
    /// The entity ID to fetch (e.g., 'src/auth.rs:validate_token')
    entity_id: String,
    /// Multiple entity IDs to fetch in batch (overrides entity_id when provided)
    entity_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExploreRpgParams {
    /// The entity ID to start exploration from
    entity_id: String,
    /// Multiple entity IDs to explore from in batch (overrides entity_id when provided)
    entity_ids: Option<Vec<String>>,
    /// Traversal direction: 'upstream', 'downstream', or 'both'
    direction: Option<String>,
    /// Maximum traversal depth (default: 2). Use -1 for unlimited depth.
    depth: Option<i64>,
    /// Filter edges by kind: 'imports', 'invokes', 'inherits', 'composes', or 'contains'
    edge_filter: Option<String>,
    /// Comma-separated entity type filter (e.g., "function,class,method"). Valid: function, class, method, file, module.
    entity_type_filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BuildRpgParams {
    /// Primary language override (auto-detected if not specified)
    language: Option<String>,
    /// Glob pattern to include files (e.g., "src/**/*.rs")
    include: Option<String>,
    /// Glob pattern to exclude files (e.g., "tests/**")
    exclude: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateRpgParams {
    /// Base commit SHA to diff from (defaults to RPG's stored base_commit)
    since: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetEntitiesForLiftingParams {
    /// Scope specifier: file glob ("src/auth/**"), hierarchy path, entity IDs, or "*"/"all".
    scope: String,
    /// Batch index to retrieve (0-based). Omit or 0 for first batch.
    batch_index: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SubmitLiftResultsParams {
    /// JSON object mapping function names to feature arrays.
    /// Example: {"my_func": ["validate input", "return result"], "other": ["compute hash"]}
    features: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SubmitHierarchyParams {
    /// JSON object mapping file paths to 3-level hierarchy paths.
    /// Example: {"src/auth/login.rs": "Authentication/manage sessions/handle login"}
    assignments: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetFilesForSynthesisParams {
    /// Batch index to retrieve (0-based). Omit or 0 for first batch.
    batch_index: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SubmitFileSynthesesParams {
    /// JSON object mapping file paths to comma-separated feature strings.
    /// Example: {"src/auth/login.rs": "handle user authentication, manage session tokens",
    ///           "src/db/query.rs": "build SQL queries, execute database operations"}
    syntheses: String,
}

/// Truncate source code to `max_lines`, preserving the signature and start of the body.
/// Appends a `(truncated)` note if the source exceeds the limit.
fn truncate_source(source: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if lines.len() <= max_lines {
        return source.to_string();
    }
    let mut out: String = lines[..max_lines].join("\n");
    out.push_str(&format!(
        "\n    // ... ({} more lines, truncated for context)",
        lines.len() - max_lines
    ));
    out
}

/// Parse a comma-separated entity type filter string into EntityKind values.
/// Accepts paper-specified names: function, class, method, file, module.
/// "file" is an alias for Module (file-level entity nodes, V_L).
/// Note: "directory" is not a V_L entity kind — hierarchy nodes (V_H) are
/// traversed via Contains edges but are not subject to entity_type_filter.
fn parse_entity_type_filter(filter: &str) -> Vec<rpg_core::graph::EntityKind> {
    filter
        .split(',')
        .filter_map(|s| match s.trim().to_lowercase().as_str() {
            "function" => Some(rpg_core::graph::EntityKind::Function),
            "class" => Some(rpg_core::graph::EntityKind::Class),
            "method" => Some(rpg_core::graph::EntityKind::Method),
            "module" | "file" => Some(rpg_core::graph::EntityKind::Module),
            _ => None,
        })
        .collect()
}

#[tool_router]
impl RpgServer {
    #[tool(
        description = "Search for code entities by intent or keywords. Returns entities with file paths, line numbers, and relevance scores. Use mode='features' for semantic intent search (use behavioral/functional phrases as query), 'snippets' for name/path matching (use file paths, qualified entities, or keywords as query), 'auto' (default) tries both."
    )]
    async fn search_node(
        &self,
        Parameters(params): Parameters<SearchNodeParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let notice = self.staleness_notice().await;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();
        let config = self.config.read().await;

        let search_mode = match params.mode.as_deref() {
            Some("features") => rpg_nav::search::SearchMode::Features,
            Some("snippets") => rpg_nav::search::SearchMode::Snippets,
            _ => rpg_nav::search::SearchMode::Auto,
        };

        let line_nums = params.line_nums.as_ref().and_then(|v| {
            if v.len() == 2 {
                Some((v[0], v[1]))
            } else {
                None
            }
        });

        let entity_type_filter = params
            .entity_type_filter
            .as_deref()
            .map(parse_entity_type_filter)
            .filter(|v| !v.is_empty());

        let results = rpg_nav::search::search_with_params(
            graph,
            &rpg_nav::search::SearchParams {
                query: &params.query,
                mode: search_mode,
                scope: params.scope.as_deref(),
                limit: config.navigation.search_result_limit,
                line_nums,
                file_pattern: params.file_pattern.as_deref(),
                entity_type_filter,
            },
        );

        if results.is_empty() {
            return Ok(format!("{}No results found for: {}", notice, params.query));
        }

        Ok(format!(
            "{}{}",
            notice,
            rpg_nav::toon::format_search_results(&results)
        ))
    }

    #[tool(
        description = "Fetch detailed metadata and source code for a known entity. Returns the entity's semantic features, dependencies (what it calls, what calls it), hierarchy position, and full source code."
    )]
    async fn fetch_node(
        &self,
        Parameters(params): Parameters<FetchNodeParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let notice = self.staleness_notice().await;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let ids: Vec<&str> = if let Some(ref batch) = params.entity_ids {
            batch.iter().map(|s| s.as_str()).collect()
        } else {
            vec![params.entity_id.as_str()]
        };

        let mut outputs = Vec::new();
        for id in &ids {
            match rpg_nav::fetch::fetch(graph, id, &self.project_root) {
                Ok(output) => outputs.push(rpg_nav::toon::format_fetch_output(&output)),
                Err(e) => outputs.push(format!("error({}): {}", id, e)),
            }
        }

        Ok(format!("{}{}", notice, outputs.join("\n---\n")))
    }

    #[tool(
        description = "Explore the dependency graph starting from an entity. Traverses import, invocation, inheritance, and composition edges. Use direction='downstream' to see what the entity calls, 'upstream' to see what calls it, 'both' for full picture."
    )]
    async fn explore_rpg(
        &self,
        Parameters(params): Parameters<ExploreRpgParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let notice = self.staleness_notice().await;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let dir = match params.direction.as_deref() {
            Some("upstream" | "up") => rpg_nav::explore::Direction::Upstream,
            Some("both") => rpg_nav::explore::Direction::Both,
            _ => rpg_nav::explore::Direction::Downstream,
        };

        let max_depth = match params.depth {
            Some(-1) => usize::MAX, // Unlimited depth per paper spec
            Some(d) if d >= 0 => usize::try_from(d).unwrap_or(2),
            _ => 2, // Default
        };

        let edge_filter = params.edge_filter.as_deref().and_then(|f| match f {
            "imports" => Some(rpg_core::graph::EdgeKind::Imports),
            "invokes" => Some(rpg_core::graph::EdgeKind::Invokes),
            "inherits" => Some(rpg_core::graph::EdgeKind::Inherits),
            "composes" => Some(rpg_core::graph::EdgeKind::Composes),
            "contains" => Some(rpg_core::graph::EdgeKind::Contains),
            _ => None,
        });

        let entity_type_filter = params
            .entity_type_filter
            .as_deref()
            .map(parse_entity_type_filter)
            .filter(|v| !v.is_empty());

        let ids: Vec<&str> = if let Some(ref batch) = params.entity_ids {
            batch.iter().map(|s| s.as_str()).collect()
        } else {
            vec![params.entity_id.as_str()]
        };

        let mut outputs = Vec::new();
        for id in &ids {
            match rpg_nav::explore::explore_filtered(
                graph,
                id,
                dir,
                max_depth,
                edge_filter,
                entity_type_filter.as_deref(),
            ) {
                Some(tree) => outputs.push(rpg_nav::explore::format_tree(&tree, 0)),
                None => outputs.push(format!("Entity not found: {}", id)),
            }
        }

        if outputs.is_empty() {
            Err("No entities found".to_string())
        } else {
            Ok(format!("{}{}", notice, outputs.join("\n")))
        }
    }

    #[tool(
        description = "Get RPG statistics: entity count, file count, functional areas, dependency edges, containment edges, and hierarchy overview. Use this first to understand the codebase structure before searching."
    )]
    async fn rpg_info(&self) -> Result<String, String> {
        self.ensure_graph().await?;
        let notice = self.staleness_notice().await;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();
        Ok(format!(
            "{}{}",
            notice,
            rpg_nav::toon::format_rpg_info(graph)
        ))
    }

    #[tool(
        description = "Build an RPG (Repository Planning Graph) from the codebase. Indexes all code entities, builds a file-path hierarchy, and resolves dependencies. Completes in seconds without requiring an LLM. To add semantic features (LLM-extracted intent descriptions), use get_entities_for_lifting afterwards. Run this once when first connecting to a repository."
    )]
    async fn build_rpg(
        &self,
        Parameters(params): Parameters<BuildRpgParams>,
    ) -> Result<String, String> {
        use rpg_parser::languages::Language;

        let project_root = &self.project_root;

        // Detect languages (multi-language support)
        let languages: Vec<Language> = if let Some(ref l) = params.language {
            // User specified a single language override
            let lang = Language::from_name(l)
                .or_else(|| Language::from_extension(l))
                .ok_or_else(|| format!("unsupported language: {}", l))?;
            vec![lang]
        } else {
            let detected = Language::detect_all(project_root);
            if detected.is_empty() {
                return Err(
                    "could not detect any supported language; specify the 'language' parameter"
                        .to_string(),
                );
            }
            detected
        };

        // Auto-preserve: load existing graph if it has lifted features
        let old_graph: Option<RPGraph> = if storage::rpg_exists(project_root) {
            match storage::load(project_root) {
                Ok(g) if g.metadata.lifted_entities > 0 => {
                    // Backup before overwriting
                    let _ = storage::create_backup(project_root);
                    Some(g)
                }
                _ => None,
            }
        } else {
            None
        };

        let primary = languages[0].name();
        let mut graph = RPGraph::new(primary);
        graph.metadata.languages = languages.iter().map(|l| l.name().to_string()).collect();

        // Parse code entities (all detected languages)
        let include_glob = params
            .include
            .as_deref()
            .and_then(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()));
        let exclude_glob = params
            .exclude
            .as_deref()
            .and_then(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()));

        let walker = ignore::WalkBuilder::new(project_root)
            .hidden(true)
            .git_ignore(true)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let Some(file_lang) = Language::from_extension(ext) else {
                continue;
            };
            if !languages.contains(&file_lang) {
                continue;
            }
            let rel_path_for_glob = path.strip_prefix(project_root).unwrap_or(path);
            if let Some(ref inc) = include_glob
                && !inc.is_match(rel_path_for_glob)
            {
                continue;
            }
            if let Some(ref exc) = exclude_glob
                && exc.is_match(rel_path_for_glob)
            {
                continue;
            }

            let Ok(source) = std::fs::read_to_string(path) else {
                continue;
            };

            let rel_path = path.strip_prefix(project_root).unwrap_or(path);
            let raw_entities = rpg_parser::entities::extract_entities(rel_path, &source, file_lang);
            for raw in raw_entities {
                graph.insert_entity(raw.into_entity());
            }
        }

        // Create Module entities for file-level nodes (paper §3.1)
        graph.create_module_entities();

        // Structural hierarchy from file paths (no LLM needed)
        graph.build_file_path_hierarchy();

        // Hierarchy enrichment
        graph.assign_hierarchy_ids();
        graph.aggregate_hierarchy_features();
        graph.materialize_containment_edges();

        // Artifact grounding + dependency resolution
        let cfg = self.get_config_blocking();
        rpg_encoder::grounding::populate_entity_deps(
            &mut graph,
            project_root,
            cfg.encoding.broadcast_imports,
            None,
        );
        rpg_encoder::grounding::ground_hierarchy(&mut graph);
        rpg_encoder::grounding::resolve_dependencies(&mut graph);

        // Set git commit
        if let Ok(sha) = rpg_encoder::evolution::get_head_sha(project_root) {
            graph.base_commit = Some(sha);
        }

        // Merge old features into new graph (auto-preservation)
        let features_restored = if let Some(ref old) = old_graph {
            rpg_encoder::evolution::merge_features(&mut graph, old)
        } else {
            0
        };

        // Refresh metadata and save
        graph.refresh_metadata();
        storage::save(project_root, &graph).map_err(|e| format!("Failed to save RPG: {}", e))?;
        let _ = storage::ensure_gitignore(project_root);

        // Update in-memory state
        let meta = graph.metadata.clone();
        *self.graph.write().await = Some(graph);

        let lang_display = if languages.len() == 1 {
            languages[0].name().to_string()
        } else {
            languages
                .iter()
                .map(|l| l.name())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut result = format!(
            "RPG built successfully (structural).\n\
             languages: {}\n\
             entities: {}\n\
             files: {}\n\
             functional_areas: {}\n\
             dependency_edges: {}\n\
             containment_edges: {}\n\
             lifted: {}/{}\n\
             hierarchy: structural",
            lang_display,
            meta.total_entities,
            meta.total_files,
            meta.functional_areas,
            meta.dependency_edges,
            meta.containment_edges,
            meta.lifted_entities,
            meta.total_entities,
        );

        if features_restored > 0 {
            result.push_str(&format!(
                "\n\nAuto-preserved {} entities with semantic features from previous graph.\n\
                 Backup saved to .rpg/graph.backup.json",
                features_restored
            ));
        } else {
            result.push_str(
                "\nTip: use get_entities_for_lifting + submit_lift_results to add semantic features.",
            );
        }
        Ok(result)
    }

    #[tool(
        description = "Check lifting progress: coverage per area, unlifted files, active session, and NEXT STEP. Call this at any point to see where you are in the lifting flow. Reads state from the persisted graph — works across sessions."
    )]
    async fn lifting_status(&self) -> Result<String, String> {
        // Check if graph is loaded
        let guard = self.graph.read().await;
        if guard.is_some() {
            let graph = guard.as_ref().unwrap();
            return self.format_lifting_status(graph).await;
        }
        drop(guard);

        // Try loading from disk
        if storage::rpg_exists(&self.project_root) {
            self.ensure_graph().await?;
            let guard = self.graph.read().await;
            let graph = guard.as_ref().unwrap();
            return self.format_lifting_status(graph).await;
        }

        Ok(
            "=== RPG Lifting Status ===\ngraph: not built\n\nNEXT STEP: Call build_rpg to index the codebase."
                .to_string(),
        )
    }

    #[tool(
        description = "LIFTER PROTOCOL step 1: Get a batch of code entities for YOU to semantically analyze. Returns source code with instructions. After analyzing ALL entities, call submit_lift_results with your features JSON, then check the NEXT_ACTION block and continue until DONE. Scope: file glob ('src/auth/**'), '*' for all unlifted, or hierarchy path. No LLM setup needed."
    )]
    async fn get_entities_for_lifting(
        &self,
        Parameters(params): Parameters<GetEntitiesForLiftingParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let batch_index = params.batch_index.unwrap_or(0);

        // On batch_index=0 (or when scope changes), rebuild the session cache.
        // This captures ALL unlifted entities at that moment so subsequent
        // batch indices work against a stable list — even as entities get
        // lifted between calls.
        {
            let mut session = self.lifting_session.write().await;
            let needs_rebuild = match session.as_ref() {
                None => true,
                Some(s) => s.scope_key != params.scope || batch_index == 0,
            };

            if needs_rebuild {
                let guard = self.graph.read().await;
                let graph = guard.as_ref().unwrap();

                let scope = rpg_encoder::lift::resolve_scope(graph, &params.scope);
                if scope.entity_ids.is_empty() {
                    *session = None;
                    return Ok(format!(
                        "No entities matched scope: {}\nTry a file glob like 'src/**' or '*' for all.",
                        params.scope
                    ));
                }

                let raw_entities =
                    rpg_encoder::lift::collect_raw_entities(graph, &scope, &self.project_root)
                        .map_err(|e| format!("Failed to collect entities: {}", e))?;

                if raw_entities.is_empty() {
                    *session = None;
                    return Ok("No source code found for matched entities.".into());
                }

                let config = self.config.read().await;
                let batch_size = config.encoding.batch_size;
                let max_batch_tokens = config.encoding.max_batch_tokens;
                drop(config);

                let mcp_batch_size = batch_size.min(25);
                let batch_ranges = rpg_encoder::lift::build_token_aware_batches(
                    &raw_entities,
                    mcp_batch_size,
                    max_batch_tokens,
                );

                *session = Some(LiftingSession {
                    scope_key: params.scope.clone(),
                    raw_entities,
                    batch_ranges,
                });
            }
        }

        let session = self.lifting_session.read().await;
        let session = session.as_ref().unwrap();

        let total_batches = session.batch_ranges.len();
        if batch_index >= total_batches {
            return Ok(format!(
                "DONE — all {} batches processed. No more entities to lift for this scope.",
                total_batches
            ));
        }

        let (batch_start, batch_end) = session.batch_ranges[batch_index];
        let batch = &session.raw_entities[batch_start..batch_end];

        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let (lifted, total) = graph.lifting_coverage();

        let module_count = graph
            .entities
            .values()
            .filter(|e| e.kind == rpg_core::graph::EntityKind::Module)
            .count();
        let mut output = format!(
            "BATCH {}/{} ({} entities) | coverage: {}/{} (excludes {} modules)\n",
            batch_index + 1,
            total_batches,
            batch.len(),
            lifted,
            total,
            module_count,
        );

        // Only include repo context and full instructions on batch 0 to save context space
        if batch_index == 0 {
            let project_name = self
                .project_root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let repo_info = rpg_encoder::lift::generate_repo_info(graph, project_name);
            output.push_str(&repo_info);
            output.push_str("\n\n");
            output.push_str(rpg_encoder::semantic_lifting::SEMANTIC_PARSING_SYSTEM);
            output.push('\n');
        }
        output.push_str("\n## Code\n\n");

        // Truncate source to prevent context overflow — signature + key logic
        // is enough for semantic feature extraction.
        for entity in batch {
            let truncated = truncate_source(&entity.source_text, 40);
            output.push_str(&format!(
                "### {} ({:?})\n```\n{}\n```\n\n",
                entity.id(),
                entity.kind,
                truncated,
            ));
        }

        output.push_str(
            "Submit: call `submit_lift_results` with JSON keys exactly as shown in the ### headers above (e.g., `{\"src/lib.rs:MyStruct::method\": [\"feature1\", ...]}`).\n\n"
        );

        if batch_index + 1 < total_batches {
            output.push_str(&format!(
                "NEXT: `get_entities_for_lifting(scope=\"{}\", batch_index={})`  ({} remaining)\n",
                params.scope,
                batch_index + 1,
                total_batches - batch_index - 1,
            ));
        } else {
            output.push_str("DONE — last batch. After submitting, call `finalize_lifting` to build the semantic hierarchy.\n");
        }

        Ok(output)
    }

    #[tool(
        description = "LIFTER PROTOCOL step 2: Submit semantic features you extracted. Pass a JSON object with keys exactly as shown by get_entities_for_lifting headers. For methods use file:Class::method format. Example: {\"src/main.rs:Server::new\": [\"create server\"], \"src/lib.rs:load\": [\"load config\"]}. After submitting, immediately proceed to the next batch — do NOT stop to ask the user."
    )]
    async fn submit_lift_results(
        &self,
        Parameters(params): Parameters<SubmitLiftResultsParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let mut features: std::collections::HashMap<String, Vec<String>> =
            serde_json::from_str(&params.features)
                .map_err(|e| format!("Invalid features JSON: {}", e))?;

        // Normalize per paper: trim, lowercase, dedup
        rpg_encoder::semantic_lifting::normalize_features(&mut features);

        let mut guard = self.graph.write().await;
        let graph = guard.as_mut().ok_or("No RPG loaded")?;

        let config = self.get_config_blocking();
        let drift_threshold = config.encoding.drift_threshold;

        let mut updated = 0usize;
        let mut unmatched = 0usize;
        let mut drift_reports: Vec<String> = Vec::new();
        let mut drifted_ids: Vec<String> = Vec::new();

        for (key, feats) in &features {
            if feats.is_empty() {
                continue;
            }

            // Find matching entities: direct ID match first, then file:name scan (all matches)
            let entity_ids: Vec<String> = if graph.entities.contains_key(key) {
                vec![key.clone()]
            } else if let Some((file_part, name_part)) = key.rsplit_once(':') {
                graph
                    .entities
                    .iter()
                    .filter(|(_, e)| {
                        e.name == name_part && e.file.to_string_lossy().as_ref() == file_part
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            } else {
                vec![]
            };

            if entity_ids.is_empty() {
                unmatched += 1;
            } else {
                for eid in &entity_ids {
                    // Drift detection: check old features before overwriting
                    let old_feats = graph
                        .entities
                        .get(eid)
                        .map(|e| e.semantic_features.clone())
                        .unwrap_or_default();

                    if !old_feats.is_empty() {
                        let drift = rpg_encoder::evolution::compute_drift(&old_feats, feats);
                        if drift > drift_threshold {
                            drift_reports
                                .push(format!("  {} drifted ({:.2}) — re-routing", eid, drift));
                            drifted_ids.push(eid.clone());
                            graph.remove_entity_from_hierarchy(eid);
                        } else if drift > 0.0 {
                            drift_reports.push(format!(
                                "  {} updated ({:.2} drift, below threshold)",
                                eid, drift,
                            ));
                        }
                    }

                    if let Some(entity) = graph.entities.get_mut(eid) {
                        entity.semantic_features = feats.clone();
                        updated += 1;
                    }
                }
            }
        }

        // Re-aggregate hierarchy features
        graph.aggregate_hierarchy_features();
        graph.refresh_metadata();

        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        let (lifted, total) = graph.lifting_coverage();
        let coverage_pct = if total > 0 {
            lifted as f64 / total as f64 * 100.0
        } else {
            0.0
        };

        let mut result = format!(
            "Applied {} feature sets ({} matched, {} unmatched).\ncoverage: {}/{} ({:.0}%)",
            features.len(),
            updated,
            unmatched,
            lifted,
            total,
            coverage_pct,
        );
        if unmatched > 0 {
            result.push_str("\nNote: Unmatched keys must match headers from get_entities_for_lifting (e.g., \"src/main.rs:MyStruct::method\" for methods).");
        }

        // Drift reports (when re-lifting modified entities)
        if !drift_reports.is_empty() {
            result.push_str(&format!(
                "\n\nDrift detection ({} entities re-lifted):\n",
                drift_reports.len()
            ));
            for report in &drift_reports {
                result.push_str(report);
                result.push('\n');
            }
            if !drifted_ids.is_empty() {
                result.push_str(&format!(
                    "{} entity(ies) drifted above threshold ({:.2}) — removed from hierarchy, will need re-routing via submit_hierarchy.\n",
                    drifted_ids.len(),
                    drift_threshold,
                ));
            }
        }

        // Per-area coverage breakdown
        let area_cov = graph.area_coverage();
        if !area_cov.is_empty() {
            result.push_str("\n\nPer-area:");
            for (name, area_lifted, area_total) in &area_cov {
                if *area_total == 0 {
                    continue;
                }
                let pct = *area_lifted as f64 / *area_total as f64 * 100.0;
                let marker = if *area_lifted == *area_total {
                    " done"
                } else {
                    ""
                };
                result.push_str(&format!(
                    "\n  {}: {}/{} ({:.0}%){}",
                    name, area_lifted, area_total, pct, marker,
                ));
            }
        }

        // Unlifted sample for retry
        if lifted < total {
            let unlifted: Vec<&str> = graph
                .entities
                .iter()
                .filter(|(_, e)| {
                    e.semantic_features.is_empty() && e.kind != rpg_core::graph::EntityKind::Module
                })
                .map(|(id, _)| id.as_str())
                .take(10)
                .collect();
            if !unlifted.is_empty() {
                result.push_str(&format!(
                    "\n\nunlifted_sample ({}+ remaining): {}",
                    total - lifted,
                    unlifted.join(", "),
                ));
            }
        }

        // NEXT action
        if lifted < total {
            result.push_str("\nNEXT: continue with get_entities_for_lifting, then call finalize_lifting when done.");
        } else {
            result.push_str("\nDONE: all entities lifted. Call finalize_lifting to build the semantic hierarchy.");
        }
        Ok(result)
    }

    #[tool(
        description = "Incrementally update the RPG from git changes since the last build. Detects added, modified, deleted, and renamed files, re-extracts entities, and updates structural metadata. Modified entities with stale features are tracked for interactive re-lifting. Much faster than a full rebuild."
    )]
    async fn update_rpg(
        &self,
        Parameters(params): Parameters<UpdateRpgParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let mut graph = self.graph.write().await;
        let g = graph.as_mut().ok_or("No RPG loaded")?;

        let summary =
            rpg_encoder::evolution::run_update(g, &self.project_root, params.since.as_deref())
                .map_err(|e| format!("Update failed: {}", e))?;

        storage::save(&self.project_root, g).map_err(|e| format!("Failed to save RPG: {}", e))?;

        // Clear lifting session — entity list changed
        *self.lifting_session.write().await = None;

        if summary.entities_added == 0
            && summary.entities_modified == 0
            && summary.entities_removed == 0
        {
            Ok("RPG is up to date. No source changes detected.".into())
        } else {
            // Count entities needing lifting (new/modified with empty features)
            let (lifted, total) = g.lifting_coverage();
            let needs_lifting = total - lifted;

            let mut result = format!(
                "RPG updated.\n\
                 entities_added: {}\n\
                 entities_modified: {}\n\
                 entities_removed: {}\n\
                 edges_added: {}\n\
                 edges_removed: {}",
                summary.entities_added,
                summary.entities_modified,
                summary.entities_removed,
                summary.edges_added,
                summary.edges_removed,
            );

            let needs_relift = summary.modified_entity_ids.len();

            if needs_lifting > 0 || needs_relift > 0 {
                if needs_lifting > 0 {
                    result.push_str(&format!("\nneeds_lifting: {}", needs_lifting));
                }
                if needs_relift > 0 {
                    result.push_str(&format!(
                        "\nneeds_relift: {} (modified entities with stale features)",
                        needs_relift,
                    ));
                }
                result.push_str("\n\nNEXT STEP: Call lifting_status to see what needs lifting, then get_entities_for_lifting to lift/re-lift them.");
            } else {
                result.push_str("\n\nAll entities have features. Graph is up to date.");
            }

            Ok(result)
        }
    }

    #[tool(
        description = "Reload the RPG graph from disk. Use after external changes to .rpg/graph.json."
    )]
    async fn reload_rpg(&self) -> Result<String, String> {
        match storage::load(&self.project_root) {
            Ok(g) => {
                let entities = g.metadata.total_entities;
                *self.graph.write().await = Some(g);
                Ok(format!("RPG reloaded. {} entities loaded.", entities))
            }
            Err(e) => Err(format!("Failed to reload RPG: {}", e)),
        }
    }

    #[tool(
        description = "Finalize the lifting process: aggregate file-level features onto Module entities and re-ground artifacts. Call this AFTER all entities have been lifted via submit_lift_results. No LLM needed — uses dedup-aggregation of already-lifted entity features. After finalizing, proceed to get_files_for_synthesis for holistic file-level features, then build_semantic_hierarchy + submit_hierarchy."
    )]
    async fn finalize_lifting(&self) -> Result<String, String> {
        self.ensure_graph().await?;

        let mut guard = self.graph.write().await;
        let graph = guard.as_mut().ok_or("No RPG loaded")?;

        let (lifted, _total) = graph.lifting_coverage();
        if lifted == 0 {
            return Err("No entities have been lifted yet. Run get_entities_for_lifting + submit_lift_results first.".into());
        }

        // Clear lifting session cache
        *self.lifting_session.write().await = None;

        let mut steps: Vec<String> = Vec::new();

        // Step 1: File-level feature synthesis — aggregate child entity features into modules.
        // No external LLM needed: dedup-aggregates from already-lifted entity features.
        graph.aggregate_module_features();
        steps.push("file_synthesis: dedup aggregation".into());

        // Step 2: Re-enrich hierarchy metadata and grounding
        graph.assign_hierarchy_ids();
        graph.aggregate_hierarchy_features();
        graph.materialize_containment_edges();
        rpg_encoder::grounding::ground_hierarchy(graph);
        graph.refresh_metadata();

        // Save
        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        let (final_lifted, final_total) = graph.lifting_coverage();
        let coverage_pct = if final_total > 0 {
            final_lifted as f64 / final_total as f64 * 100.0
        } else {
            0.0
        };

        let hierarchy_label = if graph.metadata.semantic_hierarchy {
            "semantic"
        } else {
            "structural"
        };

        let mut result = format!(
            "Finalization complete.\ncoverage: {}/{} ({:.0}%)\nhierarchy: {}\nsteps:\n",
            final_lifted, final_total, coverage_pct, hierarchy_label,
        );
        for step in &steps {
            result.push_str(&format!("  - {}\n", step));
        }

        // Per-area coverage
        let area_cov = graph.area_coverage();
        if !area_cov.is_empty() {
            result.push_str("\nPer-area coverage:\n");
            for (name, area_lifted, area_total) in &area_cov {
                if *area_total == 0 {
                    continue;
                }
                let pct = *area_lifted as f64 / *area_total as f64 * 100.0;
                result.push_str(&format!(
                    "  {}: {}/{} ({:.0}%)\n",
                    name, area_lifted, area_total, pct,
                ));
            }
        }

        // NEXT STEP
        if final_lifted < final_total {
            result.push_str(&format!(
                "\nNEXT STEP: {} entities still unlifted. Call get_entities_for_lifting(scope=\"*\") to continue.\n",
                final_total - final_lifted,
            ));
        } else if !graph.metadata.semantic_hierarchy {
            result.push_str(
                "\nNEXT STEP: Call get_files_for_synthesis to produce holistic file-level features (improves hierarchy quality), then build_semantic_hierarchy + submit_hierarchy.\n",
            );
        } else {
            result.push_str(
                "\nNEXT STEP: Graph is complete. Use search_node, fetch_node, explore_rpg to navigate.\n",
            );
        }

        Ok(result)
    }

    #[tool(
        description = "SYNTHESIS PROTOCOL step 1: Get file-level entity features for YOU to synthesize into holistic file features. Each file's child entities (functions, classes, methods) have already been lifted with verb-object features. Your job: read the entity features and synthesize them into 3-6 comma-separated high-level features for the FILE as a whole. Returns batched data. After synthesizing, call submit_file_syntheses with your results."
    )]
    async fn get_files_for_synthesis(
        &self,
        Parameters(params): Parameters<GetFilesForSynthesisParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        // Collect files that have lifted child entities
        #[allow(clippy::type_complexity)]
        let mut file_data: Vec<(String, Vec<(String, Vec<String>)>)> = Vec::new();

        for (file, ids) in &graph.file_index {
            let child_features: Vec<(String, Vec<String>)> = ids
                .iter()
                .filter_map(|id| {
                    let e = graph.entities.get(id)?;
                    if e.kind == rpg_core::graph::EntityKind::Module
                        || e.semantic_features.is_empty()
                    {
                        return None;
                    }
                    Some((e.name.clone(), e.semantic_features.clone()))
                })
                .collect();

            if !child_features.is_empty() {
                file_data.push((file.display().to_string(), child_features));
            }
        }

        file_data.sort_by(|a, b| a.0.cmp(&b.0));

        if file_data.is_empty() {
            return Ok("No files with lifted entities found. Run the lifting flow first.".into());
        }

        // Batch: ~5 files per batch to keep context manageable
        let batch_size = 8;
        let total_batches = file_data.len().div_ceil(batch_size);
        let batch_index = params.batch_index.unwrap_or(0);

        if batch_index >= total_batches {
            return Ok(format!(
                "DONE — all {} batches processed. Call build_semantic_hierarchy to construct the hierarchy.",
                total_batches
            ));
        }

        let start = batch_index * batch_size;
        let end = (start + batch_size).min(file_data.len());
        let batch = &file_data[start..end];

        let mut output = format!(
            "BATCH {}/{} ({} files) | total files: {}\n\n",
            batch_index + 1,
            total_batches,
            batch.len(),
            file_data.len(),
        );

        if batch_index == 0 {
            output.push_str("## Instructions\n\n");
            output.push_str("For each file below, read its entity features and synthesize them into 3-6 comma-separated\n");
            output.push_str("high-level features for the FILE as a whole. This is NOT a bag of features — merge and abstract\n");
            output.push_str(
                "the individual entity features into higher-level file responsibilities.\n\n",
            );
            output.push_str("Submit as: `submit_file_syntheses({\"path/to/file.rs\": \"feature1, feature2, feature3\", ...})`\n\n");
        }

        output.push_str("## Files\n\n");
        for (file_path, child_features) in batch {
            output.push_str(&format!("### {}\n", file_path));
            for (name, features) in child_features {
                output.push_str(&format!("  - {}: {}\n", name, features.join(", ")));
            }
            output.push('\n');
        }

        if batch_index + 1 < total_batches {
            output.push_str(&format!(
                "NEXT: `get_files_for_synthesis(batch_index={})` ({} batches remaining)\n",
                batch_index + 1,
                total_batches - batch_index - 1,
            ));
        } else {
            output.push_str("DONE — last batch. After submitting, call `build_semantic_hierarchy` to construct domain areas.\n");
        }

        Ok(output)
    }

    #[tool(
        description = "SYNTHESIS PROTOCOL step 2: Submit your holistic file-level features. Pass a JSON object mapping file paths to comma-separated feature strings. These replace the dedup-aggregated Module features with your synthesized features, improving hierarchy quality. Example: {\"src/auth.rs\": \"handle user authentication, manage session tokens\", \"src/db.rs\": \"manage database connections, execute queries\"}"
    )]
    async fn submit_file_syntheses(
        &self,
        Parameters(params): Parameters<SubmitFileSynthesesParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let syntheses: std::collections::HashMap<String, String> =
            serde_json::from_str(&params.syntheses)
                .map_err(|e| format!("Invalid syntheses JSON: {}", e))?;

        let mut guard = self.graph.write().await;
        let graph = guard.as_mut().ok_or("No RPG loaded")?;

        let mut updated = 0usize;
        let mut unmatched = Vec::new();

        for (file_path, summary) in &syntheses {
            if summary.trim().is_empty() {
                continue;
            }

            // Find the Module entity for this file
            let module_id = graph.file_index.iter().find_map(|(file, ids)| {
                let file_str = file.display().to_string();
                if file_str == *file_path
                    || file_str.ends_with(file_path)
                    || file_path.ends_with(&file_str)
                {
                    ids.iter()
                        .find(|id| {
                            graph
                                .entities
                                .get(id.as_str())
                                .is_some_and(|e| e.kind == rpg_core::graph::EntityKind::Module)
                        })
                        .cloned()
                } else {
                    None
                }
            });

            if let Some(module_id) = module_id {
                if let Some(module) = graph.entities.get_mut(&module_id) {
                    // Parse comma-separated features from the summary, or use as single feature
                    let features: Vec<String> = summary
                        .split(',')
                        .map(|f| f.trim().to_lowercase())
                        .filter(|f| !f.is_empty())
                        .collect();

                    if !features.is_empty() {
                        module.semantic_features = features;
                        updated += 1;
                    }
                }
            } else {
                unmatched.push(file_path.clone());
            }
        }

        // Re-aggregate hierarchy features with new module features
        graph.aggregate_hierarchy_features();
        graph.refresh_metadata();

        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        let total_modules = graph
            .entities
            .values()
            .filter(|e| {
                e.kind == rpg_core::graph::EntityKind::Module && !e.semantic_features.is_empty()
            })
            .count();

        let mut result = format!(
            "Applied {} file syntheses ({} matched, {} unmatched).\ntotal_synthesized_modules: {}\n",
            syntheses.len(),
            updated,
            unmatched.len(),
            total_modules,
        );

        if !unmatched.is_empty() {
            result.push_str(&format!(
                "unmatched_files: {}\n",
                unmatched
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        result.push_str("\nNEXT STEP: Call build_semantic_hierarchy to construct domain areas, then submit_hierarchy to apply them.\n");

        Ok(result)
    }

    #[tool(
        description = "Get file-level features and instructions for building a semantic hierarchy. Returns Module (file) entities with their aggregated features, plus the domain discovery and hierarchy assignment prompts. YOU (the LLM) analyze the features, identify functional domains, and assign each file to a 3-level hierarchy path. Then call submit_hierarchy with your assignments."
    )]
    async fn build_semantic_hierarchy(&self) -> Result<String, String> {
        self.ensure_graph().await?;

        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let (lifted, total) = graph.lifting_coverage();
        let coverage_pct = if total > 0 {
            lifted as f64 / total as f64 * 100.0
        } else {
            0.0
        };

        if coverage_pct < 30.0 || lifted < 20 {
            return Err(format!(
                "Insufficient coverage for semantic hierarchy: {}/{} ({:.0}%). Need ≥30% and ≥20 lifted entities.\nRun get_entities_for_lifting + submit_lift_results first.",
                lifted, total, coverage_pct
            ));
        }

        let repo_name = self
            .project_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        // Collect Module entities with features (file-level features)
        let mut file_features = String::new();
        let mut module_count = 0usize;
        for entity in graph.entities.values() {
            if entity.kind == rpg_core::graph::EntityKind::Module
                && !entity.semantic_features.is_empty()
            {
                file_features.push_str(&format!(
                    "- {} ({}): {}\n",
                    entity.name,
                    entity.file.display(),
                    entity.semantic_features.join(", ")
                ));
                module_count += 1;
            }
        }

        // If no modules have features, fall back to entity features grouped by file
        if module_count == 0 {
            let mut file_map: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for entity in graph.entities.values() {
                if !entity.semantic_features.is_empty() {
                    file_map
                        .entry(entity.file.display().to_string())
                        .or_default()
                        .extend(entity.semantic_features.clone());
                }
            }
            for (file, features) in &file_map {
                let deduped: Vec<&String> = features.iter().collect();
                file_features.push_str(&format!(
                    "- {}: {}\n",
                    file,
                    deduped
                        .iter()
                        .take(5)
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                module_count += 1;
            }
        }

        let domain_prompt =
            include_str!("../../../crates/rpg-encoder/src/prompts/domain_discovery.md");
        let hierarchy_prompt =
            include_str!("../../../crates/rpg-encoder/src/prompts/hierarchy_construction.md");

        let mut output = String::new();
        output.push_str(&format!(
            "## Semantic Hierarchy Construction for '{}'\n\n",
            repo_name
        ));
        output.push_str(&format!(
            "Coverage: {}/{} ({:.0}%) | Files with features: {}\n\n",
            lifted, total, coverage_pct, module_count
        ));

        output.push_str("## Step 1: Domain Discovery\n\n");
        output.push_str(domain_prompt);
        output.push_str("\n\n### File-Level Features\n");
        output.push_str(&file_features);

        output.push_str("\n\n## Step 2: Hierarchy Assignment\n\n");
        output.push_str(hierarchy_prompt);

        output.push_str("\n\n---\n\n");
        output.push_str("## Instructions\n\n");
        output.push_str("1. Read the file-level features above.\n");
        output.push_str("2. Identify 3-8 functional areas (PascalCase names).\n");
        output.push_str("3. Assign EACH file (by its path) to a 3-level hierarchy path: `Area/category/subcategory`\n");
        output.push_str("4. Call `submit_hierarchy` with a JSON object mapping file paths to hierarchy paths.\n\n");
        output.push_str("Example:\n```json\n{\n");
        output.push_str(
            "  \"src/auth/login.rs\": \"Authentication/manage sessions/handle login\",\n",
        );
        output.push_str("  \"src/db/query.rs\": \"DataAccess/execute queries/build statements\"\n");
        output.push_str("}\n```\n\n");
        output.push_str(
            "Use FILE PATHS as keys (e.g., `crates/rpg-core/src/graph.rs`), not entity IDs.\n",
        );
        output.push_str("Every entity in a file will inherit that file's hierarchy path.\n");

        Ok(output)
    }

    #[tool(
        description = "Submit hierarchy assignments from build_semantic_hierarchy. Pass a JSON object mapping file paths to 3-level hierarchy paths (Area/category/subcategory). All entities in each file inherit that file's path. After submission, the graph is re-grounded and saved."
    )]
    async fn submit_hierarchy(
        &self,
        Parameters(params): Parameters<SubmitHierarchyParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        // Parse the JSON assignments
        let assignments: std::collections::HashMap<String, String> =
            serde_json::from_str(&params.assignments).map_err(|e| {
                format!(
                    "Invalid JSON: {}. Expected {{\"file_path\": \"Area/cat/subcat\", ...}}",
                    e
                )
            })?;

        if assignments.is_empty() {
            return Err(
                "Empty assignments. Provide at least one file → hierarchy path mapping.".into(),
            );
        }

        let mut guard = self.graph.write().await;
        let graph = guard.as_mut().ok_or("No RPG loaded")?;

        // Convert file paths to Module entity IDs for apply_hierarchy
        // Module entities use the file path as their ID in the format "path:filename_stem"
        let mut entity_assignments: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut matched = 0usize;
        let mut unmatched = Vec::new();

        for (file_path, hierarchy_path) in &assignments {
            // Find the Module entity for this file, or find entities by file path
            let mut found = false;

            // Try matching by file path in entity file field
            for (id, entity) in &graph.entities {
                let entity_file = entity.file.display().to_string();
                if (entity_file == *file_path
                    || entity_file.ends_with(file_path)
                    || file_path.ends_with(&entity_file))
                    && entity.kind == rpg_core::graph::EntityKind::Module
                {
                    // Module entity — apply_hierarchy will propagate to all file siblings
                    entity_assignments.insert(id.clone(), hierarchy_path.clone());
                    found = true;
                    matched += 1;
                    break;
                }
            }

            // If no Module entity found, assign all entities in this file directly
            if !found {
                let mut file_entities = Vec::new();
                for (id, entity) in &graph.entities {
                    let entity_file = entity.file.display().to_string();
                    if entity_file == *file_path
                        || entity_file.ends_with(file_path)
                        || file_path.ends_with(&entity_file)
                    {
                        file_entities.push(id.clone());
                    }
                }

                if file_entities.is_empty() {
                    unmatched.push(file_path.clone());
                } else {
                    matched += 1;
                    for id in file_entities {
                        entity_assignments.insert(id, hierarchy_path.clone());
                    }
                }
            }
        }

        // Clear existing hierarchy and apply new assignments
        graph.hierarchy.clear();
        rpg_encoder::hierarchy::apply_hierarchy(graph, &entity_assignments);
        graph.metadata.semantic_hierarchy = true;

        // Re-enrich hierarchy metadata and grounding
        graph.assign_hierarchy_ids();
        graph.aggregate_hierarchy_features();
        graph.materialize_containment_edges();
        rpg_encoder::grounding::ground_hierarchy(graph);
        graph.refresh_metadata();

        // Save
        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        let mut result = format!(
            "Hierarchy applied.\nfiles_matched: {}\nfiles_unmatched: {}\nhierarchy_type: semantic\n",
            matched,
            unmatched.len()
        );

        if !unmatched.is_empty() {
            result.push_str(&format!(
                "unmatched_files: {}\n",
                unmatched
                    .iter()
                    .take(10)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // Show hierarchy summary
        result.push_str("\nHierarchy areas:\n");
        for (area_name, area_node) in &graph.hierarchy {
            result.push_str(&format!(
                "  - {} ({} entities)\n",
                area_name,
                area_node.entity_count()
            ));
        }

        Ok(result)
    }
}

#[tool_handler]
impl ServerHandler for RpgServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(include_str!("prompts/server_instructions.md").into()),
            capabilities: rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let project_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current directory"));

    eprintln!("RPG MCP server starting for: {}", project_root.display());

    eprintln!("  Tip: Use get_entities_for_lifting + submit_lift_results for semantic features");

    let server = RpgServer::new(project_root);

    // Auto-update graph on startup if stale (structural-only, no LLM)
    {
        let mut lock = server.graph.write().await;
        if let Some(ref mut graph) = *lock
            && let (Some(base), Ok(head)) = (
                &graph.base_commit.clone(),
                rpg_encoder::evolution::get_head_sha(&server.project_root),
            )
        {
            if *base != head {
                eprintln!(
                    "  Graph stale ({}… → {}…). Auto-updating...",
                    &base[..8.min(base.len())],
                    &head[..8.min(head.len())]
                );
                match rpg_encoder::evolution::run_update(graph, &server.project_root, None) {
                    Ok(s) => {
                        let _ = storage::save(&server.project_root, graph);
                        eprintln!(
                            "  Auto-update complete: +{} -{} ~{}",
                            s.entities_added, s.entities_removed, s.entities_modified
                        );
                    }
                    Err(e) => eprintln!("  Auto-update failed (non-fatal): {}", e),
                }
            } else {
                eprintln!("  Graph is up to date.");
            }
        }
    }

    let service = server
        .serve(rmcp::transport::io::stdio())
        .await
        .inspect_err(|e| eprintln!("serve error: {}", e))
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    service.waiting().await?;

    Ok(())
}
