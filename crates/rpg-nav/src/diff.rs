//! Diff-aware search support for PR review workflows.

use rpg_core::graph::RPGraph;
use std::collections::{HashMap, HashSet};

/// Context for diff-aware search ranking.
#[derive(Debug, Clone)]
pub struct DiffContext {
    /// Entity IDs in changed files (Tier 0)
    pub changed_entities: HashSet<String>,
    /// Entity IDs 1-hop from changed entities (Tier 1)
    pub one_hop_neighbors: HashSet<String>,
    /// Entity IDs 2-hop from changed entities (Tier 2)
    pub two_hop_neighbors: HashSet<String>,
}

/// Compute proximity tiers for entities relative to changed entities.
///
/// Uses BFS to assign each entity to a proximity tier:
/// - Tier 0: Entities in changed files
/// - Tier 1: 1-hop dependencies (directly connected to changed entities)
/// - Tier 2: 2-hop dependencies
///
/// # Arguments
/// * `graph` - The RPG graph
/// * `changed_entities` - Set of entity IDs in changed files
///
/// # Returns
/// A DiffContext with entities organized by proximity tier.
pub fn compute_change_proximity(graph: &RPGraph, changed_entities: HashSet<String>) -> DiffContext {
    if changed_entities.is_empty() {
        return DiffContext {
            changed_entities: HashSet::new(),
            one_hop_neighbors: HashSet::new(),
            two_hop_neighbors: HashSet::new(),
        };
    }

    // Build adjacency list (undirected)
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        adj.entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        adj.entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }

    // BFS to compute proximity
    let mut one_hop = HashSet::new();
    let mut two_hop = HashSet::new();
    let mut visited = changed_entities.clone();

    // 1-hop neighbors
    for entity_id in &changed_entities {
        if let Some(neighbors) = adj.get(entity_id.as_str()) {
            for &neighbor in neighbors {
                if !visited.contains(neighbor) {
                    one_hop.insert(neighbor.to_string());
                    visited.insert(neighbor.to_string());
                }
            }
        }
    }

    // 2-hop neighbors
    for entity_id in &one_hop {
        if let Some(neighbors) = adj.get(entity_id.as_str()) {
            for &neighbor in neighbors {
                if !visited.contains(neighbor) {
                    two_hop.insert(neighbor.to_string());
                    visited.insert(neighbor.to_string());
                }
            }
        }
    }

    DiffContext {
        changed_entities,
        one_hop_neighbors: one_hop,
        two_hop_neighbors: two_hop,
    }
}

/// Apply proximity boost to search scores.
///
/// Boosts scores based on proximity tier:
/// - Tier 0 (changed entities): 3x boost
/// - Tier 1 (1-hop neighbors): 2x boost
/// - Tier 2 (2-hop neighbors): 1.5x boost
/// - Tier 3+ (all others): no boost
///
/// # Arguments
/// * `scores` - Map of entity_id -> score
/// * `diff_context` - Proximity tiers from compute_change_proximity
///
/// # Returns
/// A new map with boosted scores.
pub fn apply_proximity_boost(
    scores: &HashMap<String, f64>,
    diff_context: &DiffContext,
) -> HashMap<String, f64> {
    scores
        .iter()
        .map(|(entity_id, &score)| {
            let boosted_score = if diff_context.changed_entities.contains(entity_id) {
                score * 3.0
            } else if diff_context.one_hop_neighbors.contains(entity_id) {
                score * 2.0
            } else if diff_context.two_hop_neighbors.contains(entity_id) {
                score * 1.5
            } else {
                score
            };
            (entity_id.clone(), boosted_score)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, EdgeKind, Entity, EntityDeps, EntityKind};
    use std::path::PathBuf;

    fn make_test_entity(id: &str, file: &str) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: id.to_string(),
            file: PathBuf::from(file),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: vec!["test".to_string()],
            feature_source: None,
            hierarchy_path: "Test".to_string(),
            deps: EntityDeps::default(),
            signature: None,
        }
    }

    #[test]
    fn test_compute_proximity_empty() {
        let graph = RPGraph::new("rust");
        let changed = HashSet::new();
        let context = compute_change_proximity(&graph, changed);
        assert!(context.changed_entities.is_empty());
        assert!(context.one_hop_neighbors.is_empty());
        assert!(context.two_hop_neighbors.is_empty());
    }

    #[test]
    fn test_compute_proximity_single() {
        // A -> B -> C -> D
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "a.rs"));
        graph.insert_entity(make_test_entity("B", "b.rs"));
        graph.insert_entity(make_test_entity("C", "c.rs"));
        graph.insert_entity(make_test_entity("D", "d.rs"));

        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "B".to_string(),
            target: "C".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "C".to_string(),
            target: "D".to_string(),
            kind: EdgeKind::Invokes,
        });

        let mut changed = HashSet::new();
        changed.insert("B".to_string());

        let context = compute_change_proximity(&graph, changed);

        assert_eq!(context.changed_entities.len(), 1);
        assert!(context.changed_entities.contains("B"));

        assert_eq!(context.one_hop_neighbors.len(), 2);
        assert!(context.one_hop_neighbors.contains("A"));
        assert!(context.one_hop_neighbors.contains("C"));

        assert_eq!(context.two_hop_neighbors.len(), 1);
        assert!(context.two_hop_neighbors.contains("D"));
    }

    #[test]
    fn test_compute_proximity_multiple_changed() {
        // A -> B -> C
        //      |
        //      v
        //      D
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "a.rs"));
        graph.insert_entity(make_test_entity("B", "b.rs"));
        graph.insert_entity(make_test_entity("C", "c.rs"));
        graph.insert_entity(make_test_entity("D", "d.rs"));

        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "B".to_string(),
            target: "C".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "B".to_string(),
            target: "D".to_string(),
            kind: EdgeKind::Invokes,
        });

        let mut changed = HashSet::new();
        changed.insert("A".to_string());
        changed.insert("D".to_string());

        let context = compute_change_proximity(&graph, changed);

        assert_eq!(context.changed_entities.len(), 2);
        assert!(context.changed_entities.contains("A"));
        assert!(context.changed_entities.contains("D"));

        // B is 1-hop from both A and D
        assert_eq!(context.one_hop_neighbors.len(), 1);
        assert!(context.one_hop_neighbors.contains("B"));

        // C is 2-hop from A (via B)
        assert_eq!(context.two_hop_neighbors.len(), 1);
        assert!(context.two_hop_neighbors.contains("C"));
    }

    #[test]
    fn test_apply_boost_tiers() {
        let mut scores = HashMap::new();
        scores.insert("changed".to_string(), 10.0);
        scores.insert("one_hop".to_string(), 10.0);
        scores.insert("two_hop".to_string(), 10.0);
        scores.insert("other".to_string(), 10.0);

        let mut changed = HashSet::new();
        changed.insert("changed".to_string());

        let mut one_hop = HashSet::new();
        one_hop.insert("one_hop".to_string());

        let mut two_hop = HashSet::new();
        two_hop.insert("two_hop".to_string());

        let context = DiffContext {
            changed_entities: changed,
            one_hop_neighbors: one_hop,
            two_hop_neighbors: two_hop,
        };

        let boosted = apply_proximity_boost(&scores, &context);

        assert_eq!(boosted.get("changed"), Some(&30.0)); // 3x boost
        assert_eq!(boosted.get("one_hop"), Some(&20.0)); // 2x boost
        assert_eq!(boosted.get("two_hop"), Some(&15.0)); // 1.5x boost
        assert_eq!(boosted.get("other"), Some(&10.0)); // no boost
    }

    #[test]
    fn test_apply_boost_preserves_ordering() {
        let mut scores = HashMap::new();
        scores.insert("A".to_string(), 100.0);
        scores.insert("B".to_string(), 50.0);
        scores.insert("C".to_string(), 25.0);

        let mut changed = HashSet::new();
        changed.insert("C".to_string());

        let context = DiffContext {
            changed_entities: changed,
            one_hop_neighbors: HashSet::new(),
            two_hop_neighbors: HashSet::new(),
        };

        let boosted = apply_proximity_boost(&scores, &context);

        // C should now be highest (25 * 3 = 75)
        assert_eq!(boosted.get("A"), Some(&100.0));
        assert_eq!(boosted.get("B"), Some(&50.0));
        assert_eq!(boosted.get("C"), Some(&75.0));
    }
}
