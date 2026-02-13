use rpg_core::graph::*;
use rpg_nav::explore::{Direction, explore, format_tree};
use std::path::PathBuf;

fn make_entity(id: &str, name: &str, file: &str) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Function,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: Vec::new(),
        feature_source: None,
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

fn make_graph() -> RPGraph {
    let mut graph = RPGraph::new("rust");

    graph.insert_entity(make_entity("a.rs:a", "a", "a.rs"));
    graph.insert_entity(make_entity("b.rs:b", "b", "b.rs"));
    graph.insert_entity(make_entity("c.rs:c", "c", "c.rs"));
    graph.insert_entity(make_entity("d.rs:d", "d", "d.rs"));

    // a -> b (invokes)
    graph.edges.push(DependencyEdge {
        source: "a.rs:a".to_string(),
        target: "b.rs:b".to_string(),
        kind: EdgeKind::Invokes,
    });
    // b -> c (invokes)
    graph.edges.push(DependencyEdge {
        source: "b.rs:b".to_string(),
        target: "c.rs:c".to_string(),
        kind: EdgeKind::Invokes,
    });
    // d -> a (imports)
    graph.edges.push(DependencyEdge {
        source: "d.rs:d".to_string(),
        target: "a.rs:a".to_string(),
        kind: EdgeKind::Imports,
    });

    graph
}

#[test]
fn test_explore_downstream() {
    let graph = make_graph();
    let result = explore(&graph, "a.rs:a", Direction::Downstream, 3, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    assert_eq!(tree.entity_name, "a");
    // a -> b, so b should be a child
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "b");
    // b -> c, so c should be a grandchild
    assert_eq!(tree.children[0].children.len(), 1);
    assert_eq!(tree.children[0].children[0].entity_name, "c");
}

#[test]
fn test_explore_upstream() {
    let graph = make_graph();
    let result = explore(&graph, "a.rs:a", Direction::Upstream, 3, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    assert_eq!(tree.entity_name, "a");
    // d -> a, so d should appear upstream
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "d");
}

#[test]
fn test_explore_both_directions() {
    let graph = make_graph();
    let result = explore(&graph, "a.rs:a", Direction::Both, 1, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    assert_eq!(tree.entity_name, "a");
    // Should have both downstream (b) and upstream (d)
    assert_eq!(tree.children.len(), 2);
    let child_names: Vec<&str> = tree
        .children
        .iter()
        .map(|c| c.entity_name.as_str())
        .collect();
    assert!(child_names.contains(&"b"));
    assert!(child_names.contains(&"d"));
}

#[test]
fn test_explore_max_depth_limits() {
    let graph = make_graph();
    let result = explore(&graph, "a.rs:a", Direction::Downstream, 1, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    // depth=1 means only direct neighbors, b should have no children
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "b");
    assert!(tree.children[0].children.is_empty());
}

#[test]
fn test_explore_edge_filter() {
    let graph = make_graph();
    // Filter only Imports edges from a.rs:a perspective
    let result = explore(
        &graph,
        "a.rs:a",
        Direction::Both,
        3,
        Some(EdgeKind::Imports),
    );
    assert!(result.is_some());
    let tree = result.unwrap();
    // Only d->a is an Imports edge, and it's upstream
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "d");
}

#[test]
fn test_explore_nonexistent_entity() {
    let graph = make_graph();
    let result = explore(&graph, "nonexistent", Direction::Downstream, 3, None);
    assert!(result.is_none());
}

#[test]
fn test_explore_no_cycles() {
    // Graph with cycle: a -> b -> a
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:a", "a", "a.rs"));
    graph.insert_entity(make_entity("b.rs:b", "b", "b.rs"));
    graph.edges.push(DependencyEdge {
        source: "a.rs:a".to_string(),
        target: "b.rs:b".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.edges.push(DependencyEdge {
        source: "b.rs:b".to_string(),
        target: "a.rs:a".to_string(),
        kind: EdgeKind::Invokes,
    });

    let result = explore(&graph, "a.rs:a", Direction::Downstream, 10, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    // a -> b, but b -> a is prevented by visited set
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "b");
    assert!(tree.children[0].children.is_empty());
}

#[test]
fn test_explore_unlimited_depth() {
    // Build a long chain: a -> b -> c -> d with depth = usize::MAX (simulating -1)
    let mut graph = make_graph();
    // The default make_graph has a->b->c chain at depth 3
    // With unlimited depth, we should traverse the full chain
    graph.rebuild_edge_index();

    let result = explore(&graph, "a.rs:a", Direction::Downstream, usize::MAX, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    // a -> b -> c (full chain traversed, d is only connected upstream via imports)
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "b");
    assert_eq!(tree.children[0].children.len(), 1);
    assert_eq!(tree.children[0].children[0].entity_name, "c");
}

#[test]
fn test_explore_composes_edge() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("mod.rs:mod_a", "mod_a", "mod.rs"));
    graph.insert_entity(make_entity("impl.rs:impl_a", "impl_a", "impl.rs"));
    graph.edges.push(DependencyEdge {
        source: "mod.rs:mod_a".to_string(),
        target: "impl.rs:impl_a".to_string(),
        kind: EdgeKind::Composes,
    });
    graph.rebuild_edge_index();

    // Should traverse Composes edges
    let result = explore(&graph, "mod.rs:mod_a", Direction::Downstream, 2, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "impl_a");
    assert_eq!(tree.children[0].edge_kind, Some(EdgeKind::Composes));

    // Filter to only Composes edges
    let result = explore(
        &graph,
        "mod.rs:mod_a",
        Direction::Downstream,
        2,
        Some(EdgeKind::Composes),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().children.len(), 1);

    // Filter to Invokes should find nothing
    let result = explore(
        &graph,
        "mod.rs:mod_a",
        Direction::Downstream,
        2,
        Some(EdgeKind::Invokes),
    );
    assert!(result.is_some());
    assert!(result.unwrap().children.is_empty());
}

#[test]
fn test_explore_uses_edge_index() {
    // Verify that explore works correctly when edge_index is populated
    let mut graph = make_graph();
    graph.rebuild_edge_index();

    let result = explore(&graph, "a.rs:a", Direction::Downstream, 3, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entity_name, "b");
    assert_eq!(tree.children[0].children.len(), 1);
    assert_eq!(tree.children[0].children[0].entity_name, "c");
}

#[test]
fn test_explore_depth_zero() {
    let graph = make_graph();
    // Depth 0 should return just the root node with no children
    let result = explore(&graph, "a.rs:a", Direction::Downstream, 0, None);
    assert!(result.is_some());
    let tree = result.unwrap();
    assert_eq!(tree.entity_name, "a");
    assert!(tree.children.is_empty(), "depth=0 should have no children");
}

#[test]
fn test_format_tree_output() {
    let graph = make_graph();
    let tree = explore(&graph, "a.rs:a", Direction::Downstream, 2, None).unwrap();
    let output = format_tree(&tree, 0);
    assert!(output.contains("a [a.rs]"));
    assert!(output.contains("b [b.rs]"));
    assert!(output.contains("c [c.rs]"));
}
