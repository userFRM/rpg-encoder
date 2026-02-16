//! RPG-Encoder MCP Server
//! Exposes SearchNode, FetchNode, ExploreRPG, BuildRPG, UpdateRPG as MCP tools over stdio.
//! Gives any connected LLM full semantic understanding of a codebase.

mod generation;
mod helpers;
mod hierarchy_helpers;
mod params;
mod quality;
mod server;
mod tools;
mod types;

use anyhow::Result;
use rmcp::ServiceExt;
use rpg_core::storage;
use std::path::PathBuf;

use server::RpgServer;

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
                // Detect paradigms for framework-aware classification
                let detected_langs = RpgServer::resolve_languages(&graph.metadata);
                let paradigm_defs =
                    rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
                let qcache_result =
                    rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs);
                let active_defs = rpg_parser::paradigms::detect_paradigms_toml(
                    &server.project_root,
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
                    &server.project_root,
                    None,
                    pipeline.as_ref(),
                ) {
                    Ok(s) => {
                        graph.metadata.paradigms = paradigm_names;
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
