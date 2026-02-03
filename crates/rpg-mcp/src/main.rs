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

/// The RPG MCP server state.
#[derive(Clone)]
struct RpgServer {
    project_root: PathBuf,
    graph: Arc<RwLock<Option<RPGraph>>>,
    config: Arc<RwLock<RpgConfig>>,
    embedder: Arc<Option<rpg_encoder::embeddings::EmbeddingGenerator>>,
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

impl std::fmt::Debug for RpgServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpgServer")
            .field("project_root", &self.project_root)
            .field("embedder", &self.embedder.is_some())
            .finish()
    }
}

impl RpgServer {
    fn new(project_root: PathBuf) -> Self {
        let graph = storage::load(&project_root).ok();
        let config = RpgConfig::load(&project_root).unwrap_or_default();
        let embedder =
            rpg_encoder::embeddings::EmbeddingGenerator::from_config(&config.embeddings).ok();
        Self {
            project_root,
            graph: Arc::new(RwLock::new(graph)),
            config: Arc::new(RwLock::new(config)),
            embedder: Arc::new(embedder),
            tool_router: Self::tool_router(),
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

    fn get_config_blocking(&self) -> RpgConfig {
        // For sync contexts where we need config but can't await
        RpgConfig::load(&self.project_root).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchNodeParams {
    /// The search query describing what you're looking for
    query: String,
    /// Search mode: 'features', 'snippets', 'auto', 'semantic', or 'hybrid'
    mode: Option<String>,
    /// Optional hierarchy scope to restrict search (e.g., 'Security/auth')
    scope: Option<String>,
    /// Filter to entities within a line range [start, end]
    line_nums: Option<Vec<usize>>,
    /// Glob pattern to filter entities by file path (e.g., "src/**/*.rs")
    file_pattern: Option<String>,
    /// Override semantic weight for hybrid search (0.0-1.0, default from config)
    semantic_weight: Option<f64>,
    /// Comma-separated entity type filter (e.g., "function,class,method")
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
    /// Maximum traversal depth (default: 2)
    depth: Option<usize>,
    /// Filter edges by kind: 'imports', 'invokes', 'inherits', or 'contains'
    edge_filter: Option<String>,
    /// Comma-separated entity type filter (e.g., "function,class,method")
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
    /// Generate vector embeddings for semantic/hybrid search (requires embedding provider)
    embed: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GenerateEmbeddingsParams {
    /// Scope: file glob ("src/auth/**"), hierarchy path, or "*"/"all" for all entities.
    /// If omitted, embeds all entities.
    scope: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateRpgParams {
    /// Base commit SHA to diff from (defaults to RPG's stored base_commit)
    since: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LiftAreaParams {
    /// Scope specifier: file glob ("src/auth/**"), hierarchy path ("Auth/login"),
    /// comma-separated entity IDs, or "*"/"all" for all unlifted entities.
    scope: String,
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

/// Parse a comma-separated entity type filter string into EntityKind values.
fn parse_entity_type_filter(filter: &str) -> Vec<rpg_core::graph::EntityKind> {
    filter
        .split(',')
        .filter_map(|s| match s.trim().to_lowercase().as_str() {
            "function" => Some(rpg_core::graph::EntityKind::Function),
            "class" => Some(rpg_core::graph::EntityKind::Class),
            "method" => Some(rpg_core::graph::EntityKind::Method),
            "module" => Some(rpg_core::graph::EntityKind::Module),
            _ => None,
        })
        .collect()
}

#[tool_router]
impl RpgServer {
    #[tool(
        description = "Search for code entities by intent or keywords. Returns entities with file paths, line numbers, and relevance scores. Use mode='features' for semantic intent search, 'snippets' for name/path matching, 'auto' (default) tries both."
    )]
    async fn search_node(
        &self,
        Parameters(params): Parameters<SearchNodeParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();
        let config = self.config.read().await;

        let search_mode = match params.mode.as_deref() {
            Some("features") => rpg_nav::search::SearchMode::Features,
            Some("snippets") => rpg_nav::search::SearchMode::Snippets,
            Some("semantic") => rpg_nav::search::SearchMode::Semantic,
            Some("hybrid") => rpg_nav::search::SearchMode::Hybrid,
            _ => rpg_nav::search::SearchMode::Auto,
        };

        let line_nums = params.line_nums.as_ref().and_then(|v| {
            if v.len() == 2 {
                Some((v[0], v[1]))
            } else {
                None
            }
        });

        // Compute query embedding for semantic/hybrid search when embedder is available
        let query_embedding = match search_mode {
            rpg_nav::search::SearchMode::Semantic | rpg_nav::search::SearchMode::Hybrid => {
                if let Some(ref emb) = *self.embedder {
                    emb.generate_single(&params.query).await.ok()
                } else {
                    None
                }
            }
            _ => None,
        };

        let semantic_weight = params
            .semantic_weight
            .unwrap_or(config.embeddings.semantic_weight);

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
                query_embedding: query_embedding.as_deref(),
                semantic_weight,
                entity_type_filter,
            },
        );

        if results.is_empty() {
            return Ok(format!("No results found for: {}", params.query));
        }

        Ok(rpg_nav::toon::format_search_results(&results))
    }

    #[tool(
        description = "Fetch detailed metadata and source code for a known entity. Returns the entity's semantic features, dependencies (what it calls, what calls it), hierarchy position, and full source code."
    )]
    async fn fetch_node(
        &self,
        Parameters(params): Parameters<FetchNodeParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
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

        Ok(outputs.join("\n---\n"))
    }

    #[tool(
        description = "Explore the dependency graph starting from an entity. Traverses import, invocation, and inheritance edges. Use direction='downstream' to see what the entity calls, 'upstream' to see what calls it, 'both' for full picture."
    )]
    async fn explore_rpg(
        &self,
        Parameters(params): Parameters<ExploreRpgParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let dir = match params.direction.as_deref() {
            Some("upstream" | "up") => rpg_nav::explore::Direction::Upstream,
            Some("both") => rpg_nav::explore::Direction::Both,
            _ => rpg_nav::explore::Direction::Downstream,
        };

        let max_depth = params.depth.unwrap_or(2);

        let edge_filter = params.edge_filter.as_deref().and_then(|f| match f {
            "imports" => Some(rpg_core::graph::EdgeKind::Imports),
            "invokes" => Some(rpg_core::graph::EdgeKind::Invokes),
            "inherits" => Some(rpg_core::graph::EdgeKind::Inherits),
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
            Ok(outputs.join("\n"))
        }
    }

    #[tool(
        description = "Get RPG statistics: entity count, file count, functional areas, dependency edges, containment edges, and hierarchy overview. Use this first to understand the codebase structure before searching."
    )]
    async fn rpg_info(&self) -> Result<String, String> {
        self.ensure_graph().await?;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();
        Ok(rpg_nav::toon::format_rpg_info(graph))
    }

    #[tool(
        description = "Build an RPG (Repository Planning Graph) from the codebase. Indexes all code entities, builds a file-path hierarchy, and resolves dependencies. Completes in seconds without requiring an LLM. Set embed=true to also generate vector embeddings for semantic search. To add semantic features (LLM-extracted intent descriptions), use lift_area or get_entities_for_lifting afterwards. Run this once when first connecting to a repository."
    )]
    async fn build_rpg(
        &self,
        Parameters(params): Parameters<BuildRpgParams>,
    ) -> Result<String, String> {
        use rpg_parser::languages::Language;

        let project_root = &self.project_root;

        // Detect language
        let language = if let Some(ref l) = params.language {
            Language::from_name(l)
                .or_else(|| Language::from_extension(l))
                .ok_or_else(|| format!("unsupported language: {}", l))?
        } else {
            Language::detect_primary(project_root).ok_or_else(|| {
                "could not detect language; specify the 'language' parameter".to_string()
            })?
        };

        let mut graph = RPGraph::new(language.name());

        // Parse code entities
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
            if Language::from_extension(ext) != Some(language) {
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

            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let rel_path = path.strip_prefix(project_root).unwrap_or(path);
            let raw_entities = rpg_parser::entities::extract_entities(rel_path, &source, language);
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
        rpg_encoder::grounding::populate_entity_deps(&mut graph, project_root, language);
        rpg_encoder::grounding::ground_hierarchy(&mut graph);
        rpg_encoder::grounding::resolve_dependencies(&mut graph);

        // Set git commit
        if let Ok(sha) = rpg_encoder::evolution::get_head_sha(project_root) {
            graph.base_commit = Some(sha);
        }

        // Optional: Generate embeddings
        let mut emb_count = 0usize;
        if params.embed.unwrap_or(false) {
            let config = self.get_config_blocking();
            if let Ok(generator) =
                rpg_encoder::embeddings::EmbeddingGenerator::from_config(&config.embeddings)
                && let Ok(count) = generator
                    .embed_entities(&mut graph, config.embeddings.batch_size)
                    .await
            {
                emb_count = count;
            }
        }

        // Refresh metadata and save
        graph.refresh_metadata();
        storage::save(project_root, &graph).map_err(|e| format!("Failed to save RPG: {}", e))?;
        let _ = storage::ensure_gitignore(project_root);

        // Update in-memory state
        let meta = graph.metadata.clone();
        *self.graph.write().await = Some(graph);

        let mut result = format!(
            "RPG built successfully (structural).\n\
             language: {}\n\
             entities: {}\n\
             files: {}\n\
             functional_areas: {}\n\
             dependency_edges: {}\n\
             containment_edges: {}\n\
             lifted: 0/{}\n\
             hierarchy: structural",
            language.name(),
            meta.total_entities,
            meta.total_files,
            meta.functional_areas,
            meta.dependency_edges,
            meta.containment_edges,
            meta.total_entities,
        );
        if emb_count > 0 {
            result.push_str(&format!("\nembeddings: {}", emb_count));
        }
        result.push_str("\nTip: use lift_area to add semantic features to specific areas.");
        Ok(result)
    }

    #[tool(
        description = "Semantically lift a subset of entities via LLM. Adds verb-object intent descriptions to entities matching the scope. Scope can be a file glob ('src/auth/**'), a hierarchy path ('Auth/login'), comma-separated entity IDs, or '*'/'all'. When enough entities are lifted, automatically builds a semantic hierarchy. Use this progressively on areas you're working with."
    )]
    async fn lift_area(
        &self,
        Parameters(params): Parameters<LiftAreaParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let config = self.get_config_blocking();
        let client = rpg_encoder::llm::LlmClient::from_env_with_config_async(&config.llm)
            .await
            .map_err(|e| format!("LLM required for lifting: {}", e))?;

        let mut guard = self.graph.write().await;
        let graph = guard.as_mut().ok_or("No RPG loaded")?;

        let scope = rpg_encoder::lift::resolve_scope(graph, &params.scope);
        if scope.entity_ids.is_empty() {
            return Ok(format!(
                "No entities matched scope: {}\nTry a file glob like 'src/**' or '*' for all.",
                params.scope
            ));
        }

        let result =
            rpg_encoder::lift::lift_area(graph, &scope, &client, &self.project_root, &config)
                .await
                .map_err(|e| format!("Lift failed: {}", e))?;

        // Save updated graph
        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        let (lifted, total) = graph.lifting_coverage();
        let hierarchy_type = if graph.metadata.semantic_hierarchy {
            "semantic"
        } else {
            "structural"
        };

        Ok(format!(
            "Lift complete.\n\
             scope: {}\n\
             entities_lifted: {}\n\
             entities_repaired: {}\n\
             entities_failed: {}\n\
             hierarchy_updated: {}\n\
             total_coverage: {}/{}\n\
             hierarchy: {}",
            params.scope,
            result.entities_lifted,
            result.entities_repaired,
            result.entities_failed,
            result.hierarchy_updated,
            lifted,
            total,
            hierarchy_type,
        ))
    }

    #[tool(
        description = "Get a batch of code entities for YOU to semantically analyze. Returns source code and instructions. After analyzing, call submit_lift_results with the features JSON. No API key or local LLM needed — you do the analysis. Use scope to target specific areas (e.g., 'src/auth/**')."
    )]
    async fn get_entities_for_lifting(
        &self,
        Parameters(params): Parameters<GetEntitiesForLiftingParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let scope = rpg_encoder::lift::resolve_scope(graph, &params.scope);
        if scope.entity_ids.is_empty() {
            return Ok(format!(
                "No entities matched scope: {}\nTry a file glob like 'src/**' or '*' for all.",
                params.scope
            ));
        }

        let raw_entities =
            rpg_encoder::lift::collect_raw_entities(graph, &scope, &self.project_root)
                .map_err(|e| format!("Failed to collect entities: {}", e))?;

        if raw_entities.is_empty() {
            return Ok("No source code found for matched entities.".into());
        }

        let config = self.config.read().await;
        let batch_size = config.encoding.batch_size;
        drop(config);

        let total_batches = raw_entities.len().div_ceil(batch_size);
        let batch_index = params.batch_index.unwrap_or(0);

        if batch_index >= total_batches {
            return Ok(format!(
                "DONE — all {} batches already retrieved. No more entities to lift for this scope.",
                total_batches
            ));
        }

        let batch_start = batch_index * batch_size;
        let batch_end = (batch_start + batch_size).min(raw_entities.len());
        let batch = &raw_entities[batch_start..batch_end];

        let (lifted, total) = graph.lifting_coverage();
        let repo_info = rpg_encoder::lift::generate_repo_info(graph);

        let mut output = format!(
            "BATCH {}/{} ({} entities) | coverage: {}/{}\n\n",
            batch_index + 1,
            total_batches,
            batch.len(),
            lifted,
            total,
        );

        // Include repo context and system prompt
        output.push_str("## Repository Context\n\n");
        output.push_str(&repo_info);
        output.push_str("\n\n## Instructions\n\n");
        output.push_str(rpg_encoder::semantic_lifting::SEMANTIC_PARSING_SYSTEM);
        output.push_str("\n\n## Code to Analyze\n\n");

        for entity in batch {
            output.push_str(&format!(
                "### {}:{} ({:?})\n```\n{}\n```\n\n",
                entity.file.display(),
                entity.name,
                entity.kind,
                entity.source_text,
            ));
        }

        output.push_str(
            "---\nAfter analyzing, call `submit_lift_results` with a JSON object using `file:name` keys exactly as shown above.\nExample: {\"src/storage.rs:load\": [\"load graph from disk\"], \"src/storage.rs:save\": [\"persist graph\"]}\n"
        );

        if batch_index + 1 < total_batches {
            output.push_str(&format!(
                "Then call `get_entities_for_lifting` with scope=\"{}\" and batch_index={} for the next batch.\n",
                params.scope,
                batch_index + 1
            ));
        } else {
            output.push_str("This is the last batch.\n");
        }

        Ok(output)
    }

    #[tool(
        description = "Submit semantic features you extracted from code entities. Pass a JSON object using file:name keys (as shown by get_entities_for_lifting). Example: {\"src/main.rs:my_func\": [\"validate input\"], \"src/lib.rs:load\": [\"load config\"]}. Updates the graph and saves."
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

        let mut updated = 0usize;
        for (key, feats) in &features {
            if feats.is_empty() {
                continue;
            }
            // Try "file:name" format first (e.g., "src/storage.rs:load")
            let matched = if let Some((file_part, name_part)) = key.rsplit_once(':') {
                let mut found = false;
                for entity in graph.entities.values_mut() {
                    if entity.name == name_part
                        && entity.file.to_string_lossy().as_ref() == file_part
                        && entity.semantic_features.is_empty()
                    {
                        entity.semantic_features = feats.clone();
                        found = true;
                        break;
                    }
                }
                found
            } else {
                false
            };
            // Fallback: match by name only (backward compat)
            if !matched {
                for entity in graph.entities.values_mut() {
                    if entity.name == *key && entity.semantic_features.is_empty() {
                        entity.semantic_features = feats.clone();
                        break;
                    }
                }
            }
            updated += 1;
        }

        // Re-aggregate hierarchy features
        graph.aggregate_hierarchy_features();
        graph.refresh_metadata();

        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        let (lifted, total) = graph.lifting_coverage();

        Ok(format!(
            "Applied {} feature sets ({} new entities lifted).\ncoverage: {}/{}\nlifted_entities: {}",
            features.len(),
            updated,
            lifted,
            total,
            graph.metadata.lifted_entities,
        ))
    }

    #[tool(
        description = "Incrementally update the RPG from git changes since the last build. Detects added, modified, deleted, and renamed files, re-extracts entities, re-lifts semantic features, detects semantic drift, and re-routes drifted entities. Much faster than a full rebuild."
    )]
    async fn update_rpg(
        &self,
        Parameters(params): Parameters<UpdateRpgParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let config = self.get_config_blocking();
        let client = rpg_encoder::llm::LlmClient::from_env_with_config_async(&config.llm)
            .await
            .ok();
        let embedder =
            rpg_encoder::embeddings::EmbeddingGenerator::from_config(&config.embeddings).ok();

        let mut graph = self.graph.write().await;
        let g = graph.as_mut().ok_or("No RPG loaded")?;

        let summary = rpg_encoder::evolution::run_update(
            g,
            &self.project_root,
            client.as_ref(),
            params.since.as_deref(),
            config.encoding.drift_threshold,
            config.encoding.hierarchy_chunk_size,
            embedder.as_ref(),
        )
        .await
        .map_err(|e| format!("Update failed: {}", e))?;

        storage::save(&self.project_root, g).map_err(|e| format!("Failed to save RPG: {}", e))?;

        if summary.entities_added == 0
            && summary.entities_modified == 0
            && summary.entities_removed == 0
        {
            Ok("RPG is up to date. No source changes detected.".into())
        } else {
            Ok(format!(
                "RPG updated.\n\
                 entities_added: {}\n\
                 entities_modified: {}\n\
                 entities_removed: {}\n\
                 edges_added: {}\n\
                 edges_removed: {}\n\
                 hierarchy_nodes_added: {}\n\
                 hierarchy_nodes_removed: {}",
                summary.entities_added,
                summary.entities_modified,
                summary.entities_removed,
                summary.edges_added,
                summary.edges_removed,
                summary.hierarchy_nodes_added,
                summary.hierarchy_nodes_removed,
            ))
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
        description = "Generate vector embeddings for all entities, enabling semantic and hybrid search modes. Requires an embedding provider (Ollama, OpenAI, or Anthropic). Use scope to limit to specific areas. Run after lifting for best results."
    )]
    async fn generate_embeddings(
        &self,
        Parameters(params): Parameters<GenerateEmbeddingsParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let config = self.get_config_blocking();
        let generator =
            rpg_encoder::embeddings::EmbeddingGenerator::from_config(&config.embeddings)
                .map_err(|e| format!("Embedding provider required: {}", e))?;

        let mut guard = self.graph.write().await;
        let graph = guard.as_mut().ok_or("No RPG loaded")?;

        // If scope is provided, only embed entities matching the scope
        let count = if let Some(ref scope_str) = params.scope {
            let scope = rpg_encoder::lift::resolve_scope(graph, scope_str);
            if scope.entity_ids.is_empty() {
                return Ok(format!(
                    "No entities matched scope: {}\nTry a file glob like 'src/**' or '*' for all.",
                    scope_str
                ));
            }
            // Collect texts and IDs for scoped entities
            let mut texts = Vec::new();
            let mut ids = Vec::new();
            for id in &scope.entity_ids {
                if let Some(entity) = graph.entities.get(id) {
                    let text = entity.semantic_features.join(", ");
                    if !text.is_empty() {
                        texts.push(text);
                        ids.push(id.clone());
                    }
                }
            }
            if texts.is_empty() {
                return Ok("No lifted entities in scope. Lift entities first, then embed.".into());
            }
            // Process in batches
            let batch_size = config.embeddings.batch_size;
            let mut count = 0usize;
            for chunk_start in (0..texts.len()).step_by(batch_size) {
                let chunk_end = (chunk_start + batch_size).min(texts.len());
                let chunk = &texts[chunk_start..chunk_end];
                let embeddings = generator
                    .generate_batch(chunk)
                    .await
                    .map_err(|e| format!("Embedding generation failed: {}", e))?;
                for (i, emb) in embeddings.into_iter().enumerate() {
                    let id = &ids[chunk_start + i];
                    if let Some(entity) = graph.entities.get_mut(id) {
                        entity.embedding = Some(emb);
                        count += 1;
                    }
                }
            }
            count
        } else {
            generator
                .embed_entities(graph, config.embeddings.batch_size)
                .await
                .map_err(|e| format!("Embedding generation failed: {}", e))?
        };

        graph.refresh_metadata();
        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        let total = rpg_nav::embedding_search::embedding_count(graph);
        Ok(format!(
            "Embeddings generated.\n\
             new_embeddings: {}\n\
             total_embeddings: {}\n\
             search modes semantic and hybrid are now available.",
            count, total,
        ))
    }
}

#[tool_handler]
impl ServerHandler for RpgServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "RPG-Encoder: Repository Planning Graph — gives you semantic understanding of any codebase.\n\n\
                 QUICKSTART:\n\
                 1. Call build_rpg to index the repository (instant, no LLM needed)\n\
                 2. Add semantic features to areas you're working with:\n\
                    - RECOMMENDED: Call get_entities_for_lifting → analyze the code yourself → call submit_lift_results (no setup needed)\n\
                    - Alternative: Call lift_area to auto-lift via local Ollama LLM (requires Ollama installed)\n\
                 3. Optional: Call generate_embeddings for semantic/hybrid search\n\n\
                 Tools:\n\
                 - build_rpg: Index the codebase structurally (run once per repo, add embed=true for embeddings)\n\
                 - get_entities_for_lifting: Get code for YOU to analyze (no LLM setup needed)\n\
                 - submit_lift_results: Submit your analysis results\n\
                 - lift_area: Auto-lift via local LLM (requires Ollama)\n\
                 - generate_embeddings: Generate vector embeddings for semantic/hybrid search\n\
                 - update_rpg: Incrementally update after code changes\n\
                 - search_node: Find code by intent\n\
                 - fetch_node: Get full details + source code for an entity\n\
                 - explore_rpg: Trace dependency chains\n\
                 - rpg_info: Get codebase overview\n\
                 - reload_rpg: Reload graph from disk"
                    .into(),
            ),
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

    // Log available providers
    {
        let config = RpgConfig::load(&project_root).unwrap_or_default();
        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            eprintln!("  Provider: Anthropic API key detected");
        }
        if std::env::var("OPENAI_API_KEY").is_ok() {
            eprintln!("  Provider: OpenAI API key detected");
        }
        eprintln!(
            "  Ollama: {} (model: {})",
            config.llm.local_url, config.llm.local_model
        );
        eprintln!(
            "  Tip: No LLM setup needed — use get_entities_for_lifting + submit_lift_results"
        );
    }

    let server = RpgServer::new(project_root);
    let service = server
        .serve(rmcp::transport::io::stdio())
        .await
        .inspect_err(|e| eprintln!("serve error: {}", e))
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    service.waiting().await?;

    Ok(())
}
