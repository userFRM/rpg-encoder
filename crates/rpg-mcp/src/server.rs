//! `RpgServer` struct definition, non-tool methods, and `ServerHandler` impl.

use rmcp::{ServerHandler, model::ServerInfo, tool_handler};
use rpg_core::config::RpgConfig;
use rpg_core::graph::RPGraph;
use rpg_core::storage;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use crate::types::{LiftingSession, PendingRouting, load_pending_routing};

/// The RPG MCP server state.
#[derive(Clone)]
pub(crate) struct RpgServer {
    pub(crate) project_root: PathBuf,
    pub(crate) graph: Arc<RwLock<Option<RPGraph>>>,
    pub(crate) config: Arc<RwLock<RpgConfig>>,
    pub(crate) lifting_session: Arc<RwLock<Option<LiftingSession>>>,
    pub(crate) pending_routing: Arc<RwLock<Vec<PendingRouting>>>,
    pub(crate) embedding_index: Arc<RwLock<Option<rpg_nav::embeddings::EmbeddingIndex>>>,
    /// Set to true after first failed init to avoid retrying every search.
    pub(crate) embedding_init_failed: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
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
    /// Create a new server, loading graph and config from `project_root` if present.
    pub(crate) fn new(project_root: PathBuf) -> Self {
        let graph = storage::load(&project_root).ok();
        let config = RpgConfig::load(&project_root).unwrap_or_default();
        // Restore pending routing from disk if present
        let pending = load_pending_routing(&project_root)
            .map(|s| s.entries)
            .unwrap_or_default();
        Self {
            project_root,
            graph: Arc::new(RwLock::new(graph)),
            config: Arc::new(RwLock::new(config)),
            lifting_session: Arc::new(RwLock::new(None)),
            pending_routing: Arc::new(RwLock::new(pending)),
            embedding_index: Arc::new(RwLock::new(None)),
            embedding_init_failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tool_router: Self::create_tool_router(),
        }
    }

    /// Check if the loaded graph is stale (behind git HEAD) and return a notice string.
    pub(crate) async fn staleness_notice(&self) -> String {
        let guard = self.graph.read().await;
        let Some(graph) = guard.as_ref() else {
            return String::new();
        };
        // Detect workdir changes (committed + staged + unstaged)
        let Ok(changes) = rpg_encoder::evolution::detect_workdir_changes(&self.project_root, graph)
        else {
            return String::new();
        };
        let changes = rpg_encoder::evolution::filter_rpgignore_changes(&self.project_root, changes);
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
    pub(crate) fn resolve_languages(
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

    /// Collect paradigm-specific prompt hints for detected frameworks.
    ///
    /// Uses a cached copy of the builtin paradigm defs (parsed once via OnceLock),
    /// filters by the paradigm names stored in `graph.metadata.paradigms`, and
    /// concatenates the requested hint type from each active paradigm.
    pub(crate) fn collect_paradigm_hints(
        paradigm_names: &[String],
        hint_selector: fn(&rpg_parser::paradigms::defs::PromptHints) -> &Option<String>,
    ) -> String {
        static PARADIGM_DEFS: OnceLock<Vec<rpg_parser::paradigms::defs::ParadigmDef>> =
            OnceLock::new();
        let defs = PARADIGM_DEFS
            .get_or_init(|| rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default());
        let mut hints = String::new();
        for name in paradigm_names {
            if let Some(def) = defs.iter().find(|d| &d.name == name)
                && let Some(hint) = hint_selector(&def.prompt_hints)
            {
                if !hints.is_empty() {
                    hints.push('\n');
                }
                hints.push_str(&format!("### {} Guidelines\n", def.name));
                hints.push_str(hint.trim());
                hints.push('\n');
            }
        }
        hints
    }

    /// Ensure a graph is loaded in memory, attempting disk load if needed.
    pub(crate) async fn ensure_graph(&self) -> Result<(), String> {
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
    pub(crate) fn staleness_detail(&self, graph: &RPGraph) -> Option<String> {
        let changes =
            rpg_encoder::evolution::detect_workdir_changes(&self.project_root, graph).ok()?;
        let changes = rpg_encoder::evolution::filter_rpgignore_changes(&self.project_root, changes);
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

    /// Format the full lifting status dashboard (coverage, areas, session, next step).
    pub(crate) async fn format_lifting_status(&self, graph: &RPGraph) -> Result<String, String> {
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

    /// Load config synchronously (for contexts that cannot await).
    pub(crate) fn get_config_blocking(&self) -> RpgConfig {
        RpgConfig::load(&self.project_root).unwrap_or_default()
    }

    /// Lazy-initialize the embedding index on first semantic search.
    /// If init fails, logs a warning and sets a flag to avoid retrying.
    pub(crate) async fn try_init_embeddings(&self, graph: &RPGraph) {
        // Skip if already initialized or previously failed
        if self.embedding_index.read().await.is_some() {
            return;
        }
        if self
            .embedding_init_failed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }

        let updated_at = graph.updated_at.to_rfc3339();
        match rpg_nav::embeddings::EmbeddingIndex::load_or_init(&self.project_root, &updated_at) {
            Ok(mut idx) => {
                // Incremental sync: only re-embed entities whose features changed
                if let Err(e) = idx.sync(graph) {
                    eprintln!("rpg: embedding sync failed: {e}");
                }
                *self.embedding_index.write().await = Some(idx);
            }
            Err(e) => {
                eprintln!("rpg: embedding init failed: {e} — using lexical search");
                self.embedding_init_failed
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Update embeddings for entities that just received new features.
    /// Also updates fingerprints so that the next `sync()` won't re-embed these.
    pub(crate) async fn update_embeddings(
        &self,
        entity_features: &std::collections::HashMap<String, Vec<String>>,
        graph_updated_at: &str,
    ) {
        let mut guard = self.embedding_index.write().await;
        if let Some(ref mut idx) = *guard {
            idx.set_graph_updated_at(graph_updated_at);
            if let Err(e) = idx.embed_entities(entity_features) {
                eprintln!("rpg: embedding update failed: {e}");
                return;
            }
            // Update fingerprints for the newly-embedded entities so sync() sees them as current
            idx.update_fingerprints(entity_features);
            if let Err(e) = idx.save() {
                eprintln!("rpg: embedding save failed: {e}");
            }
        }
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
