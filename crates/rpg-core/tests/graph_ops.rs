use rpg_core::graph::*;
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
        semantic_features: vec!["test feature".to_string()],
        feature_source: None,
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_insert_entity() {
    let mut graph = RPGraph::new("rust");
    let entity = make_entity("src/main.rs:main", "main", "src/main.rs");
    graph.insert_entity(entity);

    assert_eq!(graph.entities.len(), 1);
    assert!(graph.entities.contains_key("src/main.rs:main"));
    assert!(graph.file_index.contains_key(&PathBuf::from("src/main.rs")));
    assert_eq!(graph.file_index[&PathBuf::from("src/main.rs")].len(), 1);
}

#[test]
fn test_insert_multiple_entities_same_file() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:foo", "foo", "f.rs"));
    graph.insert_entity(make_entity("f.rs:bar", "bar", "f.rs"));

    assert_eq!(graph.entities.len(), 2);
    assert_eq!(graph.file_index[&PathBuf::from("f.rs")].len(), 2);
}

#[test]
fn test_remove_entity() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:foo", "foo", "f.rs"));

    let removed = graph.remove_entity("f.rs:foo");
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().name, "foo");
    assert!(graph.entities.is_empty());
    assert!(graph.file_index.is_empty());
}

#[test]
fn test_remove_entity_cleans_edges() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:a", "a", "f.rs"));
    graph.insert_entity(make_entity("f.rs:b", "b", "f.rs"));
    graph.edges.push(DependencyEdge {
        source: "f.rs:a".to_string(),
        target: "f.rs:b".to_string(),
        kind: EdgeKind::Invokes,
    });

    graph.remove_entity("f.rs:a");
    assert!(graph.edges.is_empty());
}

#[test]
fn test_remove_nonexistent_entity() {
    let mut graph = RPGraph::new("rust");
    let removed = graph.remove_entity("nonexistent");
    assert!(removed.is_none());
}

#[test]
fn test_insert_into_hierarchy() {
    let mut graph = RPGraph::new("rust");
    graph.insert_into_hierarchy("DataAccess/storage/file_io", "entity1");

    assert!(graph.hierarchy.contains_key("DataAccess"));
    let area = &graph.hierarchy["DataAccess"];
    assert!(area.children.contains_key("storage"));
    let cat = &area.children["storage"];
    assert!(cat.children.contains_key("file_io"));
    let sub = &cat.children["file_io"];
    assert!(sub.entities.contains(&"entity1".to_string()));
}

#[test]
fn test_hierarchy_no_duplicates() {
    let mut graph = RPGraph::new("rust");
    graph.insert_into_hierarchy("Area/cat/sub", "e1");
    graph.insert_into_hierarchy("Area/cat/sub", "e1");

    let sub = &graph.hierarchy["Area"].children["cat"].children["sub"];
    assert_eq!(sub.entities.len(), 1);
}

#[test]
fn test_remove_entity_from_hierarchy() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:foo", "foo", "f.rs"));
    graph.insert_into_hierarchy("Area/cat/sub", "f.rs:foo");

    graph.remove_entity("f.rs:foo");
    // Hierarchy should be pruned since no entities remain
    assert!(graph.hierarchy.is_empty());
}

#[test]
fn test_refresh_metadata() {
    let mut graph = RPGraph::new("python");
    graph.insert_entity(make_entity("a.py:f1", "f1", "a.py"));
    graph.insert_entity(make_entity("b.py:f2", "f2", "b.py"));
    graph.edges.push(DependencyEdge {
        source: "a.py:f1".to_string(),
        target: "b.py:f2".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.insert_into_hierarchy("Area/cat/sub", "a.py:f1");

    graph.refresh_metadata();
    assert_eq!(graph.metadata.total_entities, 2);
    assert_eq!(graph.metadata.total_files, 2);
    assert_eq!(graph.metadata.total_edges, 1);
    assert_eq!(graph.metadata.functional_areas, 1);
}

#[test]
fn test_get_entity() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:foo", "foo", "f.rs"));

    assert!(graph.get_entity("f.rs:foo").is_some());
    assert!(graph.get_entity("nonexistent").is_none());
}

#[test]
fn test_edges_for() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:a", "a", "f.rs"));
    graph.insert_entity(make_entity("f.rs:b", "b", "f.rs"));
    graph.insert_entity(make_entity("f.rs:c", "c", "f.rs"));
    graph.edges.push(DependencyEdge {
        source: "f.rs:a".to_string(),
        target: "f.rs:b".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.edges.push(DependencyEdge {
        source: "f.rs:c".to_string(),
        target: "f.rs:a".to_string(),
        kind: EdgeKind::Imports,
    });

    let edges = graph.edges_for("f.rs:a");
    assert_eq!(edges.len(), 2);
}

#[test]
fn test_edges_for_indexed_vs_fallback_equivalence() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:a", "a", "f.rs"));
    graph.insert_entity(make_entity("f.rs:b", "b", "f.rs"));
    graph.insert_entity(make_entity("g.rs:c", "c", "g.rs"));
    graph.edges.push(DependencyEdge {
        source: "f.rs:a".to_string(),
        target: "f.rs:b".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.edges.push(DependencyEdge {
        source: "g.rs:c".to_string(),
        target: "f.rs:a".to_string(),
        kind: EdgeKind::Imports,
    });
    graph.edges.push(DependencyEdge {
        source: "f.rs:a".to_string(),
        target: "g.rs:c".to_string(),
        kind: EdgeKind::Composes,
    });

    // Fallback path (no index built)
    assert!(graph.edge_index.is_empty());
    let fallback_count = graph.edges_for("f.rs:a").len();
    let mut fallback_kinds: Vec<String> = graph
        .edges_for("f.rs:a")
        .iter()
        .map(|e| format!("{:?}", e.kind))
        .collect();
    fallback_kinds.sort();

    // Indexed path
    graph.rebuild_edge_index();
    assert!(!graph.edge_index.is_empty());
    let indexed_count = graph.edges_for("f.rs:a").len();
    let mut indexed_kinds: Vec<String> = graph
        .edges_for("f.rs:a")
        .iter()
        .map(|e| format!("{:?}", e.kind))
        .collect();
    indexed_kinds.sort();

    // Both paths should return the same edges
    assert_eq!(fallback_count, indexed_count);
    assert_eq!(fallback_kinds, indexed_kinds);
}

#[test]
fn test_hierarchy_node_entity_count() {
    let mut node = HierarchyNode::new("root");
    node.entities.push("e1".to_string());
    let mut child = HierarchyNode::new("child");
    child.entities.push("e2".to_string());
    child.entities.push("e3".to_string());
    node.children.insert("child".to_string(), child);

    assert_eq!(node.entity_count(), 3);
}

#[test]
fn test_hierarchy_node_all_entity_ids() {
    let mut node = HierarchyNode::new("root");
    node.entities.push("e1".to_string());
    let mut child = HierarchyNode::new("child");
    child.entities.push("e2".to_string());
    node.children.insert("child".to_string(), child);

    let ids = node.all_entity_ids();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"e1".to_string()));
    assert!(ids.contains(&"e2".to_string()));
}

#[test]
fn test_assign_hierarchy_ids() {
    let mut graph = RPGraph::new("rust");
    graph.insert_into_hierarchy("Security/auth/token", "e1");
    graph.insert_into_hierarchy("Security/auth/login", "e2");
    graph.insert_into_hierarchy("DataAccess/storage/db", "e3");

    graph.assign_hierarchy_ids();

    let security = &graph.hierarchy["Security"];
    assert_eq!(security.id, "h:Security");
    assert_eq!(security.children["auth"].id, "h:Security/auth");
    assert_eq!(
        security.children["auth"].children["token"].id,
        "h:Security/auth/token"
    );
    assert_eq!(
        security.children["auth"].children["login"].id,
        "h:Security/auth/login"
    );
    assert_eq!(graph.hierarchy["DataAccess"].id, "h:DataAccess");
}

#[test]
fn test_aggregate_features() {
    let mut graph = RPGraph::new("rust");
    let mut e1 = make_entity("a.rs:foo", "foo", "a.rs");
    e1.semantic_features = vec!["auth check".into(), "token validation".into()];
    let mut e2 = make_entity("b.rs:bar", "bar", "b.rs");
    e2.semantic_features = vec!["auth check".into(), "login flow".into()];
    graph.insert_entity(e1);
    graph.insert_entity(e2);

    graph.insert_into_hierarchy("Security/auth/token", "a.rs:foo");
    graph.insert_into_hierarchy("Security/auth/login", "b.rs:bar");

    graph.aggregate_hierarchy_features();

    let security = &graph.hierarchy["Security"];
    // Should have deduplicated features from all children
    assert!(security.semantic_features.contains(&"auth check".into()));
    assert!(
        security
            .semantic_features
            .contains(&"token validation".into())
    );
    assert!(security.semantic_features.contains(&"login flow".into()));
    // "auth check" should appear only once (deduped)
    assert_eq!(
        security
            .semantic_features
            .iter()
            .filter(|f| *f == "auth check")
            .count(),
        1
    );
}

#[test]
fn test_materialize_containment_edges() {
    let mut graph = RPGraph::new("rust");
    graph.insert_into_hierarchy("Area/cat/sub", "e1");
    graph.insert_into_hierarchy("Area/cat/sub", "e2");
    graph.assign_hierarchy_ids();
    graph.materialize_containment_edges();

    // Should have: Area->cat, cat->sub, sub->e1, sub->e2
    let contains_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .collect();
    assert_eq!(contains_edges.len(), 4);

    // Verify parent→child edges exist
    assert!(
        contains_edges
            .iter()
            .any(|e| e.source == "h:Area" && e.target == "h:Area/cat")
    );
    assert!(
        contains_edges
            .iter()
            .any(|e| e.source == "h:Area/cat" && e.target == "h:Area/cat/sub")
    );
    assert!(
        contains_edges
            .iter()
            .any(|e| e.source == "h:Area/cat/sub" && e.target == "e1")
    );
    assert!(
        contains_edges
            .iter()
            .any(|e| e.source == "h:Area/cat/sub" && e.target == "e2")
    );
}

#[test]
fn test_contains_edge_kind_serde() {
    let edge = DependencyEdge {
        source: "h:Area".to_string(),
        target: "h:Area/cat".to_string(),
        kind: EdgeKind::Contains,
    };
    let json = serde_json::to_string(&edge).unwrap();
    assert!(json.contains("\"contains\""));
    let deserialized: DependencyEdge = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.kind, EdgeKind::Contains);
}

#[test]
fn test_refresh_metadata_edge_counts() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:f1", "f1", "a.rs"));
    graph.edges.push(DependencyEdge {
        source: "a.rs:f1".to_string(),
        target: "b.rs:f2".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.edges.push(DependencyEdge {
        source: "h:Area".to_string(),
        target: "a.rs:f1".to_string(),
        kind: EdgeKind::Contains,
    });

    graph.refresh_metadata();
    assert_eq!(graph.metadata.total_edges, 2);
    assert_eq!(graph.metadata.dependency_edges, 1);
    assert_eq!(graph.metadata.containment_edges, 1);
}
