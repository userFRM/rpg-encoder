use rpg_core::graph::*;
use rpg_nav::export::{ExportFormat, export};
use std::path::PathBuf;

fn make_entity(id: &str, name: &str, file: &str, kind: EntityKind) -> Entity {
    Entity {
        id: id.to_string(),
        kind,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: vec!["test feature".to_string()],
        feature_source: None,
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

fn make_graph() -> RPGraph {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:a", "a", "a.rs", EntityKind::Function));
    graph.insert_entity(make_entity("b.rs:b", "b", "b.rs", EntityKind::Class));
    graph.edges.push(DependencyEdge {
        source: "a.rs:a".to_string(),
        target: "b.rs:b".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.edges.push(DependencyEdge {
        source: "a.rs:a".to_string(),
        target: "b.rs:b".to_string(),
        kind: EdgeKind::Imports,
    });
    graph.edges.push(DependencyEdge {
        source: "b.rs:b".to_string(),
        target: "a.rs:a".to_string(),
        kind: EdgeKind::Inherits,
    });
    graph.edges.push(DependencyEdge {
        source: "a.rs:a".to_string(),
        target: "b.rs:b".to_string(),
        kind: EdgeKind::Composes,
    });
    graph
}

#[test]
fn test_export_dot_all_edge_kinds() {
    let graph = make_graph();
    let dot = export(&graph, ExportFormat::Dot);

    // All edge kinds should appear with correct styles
    assert!(dot.contains("invokes"), "DOT should contain invokes label");
    assert!(dot.contains("imports"), "DOT should contain imports label");
    assert!(
        dot.contains("inherits"),
        "DOT should contain inherits label"
    );
    assert!(
        dot.contains("composes"),
        "DOT should contain composes label"
    );

    // Entity shapes
    assert!(
        dot.contains("ellipse"),
        "functions should use ellipse shape"
    );
    assert!(dot.contains("shape=box"), "classes should use box shape");

    // Basic structure
    assert!(dot.starts_with("digraph RPG {"));
    assert!(dot.trim_end().ends_with('}'));
}

#[test]
fn test_export_mermaid_all_edge_kinds() {
    let graph = make_graph();
    let mermaid = export(&graph, ExportFormat::Mermaid);

    // All edge kinds should appear
    assert!(
        mermaid.contains("|invokes|"),
        "Mermaid should contain invokes label"
    );
    assert!(
        mermaid.contains("|imports|"),
        "Mermaid should contain imports label"
    );
    assert!(
        mermaid.contains("|inherits|"),
        "Mermaid should contain inherits label"
    );
    assert!(
        mermaid.contains("|composes|"),
        "Mermaid should contain composes label"
    );

    // Arrow styles
    assert!(
        mermaid.contains("-->"),
        "invokes/composes should use solid arrow"
    );
    assert!(mermaid.contains("-.->"), "imports should use dashed arrow");
    assert!(mermaid.contains("==>"), "inherits should use bold arrow");

    // Basic structure
    assert!(mermaid.starts_with("flowchart LR"));
}

#[test]
fn test_export_dot_contains_edges() {
    let mut graph = RPGraph::new("rust");
    graph.insert_into_hierarchy("Area/cat/sub", "e1");
    graph.assign_hierarchy_ids();
    graph.materialize_containment_edges();

    let dot = export(&graph, ExportFormat::Dot);
    assert!(
        dot.contains("contains"),
        "DOT should contain containment edges"
    );
    assert!(
        dot.contains("dotted"),
        "containment edges should use dotted style"
    );
}

#[test]
fn test_export_mermaid_skips_contains() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:a", "a", "a.rs", EntityKind::Function));
    graph.insert_into_hierarchy("Area/cat/sub", "a.rs:a");
    graph.assign_hierarchy_ids();
    graph.materialize_containment_edges();

    let mermaid = export(&graph, ExportFormat::Mermaid);
    // Mermaid skips Contains edges (shown via subgraphs instead)
    assert!(
        !mermaid.contains("|contains|"),
        "Mermaid should skip contains edge labels"
    );
}
