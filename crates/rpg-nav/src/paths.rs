//! K-shortest path finding for dependency analysis.

use rpg_core::graph::{EdgeKind, RPGraph};
use std::collections::{HashMap, HashSet, VecDeque};

/// A path through the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    /// Entity IDs in the path (source to target)
    pub nodes: Vec<String>,
    /// Edge kinds connecting the nodes
    pub edges: Vec<EdgeKind>,
}

/// Path with score for priority queue ordering in Yen's algorithm
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScoredPath {
    score: usize,
    path: Path,
}

impl PartialOrd for ScoredPath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredPath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score.cmp(&other.score)
    }
}

impl Path {
    /// Returns the length of the path (number of edges).
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Returns true if the path is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Find k-shortest paths between two entities using Yen's algorithm.
///
/// Returns up to `max_paths` paths ordered by increasing length. Paths may have
/// different lengths (unlike BFS which returns only shortest-depth paths).
///
/// # Arguments
/// * `graph` - The RPG graph to search
/// * `source` - Source entity ID
/// * `target` - Target entity ID
/// * `max_hops` - Maximum path length (None = unlimited)
/// * `max_paths` - Maximum number of paths to return
/// * `edge_filter` - Optional edge kind filter (None = all edges)
///
/// # Returns
/// A vector of paths ordered by length (shortest first), up to `max_paths` results.
pub fn find_paths(
    graph: &RPGraph,
    source: &str,
    target: &str,
    max_hops: Option<usize>,
    max_paths: usize,
    edge_filter: Option<EdgeKind>,
) -> Vec<Path> {
    // Early validation
    if max_paths == 0 {
        return Vec::new();
    }

    if !graph.entities.contains_key(source) || !graph.entities.contains_key(target) {
        return Vec::new();
    }

    if source == target {
        return vec![Path {
            nodes: vec![source.to_string()],
            edges: Vec::new(),
        }];
    }

    // Build adjacency list (filtered by edge kind if specified)
    let mut adj: HashMap<&str, Vec<(&str, EdgeKind)>> = HashMap::new();
    for edge in &graph.edges {
        if let Some(filter) = edge_filter
            && edge.kind != filter
        {
            continue;
        }
        adj.entry(edge.source.as_str())
            .or_default()
            .push((edge.target.as_str(), edge.kind));
    }

    // Yen's k-shortest paths algorithm
    let mut result_paths: Vec<Path> = Vec::new();
    let mut candidate_paths: std::collections::BinaryHeap<std::cmp::Reverse<ScoredPath>> =
        std::collections::BinaryHeap::new();

    // Find the shortest path (k=1)
    if let Some(shortest) = find_shortest_path(&adj, source, target, max_hops) {
        result_paths.push(shortest.clone());
    } else {
        return Vec::new(); // No path exists
    }

    // Find k=2 to k=max_paths
    for k in 1..max_paths {
        let prev_path = &result_paths[k - 1];

        // For each node in the previous path (except target)
        for i in 0..(prev_path.nodes.len() - 1) {
            let spur_node = &prev_path.nodes[i];
            let root_path = &prev_path.nodes[0..=i];
            let root_edges = if i > 0 { &prev_path.edges[0..i] } else { &[] };

            // Build temporary edge exclusion set
            let mut removed_edges: HashSet<(&str, &str)> = HashSet::new();

            // Remove edges that share the same root path
            for existing_path in &result_paths {
                if existing_path.nodes.len() > i
                    && &existing_path.nodes[0..=i] == root_path
                    && existing_path.nodes.len() > i + 1
                {
                    removed_edges.insert((
                        existing_path.nodes[i].as_str(),
                        existing_path.nodes[i + 1].as_str(),
                    ));
                }
            }

            // Exclude root path nodes (except spur_node) to ensure simple paths
            let excluded_nodes: HashSet<&str> = root_path[..i].iter().map(|s| s.as_str()).collect();

            // Find spur path from spur_node to target with excluded edges and nodes
            if let Some(spur_path) = find_shortest_path_excluding(
                &adj,
                spur_node,
                target,
                max_hops,
                &removed_edges,
                &excluded_nodes,
            ) {
                // Combine root + spur (avoiding duplicate spur_node)
                let mut total_path = root_path.to_vec();
                total_path.extend_from_slice(&spur_path.nodes[1..]);
                let mut total_edges = root_edges.to_vec();
                total_edges.extend_from_slice(&spur_path.edges);

                // Check max_hops constraint
                if let Some(limit) = max_hops
                    && total_edges.len() > limit
                {
                    continue;
                }

                let candidate = Path {
                    nodes: total_path,
                    edges: total_edges,
                };

                // Add to candidates if not already in results
                if !result_paths.contains(&candidate) {
                    let score = candidate.edges.len();
                    candidate_paths.push(std::cmp::Reverse(ScoredPath {
                        score,
                        path: candidate,
                    }));
                }
            }
        }

        // Pick the shortest candidate as the next path
        if let Some(std::cmp::Reverse(scored)) = candidate_paths.pop() {
            result_paths.push(scored.path);
        } else {
            break; // No more paths available
        }
    }

    result_paths
}

/// Find the shortest path between two nodes using BFS.
fn find_shortest_path(
    adj: &HashMap<&str, Vec<(&str, EdgeKind)>>,
    source: &str,
    target: &str,
    max_hops: Option<usize>,
) -> Option<Path> {
    if source == target {
        return Some(Path {
            nodes: vec![source.to_string()],
            edges: Vec::new(),
        });
    }

    let mut queue: VecDeque<(String, Vec<String>, Vec<EdgeKind>)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    queue.push_back((source.to_string(), vec![source.to_string()], Vec::new()));
    visited.insert(source.to_string());

    while let Some((current, path, edges)) = queue.pop_front() {
        if let Some(limit) = max_hops
            && edges.len() >= limit
        {
            continue;
        }

        if let Some(neighbors) = adj.get(current.as_str()) {
            for &(neighbor, edge_kind) in neighbors {
                if visited.contains(neighbor) {
                    continue;
                }

                let mut new_path = path.clone();
                new_path.push(neighbor.to_string());
                let mut new_edges = edges.clone();
                new_edges.push(edge_kind);

                if neighbor == target {
                    return Some(Path {
                        nodes: new_path,
                        edges: new_edges,
                    });
                }

                visited.insert(neighbor.to_string());
                queue.push_back((neighbor.to_string(), new_path, new_edges));
            }
        }
    }

    None
}

/// Find shortest path excluding specific edges and nodes (for Yen's algorithm).
fn find_shortest_path_excluding(
    adj: &HashMap<&str, Vec<(&str, EdgeKind)>>,
    source: &str,
    target: &str,
    max_hops: Option<usize>,
    excluded_edges: &HashSet<(&str, &str)>,
    excluded_nodes: &HashSet<&str>,
) -> Option<Path> {
    if source == target {
        return Some(Path {
            nodes: vec![source.to_string()],
            edges: Vec::new(),
        });
    }

    let mut queue: VecDeque<(String, Vec<String>, Vec<EdgeKind>)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    queue.push_back((source.to_string(), vec![source.to_string()], Vec::new()));
    visited.insert(source.to_string());

    while let Some((current, path, edges)) = queue.pop_front() {
        if let Some(limit) = max_hops
            && edges.len() >= limit
        {
            continue;
        }

        if let Some(neighbors) = adj.get(current.as_str()) {
            for &(neighbor, edge_kind) in neighbors {
                // Skip excluded edges
                if excluded_edges.contains(&(current.as_str(), neighbor)) {
                    continue;
                }

                // Skip excluded nodes (from root path)
                if excluded_nodes.contains(neighbor) {
                    continue;
                }

                if visited.contains(neighbor) {
                    continue;
                }

                let mut new_path = path.clone();
                new_path.push(neighbor.to_string());
                let mut new_edges = edges.clone();
                new_edges.push(edge_kind);

                if neighbor == target {
                    return Some(Path {
                        nodes: new_path,
                        edges: new_edges,
                    });
                }

                visited.insert(neighbor.to_string());
                queue.push_back((neighbor.to_string(), new_path, new_edges));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, Entity, EntityDeps, EntityKind};
    use std::path::PathBuf;

    fn make_test_entity(id: &str) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: id.to_string(),
            file: PathBuf::from("test.rs"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: Vec::new(),
            feature_source: None,
            hierarchy_path: "Test".to_string(),
            deps: EntityDeps::default(),
            signature: None,
        }
    }

    #[test]
    fn test_simple_path() {
        // A -> B -> C
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A"));
        graph.insert_entity(make_test_entity("B"));
        graph.insert_entity(make_test_entity("C"));
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

        let paths = find_paths(&graph, "A", "C", None, 3, None);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].nodes, vec!["A", "B", "C"]);
        assert_eq!(paths[0].edges.len(), 2);
    }

    #[test]
    fn test_multiple_paths() {
        // A -> B -> D
        //  \-> C -> D
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A"));
        graph.insert_entity(make_test_entity("B"));
        graph.insert_entity(make_test_entity("C"));
        graph.insert_entity(make_test_entity("D"));

        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "C".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "B".to_string(),
            target: "D".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "C".to_string(),
            target: "D".to_string(),
            kind: EdgeKind::Invokes,
        });

        let paths = find_paths(&graph, "A", "D", None, 3, None);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].len(), 2); // Both paths have same length
        assert_eq!(paths[1].len(), 2);
    }

    #[test]
    fn test_no_path() {
        // A -> B    C (disconnected)
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A"));
        graph.insert_entity(make_test_entity("B"));
        graph.insert_entity(make_test_entity("C"));
        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });

        let paths = find_paths(&graph, "A", "C", None, 3, None);
        assert_eq!(paths.len(), 0);
    }

    #[test]
    fn test_max_hops() {
        // A -> B -> C -> D
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A"));
        graph.insert_entity(make_test_entity("B"));
        graph.insert_entity(make_test_entity("C"));
        graph.insert_entity(make_test_entity("D"));

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

        // Can't reach D with max_hops=2
        let paths = find_paths(&graph, "A", "D", Some(2), 3, None);
        assert_eq!(paths.len(), 0);

        // Can reach D with max_hops=3
        let paths = find_paths(&graph, "A", "D", Some(3), 3, None);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn test_max_paths() {
        // Create a graph with 4 paths: A -> {B1, B2, B3, B4} -> C
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A"));
        graph.insert_entity(make_test_entity("B1"));
        graph.insert_entity(make_test_entity("B2"));
        graph.insert_entity(make_test_entity("B3"));
        graph.insert_entity(make_test_entity("B4"));
        graph.insert_entity(make_test_entity("C"));

        for i in 1..=4 {
            let b = format!("B{}", i);
            graph.edges.push(DependencyEdge {
                source: "A".to_string(),
                target: b.clone(),
                kind: EdgeKind::Invokes,
            });
            graph.edges.push(DependencyEdge {
                source: b,
                target: "C".to_string(),
                kind: EdgeKind::Invokes,
            });
        }

        // Request only 2 paths
        let paths = find_paths(&graph, "A", "C", None, 2, None);
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn test_edge_filter() {
        // A -Invokes-> B -Imports-> C
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A"));
        graph.insert_entity(make_test_entity("B"));
        graph.insert_entity(make_test_entity("C"));

        graph.edges.push(DependencyEdge {
            source: "A".to_string(),
            target: "B".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "B".to_string(),
            target: "C".to_string(),
            kind: EdgeKind::Imports,
        });

        // Filter by Invokes only - can't reach C
        let paths = find_paths(&graph, "A", "C", None, 3, Some(EdgeKind::Invokes));
        assert_eq!(paths.len(), 0);

        // No filter - can reach C
        let paths = find_paths(&graph, "A", "C", None, 3, None);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn test_same_source_target() {
        let mut graph = RPGraph::new("rust");
        graph.insert_entity(make_test_entity("A"));

        let paths = find_paths(&graph, "A", "A", None, 3, None);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].nodes, vec!["A"]);
        assert_eq!(paths[0].edges.len(), 0);
    }
}
