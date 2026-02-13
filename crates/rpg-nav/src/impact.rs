//! Impact radius: BFS reachability analysis that records edge paths.

use crate::explore::{Direction, get_neighbors};
use rpg_core::graph::{EdgeKind, RPGraph};
use std::collections::{HashSet, VecDeque};

/// Edge kinds that represent dependency relationships (not structural containment).
const DEPENDENCY_EDGE_KINDS: &[EdgeKind] = &[
    EdgeKind::Imports,
    EdgeKind::Invokes,
    EdgeKind::Inherits,
    EdgeKind::Composes,
    EdgeKind::Renders,
    EdgeKind::ReadsState,
    EdgeKind::WritesState,
    EdgeKind::Dispatches,
];

/// A single entity in the impact set with its path from the origin.
#[derive(Debug, Clone)]
pub struct ImpactEntry {
    pub entity_id: String,
    pub name: String,
    pub file: String,
    pub depth: usize,
    pub edge_path: Vec<(String, EdgeKind)>,
    pub features: Vec<String>,
}

/// The result of an impact radius computation.
#[derive(Debug, Clone)]
pub struct ImpactResult {
    pub origin: String,
    pub direction: String,
    pub reachable: Vec<ImpactEntry>,
    pub total: usize,
    pub max_depth_reached: usize,
}

/// Compute the impact radius from a starting entity via BFS.
/// Returns all reachable entities with their edge paths, ordered by depth.
/// `max_results` caps the number of reachable entities returned.
pub fn compute_impact_radius(
    graph: &RPGraph,
    entity_id: &str,
    direction: Direction,
    max_depth: usize,
    edge_filter: Option<EdgeKind>,
    max_results: Option<usize>,
) -> Option<ImpactResult> {
    // Validate start entity exists
    if graph.get_entity(entity_id).is_none() && graph.get_node_display_info(entity_id).is_none() {
        return None;
    }

    let dir_label = match direction {
        Direction::Downstream => "downstream",
        Direction::Upstream => "upstream",
        Direction::Both => "both",
    };

    let mut visited = HashSet::new();
    visited.insert(entity_id.to_string());

    // BFS queue: (entity_id, depth, edge_path)
    #[allow(clippy::type_complexity)]
    let mut queue: VecDeque<(String, usize, Vec<(String, EdgeKind)>)> = VecDeque::new();
    queue.push_back((entity_id.to_string(), 0, Vec::new()));

    let mut reachable = Vec::new();
    let mut max_depth_reached = 0;

    let result_cap = max_results.unwrap_or(usize::MAX);

    while let Some((current_id, depth, path)) = queue.pop_front() {
        if depth >= max_depth || reachable.len() >= result_cap {
            continue;
        }

        let neighbors = get_neighbors(graph, &current_id, direction, edge_filter);

        for (neighbor_id, edge_kind, _dir_str) in neighbors {
            if reachable.len() >= result_cap {
                break;
            }
            // When no explicit edge_filter is set, exclude containment edges
            // to keep results focused on dependency relationships
            if edge_filter.is_none() && !DEPENDENCY_EDGE_KINDS.contains(&edge_kind) {
                continue;
            }
            if visited.contains(&neighbor_id) {
                continue;
            }
            visited.insert(neighbor_id.clone());

            let mut new_path = path.clone();
            new_path.push((current_id.clone(), edge_kind));

            let new_depth = depth + 1;
            if new_depth > max_depth_reached {
                max_depth_reached = new_depth;
            }

            let (name, file, features) = if let Some(e) = graph.get_entity(&neighbor_id) {
                (
                    e.name.clone(),
                    e.file.display().to_string(),
                    e.semantic_features.clone(),
                )
            } else if let Some((name, desc)) = graph.get_node_display_info(&neighbor_id) {
                (name, desc, Vec::new())
            } else {
                continue;
            };

            reachable.push(ImpactEntry {
                entity_id: neighbor_id.clone(),
                name,
                file,
                depth: new_depth,
                edge_path: new_path.clone(),
                features,
            });

            queue.push_back((neighbor_id, new_depth, new_path));
        }
    }

    let total = reachable.len();

    Some(ImpactResult {
        origin: entity_id.to_string(),
        direction: dir_label.to_string(),
        reachable,
        total,
        max_depth_reached,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, Entity, EntityDeps, EntityKind};
    use std::path::PathBuf;

    fn make_entity(id: &str, name: &str) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: name.to_string(),
            file: PathBuf::from("src/lib.rs"),
            line_start: 1,
            line_end: 5,
            parent_class: None,
            semantic_features: vec!["test feature".to_string()],
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
        }
    }

    fn make_test_graph() -> RPGraph {
        // A -> B -> C (linear chain via Invokes)
        let mut graph = RPGraph::new("rust");
        graph
            .entities
            .insert("a".to_string(), make_entity("a", "fn_a"));
        graph
            .entities
            .insert("b".to_string(), make_entity("b", "fn_b"));
        graph
            .entities
            .insert("c".to_string(), make_entity("c", "fn_c"));
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "c".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();
        graph
    }

    #[test]
    fn test_impact_radius_downstream_chain() {
        let graph = make_test_graph();
        let result =
            compute_impact_radius(&graph, "a", Direction::Downstream, 5, None, None).unwrap();
        assert_eq!(result.total, 2, "should find B and C");
        assert_eq!(result.reachable[0].entity_id, "b");
        assert_eq!(result.reachable[0].depth, 1);
        assert_eq!(result.reachable[1].entity_id, "c");
        assert_eq!(result.reachable[1].depth, 2);
    }

    #[test]
    fn test_impact_radius_upstream() {
        let graph = make_test_graph();
        let result =
            compute_impact_radius(&graph, "c", Direction::Upstream, 5, None, None).unwrap();
        assert_eq!(result.total, 2, "should find B and A upstream");
        assert_eq!(result.reachable[0].entity_id, "b");
        assert_eq!(result.reachable[1].entity_id, "a");
    }

    #[test]
    fn test_impact_radius_depth_limit() {
        let graph = make_test_graph();
        let result =
            compute_impact_radius(&graph, "a", Direction::Downstream, 1, None, None).unwrap();
        assert_eq!(result.total, 1, "depth=1 should only find B");
        assert_eq!(result.reachable[0].entity_id, "b");
    }

    #[test]
    fn test_impact_radius_max_results_cap() {
        let graph = make_test_graph();
        let result =
            compute_impact_radius(&graph, "a", Direction::Downstream, 5, None, Some(1)).unwrap();
        assert_eq!(result.total, 1, "max_results=1 should cap at 1");
    }

    #[test]
    fn test_impact_radius_edge_path_recorded() {
        let graph = make_test_graph();
        let result =
            compute_impact_radius(&graph, "a", Direction::Downstream, 5, None, None).unwrap();
        // C is at depth 2: path should be a:Invokes -> b:Invokes
        let c_entry = &result.reachable[1];
        assert_eq!(c_entry.edge_path.len(), 2);
        assert_eq!(c_entry.edge_path[0].0, "a");
        assert_eq!(c_entry.edge_path[0].1, EdgeKind::Invokes);
        assert_eq!(c_entry.edge_path[1].0, "b");
    }

    #[test]
    fn test_impact_radius_nonexistent_entity() {
        let graph = make_test_graph();
        let result =
            compute_impact_radius(&graph, "nonexistent", Direction::Downstream, 5, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_impact_radius_cycle_handling() {
        // A -> B -> A (cycle)
        let mut graph = RPGraph::new("rust");
        graph
            .entities
            .insert("a".to_string(), make_entity("a", "fn_a"));
        graph
            .entities
            .insert("b".to_string(), make_entity("b", "fn_b"));
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "a".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();

        let result =
            compute_impact_radius(&graph, "a", Direction::Downstream, 10, None, None).unwrap();
        assert_eq!(
            result.total, 1,
            "cycle should not cause infinite loop — B found once"
        );
    }

    #[test]
    fn test_impact_radius_excludes_containment_by_default() {
        let mut graph = RPGraph::new("rust");
        graph
            .entities
            .insert("mod".to_string(), make_entity("mod", "module"));
        graph
            .entities
            .insert("fn1".to_string(), make_entity("fn1", "function1"));
        graph.edges = vec![DependencyEdge {
            source: "mod".to_string(),
            target: "fn1".to_string(),
            kind: EdgeKind::Contains,
        }];
        graph.refresh_metadata();

        let result =
            compute_impact_radius(&graph, "mod", Direction::Downstream, 5, None, None).unwrap();
        assert_eq!(
            result.total, 0,
            "containment edges should be excluded by default"
        );
    }

    #[test]
    fn test_impact_radius_includes_containment_when_filtered() {
        let mut graph = RPGraph::new("rust");
        graph
            .entities
            .insert("mod".to_string(), make_entity("mod", "module"));
        graph
            .entities
            .insert("fn1".to_string(), make_entity("fn1", "function1"));
        graph.edges = vec![DependencyEdge {
            source: "mod".to_string(),
            target: "fn1".to_string(),
            kind: EdgeKind::Contains,
        }];
        graph.refresh_metadata();

        let result = compute_impact_radius(
            &graph,
            "mod",
            Direction::Downstream,
            5,
            Some(EdgeKind::Contains),
            None,
        )
        .unwrap();
        assert_eq!(
            result.total, 1,
            "explicit Contains filter should include containment edges"
        );
    }
}
