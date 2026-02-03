//! Incremental RPG evolution: update graph from git diffs.
//! Implements Algorithms 2-4 from the paper.

use crate::embeddings::EmbeddingGenerator;
use crate::grounding;
use crate::hierarchy;
use crate::llm::LlmClient;
use crate::semantic_lifting;
use anyhow::{Context, Result};
use rpg_core::graph::RPGraph;
use rpg_parser::entities::RawEntity;
use rpg_parser::languages::Language;
use std::path::{Path, PathBuf};

/// Classification of a file change.
#[derive(Debug, Clone)]
pub enum FileChange {
    Added(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

/// Summary of an incremental update.
#[derive(Debug, Default)]
pub struct UpdateSummary {
    pub entities_added: usize,
    pub entities_modified: usize,
    pub entities_removed: usize,
    pub hierarchy_nodes_added: usize,
    pub hierarchy_nodes_removed: usize,
    pub edges_added: usize,
    pub edges_removed: usize,
}

/// Detect file changes since the RPG's base_commit (or a given override) using git2.
pub fn detect_changes(
    project_root: &Path,
    graph: &RPGraph,
    since: Option<&str>,
) -> Result<Vec<FileChange>> {
    let base_commit_str = match since {
        Some(s) => s.to_string(),
        None => graph
            .base_commit
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no base_commit in RPG, cannot compute diff"))?,
    };

    let repo = git2::Repository::open(project_root).context("failed to open git repo")?;
    let base_oid = git2::Oid::from_str(&base_commit_str).context("invalid base_commit SHA")?;
    let base_commit_obj = repo
        .find_commit(base_oid)
        .context("base commit not found")?;
    let base_tree = base_commit_obj.tree()?;

    let head = repo.head()?.peel_to_commit()?;
    let head_tree = head.tree()?;

    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)?;

    let mut changes = Vec::new();

    diff.foreach(
        &mut |delta, _| {
            let status = delta.status();
            match status {
                git2::Delta::Added => {
                    if let Some(path) = delta.new_file().path() {
                        changes.push(FileChange::Added(path.to_path_buf()));
                    }
                }
                git2::Delta::Deleted => {
                    if let Some(path) = delta.old_file().path() {
                        changes.push(FileChange::Deleted(path.to_path_buf()));
                    }
                }
                git2::Delta::Modified => {
                    if let Some(path) = delta.new_file().path() {
                        changes.push(FileChange::Modified(path.to_path_buf()));
                    }
                }
                git2::Delta::Renamed => {
                    let from = delta.old_file().path().map(|p| p.to_path_buf());
                    let to = delta.new_file().path().map(|p| p.to_path_buf());
                    if let (Some(from), Some(to)) = (from, to) {
                        changes.push(FileChange::Renamed { from, to });
                    }
                }
                _ => {}
            }
            true
        },
        None,
        None,
        None,
    )?;

    Ok(changes)
}

/// Filter changes to only include source files for the given language.
pub fn filter_source_changes(changes: Vec<FileChange>, language: Language) -> Vec<FileChange> {
    changes
        .into_iter()
        .filter(|change| {
            let path = match change {
                FileChange::Added(p) | FileChange::Modified(p) | FileChange::Deleted(p) => p,
                FileChange::Renamed { to, .. } => to,
            };
            path.extension()
                .and_then(|e| e.to_str())
                .and_then(Language::from_extension)
                == Some(language)
        })
        .collect()
}

/// Apply deletions to the graph (Algorithm 2: recursive pruning).
pub fn apply_deletions(graph: &mut RPGraph, deleted_files: &[PathBuf]) -> usize {
    let mut removed = 0;
    for file in deleted_files {
        if let Some(entity_ids) = graph.file_index.get(file).cloned() {
            for id in entity_ids {
                if graph.remove_entity(&id).is_some() {
                    removed += 1;
                }
            }
        }
    }
    removed
}

/// Apply modifications: re-extract entities, detect semantic drift, re-route if needed.
/// (Algorithm 3 from the paper)
///
/// When `embedder` is provided, computes new embeddings for modified entities to enable
/// cosine-distance-based drift detection instead of the Jaccard fallback.
#[allow(clippy::too_many_arguments)]
pub async fn apply_modifications(
    graph: &mut RPGraph,
    modified_files: &[PathBuf],
    project_root: &Path,
    language: Language,
    client: Option<&LlmClient>,
    drift_threshold: f64,
    hierarchy_chunk_size: usize,
    embedder: Option<&EmbeddingGenerator>,
) -> Result<(usize, usize, usize)> {
    let mut modified_count = 0;
    let mut added_count = 0;
    let mut removed_count = 0;
    let mut drifted_entity_ids: Vec<String> = Vec::new();

    for file in modified_files {
        let abs_path = project_root.join(file);
        let source = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("failed to read {}", abs_path.display()))?;

        let new_raw: Vec<RawEntity> =
            rpg_parser::entities::extract_entities(file, &source, language);

        let new_ids: std::collections::HashSet<String> = new_raw.iter().map(|e| e.id()).collect();

        let old_ids: Vec<String> = graph.file_index.get(file).cloned().unwrap_or_default();

        let old_ids_set: std::collections::HashSet<String> = old_ids.iter().cloned().collect();

        for old_id in &old_ids {
            if !new_ids.contains(old_id) {
                graph.remove_entity(old_id);
                removed_count += 1;
            }
        }

        let added_raws: Vec<&RawEntity> = new_raw
            .iter()
            .filter(|e| !old_ids_set.contains(&e.id()))
            .collect();

        let modified_raws: Vec<&RawEntity> = new_raw
            .iter()
            .filter(|e| old_ids_set.contains(&e.id()))
            .collect();

        for raw in &modified_raws {
            let id = raw.id();
            if let Some(entity) = graph.entities.get_mut(&id) {
                entity.line_start = raw.line_start;
                entity.line_end = raw.line_end;
                modified_count += 1;
            }
        }

        if let Some(client) = client {
            let repo_name = project_root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            if !modified_raws.is_empty() {
                let batch: Vec<RawEntity> = modified_raws.iter().map(|r| (*r).clone()).collect();
                if let Ok(features) =
                    semantic_lifting::lift_batch(client, &batch, repo_name, "", None).await
                {
                    for raw in &modified_raws {
                        let id = raw.id();
                        if let Some(new_feats) = features.get(&raw.name)
                            && let Some(entity) = graph.entities.get_mut(&id)
                        {
                            let old_feats = entity.semantic_features.clone();
                            let old_embedding = entity.embedding.as_deref();
                            // Compute new embedding when embedder is available
                            let new_embedding_vec = if old_embedding.is_some() {
                                if let Some(emb) = embedder {
                                    let text = new_feats.join(", ");
                                    emb.generate_single(&text).await.ok()
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            // Use embedding-based drift when available (Algorithm 3)
                            let drift = compute_drift_with_embeddings(
                                &old_feats,
                                new_feats,
                                old_embedding,
                                new_embedding_vec.as_deref(),
                            );
                            entity.semantic_features = new_feats.clone();
                            if let Some(ref new_emb) = new_embedding_vec {
                                entity.embedding = Some(new_emb.clone());
                            }

                            // Two-part drift assessment: computational + LLM judge for ambiguous cases
                            let final_drifted = llm_drift_judge(
                                client,
                                drift,
                                drift_threshold,
                                &old_feats,
                                new_feats,
                            )
                            .await;

                            if final_drifted {
                                graph.remove_entity_from_hierarchy(&id);
                                drifted_entity_ids.push(id);
                            }
                        }
                    }
                }
            }

            if !added_raws.is_empty() {
                let batch: Vec<RawEntity> = added_raws.iter().map(|r| (*r).clone()).collect();
                if let Ok(features) =
                    semantic_lifting::lift_batch(client, &batch, repo_name, "", None).await
                {
                    for raw in &added_raws {
                        let mut entity = (*raw).clone().into_entity();
                        if let Some(feats) = features.get(&raw.name) {
                            entity.semantic_features = feats.clone();
                        }
                        let entity_id = entity.id.clone();
                        let has_features = !entity.semantic_features.is_empty();
                        graph.insert_entity(entity);
                        added_count += 1;

                        // Route new entities to hierarchy (same as apply_additions)
                        if !graph.hierarchy.is_empty()
                            && has_features
                            && let Some(e) = graph.entities.get(&entity_id).cloned()
                            && let Ok(path) =
                                hierarchy::find_best_parent(client, &e, &graph.hierarchy, repo_name)
                                    .await
                        {
                            if let Some(ent) = graph.entities.get_mut(&entity_id) {
                                ent.hierarchy_path = path.clone();
                            }
                            graph.insert_into_hierarchy(&path, &entity_id);
                        }
                    }
                }
            }

            // Re-route drifted entities using FindBestParent (Algorithm 4)
            if !drifted_entity_ids.is_empty() {
                for id in &drifted_entity_ids {
                    if let Some(entity) = graph.entities.get(id).cloned() {
                        match hierarchy::find_best_parent(
                            client,
                            &entity,
                            &graph.hierarchy,
                            repo_name,
                        )
                        .await
                        {
                            Ok(path) => {
                                if let Some(e) = graph.entities.get_mut(id) {
                                    e.hierarchy_path = path.clone();
                                }
                                graph.insert_into_hierarchy(&path, id);
                            }
                            Err(_) => {
                                // Fallback: batch re-route via build_hierarchy
                                let areas: Vec<String> = graph.hierarchy.keys().cloned().collect();
                                if !areas.is_empty()
                                    && let Ok(assignments) = hierarchy::build_hierarchy(
                                        client,
                                        std::slice::from_ref(&entity),
                                        &areas,
                                        repo_name,
                                        hierarchy_chunk_size,
                                        1, // single entity, no parallelism needed
                                    )
                                    .await
                                {
                                    for (name, path) in &assignments {
                                        if let Some(e) = graph.entities.get_mut(id)
                                            && &e.name == name
                                        {
                                            e.hierarchy_path = path.clone();
                                            graph.insert_into_hierarchy(path, id);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            for raw in &added_raws {
                let entity = (*raw).clone().into_entity();
                graph.insert_entity(entity);
                added_count += 1;
            }
        }
    }

    Ok((modified_count, added_count, removed_count))
}

/// Apply additions: insert new entities with semantic routing (Algorithm 4).
pub async fn apply_additions(
    graph: &mut RPGraph,
    added_files: &[PathBuf],
    project_root: &Path,
    language: Language,
    client: Option<&LlmClient>,
    hierarchy_chunk_size: usize,
) -> Result<usize> {
    let mut added_count = 0;

    for file in added_files {
        let abs_path = project_root.join(file);
        let source = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("failed to read {}", abs_path.display()))?;

        let raw_entities: Vec<RawEntity> =
            rpg_parser::entities::extract_entities(file, &source, language);

        if raw_entities.is_empty() {
            continue;
        }

        // Lift semantic features if LLM available
        let mut entities: Vec<rpg_core::graph::Entity> = raw_entities
            .iter()
            .map(|raw| raw.clone().into_entity())
            .collect();

        if let Some(client) = client {
            let repo_name = project_root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            if let Ok(features) =
                semantic_lifting::lift_batch(client, &raw_entities, repo_name, "", None).await
            {
                semantic_lifting::apply_features(&mut entities, &features);
            }

            // Semantic routing: per-entity FindBestParent (Algorithm 4)
            if !graph.hierarchy.is_empty() {
                for entity in &mut entities {
                    match hierarchy::find_best_parent(client, entity, &graph.hierarchy, repo_name)
                        .await
                    {
                        Ok(path) => {
                            entity.hierarchy_path = path;
                        }
                        Err(_) => {
                            // Fallback: batch hierarchy assignment
                            let areas: Vec<String> = graph.hierarchy.keys().cloned().collect();
                            if !areas.is_empty()
                                && let Ok(assignments) = hierarchy::build_hierarchy(
                                    client,
                                    std::slice::from_ref(entity),
                                    &areas,
                                    repo_name,
                                    hierarchy_chunk_size,
                                    1, // single entity, no parallelism needed
                                )
                                .await
                                && let Some(path) = assignments.get(&entity.name)
                            {
                                entity.hierarchy_path = path.clone();
                            }
                        }
                    }
                }
            }
        }

        for entity in entities {
            let hierarchy_path = entity.hierarchy_path.clone();
            let entity_id = entity.id.clone();
            graph.insert_entity(entity);
            if !hierarchy_path.is_empty() {
                graph.insert_into_hierarchy(&hierarchy_path, &entity_id);
            }
            added_count += 1;
        }
    }

    Ok(added_count)
}

/// Handle renamed files: treat as delete old + add new.
pub fn apply_renames(graph: &mut RPGraph, renames: &[(PathBuf, PathBuf)]) -> (usize, usize) {
    let mut migrated_files = 0;
    let mut renamed = 0;

    for (from, to) in renames {
        if let Some(entity_ids) = graph.file_index.get(from).cloned() {
            for id in &entity_ids {
                if let Some(entity) = graph.entities.get_mut(id) {
                    entity.file = to.clone();
                    renamed += 1;
                }
            }
            // Update file_index
            if let Some(ids) = graph.file_index.remove(from) {
                migrated_files += 1;
                graph.file_index.insert(to.clone(), ids);
            }
        }
    }

    (migrated_files, renamed)
}

/// Compute semantic drift using embeddings when available, falling back to Jaccard distance.
/// (Algorithm 3 from the paper: SemanticShift uses cosine distance on embeddings)
pub fn compute_drift_with_embeddings(
    old_features: &[String],
    new_features: &[String],
    old_embedding: Option<&[f32]>,
    new_embedding: Option<&[f32]>,
) -> f64 {
    // When both embeddings are available, use cosine distance
    if let (Some(old_emb), Some(new_emb)) = (old_embedding, new_embedding) {
        let sim = cosine_similarity(old_emb, new_emb);
        return 1.0 - sim as f64;
    }

    // Fallback: Jaccard distance on feature sets
    compute_drift(old_features, new_features)
}

/// Compute semantic drift between old and new features using Jaccard distance.
/// (0.0 = no drift, 1.0 = complete drift)
pub fn compute_drift(old: &[String], new: &[String]) -> f64 {
    if old.is_empty() && new.is_empty() {
        return 0.0;
    }
    if old.is_empty() || new.is_empty() {
        return 1.0;
    }

    let old_set: std::collections::HashSet<&str> = old.iter().map(|s| s.as_str()).collect();
    let new_set: std::collections::HashSet<&str> = new.iter().map(|s| s.as_str()).collect();

    let intersection = old_set.intersection(&new_set).count();
    let union = old_set.union(&new_set).count();

    if union == 0 {
        0.0
    } else {
        1.0 - (intersection as f64 / union as f64) // Jaccard distance
    }
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

/// Two-part drift assessment: skip LLM for clear-cut cases, consult LLM for ambiguous ones.
///
/// Clear drifted (>120% of threshold) or clear stable (<80% of threshold) are decided
/// computationally. Ambiguous cases (within ±20% of threshold) use an LLM judge with
/// a tight prompt to minimize cost.
pub async fn llm_drift_judge(
    client: &LlmClient,
    drift: f64,
    threshold: f64,
    old_features: &[String],
    new_features: &[String],
) -> bool {
    // Clear-cut cases: skip LLM
    if drift > threshold * 1.2 {
        return true; // Clearly drifted
    }
    if drift < threshold * 0.8 {
        return false; // Clearly stable
    }

    // Ambiguous range: ask LLM judge
    let prompt = format!(
        "Old features: [{}]\nNew features: [{}]\n\nDrift score: {:.3} (threshold: {:.3})\n\nHas this entity's semantic role changed significantly? Answer exactly: drifted or stable",
        old_features.join(", "),
        new_features.join(", "),
        drift,
        threshold
    );

    match client
        .complete_with_max_tokens(
            "You are a semantic drift detector. Answer exactly 'drifted' or 'stable'.",
            &prompt,
            16,
        )
        .await
    {
        Ok(response) => {
            let answer = response.trim().to_lowercase();
            answer.contains("drifted")
        }
        Err(_) => {
            // Fallback to computational decision
            drift > threshold
        }
    }
}

/// Run the full incremental update pipeline.
///
/// When `embedder` is provided, computes new embeddings for modified entities
/// to enable cosine-distance-based drift detection.
pub async fn run_update(
    graph: &mut RPGraph,
    project_root: &Path,
    client: Option<&LlmClient>,
    since: Option<&str>,
    drift_threshold: f64,
    hierarchy_chunk_size: usize,
    embedder: Option<&EmbeddingGenerator>,
) -> Result<UpdateSummary> {
    let language = Language::from_name(&graph.metadata.language).ok_or_else(|| {
        anyhow::anyhow!("unsupported language in RPG: {}", graph.metadata.language)
    })?;

    let changes = detect_changes(project_root, graph, since)?;
    let changes = filter_source_changes(changes, language);

    if changes.is_empty() {
        return Ok(UpdateSummary::default());
    }

    let mut summary = UpdateSummary::default();
    let old_edge_count = graph.edges.len();
    let old_hierarchy_count: usize = graph.hierarchy.values().map(count_hierarchy_nodes).sum();

    // Classify changes
    let mut deleted_files = Vec::new();
    let mut modified_files = Vec::new();
    let mut added_files = Vec::new();
    let mut renames = Vec::new();

    for change in changes {
        match change {
            FileChange::Deleted(p) => deleted_files.push(p),
            FileChange::Modified(p) => modified_files.push(p),
            FileChange::Added(p) => added_files.push(p),
            FileChange::Renamed { from, to } => renames.push((from, to)),
        }
    }

    // Step 1: Deletions (Algorithm 2)
    summary.entities_removed = apply_deletions(graph, &deleted_files);

    // Step 2: Renames
    let (_, _renamed) = apply_renames(graph, &renames);

    // Step 3: Modifications (Algorithm 3)
    let (modified, mod_added, mod_removed) = apply_modifications(
        graph,
        &modified_files,
        project_root,
        language,
        client,
        drift_threshold,
        hierarchy_chunk_size,
        embedder,
    )
    .await?;
    summary.entities_modified = modified;
    summary.entities_added += mod_added;
    summary.entities_removed += mod_removed;

    // Step 4: Additions (Algorithm 4)
    let added = apply_additions(
        graph,
        &added_files,
        project_root,
        language,
        client,
        hierarchy_chunk_size,
    )
    .await?;
    summary.entities_added += added;

    // Step 5: Re-populate and re-resolve dependencies
    grounding::populate_entity_deps(graph, project_root, language);
    grounding::resolve_dependencies(graph);

    // Step 6: Re-ground hierarchy
    grounding::ground_hierarchy(graph);

    // Step 7: Hierarchy enrichment (V_H unification)
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();

    // Step 8: Update metadata
    if let Ok(sha) = get_head_sha(project_root) {
        graph.base_commit = Some(sha);
    }
    graph.refresh_metadata();

    // Compute deltas
    let new_edge_count = graph.edges.len();
    let new_hierarchy_count: usize = graph.hierarchy.values().map(count_hierarchy_nodes).sum();

    summary.edges_added = new_edge_count.saturating_sub(old_edge_count);
    summary.edges_removed = old_edge_count.saturating_sub(new_edge_count);
    summary.hierarchy_nodes_added = new_hierarchy_count.saturating_sub(old_hierarchy_count);
    summary.hierarchy_nodes_removed = old_hierarchy_count.saturating_sub(new_hierarchy_count);

    Ok(summary)
}

fn count_hierarchy_nodes(node: &rpg_core::graph::HierarchyNode) -> usize {
    1 + node
        .children
        .values()
        .map(count_hierarchy_nodes)
        .sum::<usize>()
}

/// Get the current HEAD commit SHA.
pub fn get_head_sha(project_root: &Path) -> Result<String> {
    let repo = git2::Repository::open(project_root).context("failed to open git repo")?;
    let head = repo.head()?.peel_to_commit()?;
    Ok(head.id().to_string())
}
