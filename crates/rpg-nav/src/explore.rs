//! ExploreRPG: dependency traversal along graph edges.

use rpg_core::graph::{EdgeKind, EntityKind, RPGraph};
use std::collections::{HashSet, VecDeque};

/// Traversal direction.
#[derive(Debug, Clone, Copy)]
pub enum Direction {
    /// Follow edges where entity is the source (what it uses).
    Downstream,
    /// Follow edges where entity is the target (what uses it).
    Upstream,
    /// Both directions.
    Both,
}

/// A node in the traversal result tree.
#[derive(Debug, Clone)]
pub struct TraversalNode {
    pub entity_id: String,
    pub entity_name: String,
    pub file: String,
    pub edge_kind: Option<EdgeKind>,
    pub direction: Option<String>, // "downstream" or "upstream"
    pub depth: usize,
    pub children: Vec<TraversalNode>,
}

/// Explore the dependency graph from a starting entity or hierarchy node.
pub fn explore(
    graph: &RPGraph,
    start_entity_id: &str,
    direction: Direction,
    max_depth: usize,
    edge_filter: Option<EdgeKind>,
) -> Option<TraversalNode> {
    explore_filtered(
        graph,
        start_entity_id,
        direction,
        max_depth,
        edge_filter,
        None,
    )
}

/// Explore with optional entity type filtering on neighbors.
pub fn explore_filtered(
    graph: &RPGraph,
    start_entity_id: &str,
    direction: Direction,
    max_depth: usize,
    edge_filter: Option<EdgeKind>,
    entity_type_filter: Option<&[EntityKind]>,
) -> Option<TraversalNode> {
    // Try V_L entity first, then V_H hierarchy node
    let (name, file_or_desc) = if let Some(entity) = graph.get_entity(start_entity_id) {
        (entity.name.clone(), entity.file.display().to_string())
    } else if let Some((name, desc)) = graph.get_node_display_info(start_entity_id) {
        (name, desc)
    } else {
        return None;
    };

    let mut root = TraversalNode {
        entity_id: start_entity_id.to_string(),
        entity_name: name,
        file: file_or_desc,
        edge_kind: None,
        direction: None,
        depth: 0,
        children: Vec::new(),
    };

    let mut visited = HashSet::new();
    visited.insert(start_entity_id.to_string());

    let mut queue: VecDeque<(String, usize, Vec<usize>)> = VecDeque::new();
    queue.push_back((start_entity_id.to_string(), 0, Vec::new()));

    // BFS traversal
    while let Some((current_id, depth, path_indices)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let neighbors = get_neighbors(graph, &current_id, direction, edge_filter);

        for (neighbor_id, edge_kind, dir_str) in neighbors {
            if visited.contains(&neighbor_id) {
                continue;
            }
            visited.insert(neighbor_id.clone());

            // Try V_L entity first, then V_H hierarchy node via unified lookup
            let (name, file_or_desc) = if let Some(neighbor_entity) = graph.get_entity(&neighbor_id)
            {
                // Apply entity type filter
                if let Some(kinds) = entity_type_filter
                    && !kinds.contains(&neighbor_entity.kind)
                {
                    continue;
                }
                (
                    neighbor_entity.name.clone(),
                    neighbor_entity.file.display().to_string(),
                )
            } else if let Some((name, desc)) = graph.get_node_display_info(&neighbor_id) {
                (name, desc)
            } else {
                continue;
            };

            let child_node = TraversalNode {
                entity_id: neighbor_id.clone(),
                entity_name: name,
                file: file_or_desc,
                edge_kind: Some(edge_kind),
                direction: Some(dir_str.to_string()),
                depth: depth + 1,
                children: Vec::new(),
            };

            // Insert into the tree at the correct position
            insert_child(&mut root, &path_indices, child_node);

            let mut new_path = path_indices.clone();
            new_path.push(get_child_count(&root, &path_indices) - 1);
            queue.push_back((neighbor_id, depth + 1, new_path));
        }
    }

    Some(root)
}

fn get_neighbors(
    graph: &RPGraph,
    entity_id: &str,
    direction: Direction,
    edge_filter: Option<EdgeKind>,
) -> Vec<(String, EdgeKind, &'static str)> {
    let mut neighbors = Vec::new();

    for edge in &graph.edges {
        if let Some(filter) = edge_filter
            && edge.kind != filter
        {
            continue;
        }

        match direction {
            Direction::Downstream => {
                if edge.source == entity_id {
                    neighbors.push((edge.target.clone(), edge.kind, "downstream"));
                }
            }
            Direction::Upstream => {
                if edge.target == entity_id {
                    neighbors.push((edge.source.clone(), edge.kind, "upstream"));
                }
            }
            Direction::Both => {
                if edge.source == entity_id {
                    neighbors.push((edge.target.clone(), edge.kind, "downstream"));
                }
                if edge.target == entity_id {
                    neighbors.push((edge.source.clone(), edge.kind, "upstream"));
                }
            }
        }
    }

    neighbors
}

fn insert_child(root: &mut TraversalNode, path: &[usize], child: TraversalNode) {
    if path.is_empty() {
        root.children.push(child);
        return;
    }

    if let Some(node) = root.children.get_mut(path[0]) {
        insert_child(node, &path[1..], child);
    }
}

fn get_child_count(root: &TraversalNode, path: &[usize]) -> usize {
    if path.is_empty() {
        return root.children.len();
    }

    if let Some(node) = root.children.get(path[0]) {
        get_child_count(node, &path[1..])
    } else {
        0
    }
}

/// Format a traversal result as an indented tree string.
pub fn format_tree(node: &TraversalNode, indent: usize) -> String {
    format_tree_inner(node, indent, false)
}

fn format_tree_inner(node: &TraversalNode, indent: usize, is_last: bool) -> String {
    let mut output = String::new();
    let prefix = "  ".repeat(indent);

    if indent == 0 {
        output.push_str(&format!("{} [{}]\n", node.entity_name, node.file));
    } else {
        let connector = if is_last { "└──" } else { "├──" };
        let edge_str = node
            .edge_kind
            .map(|k| format!("{:?}", k).to_lowercase())
            .unwrap_or_default();
        let dir_str = node.direction.as_deref().unwrap_or("");
        output.push_str(&format!(
            "{}{} {} ({}): {} [{}]\n",
            prefix, connector, edge_str, dir_str, node.entity_name, node.file
        ));
    }

    let child_count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        output.push_str(&format_tree_inner(child, indent + 1, i == child_count - 1));
    }

    output
}
