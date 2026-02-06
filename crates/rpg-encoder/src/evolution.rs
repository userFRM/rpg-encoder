//! Incremental RPG evolution: update graph from git diffs.
//! Implements Algorithms 2-4 from the paper (structural updates only;
//! semantic re-lifting is done interactively via MCP).

use crate::grounding;
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
    /// Entity IDs that were structurally modified but not re-lifted (features may be stale).
    /// These should be re-lifted interactively via MCP.
    pub modified_entity_ids: Vec<String>,
}

/// Merge semantic features from an old graph into a new graph by matching entity IDs.
/// Used by `build_rpg` to auto-preserve lifted features across rebuilds.
/// Returns the count of entities that had features restored.
pub fn merge_features(new_graph: &mut RPGraph, old_graph: &RPGraph) -> usize {
    let mut restored = 0;
    for (id, new_entity) in &mut new_graph.entities {
        // Only restore if the new entity has no features
        if new_entity.semantic_features.is_empty() {
            if let Some(old_entity) = old_graph.entities.get(id) {
                if !old_entity.semantic_features.is_empty() {
                    new_entity.semantic_features = old_entity.semantic_features.clone();
                    restored += 1;
                }
            }
        }
    }
    restored
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
            .clone()
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

/// Detect ALL changes: committed since base_commit + staged + unstaged (working tree).
/// This is the equivalent of `git diff <base_commit>` against the working directory.
/// Catches everything regardless of whether the user has committed or not.
pub fn detect_workdir_changes(project_root: &Path, graph: &RPGraph) -> Result<Vec<FileChange>> {
    let base_commit_str = graph
        .base_commit
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no base_commit in RPG, cannot compute diff"))?;

    let repo = git2::Repository::open(project_root).context("failed to open git repo")?;
    let base_oid = git2::Oid::from_str(base_commit_str).context("invalid base_commit SHA")?;
    let base_commit_obj = repo
        .find_commit(base_oid)
        .context("base commit not found")?;
    let base_tree = base_commit_obj.tree()?;

    // Diff base tree vs working directory (includes staged + unstaged)
    let diff = repo.diff_tree_to_workdir_with_index(Some(&base_tree), None)?;

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

/// Filter changes to only include source files for the given languages.
pub fn filter_source_changes(changes: Vec<FileChange>, languages: &[Language]) -> Vec<FileChange> {
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
                .is_some_and(|lang| languages.contains(&lang))
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

/// Apply modifications: re-extract entities and update structural metadata.
///
/// Structurally updates line numbers for modified entities. New entities within
/// modified files are inserted without features. Modified entities that previously
/// had features are tracked in `modified_entity_ids` for interactive re-lifting.
pub fn apply_modifications(
    graph: &mut RPGraph,
    modified_files: &[PathBuf],
    project_root: &Path,
) -> Result<(usize, usize, usize, Vec<String>)> {
    let mut modified_count = 0;
    let mut added_count = 0;
    let mut removed_count = 0;
    let mut structurally_modified_ids: Vec<String> = Vec::new();

    for file in modified_files {
        let abs_path = project_root.join(file);
        let file_lang = file
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);
        let Some(language) = file_lang else {
            continue;
        };
        let source = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("failed to read {}", abs_path.display()))?;

        let new_raw: Vec<RawEntity> =
            rpg_parser::entities::extract_entities(file, &source, language);

        let new_ids: std::collections::HashSet<String> = new_raw.iter().map(|e| e.id()).collect();

        let old_ids: Vec<String> = graph.file_index.get(file).cloned().unwrap_or_default();

        let old_ids_set: std::collections::HashSet<String> = old_ids.iter().cloned().collect();

        // Remove entities that no longer exist in the file
        for old_id in &old_ids {
            if !new_ids.contains(old_id) {
                graph.remove_entity(old_id);
                removed_count += 1;
            }
        }

        // Update existing entities (line numbers)
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
                // Track entities with existing features that need re-lifting
                if !entity.semantic_features.is_empty() {
                    structurally_modified_ids.push(id);
                }
            }
        }

        // Insert newly-added entities (no features — they need interactive lifting)
        let added_raws: Vec<&RawEntity> = new_raw
            .iter()
            .filter(|e| !old_ids_set.contains(&e.id()))
            .collect();

        for raw in &added_raws {
            let entity = (*raw).clone().into_entity();
            graph.insert_entity(entity);
            added_count += 1;
        }
    }

    Ok((
        modified_count,
        added_count,
        removed_count,
        structurally_modified_ids,
    ))
}

/// Apply additions: insert new entities structurally (no features).
///
/// Entities are inserted without semantic features. Use the MCP interactive
/// lifting protocol (get_entities_for_lifting → submit_lift_results) to
/// add features after the structural update.
pub fn apply_additions(
    graph: &mut RPGraph,
    added_files: &[PathBuf],
    project_root: &Path,
) -> Result<usize> {
    let mut added_count = 0;

    for file in added_files {
        let abs_path = project_root.join(file);
        let file_lang = file
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);
        let Some(language) = file_lang else {
            continue;
        };
        let source = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("failed to read {}", abs_path.display()))?;

        let raw_entities: Vec<RawEntity> =
            rpg_parser::entities::extract_entities(file, &source, language);

        for raw in raw_entities {
            let entity = raw.into_entity();
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

/// Run the full incremental update pipeline (structural only).
///
/// Semantic re-lifting of modified entities is left to the connected
/// coding agent via the MCP interactive protocol.
pub fn run_update(
    graph: &mut RPGraph,
    project_root: &Path,
    since: Option<&str>,
) -> Result<UpdateSummary> {
    // Resolve all indexed languages (multi-language support)
    let languages: Vec<Language> = if graph.metadata.languages.is_empty() {
        // Backward compat: single-language graph
        Language::from_name(&graph.metadata.language)
            .map(|l| vec![l])
            .unwrap_or_default()
    } else {
        graph
            .metadata
            .languages
            .iter()
            .filter_map(|n| Language::from_name(n))
            .collect()
    };
    if languages.is_empty() {
        return Err(anyhow::anyhow!(
            "no supported languages in RPG metadata: {}",
            graph.metadata.language
        ));
    }

    let changes = detect_changes(project_root, graph, since)?;
    let changes = filter_source_changes(changes, &languages);

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

    // Step 3: Modifications
    let (modified, mod_added, mod_removed, mod_stale_ids) =
        apply_modifications(graph, &modified_files, project_root)?;
    summary.entities_modified = modified;
    summary.modified_entity_ids = mod_stale_ids;
    summary.entities_added += mod_added;
    summary.entities_removed += mod_removed;

    // Step 4: Additions
    let added = apply_additions(graph, &added_files, project_root)?;
    summary.entities_added += added;

    // Step 5: Re-populate and re-resolve dependencies
    grounding::populate_entity_deps(graph, project_root, false);
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
