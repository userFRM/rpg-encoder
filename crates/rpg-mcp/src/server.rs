//! `RpgServer` struct definition, non-tool methods, and `ServerHandler` impl.

use rmcp::{ServerHandler, model::ServerInfo, tool_handler};
use rpg_core::config::RpgConfig;
use rpg_core::graph::RPGraph;
use rpg_core::storage;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use crate::types::{HierarchySession, LiftingSession, PendingRouting, load_pending_routing};

/// Cached protocol prompt versions (SHA256 hashes) for deduplication.
#[derive(Clone)]
pub(crate) struct PromptVersions {
    pub(crate) synthesis: String,
}

impl PromptVersions {
    /// Compute SHA256 hash of a prompt and return first 8 hex chars as version ID.
    fn hash_prompt(content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)[..8].to_string()
    }

    /// Initialize with hashes of all protocol prompts.
    pub(crate) fn new() -> Self {
        Self {
            synthesis: Self::hash_prompt(include_str!("prompts/synthesis_instructions.md")),
        }
    }
}

/// The RPG MCP server state.
///
/// # Lock order invariant
///
/// Several of the fields below are `Arc<RwLock<T>>`. When a code path holds
/// more than one of them at the same time, locks must be acquired in the
/// following order (outermost first):
///
/// 1. `graph`
/// 2. `lifting_session` / `hierarchy_session`
/// 3. `stale_entity_ids`
/// 4. `pending_routing`
/// 5. `last_auto_sync_head` / `last_auto_sync_changeset` / `last_auto_sync_workdir_paths`
/// 6. `config`
/// 7. `embedding_index`
/// 8. `project_root_cell`
///
/// Paths that touch only one lock at a time are unaffected. Paths that
/// acquire several locks but release each before acquiring the next
/// (statement-per-lock pattern in `set_project_root` and `reload_rpg`)
/// are also unaffected — at no moment do they hold two locks, so no
/// cycle can form.
///
/// The invariant is needed because tokio's `RwLock` is not re-entrant
/// and writers block readers while waiting: two tasks that each hold
/// one inner lock and wait for the other's outer lock would deadlock.
/// Keeping `graph` as the outermost held lock everywhere ensures that
/// any two nested paths serialize through `graph` and never form a
/// cycle on the inner locks.
#[derive(Clone)]
pub(crate) struct RpgServer {
    /// Active project root. Mutable at runtime via the `set_project_root` tool
    /// so a single long-lived session can switch between projects without
    /// restart. Tools acquire a snapshot via [`RpgServer::project_root`].
    pub(crate) project_root_cell: Arc<RwLock<PathBuf>>,
    pub(crate) graph: Arc<RwLock<Option<RPGraph>>>,
    pub(crate) config: Arc<RwLock<RpgConfig>>,
    pub(crate) lifting_session: Arc<RwLock<Option<LiftingSession>>>,
    pub(crate) hierarchy_session: Arc<RwLock<Option<HierarchySession>>>,
    pub(crate) pending_routing: Arc<RwLock<Vec<PendingRouting>>>,
    #[cfg(feature = "embeddings")]
    pub(crate) embedding_index: Arc<RwLock<Option<rpg_nav::embeddings::EmbeddingIndex>>>,
    /// Set to true after first failed init to avoid retrying every search.
    #[cfg(feature = "embeddings")]
    pub(crate) embedding_init_failed: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
    /// Protocol prompt versions for deduplication.
    pub(crate) prompt_versions: PromptVersions,
    /// Last git HEAD SHA at which auto-sync ran. Prevents redundant updates.
    pub(crate) last_auto_sync_head: Arc<RwLock<Option<String>>>,
    /// Entity IDs whose source was modified after their features were lifted.
    /// Populated by `auto_sync_if_stale` (from `summary.modified_entity_ids`),
    /// drained by `submit_lift_results` as entities get re-lifted. Lets
    /// `lifting_status` surface stale-feature drift even though those entities
    /// still appear "lifted" in the coverage count.
    pub(crate) stale_entity_ids: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Hash of the last-synced workdir changeset (dirty files + their stat).
    /// Combined with `last_auto_sync_head` to detect when a re-sync is needed
    /// for uncommitted/staged/unstaged changes.
    pub(crate) last_auto_sync_changeset: Arc<RwLock<Option<String>>>,
    /// Paths that were dirty at the last successful auto-sync. Lets us detect
    /// reverts: when a previously-dirty file returns to clean, the workdir
    /// diff no longer lists it — we must re-parse it to restore HEAD content.
    pub(crate) last_auto_sync_workdir_paths:
        Arc<RwLock<std::collections::HashSet<std::path::PathBuf>>>,
    /// Guard: true while auto_lift is running. Rejects concurrent lift calls.
    pub(crate) lift_in_progress: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for RpgServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpgServer")
            .field("project_root", &"<lock>")
            .field("lifting_session", &"...")
            .finish()
    }
}

impl RpgServer {
    /// Snapshot of the active project root. Cheap — locks for a single clone.
    pub(crate) async fn project_root(&self) -> PathBuf {
        self.project_root_cell.read().await.clone()
    }

    /// Reload `.rpg/config.toml` into the given config slot.
    /// - File missing → silently use defaults (the no-config-yet case).
    /// - File present but malformed → log a warning, keep the existing config.
    /// - File present and valid → swap.
    pub(crate) async fn reload_config_with_warning(
        slot: &Arc<RwLock<RpgConfig>>,
        project_root: &std::path::Path,
    ) {
        let config_path = project_root.join(".rpg/config.toml");
        if !config_path.exists() {
            // Missing is a normal state — use defaults silently.
            *slot.write().await = RpgConfig::default();
            return;
        }
        match RpgConfig::load(project_root) {
            Ok(cfg) => {
                *slot.write().await = cfg;
            }
            Err(e) => {
                eprintln!(
                    "rpg: failed to parse {} ({}); keeping previous in-memory config",
                    config_path.display(),
                    e
                );
                // Do NOT overwrite — leave the previous (working) config in place.
            }
        }
    }

    /// Create a new server, loading graph and config from `project_root` if present.
    pub(crate) fn new(project_root: PathBuf) -> Self {
        let graph = storage::load(&project_root).ok();
        let initial_head = graph.as_ref().and_then(|g| g.base_commit.clone());
        let config = RpgConfig::load(&project_root).unwrap_or_default();
        // Restore pending routing from disk if present
        let pending = load_pending_routing(&project_root)
            .map(|s| s.entries)
            .unwrap_or_default();
        Self {
            project_root_cell: Arc::new(RwLock::new(project_root)),
            graph: Arc::new(RwLock::new(graph)),
            config: Arc::new(RwLock::new(config)),
            lifting_session: Arc::new(RwLock::new(None)),
            hierarchy_session: Arc::new(RwLock::new(None)),
            pending_routing: Arc::new(RwLock::new(pending)),
            #[cfg(feature = "embeddings")]
            embedding_index: Arc::new(RwLock::new(None)),
            #[cfg(feature = "embeddings")]
            embedding_init_failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tool_router: Self::create_tool_router(),
            prompt_versions: PromptVersions::new(),
            last_auto_sync_head: Arc::new(RwLock::new(initial_head)),
            stale_entity_ids: Arc::new(RwLock::new(std::collections::HashSet::new())),
            last_auto_sync_changeset: Arc::new(RwLock::new(None)),
            last_auto_sync_workdir_paths: Arc::new(RwLock::new(std::collections::HashSet::new())),
            lift_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Check if the loaded graph is stale (behind git HEAD) and return a notice string.
    pub(crate) async fn staleness_notice(&self) -> String {
        let project_root = self.project_root().await;
        let guard = self.graph.read().await;
        let Some(graph) = guard.as_ref() else {
            return String::new();
        };
        // Detect workdir changes (committed + staged + unstaged)
        let Ok(changes) = rpg_encoder::evolution::detect_workdir_changes(&project_root, graph)
        else {
            return String::new();
        };
        let changes = rpg_encoder::evolution::filter_rpgignore_changes(&project_root, changes);
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

    /// Auto-sync the graph if stale, returning a notice string.
    ///
    /// Syncs the graph to match the current **working tree** (committed + staged
    /// + unstaged). Triggers on:
    ///
    /// 1. HEAD changed (commits, merges, rebases).
    /// 2. Any workdir file in the relevant language set added/modified/deleted/renamed.
    /// 3. A previously-dirty file returning to clean state (revert detection).
    ///
    /// Uses `last_auto_sync_head` + `last_auto_sync_changeset` to skip re-sync
    /// when nothing changed since the last successful run. The changeset hash
    /// covers path + `(size, mtime)` stat so repeated saves trigger re-sync
    /// but idle queries don't.
    ///
    /// Structural-only update (no re-lifting). On error, does **not** cache
    /// markers — the next query will retry, so transient failures don't
    /// silently leave the server stale.
    pub(crate) async fn auto_sync_if_stale(&self) -> String {
        let project_root = self.project_root().await;
        // Step 1: Get current HEAD (cheap, just opens .git/HEAD)
        let Ok(current_head) = rpg_encoder::evolution::get_head_sha(&project_root) else {
            return self.staleness_notice().await;
        };

        // Step 2: Detect current workdir state under read lock
        let (source_changes, current_paths, current_changeset) = {
            let guard = self.graph.read().await;
            let Some(graph) = guard.as_ref() else {
                return String::new();
            };
            let Ok(changes) = rpg_encoder::evolution::detect_workdir_changes(&project_root, graph)
            else {
                return String::new();
            };
            let changes = rpg_encoder::evolution::filter_rpgignore_changes(&project_root, changes);
            let languages = Self::resolve_languages(&graph.metadata);
            let source_changes = if languages.is_empty() {
                changes
            } else {
                rpg_encoder::evolution::filter_source_changes(changes, &languages)
            };
            let paths = Self::change_paths(&source_changes);
            let hash = Self::compute_changeset_hash(&source_changes, &project_root);
            (source_changes, paths, hash)
        };

        // Step 3: Union with previously-dirty paths (revert detection).
        // Any file that was dirty last time but isn't in the current workdir
        // diff has returned to clean state — we need to re-parse it to restore
        // HEAD content in the graph.
        let effective_changes = {
            let last_paths = self.last_auto_sync_workdir_paths.read().await;
            let mut effective = source_changes.clone();
            for path in last_paths.difference(&current_paths) {
                let abs = project_root.join(path);
                if abs.is_file() {
                    effective.push(rpg_encoder::evolution::FileChange::Modified(path.clone()));
                } else {
                    effective.push(rpg_encoder::evolution::FileChange::Deleted(path.clone()));
                }
            }
            effective
        };

        // Step 4: Check if (HEAD, changeset) matches last-synced state
        {
            let last_head = self.last_auto_sync_head.read().await;
            let last_changeset = self.last_auto_sync_changeset.read().await;
            if last_head.as_deref() == Some(current_head.as_str())
                && last_changeset.as_deref() == Some(current_changeset.as_str())
            {
                return String::new(); // already synced this exact state
            }
        }

        // Step 5: Nothing to apply — just update markers (HEAD moved with no source diff)
        if effective_changes.is_empty() {
            *self.last_auto_sync_head.write().await = Some(current_head);
            *self.last_auto_sync_changeset.write().await = Some(current_changeset);
            *self.last_auto_sync_workdir_paths.write().await = current_paths;
            return String::new();
        }

        // Step 6: Acquire write lock and run update with our composed change set
        let mut guard = self.graph.write().await;
        let Some(graph) = guard.as_mut() else {
            return String::new();
        };

        // Paradigm setup for framework-aware classification
        let detected_langs = Self::resolve_languages(&graph.metadata);
        let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
        let qcache_result =
            rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs);
        let active_defs = rpg_parser::paradigms::detect_paradigms_toml(
            &project_root,
            &detected_langs,
            &paradigm_defs,
        );
        let paradigm_names: Vec<String> = active_defs.iter().map(|d| d.name.clone()).collect();
        let pipeline_and_result = qcache_result.ok().map(|qcache| (qcache, active_defs));
        let pipeline = pipeline_and_result.as_ref().map(|(qcache, active_defs)| {
            rpg_encoder::evolution::ParadigmPipeline {
                active_defs: active_defs.clone(),
                qcache,
            }
        });

        match rpg_encoder::evolution::run_update_from_changes(
            graph,
            &project_root,
            effective_changes,
            pipeline.as_ref(),
        ) {
            Ok(summary) => {
                graph.metadata.paradigms = paradigm_names;
                let _ = storage::save(&project_root, graph);

                // Persist stale entity IDs so lifting_status can surface
                // stale-feature drift in subsequent calls. These entities
                // still count as "lifted" by coverage(), so without this
                // set, lifting_status would report "100% coverage" while
                // search_node returns outdated features.
                //
                // Each inner write below is statement-per-lock — no two
                // inner locks are held at once while we're also holding
                // graph.write(), so the order between them is irrelevant
                // for correctness (see the lock-order doc on RpgServer).
                {
                    let mut stale = self.stale_entity_ids.write().await;
                    for id in &summary.modified_entity_ids {
                        stale.insert(id.clone());
                    }
                    // Prune entries for entities that no longer exist
                    stale.retain(|id| graph.entities.contains_key(id));
                }
                *self.last_auto_sync_head.write().await = Some(current_head);
                *self.last_auto_sync_changeset.write().await = Some(current_changeset);
                *self.last_auto_sync_workdir_paths.write().await = current_paths;

                if summary.entities_added == 0
                    && summary.entities_modified == 0
                    && summary.entities_removed == 0
                {
                    return String::new();
                }

                let (lifted, total) = graph.lifting_coverage();
                let total_unlifted = total - lifted;
                let added_now = summary.entities_added;
                let stale_now = summary.modified_entity_ids.len();
                let aggregate_drift = total_unlifted + stale_now;

                let mut notice = format!(
                    "[auto-synced: +{} -{} ~{} entities",
                    summary.entities_added, summary.entities_removed, summary.entities_modified,
                );

                if aggregate_drift > 0 {
                    // Per-update delta — what THIS sync changed
                    let mut parts: Vec<String> = Vec::new();
                    if added_now > 0 {
                        parts.push(format!("+{} added unlifted", added_now));
                    }
                    if stale_now > 0 {
                        parts.push(format!("~{} stale features", stale_now));
                    }
                    if !parts.is_empty() {
                        notice.push_str("; ");
                        notice.push_str(&parts.join(", "));
                    }
                    // Pre-existing backlog (entities that were already unlifted before this update)
                    let pre_existing = total_unlifted.saturating_sub(added_now);
                    if pre_existing > 0 {
                        if parts.is_empty() {
                            notice.push_str(&format!("; {} unlifted total", pre_existing));
                        } else {
                            notice.push_str(&format!(" (+{} pre-existing)", pre_existing));
                        }
                    }
                    // Active recommendation — graded by aggregate severity. The
                    // batch-0 NOTE in get_entities_for_lifting is authoritative
                    // for the dispatch decision; this is a heuristic gate.
                    if aggregate_drift >= crate::LARGE_SCOPE_ENTITIES {
                        notice.push_str(
                            " — semantic search is incomplete; call lifting_status for re-lift dispatch guidance",
                        );
                    } else {
                        notice.push_str(
                            " — semantic search is incomplete; call lifting_status to refresh",
                        );
                    }
                }
                notice.push_str("]\n\n");
                notice
            }
            Err(e) => {
                eprintln!("rpg: auto-sync failed (non-fatal): {e}");
                // Do NOT cache markers on error — the next call must retry.
                // Silent staleness is worse than repeated sync attempts.
                drop(guard);
                self.staleness_notice().await
            }
        }
    }

    /// Extract the path set from a changeset (for revert detection).
    fn change_paths(
        changes: &[rpg_encoder::evolution::FileChange],
    ) -> std::collections::HashSet<std::path::PathBuf> {
        use rpg_encoder::evolution::FileChange;
        changes
            .iter()
            .map(|c| match c {
                FileChange::Added(p) | FileChange::Modified(p) | FileChange::Deleted(p) => {
                    p.clone()
                }
                FileChange::Renamed { to, .. } => to.clone(),
            })
            .collect()
    }

    /// Compute a stable hash of the current workdir changeset.
    ///
    /// Includes the path, change type, and `(size, mtime)` stat for each
    /// added/modified/renamed file. Deleted files hash their path only.
    /// Same changeset + same stat = same hash = no re-sync. Second save of the
    /// same file changes mtime → different hash → re-sync fires.
    pub(crate) fn compute_changeset_hash(
        changes: &[rpg_encoder::evolution::FileChange],
        project_root: &std::path::Path,
    ) -> String {
        use rpg_encoder::evolution::FileChange;
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        for change in changes {
            match change {
                FileChange::Added(p) | FileChange::Modified(p) => {
                    hasher.update(format!("{:?}", change).as_bytes());
                    if let Ok(meta) = std::fs::metadata(project_root.join(p)) {
                        hasher.update(meta.len().to_le_bytes());
                        if let Ok(modified) = meta.modified()
                            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
                        {
                            hasher.update(duration.as_nanos().to_le_bytes());
                        }
                    }
                }
                FileChange::Deleted(p) => {
                    hasher.update(format!("deleted:{}", p.display()).as_bytes());
                }
                FileChange::Renamed { from, to } => {
                    hasher
                        .update(format!("renamed:{}->{}", from.display(), to.display()).as_bytes());
                    if let Ok(meta) = std::fs::metadata(project_root.join(to)) {
                        hasher.update(meta.len().to_le_bytes());
                        if let Ok(modified) = meta.modified()
                            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
                        {
                            hasher.update(duration.as_nanos().to_le_bytes());
                        }
                    }
                }
            }
        }
        let result = hasher.finalize();
        format!("{:x}", result)[..16].to_string()
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

        let project_root = self.project_root().await;
        match storage::load(&project_root) {
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
    pub(crate) async fn staleness_detail(&self, graph: &RPGraph) -> Option<String> {
        let project_root = self.project_root().await;
        let changes = rpg_encoder::evolution::detect_workdir_changes(&project_root, graph).ok()?;
        let changes = rpg_encoder::evolution::filter_rpgignore_changes(&project_root, changes);
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
        let stale_detail = self.staleness_detail(graph).await;
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

        // Stale-feature drift — entities still counted as "lifted" because
        // they have features, but those features are out of date because
        // the source was modified after lifting. Tracked across syncs by
        // auto_sync_if_stale.
        let stale_features_count = {
            let stale = self.stale_entity_ids.read().await;
            // Filter to entities still present in the graph
            stale
                .iter()
                .filter(|id| graph.entities.contains_key(*id))
                .count()
        };

        let mut out = format!(
            "=== RPG Lifting Status ===\n\
             {}\n\
             coverage: {}/{} ({:.0}%)\n\
             hierarchy: {}\n",
            graph_line, lifted, total, coverage_pct, hierarchy_type,
        );
        if stale_features_count > 0 {
            out.push_str(&format!(
                "stale_features: {} entities modified since last lift (features outdated)\n",
                stale_features_count,
            ));
        }

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

        // NEXT STEP — state machine guidance, staleness takes priority.
        // `LARGE_SCOPE_ENTITIES` is the threshold above which direct
        // foreground lifting is not recommended. See also the matching
        // check in `get_entities_for_lifting` which expresses the same
        // heuristic in terms of batches.
        //
        // The state machine considers two kinds of "work remaining":
        //   - `remaining` — entities that have never been lifted
        //   - `stale_features_count` — entities with outdated features after
        //     a source modification (tracked across syncs via
        //     `stale_entity_ids`). These look "lifted" in coverage but their
        //     features no longer reflect the source.
        // Their sum is what actually needs LLM work.
        out.push('\n');
        let remaining = total.saturating_sub(lifted);
        let work_remaining = remaining + stale_features_count;

        if stale_detail.is_some() {
            out.push_str("NEXT STEP: Graph is stale. Call update_rpg to sync with code changes, then lift any new or modified entities.\n");
        } else if work_remaining >= crate::LARGE_SCOPE_ENTITIES {
            // Large repo — recommend delegating the mechanical loop so the
            // caller doesn't exhaust its own context. Give the dispatch
            // pattern *directly* here rather than bouncing the caller through
            // get_entities_for_lifting first (which would burn batch-0's
            // source payload in the caller's context, the exact thing we're
            // trying to avoid).
            //
            // Note: `remaining` is the raw unlifted count *before* auto-lift
            // runs (which happens inside get_entities_for_lifting). Auto-lift
            // shrinks the LLM-needed set considerably for repos with many
            // trivial entities (getters, setters, constructors). The agent
            // can skip the dispatch if, once the worker calls
            // get_entities_for_lifting batch 0, no delegation NOTE appears —
            // in that case the queue is small enough to lift in one context.
            let batch_tokens = self.config.read().await.encoding.max_batch_tokens;
            let workload_desc = if remaining > 0 && stale_features_count > 0 {
                format!(
                    "{} unlifted + {} stale = {} entities",
                    remaining, stale_features_count, work_remaining,
                )
            } else if stale_features_count > 0 {
                format!(
                    "{} stale entities to re-lift (modified since last lift)",
                    stale_features_count,
                )
            } else {
                format!("{} entities unlifted", remaining)
            };
            out.push_str(&format!(
                "NEXT STEP: Likely-large lifting workload — {} (auto-lift may reduce this). Dispatch a sub-agent to run the LOOP below; do not run it in this context — each batch is ~{}K tokens of source and will exhaust caller context over many iterations.\n",
                workload_desc,
                batch_tokens.div_ceil(1000),
            ));
            out.push_str(
                "\nLOOP (sub-agent runs this in its own context):\n  \
                 get_entities_for_lifting(scope=\"*\") -> analyze batch -> submit_lift_results -> repeat until DONE -> finalize_lifting\n\
                 \nDISPATCH:\n  \
                 Use whatever sub-agent / cheaper-model mechanism your runtime provides. The MCP graph persists to disk after every submit, so the worker's writes survive. **After the worker returns, call `reload_rpg`** — some runtimes give sub-agents an isolated MCP session, in which case the caller's in-memory graph is stale until reloaded. (No-op if your runtime shares the MCP session.)\n\
                 \nFALLBACK (no sub-agent mechanism, no API key):\n  \
                 Scope the lift to one subtree at a time — e.g. get_entities_for_lifting(scope=\"src/auth/**\") — and submit features per batch within that scope. Each scoped batch fits in foreground context. Call finalize_lifting ONCE at the very end after all scopes are complete; calling it mid-flow auto-routes pending entities against incomplete signals and locks the hierarchy in early.\n",
            );
            // The CLI fallback only helps when there is unlifted work.
            // `rpg-encoder lift` resolves `scope="*"` to entities with no
            // features, so a stale-only backlog (features present, sources
            // modified) is a no-op for the CLI — surfacing it there would
            // be a dead-end recipe.
            if remaining > 0 {
                out.push_str(
                    "\nFALLBACK (no sub-agent mechanism, API key available, unlifted entities only):\n  \
                     Run `rpg-encoder lift --provider anthropic` (or `openai`) from the terminal — the CLI drives an external LLM directly with no agent involvement. After the CLI finishes, call `reload_rpg` in this session so the server picks up the updated graph from disk. Note: the CLI lifts entities with no features; stale entities (features present but outdated) must be re-lifted via the MCP loop above.\n",
                );
            }
        } else if lifted == 0 {
            out.push_str(
                "NEXT STEP: Call get_entities_for_lifting(scope=\"*\") to start lifting.\n",
            );
        } else if remaining > 0 && stale_features_count > 0 {
            out.push_str(&format!(
                "NEXT STEP: {} unlifted + {} stale = {} entities need LLM work. Call get_entities_for_lifting(scope=\"*\") — it returns both unlifted entities and stale ones that need re-lifting in the same batches.\n",
                remaining, stale_features_count, work_remaining,
            ));
        } else if remaining > 0 {
            out.push_str(&format!(
                "NEXT STEP: {} entities remaining. Call get_entities_for_lifting(scope=\"*\") to continue lifting.\n",
                remaining,
            ));
        } else if stale_features_count > 0 {
            // All entities have features, but some features are outdated.
            // The post-sync delta is what matters here — we track modified
            // entities in stale_entity_ids so agents know to re-lift them.
            out.push_str(&format!(
                "NEXT STEP: Coverage is 100% but {} entities have stale features (source modified after lift). Call get_entities_for_lifting(scope=\"*\") to re-lift just those — it surfaces stale entities as if they were unlifted.\n",
                stale_features_count,
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

    /// Load the RPG config for the active project.
    pub(crate) async fn load_config(&self) -> RpgConfig {
        let project_root = self.project_root().await;
        RpgConfig::load(&project_root).unwrap_or_default()
    }

    /// Lazy-initialize the embedding index on first semantic search.
    /// If init fails, logs a warning and sets a flag to avoid retrying.
    #[cfg(feature = "embeddings")]
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

        let project_root = self.project_root().await;
        let updated_at = graph.updated_at.to_rfc3339();
        match rpg_nav::embeddings::EmbeddingIndex::load_or_init(&project_root, &updated_at) {
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
    #[cfg(feature = "embeddings")]
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
                .enable_tool_list_changed()
                .build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_version_stability() {
        // Hash should be stable for same content
        let v1 = PromptVersions::new();
        let v2 = PromptVersions::new();
        assert_eq!(v1.synthesis, v2.synthesis);
    }

    #[test]
    fn test_prompt_version_format() {
        // Version should be exactly 8 hex characters
        let versions = PromptVersions::new();
        assert_eq!(versions.synthesis.len(), 8);
        assert!(
            versions.synthesis.chars().all(|c| c.is_ascii_hexdigit()),
            "Version should be hex: {}",
            versions.synthesis
        );
    }

    #[test]
    fn test_prompt_version_different_content() {
        // Different content should produce different hashes
        let hash1 = PromptVersions::hash_prompt("content A");
        let hash2 = PromptVersions::hash_prompt("content B");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_changeset_hash_empty_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let h1 = RpgServer::compute_changeset_hash(&[], tmp.path());
        let h2 = RpgServer::compute_changeset_hash(&[], tmp.path());
        assert_eq!(h1, h2, "empty changeset hash must be deterministic");
    }

    #[test]
    fn test_changeset_hash_differs_by_path() {
        use rpg_encoder::evolution::FileChange;
        let tmp = tempfile::tempdir().unwrap();
        let h_deleted_a =
            RpgServer::compute_changeset_hash(&[FileChange::Deleted("a.rs".into())], tmp.path());
        let h_deleted_b =
            RpgServer::compute_changeset_hash(&[FileChange::Deleted("b.rs".into())], tmp.path());
        assert_ne!(
            h_deleted_a, h_deleted_b,
            "different paths → different hashes"
        );
    }

    #[test]
    fn test_changeset_hash_reflects_mtime() {
        use rpg_encoder::evolution::FileChange;
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("x.rs");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"v1")
            .unwrap();

        let change = FileChange::Modified("x.rs".into());
        let h1 = RpgServer::compute_changeset_hash(std::slice::from_ref(&change), tmp.path());

        // Different content + guaranteed-later mtime by sleeping a moment
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"v2_different_length")
            .unwrap();

        let h2 = RpgServer::compute_changeset_hash(std::slice::from_ref(&change), tmp.path());
        assert_ne!(
            h1, h2,
            "same path + different size/mtime must yield different hashes"
        );
    }
}
