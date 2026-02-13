//! Minimal connecting subgraph extraction (Steiner tree approximation).

use rpg_core::graph::{DependencyEdge, EdgeKind, RPGraph};
use std::collections::{HashMap, HashSet, VecDeque};

/// Path with edges: (nodes, edges_on_path)
type PathWithEdges = (Vec<String>, Vec<(String, String, EdgeKind)>);

/// A minimal subgraph connecting a set of entities.
#[derive(Debug, Clone)]
pub struct SubgraphSlice {
    /// Entity IDs in the subgraph
    pub entities: Vec<String>,
    /// Edges in the subgraph
    pub edges: Vec<DependencyEdge>,
    /// Optional entity metadata (if requested)
    pub metadata: Option<HashMap<String, EntityMetadata>>,
}

/// Lightweight entity metadata for subgraph output.
#[derive(Debug, Clone)]
pub struct EntityMetadata {
    pub name: String,
    pub file: String,
    pub features: Vec<String>,
}

/// Extract the minimal subgraph connecting a set of entities.
///
/// Uses a Steiner tree 2-approximation: compute shortest paths between all pairs,
/// then take the union of nodes and edges on those paths.
///
/// # Arguments
/// * `graph` - The RPG graph
/// * `entity_ids` - IDs of entities to connect (minimum 2)
/// * `max_depth` - Maximum path length when searching for connections
/// * `include_metadata` - Whether to include entity metadata in the output
///
/// # Returns
/// A SubgraphSlice containing only the entities and edges on shortest paths between the given entities.
pub fn slice_between(
    graph: &RPGraph,
    entity_ids: &[String],
    max_depth: usize,
    include_metadata: bool,
) -> Result<SubgraphSlice, String> {
    if entity_ids.len() < 2 {
        return Err("At least 2 entities required for subgraph slicing".to_string());
    }

    // Validate all entities exist
    for id in entity_ids {
        if !graph.entities.contains_key(id) {
            return Err(format!("Entity not found: {}", id));
        }
    }

    // Build edge-aware map (undirected for Steiner tree, tracking edge kinds)
    let mut edge_map: HashMap<(&str, &str), EdgeKind> = HashMap::new();
    // Track original edge directions for output
    let mut original_edges: HashSet<(&str, &str, EdgeKind)> = HashSet::new();
    for edge in &graph.edges {
        edge_map.insert((edge.source.as_str(), edge.target.as_str()), edge.kind);
        edge_map.insert((edge.target.as_str(), edge.source.as_str()), edge.kind);
        original_edges.insert((edge.source.as_str(), edge.target.as_str(), edge.kind));
    }

    // Collect all nodes and edges in connecting paths
    let mut subgraph_nodes: HashSet<String> = HashSet::new();
    let mut subgraph_edges: HashSet<(String, String, EdgeKind)> = HashSet::new();

    // Always include the requested entities
    for id in entity_ids {
        subgraph_nodes.insert(id.clone());
    }

    // For each pair of entities, find shortest path
    for i in 0..entity_ids.len() {
        for j in (i + 1)..entity_ids.len() {
            if let Some((path, edges)) = find_shortest_path_with_edges(
                &edge_map,
                &original_edges,
                &entity_ids[i],
                &entity_ids[j],
                max_depth,
            ) {
                // Add all nodes in the path (including intermediate Steiner nodes)
                for node in &path {
                    subgraph_nodes.insert(node.clone());
                }

                // Add only the edges that were actually traversed
                subgraph_edges.extend(edges);
            }
        }
    }

    // Convert to output format
    let mut entities: Vec<String> = subgraph_nodes.into_iter().collect();
    entities.sort();

    let edges: Vec<DependencyEdge> = subgraph_edges
        .into_iter()
        .map(|(source, target, kind)| DependencyEdge {
            source,
            target,
            kind,
        })
        .collect();

    let metadata = if include_metadata {
        Some(
            entities
                .iter()
                .filter_map(|id| {
                    graph.entities.get(id).map(|e| {
                        (
                            id.clone(),
                            EntityMetadata {
                                name: e.name.clone(),
                                file: e.file.display().to_string(),
                                features: e.semantic_features.clone(),
                            },
                        )
                    })
                })
                .collect(),
        )
    } else {
        None
    };

    Ok(SubgraphSlice {
        entities,
        edges,
        metadata,
    })
}

/// Find shortest path between two nodes using BFS, returning both path and edges.
fn find_shortest_path_with_edges(
    edge_map: &HashMap<(&str, &str), EdgeKind>,
    original_edges: &HashSet<(&str, &str, EdgeKind)>,
    start: &str,
    end: &str,
    max_depth: usize,
) -> Option<PathWithEdges> {
    if start == end {
        return Some((vec![start.to_string()], Vec::new()));
    }

    let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    queue.push_back((start.to_string(), vec![start.to_string()]));
    visited.insert(start.to_string());

    while let Some((current, path)) = queue.pop_front() {
        if path.len() > max_depth {
            continue;
        }

        // Get neighbors from edge_map
        let neighbors: Vec<&str> = edge_map
            .keys()
            .filter(|(from, _)| *from == current.as_str())
            .map(|(_, to)| *to)
            .collect();

        for neighbor in neighbors {
            if visited.contains(neighbor) {
                continue;
            }

            let mut new_path = path.clone();
            new_path.push(neighbor.to_string());

            if neighbor == end {
                // Reconstruct edges from the path, normalizing to original direction
                let mut edges = Vec::new();
                for window in new_path.windows(2) {
                    if let [a, b] = window
                        && let Some(&kind) = edge_map.get(&(a.as_str(), b.as_str()))
                    {
                        // Check if edge exists in original direction (a -> b)
                        if original_edges.contains(&(a.as_str(), b.as_str(), kind)) {
                            edges.push((a.clone(), b.clone(), kind));
                        }
                        // Otherwise, it must exist in reverse (b -> a)
                        else if original_edges.contains(&(b.as_str(), a.as_str(), kind)) {
                            edges.push((b.clone(), a.clone(), kind));
                        }
                    }
                }
                return Some((new_path, edges));
            }

            visited.insert(neighbor.to_string());
            queue.push_back((neighbor.to_string(), new_path));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{Entity, EntityDeps, EntityKind};
    use std::path::PathBuf;

    fn make_test_entity(id: &str, name: &str) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: name.to_string(),
            file: PathBuf::from("test.rs"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: vec!["test feature".to_string()],
            feature_source: None,
            hierarchy_path: "Test".to_string(),
            deps: EntityDeps::default(),
        }
    }

    #[test]
    fn test_two_entities_direct() {
        // A -> B
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "a"));
        graph.insert_entity(make_test_entity("B", "b"));
        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = slice_between(&graph, &["A".to_string(), "B".to_string()], 5, false);
        assert!(result.is_ok());
        let slice = result.unwrap();
        assert_eq!(slice.entities.len(), 2);
        assert!(slice.entities.contains(&"A".to_string()));
        assert!(slice.entities.contains(&"B".to_string()));
        assert_eq!(slice.edges.len(), 1);
    }

    #[test]
    fn test_triangle() {
        // A -> B -> C
        //  \------->/
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "a"));
        graph.insert_entity(make_test_entity("B", "b"));
        graph.insert_entity(make_test_entity("C", "c"));

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
            source: "A".to_string(),
            target: "C".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = slice_between(&graph, &["A".to_string(), "C".to_string()], 5, false);
        assert!(result.is_ok());
        let slice = result.unwrap();

        // Should use shortest path (A -> C directly)
        assert_eq!(slice.entities.len(), 2);
        assert!(slice.entities.contains(&"A".to_string()));
        assert!(slice.entities.contains(&"C".to_string()));
    }

    #[test]
    fn test_disconnected() {
        // A -> B    C -> D (disconnected)
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "a"));
        graph.insert_entity(make_test_entity("B", "b"));
        graph.insert_entity(make_test_entity("C", "c"));
        graph.insert_entity(make_test_entity("D", "d"));

        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "C".to_string(),
            target: "D".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = slice_between(&graph, &["A".to_string(), "C".to_string()], 5, false);
        assert!(result.is_ok());
        let slice = result.unwrap();

        // Only the requested entities (no connecting path)
        assert_eq!(slice.entities.len(), 2);
        assert_eq!(slice.edges.len(), 0);
    }

    #[test]
    fn test_steiner_node() {
        // A -> B -> C
        //       \-> D
        // Request A, C, D - should include B as Steiner node
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "a"));
        graph.insert_entity(make_test_entity("B", "b"));
        graph.insert_entity(make_test_entity("C", "c"));
        graph.insert_entity(make_test_entity("D", "d"));

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

        let result = slice_between(
            &graph,
            &["A".to_string(), "C".to_string(), "D".to_string()],
            5,
            false,
        );
        assert!(result.is_ok());
        let slice = result.unwrap();

        // Should include B as Steiner node
        assert_eq!(slice.entities.len(), 4);
        assert!(slice.entities.contains(&"B".to_string()));
    }

    #[test]
    fn test_metadata_included() {
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "function_a"));
        graph.insert_entity(make_test_entity("B", "function_b"));
        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = slice_between(&graph, &["A".to_string(), "B".to_string()], 5, true);
        assert!(result.is_ok());
        let slice = result.unwrap();

        assert!(slice.metadata.is_some());
        let metadata = slice.metadata.unwrap();
        assert_eq!(metadata.len(), 2);
        assert_eq!(metadata.get("A").unwrap().name, "function_a");
        assert_eq!(metadata.get("B").unwrap().name, "function_b");
    }

    #[test]
    fn test_metadata_excluded() {
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A", "a"));
        graph.insert_entity(make_test_entity("B", "b"));
        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = slice_between(&graph, &["A".to_string(), "B".to_string()], 5, false);
        assert!(result.is_ok());
        let slice = result.unwrap();
        assert!(slice.metadata.is_none());
    }

    #[test]
    fn test_invalid_entity() {
        let graph = RPGraph::new("rust");
        let result = slice_between(&graph, &["A".to_string(), "B".to_string()], 5, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Entity not found"));
    }

    #[test]
    fn test_too_few_entities() {
        let graph = RPGraph::new("rust");
        let result = slice_between(&graph, &["A".to_string()], 5, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("At least 2 entities required"));
    }
}
