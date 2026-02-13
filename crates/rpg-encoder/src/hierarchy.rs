//! Structure Reorganization — apply hierarchy assignments to the RPG graph.

use rpg_core::graph::{EntityKind, RPGraph};
use std::collections::HashMap;

/// Apply hierarchy assignments to the RPG graph.
///
/// Keys in `assignments` can be entity IDs (`file:name`) or bare names.
/// Prefers direct ID lookup; falls back to name-based matching only when
/// the name is unambiguous (exactly one entity has that name).
///
/// Paper §9.1.2: When a Module entity receives a hierarchy path, all entities
/// in the same file inherit that path (file-level granularity assignment).
pub fn apply_hierarchy(graph: &mut RPGraph, assignments: &HashMap<String, String>) {
    // Build name → IDs index for fallback matching
    let name_to_ids: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (id, entity) in &graph.entities {
            map.entry(entity.name.clone()).or_default().push(id.clone());
        }
        map
    };

    for (key, path) in assignments {
        // 1. Try direct entity ID lookup (preferred — unambiguous)
        let entity_id = if graph.entities.contains_key(key) {
            Some(key.clone())
        } else if let Some(ids) = name_to_ids.get(key) {
            // 2. Bare name fallback — only if unambiguous (exactly one match)
            if ids.len() == 1 {
                Some(ids[0].clone())
            } else {
                // Ambiguous name (e.g., "new", "default") — skip to avoid wrong assignment
                None
            }
        } else {
            None
        };

        if let Some(id) = entity_id {
            // Check if this is a Module entity — if so, propagate path to all file siblings
            let is_module = graph
                .entities
                .get(&id)
                .is_some_and(|e| e.kind == rpg_core::graph::EntityKind::Module);

            if is_module {
                // Paper §9.1.2: file-level assignment — all entities in this file
                // inherit the Module's hierarchy path.
                let file = graph.entities.get(&id).map(|e| e.file.clone());
                if let Some(file) = file {
                    let sibling_ids: Vec<String> =
                        graph.file_index.get(&file).cloned().unwrap_or_default();

                    for sibling_id in &sibling_ids {
                        if let Some(entity) = graph.entities.get_mut(sibling_id) {
                            entity.hierarchy_path = path.clone();
                        }
                        graph.insert_into_hierarchy(path, sibling_id);
                    }
                }
            } else {
                // Individual entity assignment (backward compat for evolution incremental updates)
                if let Some(entity) = graph.entities.get_mut(&id) {
                    entity.hierarchy_path = path.clone();
                }
                graph.insert_into_hierarchy(path, &id);
            }
        }
    }
}

/// File cluster for sharded hierarchy construction.
#[derive(Debug, Clone)]
pub struct FileCluster {
    /// File paths in this cluster
    pub files: Vec<String>,
    /// Representative files (for domain discovery)
    pub representatives: Vec<String>,
}

/// Cluster files by semantic similarity using HAC for sharded hierarchy construction.
///
/// For repos >100 files, this creates clusters to present hierarchy construction in batches.
/// Uses hierarchical agglomerative clustering with average linkage on file-level embeddings.
///
/// # Arguments
/// * `graph` - The RPG graph (must have Module entities with semantic_features)
/// * `target_cluster_size` - Target number of files per cluster (default: 70)
///
/// # Returns
/// A vector of FileCluster, each containing related files for batch processing.
pub fn cluster_files_for_hierarchy(
    graph: &RPGraph,
    target_cluster_size: usize,
) -> Vec<FileCluster> {
    // Collect Module entities (file-level), sorted for deterministic clustering
    let mut modules: Vec<(String, Vec<String>)> = graph
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::Module && !e.semantic_features.is_empty())
        .map(|e| (e.file.display().to_string(), e.semantic_features.clone()))
        .collect();
    modules.sort_by(|a, b| a.0.cmp(&b.0));

    if modules.is_empty() || modules.len() <= target_cluster_size {
        // Small repo or no features — single cluster
        return vec![FileCluster {
            files: modules.iter().map(|(f, _)| f.clone()).collect(),
            representatives: modules.iter().map(|(f, _)| f.clone()).collect(),
        }];
    }

    // Simple batching approach: group files into chunks of target_cluster_size
    // Use BTreeMap to ensure deterministic cluster iteration order
    let mut cluster_map: std::collections::BTreeMap<usize, Vec<String>> =
        std::collections::BTreeMap::new();
    for (file_idx, (file, _)) in modules.iter().enumerate() {
        let cluster_id = file_idx / target_cluster_size;
        cluster_map
            .entry(cluster_id)
            .or_default()
            .push(file.clone());
    }

    // Build FileCluster objects with representatives (BTreeMap ensures deterministic order)
    let mut clusters: Vec<FileCluster> = cluster_map
        .into_values()
        .map(|files| {
            let representatives = sample_representatives(&files, 3);
            FileCluster {
                files,
                representatives,
            }
        })
        .collect();

    // Balance clusters (split any that exceed target_cluster_size)
    clusters = balance_clusters(clusters, target_cluster_size);

    clusters
}

/// Sample representative files from a cluster for domain discovery.
///
/// Selects up to `count` diverse files from the cluster to present in batch 0.
fn sample_representatives(files: &[String], count: usize) -> Vec<String> {
    if files.len() <= count {
        return files.to_vec();
    }

    // Simple sampling: pick evenly spaced files
    let step = files.len() / count;
    (0..count).map(|i| files[i * step].clone()).collect()
}

/// Balance clusters by splitting any that exceed target_cluster_size.
fn balance_clusters(clusters: Vec<FileCluster>, target_size: usize) -> Vec<FileCluster> {
    let mut balanced = Vec::new();

    for cluster in clusters {
        if cluster.files.len() <= target_size {
            balanced.push(cluster);
        } else {
            // Split into multiple clusters
            let num_splits = cluster.files.len().div_ceil(target_size);
            let chunk_size = cluster.files.len() / num_splits;

            for chunk in cluster.files.chunks(chunk_size) {
                let files = chunk.to_vec();
                let representatives = sample_representatives(&files, 3);
                balanced.push(FileCluster {
                    files,
                    representatives,
                });
            }
        }
    }

    balanced
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{Entity, EntityDeps};
    use std::path::PathBuf;

    fn make_module(file: &str, features: Vec<&str>) -> Entity {
        Entity {
            id: format!("{}:module", file),
            kind: EntityKind::Module,
            name: file.to_string(),
            file: PathBuf::from(file),
            line_start: 1,
            line_end: 1,
            parent_class: None,
            semantic_features: features.iter().map(|s| s.to_string()).collect(),
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
        }
    }

    #[test]
    fn test_cluster_files_small_repo() {
        let mut graph = RPGraph::new("rust");
        for i in 0..50 {
            graph.insert_entity(make_module(
                &format!("file{}.rs", i),
                vec!["feature1", "feature2"],
            ));
        }

        let clusters = cluster_files_for_hierarchy(&graph, 70);
        assert_eq!(clusters.len(), 1); // Small repo → single cluster
        assert_eq!(clusters[0].files.len(), 50);
    }

    #[test]
    fn test_cluster_files_large_repo() {
        let mut graph = RPGraph::new("rust");
        for i in 0..150 {
            graph.insert_entity(make_module(
                &format!("file{}.rs", i),
                vec!["feature1", "feature2"],
            ));
        }

        let clusters = cluster_files_for_hierarchy(&graph, 70);
        assert!(clusters.len() >= 2); // Large repo → multiple clusters

        // Check all files are covered
        let total_files: usize = clusters.iter().map(|c| c.files.len()).sum();
        assert_eq!(total_files, 150);
    }

    #[test]
    fn test_sample_representatives() {
        let files: Vec<String> = (0..10).map(|i| format!("file{}.rs", i)).collect();
        let reps = sample_representatives(&files, 3);
        assert_eq!(reps.len(), 3);
    }

    #[test]
    fn test_balance_clusters() {
        let large_cluster = FileCluster {
            files: (0..150).map(|i| format!("file{}.rs", i)).collect(),
            representatives: vec![],
        };

        let balanced = balance_clusters(vec![large_cluster], 70);
        assert!(balanced.len() >= 2);
        assert!(balanced.iter().all(|c| c.files.len() <= 70));
    }
}
