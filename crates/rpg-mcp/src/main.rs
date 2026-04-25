//! RPG-Encoder MCP Server
//! Exposes SearchNode, FetchNode, ExploreRPG, BuildRPG, UpdateRPG as MCP tools over stdio.
//! Gives any connected LLM full semantic understanding of a codebase.

mod helpers;
mod hierarchy_helpers;
mod params;
mod server;
mod tools;
mod types;

/// Entity count above which `lifting_status` and similar dashboards switch to
/// recommending sub-agent delegation. **This is a heuristic gate, not the
/// authoritative dispatch decision.** The authoritative signal is the
/// batch-0 NOTE in `get_entities_for_lifting`, which sees the post-auto-lift
/// queue and uses the actual token-aware batch count. With user-tuned
/// `max_batch_tokens` or unusually small/large entities, the two can
/// diverge — when they do, the batch-0 NOTE wins. Both messages defer to
/// each other: dashboard says "check the NOTE", batch-0 NOTE is silent
/// when delegation isn't warranted.
pub(crate) const LARGE_SCOPE_ENTITIES: usize = 100;

/// Batch count above which `get_entities_for_lifting` emits the batch-0
/// dispatch note. Derived from `LARGE_SCOPE_ENTITIES` assuming ~10 entities
/// per token-aware batch at default config (batch_size=25,
/// max_batch_tokens=8000). Kept as a separate constant because the
/// auto-lifter shrinks the LLM-needed set before batching, so the ratio is
/// conservative. Authoritative for the dispatch decision (see
/// `LARGE_SCOPE_ENTITIES` for why).
pub(crate) const LARGE_SCOPE_BATCHES: usize = 10;

use anyhow::Result;
use rmcp::ServiceExt;
use rpg_core::storage;
use server::RpgServer;

#[tokio::main]
async fn main() -> Result<()> {
    let cli_args: Vec<String> = std::env::args().collect();
    let cli_root = RpgServer::startup_project_root_arg(cli_args.iter().map(String::as_str));
    let cwd = std::env::current_dir().expect("failed to get current directory");
    let project_root = RpgServer::resolve_startup_project_root(cli_root.as_deref(), cwd);

    eprintln!("RPG MCP server starting for: {}", project_root.display());

    eprintln!("  Tip: Use get_entities_for_lifting + submit_lift_results for semantic features");

    let server = RpgServer::new(project_root);

    // Auto-update graph on startup if stale (structural-only, no LLM)
    {
        let project_root = server.project_root().await;
        let root_state = server.root_state(&project_root).await;
        let mut root_state = root_state.write().await;
        if let Some(ref mut graph) = root_state.graph
            && let (Some(base), Ok(head)) = (
                &graph.base_commit.clone(),
                rpg_encoder::evolution::get_head_sha(&project_root),
            )
        {
            if *base != head {
                eprintln!(
                    "  Graph stale ({}… → {}…). Auto-updating...",
                    &base[..8.min(base.len())],
                    &head[..8.min(head.len())]
                );
                // Detect paradigms for framework-aware classification
                let detected_langs = RpgServer::resolve_languages(&graph.metadata);
                let paradigm_defs =
                    rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
                let qcache_result =
                    rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs);
                let active_defs = rpg_parser::paradigms::detect_paradigms_toml(
                    &project_root,
                    &detected_langs,
                    &paradigm_defs,
                );
                let paradigm_names: Vec<String> =
                    active_defs.iter().map(|d| d.name.clone()).collect();
                let pipeline_and_result = qcache_result.ok().map(|qcache| (qcache, active_defs));
                let pipeline = pipeline_and_result.as_ref().map(|(qcache, active_defs)| {
                    rpg_encoder::evolution::ParadigmPipeline {
                        active_defs: active_defs.clone(),
                        qcache,
                    }
                });
                match rpg_encoder::evolution::run_update(
                    graph,
                    &project_root,
                    None,
                    pipeline.as_ref(),
                ) {
                    Ok(s) => {
                        graph.metadata.paradigms = paradigm_names;
                        let _ = storage::save(&project_root, graph);
                        let existing_ids: std::collections::HashSet<String> =
                            graph.entities.keys().cloned().collect();
                        {
                            let stale = &mut root_state.stale_entity_ids;
                            for id in &s.modified_entity_ids {
                                stale.insert(id.clone());
                            }
                            stale.retain(|id| existing_ids.contains(id));
                        }
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
            // Seed auto-sync markers for a clean post-startup workdir so
            // the first query short-circuits at server.rs's (HEAD,
            // changeset) match instead of redundantly re-running the
            // workdir diff. Must use the real empty-workdir changeset
            // hash (not an empty string) for the match to fire.
            root_state.last_auto_sync_head =
                rpg_encoder::evolution::get_head_sha(&project_root).ok();
            root_state.last_auto_sync_changeset =
                Some(RpgServer::compute_changeset_hash(&[], &project_root));
            root_state.last_auto_sync_workdir_paths = std::collections::HashSet::new();
            drop(root_state);
            server
                .sync_default_root_compat_from_state(&project_root)
                .await;
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
