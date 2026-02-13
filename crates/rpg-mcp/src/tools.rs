//! MCP tool handlers — all 17 `#[tool]` methods in a single `#[tool_router]` impl block.
//!
//! The `#[tool_router]` proc macro requires every `#[tool]` method to live in one
//! `impl` block, so this file cannot be split further without upstream changes.

use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use rpg_core::graph::RPGraph;
use rpg_core::storage;

use crate::server::RpgServer;
use crate::types::*;

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

        // Attempt hybrid embedding search for features/auto modes
        let use_embeddings = matches!(
            search_mode,
            rpg_nav::search::SearchMode::Features | rpg_nav::search::SearchMode::Auto
        );
        let mut search_mode_label = "lexical";

        let embedding_scores = if use_embeddings {
            self.try_init_embeddings(graph).await;
            let mut emb_guard = self.embedding_index.write().await;
            if let Some(ref mut idx) = *emb_guard {
                match idx.score_all(&params.query) {
                    Ok(scores) if !scores.is_empty() => {
                        search_mode_label = "hybrid";
                        Some(scores)
                    }
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };

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
                embedding_scores: embedding_scores.as_ref(),
            },
        );

        if results.is_empty() {
            return Ok(format!(
                "{}No results found for: {} (search_mode: {})",
                notice, params.query, search_mode_label,
            ));
        }

        Ok(format!(
            "{}{}\n\nsearch_mode: {}",
            notice,
            rpg_nav::toon::format_search_results(&results),
            search_mode_label,
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

        let projection = rpg_nav::toon::FetchProjection::from_params(
            params.fields.as_deref(),
            params.source_max_lines,
        )?;

        let mut outputs = Vec::new();
        for id in &ids {
            match rpg_nav::fetch::fetch(graph, id, &self.project_root) {
                Ok(output) => outputs.push(rpg_nav::toon::format_fetch_output_projected(
                    &output,
                    &projection,
                )),
                Err(e) => outputs.push(format!("error({}): {}", id, e)),
            }
        }

        Ok(format!("{}{}", notice, outputs.join("\n---\n")))
    }

    #[tool(
        description = "Explore the dependency graph starting from an entity. Traverses import, invocation, inheritance, composition, render, state-read/state-write, and dispatch edges. Use direction='downstream' to see what the entity calls, 'upstream' to see what calls it, 'both' for full picture."
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
            "renders" => Some(rpg_core::graph::EdgeKind::Renders),
            "reads_state" => Some(rpg_core::graph::EdgeKind::ReadsState),
            "writes_state" => Some(rpg_core::graph::EdgeKind::WritesState),
            "dispatches" => Some(rpg_core::graph::EdgeKind::Dispatches),
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

        let use_compact = matches!(params.format.as_deref(), Some("compact"));

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
                Some(tree) => {
                    let formatted = if use_compact {
                        rpg_nav::explore::format_compact(&tree)
                    } else {
                        rpg_nav::explore::format_tree(&tree, 0)
                    };
                    // Apply max_results truncation (by line count)
                    if let Some(max) = params.max_results {
                        let lines: Vec<&str> = formatted.lines().collect();
                        if lines.len() > max {
                            let mut truncated = lines[..max].join("\n");
                            truncated.push_str(&format!(
                                "\n... ({} more nodes, truncated. Use max_results to increase.)",
                                lines.len() - max
                            ));
                            outputs.push(truncated);
                        } else {
                            outputs.push(formatted);
                        }
                    } else {
                        outputs.push(formatted);
                    }
                }
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
        let emb_guard = self.embedding_index.read().await;
        let emb_status = if let Some(ref idx) = *emb_guard {
            format!(
                "\nembedding_index: {} entities indexed (BGE-small-en-v1.5)",
                idx.entity_count()
            )
        } else if self
            .embedding_init_failed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            "\nembedding_index: init failed (lexical-only search)".to_string()
        } else {
            "\nembedding_index: not initialized (will load on first semantic search)".to_string()
        };
        Ok(format!(
            "{}{}{}",
            notice,
            rpg_nav::toon::format_rpg_info(graph),
            emb_status,
        ))
    }

    #[tool(
        description = "Build a dependency-safe reconstruction execution plan. Returns a topological ordering of entities with area-coherent batching, suitable for guided code reconstruction workflows. Requires a built RPG with a semantic hierarchy."
    )]
    async fn reconstruct_plan(
        &self,
        Parameters(params): Parameters<ReconstructPlanParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        if !graph.metadata.semantic_hierarchy {
            return Err(
                "No semantic hierarchy. Run the full lifting + hierarchy flow first.".into(),
            );
        }

        let options = rpg_encoder::reconstruction::ReconstructionOptions {
            max_batch_size: params.max_batch_size.unwrap_or(8).max(1),
            include_modules: params.include_modules.unwrap_or(false),
        };
        let plan = rpg_encoder::reconstruction::schedule_reconstruction(graph, options);

        let json = serde_json::to_string_pretty(&plan)
            .map_err(|e| format!("Failed to serialize plan: {}", e))?;

        Ok(format!(
            "Reconstruction plan: {} entities, {} batches (max_batch_size: {})\n\n{}",
            plan.topological_order.len(),
            plan.batches.len(),
            options.max_batch_size,
            json,
        ))
    }

    #[tool(
        description = "Build an RPG (Repository Planning Graph) from the codebase. Indexes all code entities, builds a file-path hierarchy, and resolves dependencies. Completes in seconds without requiring an LLM. To add semantic features (LLM-extracted intent descriptions), use get_entities_for_lifting afterwards. Run this once when first connecting to a repository. Respects .rpgignore files (gitignore syntax) for excluding files from the graph."
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
        let mut backup_failed = false;
        let old_graph: Option<RPGraph> = if storage::rpg_exists(project_root) {
            match storage::load(project_root) {
                Ok(g) if g.metadata.lifted_entities > 0 => {
                    // Backup before overwriting
                    if let Err(e) = storage::create_backup(project_root) {
                        eprintln!("rpg: WARNING: backup failed: {e}");
                        backup_failed = true;
                    }
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

        // Load TOML paradigm definitions + compile tree-sitter queries
        let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs().map_err(|errs| {
            format!(
                "paradigm definition errors: {}",
                errs.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;
        let qcache = rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs)
            .map_err(|errs| format!("query compile errors: {}", errs.join("; ")))?;

        // Detect paradigms using TOML-driven engine
        let active_defs =
            rpg_parser::paradigms::detect_paradigms_toml(project_root, &languages, &paradigm_defs);
        graph.metadata.paradigms = active_defs.iter().map(|d| d.name.clone()).collect();

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
            .add_custom_ignore_filename(".rpgignore")
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
            let mut raw_entities =
                rpg_parser::entities::extract_entities(rel_path, &source, file_lang);

            // TOML-driven paradigm pipeline: classify + entity queries + builtin features
            rpg_parser::paradigms::classify::classify_entities(
                &active_defs,
                rel_path,
                &mut raw_entities,
            );
            let extra = rpg_parser::paradigms::query_engine::execute_entity_queries(
                &qcache,
                &active_defs,
                rel_path,
                &source,
                file_lang,
                &raw_entities,
            );
            raw_entities.extend(extra);
            rpg_parser::paradigms::features::apply_builtin_entity_features(
                &active_defs,
                rel_path,
                &source,
                file_lang,
                &mut raw_entities,
            );

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
        let paradigm_ctx = rpg_encoder::grounding::ParadigmContext {
            active_defs: active_defs.clone(),
            qcache: &qcache,
        };
        rpg_encoder::grounding::populate_entity_deps(
            &mut graph,
            project_root,
            cfg.encoding.broadcast_imports,
            None,
            Some(&paradigm_ctx),
        );
        rpg_encoder::grounding::ground_hierarchy(&mut graph);
        rpg_encoder::grounding::resolve_dependencies(&mut graph);

        // Set git commit
        if let Ok(sha) = rpg_encoder::evolution::get_head_sha(project_root) {
            graph.base_commit = Some(sha);
        }

        // Merge old features, hierarchy paths, and module features (auto-preservation)
        let merge_stats = if let Some(ref old) = old_graph {
            let old_had_semantic = old.metadata.semantic_hierarchy;
            let stats = rpg_encoder::evolution::merge_features(&mut graph, old);

            // Only rebuild semantic hierarchy if paths were actually restored
            if old_had_semantic && stats.hierarchy_restored > 0 {
                rpg_encoder::evolution::rebuild_hierarchy_from_entities(&mut graph, true);
                graph.assign_hierarchy_ids();
                graph.aggregate_hierarchy_features();
                graph.materialize_containment_edges();
            }

            Some(stats)
        } else {
            None
        };

        // Refresh metadata and save
        graph.refresh_metadata();
        storage::save(project_root, &graph).map_err(|e| format!("Failed to save RPG: {}", e))?;
        let _ = storage::ensure_gitignore(project_root);

        // Update in-memory state
        let meta = graph.metadata.clone();
        *self.graph.write().await = Some(graph);
        // Sync embedding index incrementally (fingerprints detect what changed)
        {
            let graph_guard = self.graph.read().await;
            if let Some(ref graph) = *graph_guard {
                let mut emb_guard = self.embedding_index.write().await;
                if let Some(ref mut idx) = *emb_guard
                    && let Err(e) = idx.sync(graph)
                {
                    eprintln!("rpg: embedding sync failed: {e}");
                    *emb_guard = None;
                }
            }
        }
        self.embedding_init_failed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        // Clear stale pending routing (graph was fully replaced)
        self.pending_routing.write().await.clear();
        clear_pending_routing(&self.project_root);

        let lang_display = if languages.len() == 1 {
            languages[0].name().to_string()
        } else {
            languages
                .iter()
                .map(|l| l.name())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let hierarchy_label = if meta.semantic_hierarchy {
            "semantic"
        } else {
            "structural"
        };
        let mut result = format!(
            "RPG built successfully.\n\
             languages: {}\n\
             entities: {}\n\
             files: {}\n\
             functional_areas: {}\n\
             dependency_edges: {}\n\
             containment_edges: {}\n\
             lifted: {}/{}\n\
             hierarchy: {}",
            lang_display,
            meta.total_entities,
            meta.total_files,
            meta.functional_areas,
            meta.dependency_edges,
            meta.containment_edges,
            meta.lifted_entities,
            meta.total_entities,
            hierarchy_label,
        );

        if let Some(ref stats) = merge_stats {
            let total_restored =
                stats.features_restored + stats.modules_restored + stats.hierarchy_restored;
            if total_restored > 0 {
                let backup_note = if backup_failed {
                    "backup FAILED"
                } else {
                    "backup saved"
                };
                result.push_str(&format!(
                    "\n\nAuto-preserved from previous graph ({}):\n\
                     features_restored: {}\n\
                     hierarchy_paths_restored: {}\n\
                     module_features_restored: {}\n\
                     orphaned: {}\n\
                     new_entities: {}",
                    backup_note,
                    stats.features_restored,
                    stats.hierarchy_restored,
                    stats.modules_restored,
                    stats.orphaned,
                    stats.new_entities,
                ));
            } else {
                result.push_str(
                    "\nTip: use get_entities_for_lifting + submit_lift_results to add semantic features.",
                );
            }
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
            // Check rebuild need with a brief read (no graph lock held)
            let needs_rebuild = {
                let session = self.lifting_session.read().await;
                match session.as_ref() {
                    None => true,
                    Some(s) => s.scope_key != params.scope || batch_index == 0,
                }
            };

            if needs_rebuild {
                // Lock order: graph first, then session (consistent with lifting_status)
                let mut guard = self.graph.write().await;
                let mut session = self.lifting_session.write().await;
                let graph = guard.as_mut().ok_or("No RPG loaded")?;

                let scope = rpg_encoder::lift::resolve_scope(graph, &params.scope);
                if scope.entity_ids.is_empty() {
                    *session = None;
                    return Ok(format!(
                        "No entities matched scope: {}\nTry a file glob like 'src/**' or '*' for all.",
                        params.scope
                    ));
                }

                let all_raw_entities =
                    rpg_encoder::lift::collect_raw_entities(graph, &scope, &self.project_root)
                        .map_err(|e| format!("Failed to collect entities: {}", e))?;

                if all_raw_entities.is_empty() {
                    *session = None;
                    return Ok("No source code found for matched entities.".into());
                }

                // Auto-lift trivial entities (getters, setters, constructors, etc.)
                let paradigm_defs =
                    rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
                let engine = rpg_encoder::lift::AutoLiftEngine::new(
                    &paradigm_defs,
                    &graph.metadata.paradigms,
                );
                let mut auto_lifted = 0usize;
                let mut needs_llm = Vec::new();
                let mut review_candidates: Vec<(String, Vec<String>)> = Vec::new();
                for raw in all_raw_entities {
                    // Skip entities that already have curated features
                    let already_lifted = graph
                        .entities
                        .get(&raw.id())
                        .is_some_and(|e| !e.semantic_features.is_empty());
                    if already_lifted {
                        continue;
                    }
                    match engine.try_lift_with_confidence(&raw) {
                        Some((features, rpg_encoder::lift::LiftConfidence::Accept)) => {
                            // High confidence — apply features directly
                            if let Some(entity) = graph.entities.get_mut(&raw.id()) {
                                entity.semantic_features = features;
                                entity.feature_source = Some("auto".to_string());
                                auto_lifted += 1;
                            }
                        }
                        Some((features, rpg_encoder::lift::LiftConfidence::Review)) => {
                            // Medium confidence — apply features but flag for review
                            let eid = raw.id();
                            if let Some(entity) = graph.entities.get_mut(&eid) {
                                entity.semantic_features = features.clone();
                                entity.feature_source = Some("auto".to_string());
                                auto_lifted += 1;
                            }
                            review_candidates.push((eid, features));
                        }
                        Some((_, rpg_encoder::lift::LiftConfidence::Reject)) | None => {
                            needs_llm.push(raw);
                        }
                    }
                }

                // Save if we auto-lifted anything
                if auto_lifted > 0 {
                    graph.refresh_metadata();
                    if let Err(e) = rpg_core::storage::save(&self.project_root, graph) {
                        eprintln!("Warning: failed to persist auto-lifted features: {e}");
                    }
                }

                if needs_llm.is_empty() {
                    *session = None;
                    let (lifted, total) = graph.lifting_coverage();
                    return Ok(format!(
                        "AUTO-LIFTED: {} trivial entities. No entities need LLM analysis.\ncoverage: {}/{}\nNEXT: Call finalize_lifting.",
                        auto_lifted, lifted, total,
                    ));
                }

                let config = self.config.read().await;
                let batch_size = config.encoding.batch_size;
                let max_batch_tokens = config.encoding.max_batch_tokens;
                drop(config);

                let mcp_batch_size = batch_size.min(25);
                let batch_ranges = rpg_encoder::lift::build_token_aware_batches(
                    &needs_llm,
                    mcp_batch_size,
                    max_batch_tokens,
                );

                // Store auto-lift count for batch 0 output
                let auto_lift_count = auto_lifted;

                *session = Some(LiftingSession {
                    scope_key: params.scope.clone(),
                    raw_entities: needs_llm,
                    batch_ranges,
                    auto_lifted: auto_lift_count,
                    review_candidates,
                });
            }
        }

        // Lock order: graph first, then session (consistent with rebuild block above)
        let guard = self.graph.read().await;
        let graph = guard.as_ref().ok_or("No RPG loaded")?;
        let session = self.lifting_session.read().await;
        let Some(session) = session.as_ref() else {
            return Err("Lifting session expired. Call get_entities_for_lifting with batch_index=0 to restart.".into());
        };

        let total_batches = session.batch_ranges.len();
        if batch_index >= total_batches {
            return Ok(format!(
                "DONE — all {} batches processed. No more entities to lift for this scope.",
                total_batches
            ));
        }

        let (batch_start, batch_end) = session.batch_ranges[batch_index];
        let batch = &session.raw_entities[batch_start..batch_end];

        let (lifted, total) = graph.lifting_coverage();

        let module_count = graph
            .entities
            .values()
            .filter(|e| e.kind == rpg_core::graph::EntityKind::Module)
            .count();
        let auto_lifted_count = session.auto_lifted;
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
            if auto_lifted_count > 0 {
                output.push_str(&format!(
                    "AUTO-LIFTED: {} trivial entities (getters/setters/constructors). Override by re-submitting features.\n\n",
                    auto_lifted_count,
                ));
            }

            // Show review candidates — auto-lifted with medium confidence
            output.push_str(&crate::types::format_review_candidates(
                &session.review_candidates,
            ));
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

            // Inject paradigm-specific lifting hints
            let lifting_hints =
                Self::collect_paradigm_hints(&graph.metadata.paradigms, |h| &h.lifting);
            if !lifting_hints.is_empty() {
                output.push_str("\n## Framework-Specific Guidelines\n\n");
                output.push_str(&lifting_hints);
            }
        }
        output.push_str("\n## Code\n\n");

        // Truncate source to prevent context overflow — signature + key logic
        // is enough for semantic feature extraction.
        for entity in batch {
            let truncated = truncate_source(&entity.source_text, 40);
            output.push_str(&format!(
                "### {} ({:?})\n```\n{}\n```\n",
                entity.id(),
                entity.kind,
                truncated,
            ));
            // Append compact dependency context when available
            if let Some(graph_entity) = graph.entities.get(&entity.id()) {
                let dep_line = format_dep_context(&graph_entity.deps);
                if !dep_line.is_empty() {
                    output.push_str(&dep_line);
                    output.push('\n');
                }
            }
            output.push('\n');
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
        let drift_ignore = config.encoding.drift_ignore_threshold;
        let drift_auto = config.encoding.drift_auto_threshold;

        let mut updated = 0usize;
        let mut unmatched = 0usize;
        let mut drift_reports: Vec<String> = Vec::new();
        let mut auto_route_ids: Vec<String> = Vec::new();
        let mut borderline_ids: Vec<(String, f64)> = Vec::new();
        let mut newly_lifted_ids: Vec<String> = Vec::new();
        // Track resolved entity_id → features for embedding update (canonical IDs)
        let mut resolved_features: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

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
                    // Three-zone drift detection (paper Algorithm 3):
                    // < drift_ignore: minor edit, in-place update
                    // drift_ignore..drift_auto: borderline, ask agent to judge
                    // > drift_auto: clear drift, auto-route
                    let old_feats = graph
                        .entities
                        .get(eid)
                        .map(|e| e.semantic_features.clone())
                        .unwrap_or_default();

                    if !old_feats.is_empty() {
                        let drift = rpg_encoder::evolution::compute_drift(&old_feats, feats);
                        if drift > drift_auto {
                            drift_reports.push(format!(
                                "  {} drifted ({:.2}) — routing required",
                                eid, drift
                            ));
                            auto_route_ids.push(eid.clone());
                        } else if drift >= drift_ignore {
                            drift_reports.push(format!(
                                "  {} borderline drift ({:.2}) — agent review requested",
                                eid, drift,
                            ));
                            borderline_ids.push((eid.clone(), drift));
                        } else if drift > 0.0 {
                            drift_reports.push(format!(
                                "  {} updated ({:.2} drift, below threshold)",
                                eid, drift,
                            ));
                        }
                    } else {
                        // First-time lift — candidate for semantic routing
                        newly_lifted_ids.push(eid.clone());
                    }

                    if let Some(entity) = graph.entities.get_mut(eid) {
                        entity.semantic_features = feats.clone();
                        entity.feature_source = Some("llm".to_string());
                        resolved_features.insert(eid.clone(), feats.clone());
                        updated += 1;
                    }
                }
            }
        }

        // Accumulate entities needing routing into pending state (LLM-based routing).
        // Instead of auto-routing via Jaccard, we store candidates and let the agent
        // make routing decisions via get_routing_candidates + submit_routing_decisions.
        let needs_routing = graph.metadata.semantic_hierarchy
            && (!auto_route_ids.is_empty()
                || !borderline_ids.is_empty()
                || !newly_lifted_ids.is_empty());

        let routing_count;
        if needs_routing {
            let mut pending = self.pending_routing.write().await;

            for eid in &auto_route_ids {
                let original_path = graph
                    .entities
                    .get(eid)
                    .map(|e| e.hierarchy_path.clone())
                    .unwrap_or_default();
                let feats = graph
                    .entities
                    .get(eid)
                    .map(|e| e.semantic_features.clone())
                    .unwrap_or_default();
                pending.push(PendingRouting {
                    entity_id: eid.clone(),
                    original_path,
                    features: feats,
                    reason: "drifted".into(),
                });
            }

            for (eid, drift) in &borderline_ids {
                let original_path = graph
                    .entities
                    .get(eid)
                    .map(|e| e.hierarchy_path.clone())
                    .unwrap_or_default();
                let feats = graph
                    .entities
                    .get(eid)
                    .map(|e| e.semantic_features.clone())
                    .unwrap_or_default();
                pending.push(PendingRouting {
                    entity_id: eid.clone(),
                    original_path,
                    features: feats,
                    reason: format!("borderline drift ({:.2})", drift),
                });
            }

            for eid in &newly_lifted_ids {
                let original_path = graph
                    .entities
                    .get(eid)
                    .map(|e| e.hierarchy_path.clone())
                    .unwrap_or_default();
                let feats = graph
                    .entities
                    .get(eid)
                    .map(|e| e.semantic_features.clone())
                    .unwrap_or_default();
                pending.push(PendingRouting {
                    entity_id: eid.clone(),
                    original_path,
                    features: feats,
                    reason: "newly lifted".into(),
                });
            }

            routing_count = pending.len();

            // Persist to disk for crash safety
            let revision = graph_revision(graph);
            let state = PendingRoutingState {
                graph_revision: revision,
                entries: pending.clone(),
            };
            if let Err(e) = save_pending_routing(&self.project_root, &state) {
                eprintln!("rpg: failed to persist pending routing: {e}");
            }
        } else {
            routing_count = 0;
        }

        graph.refresh_metadata();

        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        // Update embedding index for newly-lifted entities (non-blocking on failure)
        let graph_ts = graph.updated_at.to_rfc3339();
        drop(guard); // Release graph write lock before async embedding update
        self.update_embeddings(&resolved_features, &graph_ts).await;

        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();
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
        }

        // Routing notice
        if routing_count > 0 {
            result.push_str(&format!(
                "\n## ROUTING\n\n{} entities need semantic routing.\nCall `get_routing_candidates` to review and route them, or they will be auto-routed when you call `finalize_lifting`.\n",
                routing_count,
            ));
        }

        // Quality critique — soft feedback on submitted features
        {
            let mut all_warnings = Vec::new();
            for (eid, feats) in &resolved_features {
                let warnings = rpg_encoder::critic::critique(eid, feats);
                all_warnings.extend(warnings);
            }
            let quality_output = rpg_encoder::critic::format_warnings(&all_warnings);
            if !quality_output.is_empty() {
                result.push_str(&quality_output);
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
        description = "Get entities pending semantic routing. After submit_lift_results detects drifted or newly-lifted entities, they accumulate here for LLM-based routing. Returns entities with their features, the hierarchy structure, and routing instructions. Call submit_routing_decisions with your assignments."
    )]
    async fn get_routing_candidates(
        &self,
        Parameters(params): Parameters<GetRoutingCandidatesParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let guard = self.graph.read().await;
        let graph = guard.as_ref().ok_or("No RPG loaded")?;
        let revision = graph_revision(graph);

        let pending = self.pending_routing.read().await;
        if pending.is_empty() {
            return Ok("No entities pending routing.".into());
        }

        let batch_size = 20;
        let batch_index = params.batch_index.unwrap_or(0);
        let total_batches = pending.len().div_ceil(batch_size);
        let start = batch_index * batch_size;
        let end = (start + batch_size).min(pending.len());

        if start >= pending.len() {
            return Err(format!(
                "Batch index {} out of range (0..{})",
                batch_index,
                total_batches - 1,
            ));
        }

        let batch = &pending[start..end];

        let mut result = format!(
            "## ROUTING CANDIDATES (batch {} of {}, revision: {})\n\n",
            batch_index, total_batches, revision,
        );

        // Include routing instructions on batch 0
        if batch_index == 0 {
            result.push_str("### Instructions\n\n");
            result.push_str(ROUTING_PROMPT);
            result.push_str("\n\n");
        }

        // List entities to route
        result.push_str("### Entities to Route\n\n");
        result.push_str("| Entity | Features | Current Path | Reason |\n");
        result.push_str("|--------|----------|--------------|--------|\n");
        for p in batch {
            let feats_str = if p.features.len() > 4 {
                format!("{}, ...", p.features[..4].join(", "))
            } else {
                p.features.join(", ")
            };
            let path_display = if p.original_path.is_empty() {
                "(none)".to_string()
            } else {
                p.original_path.clone()
            };
            result.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                p.entity_id, feats_str, path_display, p.reason,
            ));
        }

        // Scoped hierarchy context: show top-3 areas by similarity to pending entities
        result.push_str("\n### Relevant Hierarchy\n\n");
        let all_pending_features: Vec<String> =
            batch.iter().flat_map(|p| p.features.clone()).collect();
        let mut area_scores: Vec<(&str, f64)> = graph
            .hierarchy
            .iter()
            .map(|(name, node)| {
                let sim = rpg_encoder::evolution::semantic_similarity(
                    &all_pending_features,
                    &node.semantic_features,
                );
                (name.as_str(), sim)
            })
            .collect();
        area_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let top_n = 3.min(area_scores.len());
        for (area_name, _) in &area_scores[..top_n] {
            if let Some(area_node) = graph.hierarchy.get(*area_name) {
                let area_feats = if area_node.semantic_features.len() > 5 {
                    format!("{}, ...", area_node.semantic_features[..5].join(", "))
                } else {
                    area_node.semantic_features.join(", ")
                };
                let entity_count: usize = area_node.entities.len()
                    + area_node
                        .children
                        .values()
                        .map(|c| {
                            c.entities.len()
                                + c.children
                                    .values()
                                    .map(|sc| sc.entities.len())
                                    .sum::<usize>()
                        })
                        .sum::<usize>();
                result.push_str(&format!(
                    "**{}** ({} entities): {}\n",
                    area_name, entity_count, area_feats,
                ));
                for (cat_name, cat_node) in &area_node.children {
                    let cat_feats = if cat_node.semantic_features.len() > 3 {
                        format!("{}, ...", cat_node.semantic_features[..3].join(", "))
                    } else {
                        cat_node.semantic_features.join(", ")
                    };
                    result.push_str(&format!("  - {}/{}: {}\n", area_name, cat_name, cat_feats,));
                    for sub_name in cat_node.children.keys() {
                        result
                            .push_str(&format!("    - {}/{}/{}\n", area_name, cat_name, sub_name,));
                    }
                }
            }
        }

        // List remaining areas by name only
        if area_scores.len() > top_n {
            let others: Vec<&str> = area_scores[top_n..].iter().map(|(n, _)| *n).collect();
            result.push_str(&format!(
                "\n(Other areas: {} — use \"Area/category/subcategory\" format for any area)\n",
                others.join(", "),
            ));
        }

        result.push_str(&format!(
            "\n## NEXT_ACTION\nROUTING: Call `submit_routing_decisions` with graph_revision \"{}\":\n{{\"entity_id\": \"Area/cat/subcat\" or \"keep\", ...}}\n",
            revision,
        ));

        Ok(result)
    }

    #[tool(
        description = "Submit semantic routing decisions for entities pending hierarchy placement. Pass a JSON object mapping entity IDs to hierarchy paths (route) or \"keep\" (confirm current position). Requires graph_revision from get_routing_candidates to prevent stale decisions."
    )]
    async fn submit_routing_decisions(
        &self,
        Parameters(params): Parameters<SubmitRoutingDecisionsParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;

        let decisions: std::collections::HashMap<String, String> =
            serde_json::from_str(&params.decisions)
                .map_err(|e| format!("Invalid decisions JSON: {}", e))?;

        let mut guard = self.graph.write().await;
        let graph = guard.as_mut().ok_or("No RPG loaded")?;

        // Validate graph revision
        let current_revision = graph_revision(graph);
        if params.graph_revision != current_revision {
            return Err(format!(
                "Stale graph_revision: expected \"{}\", got \"{}\". Call get_routing_candidates again.",
                current_revision, params.graph_revision,
            ));
        }

        let mut pending = self.pending_routing.write().await;
        let mut routed = 0usize;
        let mut kept = 0usize;
        let mut reports: Vec<String> = Vec::new();

        // Validate: decisions should only target entities currently pending routing
        let pending_ids: std::collections::HashSet<&str> =
            pending.iter().map(|p| p.entity_id.as_str()).collect();
        let invalid_entities: Vec<&String> = decisions
            .keys()
            .filter(|id| !pending_ids.contains(id.as_str()))
            .collect();
        if !invalid_entities.is_empty() {
            let sample: Vec<&str> = invalid_entities
                .iter()
                .take(10)
                .map(|s| s.as_str())
                .collect();
            return Err(format!(
                "Routing decisions may only target entities currently pending routing.\n\
                 Not pending (showing up to 10): {}",
                sample.join(", "),
            ));
        }

        // Validate: non-keep routes must be valid 3-level hierarchy paths
        let invalid_paths: Vec<String> = decisions
            .iter()
            .filter_map(|(entity_id, action)| {
                if action == "keep" {
                    return None;
                }
                if !is_three_level_hierarchy_path(action) {
                    Some(format!(
                        "{} -> {} (must be `Area/category/subcategory`)",
                        entity_id, action
                    ))
                } else if !hierarchy_path_exists(&graph.hierarchy, action) {
                    Some(format!(
                        "{} -> {} (path not found in current hierarchy)",
                        entity_id, action
                    ))
                } else {
                    None
                }
            })
            .collect();
        if !invalid_paths.is_empty() {
            let sample: Vec<&str> = invalid_paths.iter().take(10).map(|s| s.as_str()).collect();
            return Err(format!(
                "Invalid routing paths (showing up to 10):\n{}",
                sample.join("\n"),
            ));
        }

        for (entity_id, action) in &decisions {
            // Validate entity exists
            if !graph.entities.contains_key(entity_id) {
                reports.push(format!("  {} — not found, skipped", entity_id));
                continue;
            }

            if action == "keep" {
                // Confirm current position — remove from pending
                kept += 1;
            } else {
                // Route to new path
                let original_path = graph
                    .entities
                    .get(entity_id)
                    .map(|e| e.hierarchy_path.clone())
                    .unwrap_or_default();

                graph.remove_entity_from_hierarchy(entity_id);

                if let Some(entity) = graph.entities.get_mut(entity_id) {
                    entity.hierarchy_path = action.clone();
                }
                graph.insert_into_hierarchy(action, entity_id);

                if *action != original_path {
                    reports.push(format!(
                        "  {} → {} (was: {})",
                        entity_id, action, original_path
                    ));
                }
                routed += 1;
            }
        }

        // Remove decided entities from pending
        let decided_ids: std::collections::HashSet<&String> = decisions.keys().collect();
        pending.retain(|p| !decided_ids.contains(&p.entity_id));

        // Re-aggregate and rebuild
        if routed > 0 {
            graph.aggregate_hierarchy_features();
            graph.assign_hierarchy_ids();
            graph.materialize_containment_edges();
        }
        graph.refresh_metadata();

        storage::save(&self.project_root, graph)
            .map_err(|e| format!("Failed to save RPG: {}", e))?;

        // Update or clear persisted pending state
        if pending.is_empty() {
            clear_pending_routing(&self.project_root);
        } else {
            let state = PendingRoutingState {
                graph_revision: current_revision,
                entries: pending.clone(),
            };
            if let Err(e) = save_pending_routing(&self.project_root, &state) {
                eprintln!("rpg: failed to persist pending routing: {e}");
            }
        }

        let remaining = pending.len();
        let mut result = format!("Routed {} entities, kept {} in place.\n", routed, kept);
        for report in &reports {
            result.push_str(report);
            result.push('\n');
        }

        if remaining > 0 {
            result.push_str(&format!(
                "\n{} entities still pending routing. Call get_routing_candidates for next batch.\n",
                remaining,
            ));
        } else {
            result.push_str("\nAll routing complete.\n");
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

        // Detect paradigms BEFORE running update so entities get classified
        let detected_langs = Self::resolve_languages(&g.metadata);
        let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs()
            .map_err(|e| format!("Failed to load paradigm defs: {:?}", e))?;
        let qcache = rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs)
            .map_err(|errs| format!("query compile errors: {}", errs.join("; ")))?;
        let active_defs = rpg_parser::paradigms::detect_paradigms_toml(
            &self.project_root,
            &detected_langs,
            &paradigm_defs,
        );
        g.metadata.paradigms = active_defs.iter().map(|d| d.name.clone()).collect();

        let paradigm_pipeline = rpg_encoder::evolution::ParadigmPipeline {
            active_defs,
            qcache: &qcache,
        };

        let summary = rpg_encoder::evolution::run_update(
            g,
            &self.project_root,
            params.since.as_deref(),
            Some(&paradigm_pipeline),
        )
        .map_err(|e| format!("Update failed: {}", e))?;

        storage::save(&self.project_root, g).map_err(|e| format!("Failed to save RPG: {}", e))?;

        // Clear lifting session — entity list changed
        *self.lifting_session.write().await = None;
        // Sync embedding index incrementally — entities changed
        {
            let mut emb_guard = self.embedding_index.write().await;
            if let Some(ref mut idx) = *emb_guard
                && let Err(e) = idx.sync(g)
            {
                eprintln!("rpg: embedding sync failed: {e}");
                *emb_guard = None;
            }
        }
        self.embedding_init_failed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        // Reconcile pending routing against the updated graph:
        // preserve entries whose entities still exist and have features, drop the rest.
        let mut pending_preserved = 0usize;
        let mut pending_dropped = 0usize;
        {
            let mut pending = self.pending_routing.write().await;
            let previous = std::mem::take(&mut *pending);
            if g.metadata.semantic_hierarchy {
                let mut preserved = Vec::new();
                for mut entry in previous {
                    if let Some(entity) = g.entities.get(&entry.entity_id) {
                        if entity.semantic_features.is_empty() {
                            pending_dropped += 1;
                            continue;
                        }
                        entry.features = entity.semantic_features.clone();
                        entry.original_path = entity.hierarchy_path.clone();
                        preserved.push(entry);
                    } else {
                        pending_dropped += 1;
                    }
                }
                pending_preserved = preserved.len();
                *pending = preserved.clone();
                if pending.is_empty() {
                    clear_pending_routing(&self.project_root);
                } else {
                    let state = PendingRoutingState {
                        graph_revision: graph_revision(g),
                        entries: preserved,
                    };
                    if let Err(e) = save_pending_routing(&self.project_root, &state) {
                        eprintln!("rpg: failed to persist pending routing: {e}");
                    }
                }
            } else {
                pending_dropped = previous.len();
                clear_pending_routing(&self.project_root);
            }
        }

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
            if pending_preserved > 0 || pending_dropped > 0 {
                result.push_str(&format!(
                    "\nrouting_pending_preserved: {}\nrouting_pending_dropped: {}",
                    pending_preserved, pending_dropped,
                ));
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
                // Sync embedding index incrementally
                {
                    let graph_guard = self.graph.read().await;
                    if let Some(ref graph) = *graph_guard {
                        let mut emb_guard = self.embedding_index.write().await;
                        if let Some(ref mut idx) = *emb_guard
                            && let Err(e) = idx.sync(graph)
                        {
                            eprintln!("rpg: embedding sync failed: {e}");
                            *emb_guard = None;
                        }
                    }
                }
                self.embedding_init_failed
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                // Reload pending routing from disk (may have changed externally)
                let pending = load_pending_routing(&self.project_root)
                    .map(|s| s.entries)
                    .unwrap_or_default();
                *self.pending_routing.write().await = pending;
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

        // Drain pending routing via Jaccard fallback if agent didn't route explicitly
        let mut fallback_routed = 0usize;
        {
            let mut pending = self.pending_routing.write().await;
            if !pending.is_empty() && graph.metadata.semantic_hierarchy {
                for p in pending.drain(..) {
                    let feats = graph
                        .entities
                        .get(&p.entity_id)
                        .map(|e| e.semantic_features.clone())
                        .unwrap_or_default();
                    if feats.is_empty() {
                        // No features — keep at original path
                        continue;
                    }

                    graph.remove_entity_from_hierarchy(&p.entity_id);
                    graph.aggregate_hierarchy_features();

                    match rpg_encoder::evolution::find_best_hierarchy_path(graph, &feats) {
                        Some(new_path) if new_path != p.original_path => {
                            if let Some(entity) = graph.entities.get_mut(&p.entity_id) {
                                entity.hierarchy_path = new_path.clone();
                            }
                            graph.insert_into_hierarchy(&new_path, &p.entity_id);
                            fallback_routed += 1;
                        }
                        _ => {
                            if !p.original_path.is_empty() {
                                graph.insert_into_hierarchy(&p.original_path, &p.entity_id);
                            }
                        }
                    }
                }

                graph.aggregate_hierarchy_features();
                graph.assign_hierarchy_ids();
                graph.materialize_containment_edges();
            }
            clear_pending_routing(&self.project_root);
        }

        // Clear lifting session cache
        *self.lifting_session.write().await = None;

        let mut steps: Vec<String> = Vec::new();

        if fallback_routed > 0 {
            steps.push(format!(
                "routing_fallback: {} entities auto-routed via Jaccard",
                fallback_routed,
            ));
        }

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
            output.push_str(include_str!("prompts/synthesis_instructions.md"));

            // Inject paradigm-specific synthesis hints
            let synthesis_hints =
                Self::collect_paradigm_hints(&graph.metadata.paradigms, |h| &h.synthesis);
            if !synthesis_hints.is_empty() {
                output.push_str("## Framework-Specific Synthesis Guidelines\n\n");
                output.push_str(&synthesis_hints);
                output.push('\n');
            }
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
                        module.feature_source = Some("synthesized".to_string());
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

        // Inject paradigm-specific discovery hints
        let discovery_hints =
            Self::collect_paradigm_hints(&graph.metadata.paradigms, |h| &h.discovery);
        if !discovery_hints.is_empty() {
            output.push_str("\n\n## Framework-Specific Discovery Guidelines\n\n");
            output.push_str(&discovery_hints);
        }

        output.push_str("\n\n### File-Level Features\n");
        output.push_str(&file_features);

        output.push_str("\n\n## Step 2: Hierarchy Assignment\n\n");
        output.push_str(hierarchy_prompt);

        // Inject paradigm-specific hierarchy hints
        let hierarchy_hints =
            Self::collect_paradigm_hints(&graph.metadata.paradigms, |h| &h.hierarchy);
        if !hierarchy_hints.is_empty() {
            output.push_str("\n\n## Framework-Specific Hierarchy Patterns\n\n");
            output.push_str(&hierarchy_hints);
        }

        output.push_str("\n\n");
        output.push_str(include_str!("prompts/hierarchy_instructions.md"));

        Ok(output)
    }

    #[tool(
        description = "Build a focused context pack in a single call. Searches for entities matching your query, fetches their details and source code, expands neighbors to the specified depth (default 1), and trims to a token budget. Replaces the typical search→fetch→explore multi-step workflow. Returns primary entities with source + features + deps, plus neighborhood entities for broader context."
    )]
    async fn context_pack(
        &self,
        Parameters(params): Parameters<ContextPackParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let notice = self.staleness_notice().await;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        // Attempt hybrid embedding search
        self.try_init_embeddings(graph).await;
        let embedding_scores = {
            let mut emb_guard = self.embedding_index.write().await;
            if let Some(ref mut idx) = *emb_guard {
                idx.score_all(&params.query).ok().filter(|s| !s.is_empty())
            } else {
                None
            }
        };

        let request = rpg_nav::context::ContextPackRequest {
            query: &params.query,
            scope: params.scope.as_deref(),
            token_budget: params.token_budget.unwrap_or(4000),
            include_source: params.include_source.unwrap_or(true),
            depth: params.depth.unwrap_or(1),
        };

        let result = rpg_nav::context::build_context_pack(
            graph,
            &self.project_root,
            &request,
            embedding_scores.as_ref(),
        );

        if result.primary_entities.is_empty() {
            return Ok(format!("{}No entities found for: {}", notice, params.query,));
        }

        Ok(format!(
            "{}{}",
            notice,
            rpg_nav::toon::format_context_pack(&result),
        ))
    }

    #[tool(
        description = "Compute the impact radius of an entity: find all entities reachable via dependency edges with edge paths. Use direction='upstream' to answer 'what depends on this?', 'downstream' for 'what does this depend on?'. Returns a flat list with depth, edge paths, and features — ideal for change impact analysis."
    )]
    async fn impact_radius(
        &self,
        Parameters(params): Parameters<ImpactRadiusParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let notice = self.staleness_notice().await;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let dir = match params.direction.as_deref() {
            Some("downstream" | "down") => rpg_nav::explore::Direction::Downstream,
            Some("both") => rpg_nav::explore::Direction::Both,
            _ => rpg_nav::explore::Direction::Upstream, // Default: "what depends on this?"
        };

        let max_depth = match params.max_depth {
            Some(-1) => usize::MAX,
            Some(d) if d >= 0 => usize::try_from(d).unwrap_or(3),
            _ => 3,
        };

        let edge_filter = params.edge_filter.as_deref().and_then(|f| match f {
            "imports" => Some(rpg_core::graph::EdgeKind::Imports),
            "invokes" => Some(rpg_core::graph::EdgeKind::Invokes),
            "inherits" => Some(rpg_core::graph::EdgeKind::Inherits),
            "composes" => Some(rpg_core::graph::EdgeKind::Composes),
            "renders" => Some(rpg_core::graph::EdgeKind::Renders),
            "reads_state" => Some(rpg_core::graph::EdgeKind::ReadsState),
            "writes_state" => Some(rpg_core::graph::EdgeKind::WritesState),
            "dispatches" => Some(rpg_core::graph::EdgeKind::Dispatches),
            "contains" => Some(rpg_core::graph::EdgeKind::Contains),
            _ => None,
        });

        let max_results = params.max_results.or(Some(100));

        match rpg_nav::impact::compute_impact_radius(
            graph,
            &params.entity_id,
            dir,
            max_depth,
            edge_filter,
            max_results,
        ) {
            Some(result) => Ok(format!(
                "{}{}",
                notice,
                rpg_nav::toon::format_impact_radius(&result),
            )),
            None => Err(format!("Entity not found: {}", params.entity_id)),
        }
    }

    #[tool(
        description = "Plan code changes: find relevant entities, compute modification order, assess impact radius. Returns dependency-ordered entity list with blast radius analysis."
    )]
    async fn plan_change(
        &self,
        Parameters(params): Parameters<PlanChangeParams>,
    ) -> Result<String, String> {
        self.ensure_graph().await?;
        let notice = self.staleness_notice().await;
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        // Attempt hybrid embedding search
        self.try_init_embeddings(graph).await;
        let embedding_scores = {
            let mut emb_guard = self.embedding_index.write().await;
            if let Some(ref mut idx) = *emb_guard {
                idx.score_all(&params.goal).ok().filter(|s| !s.is_empty())
            } else {
                None
            }
        };

        let request = rpg_nav::planner::PlanChangeRequest {
            goal: &params.goal,
            scope: params.scope.as_deref(),
            max_entities: params.max_entities.unwrap_or(15),
        };

        let plan = rpg_nav::planner::plan_change(graph, &request, embedding_scores.as_ref());

        if plan.relevant_entities.is_empty() {
            return Ok(format!(
                "{}No relevant entities found for: {}",
                notice, params.goal
            ));
        }

        Ok(format!(
            "{}{}",
            notice,
            rpg_nav::planner::format_change_plan(&plan),
        ))
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

        // Validate all paths are strict 3-level hierarchy format
        let invalid: Vec<String> = assignments
            .iter()
            .filter_map(|(file, path)| {
                if is_three_level_hierarchy_path(path) {
                    None
                } else {
                    Some(format!("{} -> {}", file, path))
                }
            })
            .collect();
        if !invalid.is_empty() {
            let sample: Vec<&str> = invalid.iter().take(10).map(|s| s.as_str()).collect();
            return Err(format!(
                "Hierarchy paths must be `Area/category/subcategory` (3 levels).\n\
                 Invalid (showing up to 10):\n{}",
                sample.join("\n"),
            ));
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

impl RpgServer {
    /// Public accessor for the tool router generated by `#[tool_router]`.
    /// Needed because the macro generates a private method, but `new()` lives in server.rs.
    pub(crate) fn create_tool_router() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        Self::tool_router()
    }
}
