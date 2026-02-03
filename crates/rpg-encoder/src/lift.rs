//! On-demand semantic lifting: lift a subset of entities and incrementally update the hierarchy.

use anyhow::Result;
use rpg_core::config::RpgConfig;
use rpg_core::graph::RPGraph;
use rpg_parser::entities::RawEntity;
use rpg_parser::languages::Language;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A resolved set of entity IDs to lift.
pub struct LiftScope {
    pub entity_ids: Vec<String>,
}

/// Result of a lift operation.
pub struct LiftResult {
    pub entities_lifted: usize,
    pub entities_failed: usize,
    pub entities_repaired: usize,
    pub hierarchy_updated: bool,
}

/// Resolve a scope specification into concrete entity IDs.
///
/// Supports:
/// - File globs: `src/auth/**` or `*.rs` — matched against entity file paths
/// - Hierarchy path prefix: `Auth/login` — collects via hierarchy subtree
/// - Comma-separated entity IDs: `src/foo.rs:bar,src/baz.rs:qux`
/// - `*` or `all` — all unlifted entities
pub fn resolve_scope(graph: &RPGraph, scope: &str) -> LiftScope {
    let scope = scope.trim();

    // "all" or "*" → all unlifted entities
    if scope == "*" || scope.eq_ignore_ascii_case("all") {
        let entity_ids = graph
            .entities
            .iter()
            .filter(|(_, e)| {
                e.semantic_features.is_empty() && e.kind != rpg_core::graph::EntityKind::Module
            })
            .map(|(id, _)| id.clone())
            .collect();
        return LiftScope { entity_ids };
    }

    // Try as glob pattern (contains * or ?)
    if (scope.contains('*') || scope.contains('?'))
        && let Ok(glob) = globset::Glob::new(scope)
    {
        let matcher = glob.compile_matcher();
        let entity_ids = graph
            .entities
            .iter()
            .filter(|(_, e)| matcher.is_match(&e.file))
            .map(|(id, _)| id.clone())
            .collect();
        return LiftScope { entity_ids };
    }

    // Try as hierarchy path prefix
    let mut hierarchy_ids: Vec<String> = Vec::new();
    for (area_name, area_node) in &graph.hierarchy {
        if area_name == scope || scope.starts_with(&format!("{}/", area_name)) {
            // Collect from this subtree
            if area_name == scope {
                hierarchy_ids.extend(area_node.all_entity_ids());
            } else {
                // Walk deeper into the path
                let remainder = &scope[area_name.len() + 1..];
                let parts: Vec<&str> = remainder.split('/').collect();
                let mut current = area_node;
                let mut found = true;
                for part in &parts {
                    if let Some(child) = current.children.get(*part) {
                        current = child;
                    } else {
                        found = false;
                        break;
                    }
                }
                if found {
                    hierarchy_ids.extend(current.all_entity_ids());
                }
            }
        }
    }
    if !hierarchy_ids.is_empty() {
        return LiftScope {
            entity_ids: hierarchy_ids,
        };
    }

    // Try as comma-separated entity IDs
    let entity_ids: Vec<String> = scope
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|id| graph.entities.contains_key(id))
        .collect();

    LiftScope { entity_ids }
}

/// Generate a compact repo overview from graph metadata (paper's `repo_info` context).
pub fn generate_repo_info(graph: &RPGraph) -> String {
    let lang = &graph.metadata.language;
    let total = graph.entities.len();
    let files = graph.metadata.total_files;

    let areas: Vec<&String> = graph.hierarchy.keys().collect();
    let area_list = if areas.len() <= 8 {
        areas
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("{} functional areas", areas.len())
    };

    let (lifted, _) = graph.lifting_coverage();
    if lifted > 0 {
        format!(
            "{} repository with {} entities across {} files ({} semantically lifted). Top-level modules: {}.",
            lang, total, files, lifted, area_list
        )
    } else {
        format!(
            "{} repository with {} entities across {} files. Top-level modules: {}.",
            lang, total, files, area_list
        )
    }
}

/// Re-read source files and collect RawEntity objects for the scoped entities.
/// This is needed because the graph Entity doesn't store source text.
pub fn collect_raw_entities(
    graph: &RPGraph,
    scope: &LiftScope,
    project_root: &Path,
) -> Result<Vec<RawEntity>> {
    let mut files_to_read: HashMap<std::path::PathBuf, Vec<String>> = HashMap::new();
    for id in &scope.entity_ids {
        if let Some(entity) = graph.entities.get(id) {
            // Skip Module entities — they get features via aggregation, not lifting
            if entity.kind == rpg_core::graph::EntityKind::Module {
                continue;
            }
            files_to_read
                .entry(entity.file.clone())
                .or_default()
                .push(id.clone());
        }
    }

    let language = Language::from_name(&graph.metadata.language)
        .ok_or_else(|| anyhow::anyhow!("unknown language: {}", graph.metadata.language))?;

    let mut raw_entities: Vec<RawEntity> = Vec::new();
    for (rel_path, entity_ids) in &files_to_read {
        let abs_path = project_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  Warning: could not read {}: {}", rel_path.display(), e);
                continue;
            }
        };

        let file_raws = rpg_parser::entities::extract_entities(rel_path, &source, language);

        let wanted: HashSet<&String> = entity_ids.iter().collect();
        for raw in file_raws {
            let raw_id = raw.id();
            if wanted.contains(&raw_id) {
                raw_entities.push(raw);
            }
        }
    }

    Ok(raw_entities)
}

/// Build token-budget-aware batches from a list of raw entities.
///
/// Per the paper's batching strategy: "accommodate repositories of varying scales
/// while respecting model context limits." Each batch is filled until either the
/// token budget or entity count cap is reached.
///
/// Returns a list of `(start, end)` index ranges into the input slice.
pub fn build_token_aware_batches(
    entities: &[RawEntity],
    max_count: usize,
    max_tokens: usize,
) -> Vec<(usize, usize)> {
    let mut batches = Vec::new();
    let mut batch_start = 0;
    let mut batch_tokens = 0usize;
    let mut batch_count = 0usize;

    for (i, entity) in entities.iter().enumerate() {
        // Estimate tokens: ~4 characters per token is a reasonable heuristic
        let est_tokens = entity.source_text.len() / 4 + 1;

        // Flush if adding this entity would exceed budget (but always include at least 1)
        if batch_count > 0 && (batch_tokens + est_tokens > max_tokens || batch_count >= max_count) {
            batches.push((batch_start, i));
            batch_start = i;
            batch_tokens = 0;
            batch_count = 0;
        }

        batch_tokens += est_tokens;
        batch_count += 1;
    }

    // Flush remaining
    if batch_count > 0 {
        batches.push((batch_start, entities.len()));
    }

    batches
}

/// Apply features from a batch to the graph, returning the number of entities that got features.
fn apply_batch_features(
    graph: &mut RPGraph,
    features: &HashMap<String, Vec<String>>,
    scope: &LiftScope,
    scoped_ids: &HashSet<&String>,
) -> usize {
    let mut count = 0;
    for (name, feats) in features {
        for id in &scope.entity_ids {
            if let Some(entity) = graph.entities.get_mut(id)
                && entity.name == *name
                && scoped_ids.contains(id)
                && entity.semantic_features.is_empty()
            {
                entity.semantic_features = feats.clone();
                count += 1;
            }
        }
    }
    count
}

/// Checkpoint: save graph to disk after each batch to preserve progress.
fn checkpoint_graph(graph: &RPGraph, project_root: &Path) {
    if let Err(e) = rpg_core::storage::save(project_root, graph) {
        eprintln!("  Warning: checkpoint save failed: {}", e);
    }
}

/// Lift entities matching the scope: run semantic lifting (features) on them,
/// then optionally update the hierarchy.
pub async fn lift_area(
    graph: &mut RPGraph,
    scope: &LiftScope,
    client: &crate::llm::LlmClient,
    project_root: &Path,
    config: &RpgConfig,
) -> Result<LiftResult> {
    if scope.entity_ids.is_empty() {
        return Ok(LiftResult {
            entities_lifted: 0,
            entities_failed: 0,
            entities_repaired: 0,
            hierarchy_updated: false,
        });
    }

    let scoped_ids: HashSet<&String> = scope.entity_ids.iter().collect();
    let raw_entities = collect_raw_entities(graph, scope, project_root)?;

    if raw_entities.is_empty() {
        return Ok(LiftResult {
            entities_lifted: 0,
            entities_failed: 0,
            entities_repaired: 0,
            hierarchy_updated: false,
        });
    }

    // Semantic lifting in token-budget-aware batches.
    // Per the paper: "batches are constructed to balance completeness and efficiency,
    // such that every semantic unit is analyzed exactly once while enabling scalable
    // processing of large repositories."
    let batch_ranges = build_token_aware_batches(
        &raw_entities,
        config.encoding.batch_size,
        config.encoding.max_batch_tokens,
    );
    let repo_name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let mut total_lifted = 0usize;
    let mut total_failed = 0usize;
    let total_batches = batch_ranges.len();
    let total_entities = scope.entity_ids.len();

    for (batch_idx, range) in batch_ranges.iter().enumerate() {
        let batch = &raw_entities[range.0..range.1];
        eprintln!(
            "  Lifting batch {}/{} ({} entities)...",
            batch_idx + 1,
            total_batches,
            batch.len()
        );

        let repo_info = generate_repo_info(graph);
        match crate::semantic_lifting::lift_batch(
            client,
            batch,
            repo_name,
            &repo_info,
            Some(&config.llm),
        )
        .await
        {
            Ok(features) => {
                let lifted = apply_batch_features(graph, &features, scope, &scoped_ids);
                total_lifted += lifted;

                // Checkpoint after each successful batch
                checkpoint_graph(graph, project_root);

                eprintln!(
                    "    {}/{} entities lifted so far",
                    total_lifted, total_entities
                );
            }
            Err(e) => {
                eprintln!("  Warning: batch {} failed: {}", batch_idx + 1, e);
                total_failed += batch.len();
            }
        }
    }

    // --- Repair pass: retry entities that still have no features ---
    let mut total_repaired = 0usize;
    let unlifted_indices: Vec<usize> = raw_entities
        .iter()
        .enumerate()
        .filter(|(_, raw)| {
            let raw_id = raw.id();
            graph
                .entities
                .get(&raw_id)
                .is_some_and(|e| e.semantic_features.is_empty())
        })
        .map(|(i, _)| i)
        .collect();

    if !unlifted_indices.is_empty() {
        let repair_batch_size = (config.encoding.batch_size / 2).max(1);
        let repair_batches = unlifted_indices.len().div_ceil(repair_batch_size);
        eprintln!(
            "  Repair pass: {} entities need retry ({} batches)...",
            unlifted_indices.len(),
            repair_batches
        );

        for (repair_idx, chunk) in unlifted_indices.chunks(repair_batch_size).enumerate() {
            let repair_batch: Vec<&RawEntity> = chunk.iter().map(|&i| &raw_entities[i]).collect();

            eprintln!(
                "  Repair batch {}/{} ({} entities)...",
                repair_idx + 1,
                repair_batches,
                repair_batch.len()
            );

            // Build a temporary slice for lift_batch (it needs &[RawEntity])
            let repair_entities: Vec<RawEntity> =
                repair_batch.iter().map(|r| (*r).clone()).collect();

            let repo_info = generate_repo_info(graph);
            match crate::semantic_lifting::lift_batch(
                client,
                &repair_entities,
                repo_name,
                &repo_info,
                Some(&config.llm),
            )
            .await
            {
                Ok(features) => {
                    let repaired = apply_batch_features(graph, &features, scope, &scoped_ids);
                    total_repaired += repaired;
                    total_failed = total_failed.saturating_sub(repaired);

                    // Checkpoint after each repair batch
                    checkpoint_graph(graph, project_root);
                }
                Err(e) => {
                    eprintln!("  Warning: repair batch {} failed: {}", repair_idx + 1, e);
                }
            }
        }

        if total_repaired > 0 {
            eprintln!("  Repair pass recovered {} entities", total_repaired);
        }
    }

    // Update hierarchy for newly-lifted entities
    let hierarchy_updated = if graph.metadata.semantic_hierarchy && !graph.hierarchy.is_empty() {
        // Semantic hierarchy exists: re-route lifted entities via find_best_parent
        let mut rerouted = 0;
        for id in &scope.entity_ids {
            if let Some(entity) = graph.entities.get(id).cloned() {
                if entity.semantic_features.is_empty() {
                    continue;
                }
                match crate::hierarchy::find_best_parent(
                    client,
                    &entity,
                    &graph.hierarchy,
                    repo_name,
                )
                .await
                {
                    Ok(new_path) => {
                        graph.remove_entity_from_hierarchy(id);
                        if let Some(e) = graph.entities.get_mut(id) {
                            e.hierarchy_path = new_path.clone();
                        }
                        graph.insert_into_hierarchy(&new_path, id);
                        rerouted += 1;
                    }
                    Err(e) => {
                        eprintln!("  Warning: re-routing {} failed: {}", id, e);
                    }
                }
            }
        }
        rerouted > 0
    } else {
        // No semantic hierarchy yet — check if we have enough coverage to build one
        let (lifted, total) = graph.lifting_coverage();
        let coverage = if total > 0 {
            lifted as f64 / total as f64
        } else {
            0.0
        };

        if coverage >= 0.3 && lifted >= 20 {
            // Enough coverage to build semantic hierarchy
            eprintln!(
                "  Lifting coverage {:.0}% — building semantic hierarchy...",
                coverage * 100.0
            );
            let entities_vec: Vec<_> = graph.entities.values().cloned().collect();
            match crate::hierarchy::discover_domains(client, &entities_vec, repo_name).await {
                Ok(areas) => {
                    eprintln!("  Discovered {} functional areas", areas.len());
                    match crate::hierarchy::build_hierarchy(
                        client,
                        &entities_vec,
                        &areas,
                        repo_name,
                        config.encoding.hierarchy_chunk_size,
                        config.encoding.hierarchy_concurrency,
                    )
                    .await
                    {
                        Ok(assignments) => {
                            // Clear file-path hierarchy and build semantic one
                            graph.hierarchy.clear();
                            crate::hierarchy::apply_hierarchy(graph, &assignments);
                            graph.metadata.semantic_hierarchy = true;
                            eprintln!(
                                "  Built semantic hierarchy ({} assignments)",
                                assignments.len()
                            );

                            // Generate repo-level summary
                            match crate::hierarchy::generate_repo_summary(client, graph, repo_name)
                                .await
                            {
                                Ok(summary) => {
                                    eprintln!("  Generated repo summary");
                                    graph.metadata.repo_summary = Some(summary);
                                }
                                Err(e) => {
                                    eprintln!("  Warning: repo summary generation failed: {}", e);
                                }
                            }

                            true
                        }
                        Err(e) => {
                            eprintln!("  Warning: hierarchy construction failed: {}", e);
                            false
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  Warning: domain discovery failed: {}", e);
                    false
                }
            }
        } else {
            false
        }
    };

    // Aggregate child entity features onto Module entities (per-file summaries)
    graph.aggregate_module_features();

    // Re-run hierarchy enrichment
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();
    crate::grounding::ground_hierarchy(graph);
    graph.refresh_metadata();

    // Auto-embed lifted entities if an embedding provider is available
    let entities_embedded =
        match crate::embeddings::EmbeddingGenerator::from_config(&config.embeddings) {
            Ok(embedder) => {
                eprintln!(
                    "  Generating embeddings ({} provider)...",
                    embedder.provider_name()
                );
                match embedder
                    .embed_entities(graph, config.embeddings.batch_size)
                    .await
                {
                    Ok(count) => {
                        eprintln!("  Embedded {} entities", count);
                        count
                    }
                    Err(e) => {
                        eprintln!("  Warning: embedding generation failed: {}", e);
                        0
                    }
                }
            }
            Err(_) => 0, // No embedding provider available, silently skip
        };

    // Final checkpoint
    checkpoint_graph(graph, project_root);

    let _ = entities_embedded;

    Ok(LiftResult {
        entities_lifted: total_lifted + total_repaired,
        entities_failed: total_failed,
        entities_repaired: total_repaired,
        hierarchy_updated,
    })
}
