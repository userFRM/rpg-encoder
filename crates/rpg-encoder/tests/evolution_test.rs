use rpg_core::graph::*;
use rpg_encoder::evolution::{apply_deletions, apply_renames, compute_drift};
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
        semantic_features: vec!["test".to_string()],
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

// --- apply_deletions tests ---

#[test]
fn test_apply_deletions_removes_entities() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:foo", "foo", "a.rs"));
    graph.insert_entity(make_entity("a.rs:bar", "bar", "a.rs"));
    graph.insert_entity(make_entity("b.rs:baz", "baz", "b.rs"));

    let removed = apply_deletions(&mut graph, &[PathBuf::from("a.rs")]);
    assert_eq!(removed, 2);
    assert_eq!(graph.entities.len(), 1);
    assert!(graph.entities.contains_key("b.rs:baz"));
}

#[test]
fn test_apply_deletions_nonexistent_file() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:foo", "foo", "a.rs"));

    let removed = apply_deletions(&mut graph, &[PathBuf::from("nonexistent.rs")]);
    assert_eq!(removed, 0);
    assert_eq!(graph.entities.len(), 1);
}

#[test]
fn test_apply_deletions_cleans_edges() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:foo", "foo", "a.rs"));
    graph.insert_entity(make_entity("b.rs:bar", "bar", "b.rs"));
    graph.edges.push(DependencyEdge {
        source: "a.rs:foo".to_string(),
        target: "b.rs:bar".to_string(),
        kind: EdgeKind::Invokes,
    });

    apply_deletions(&mut graph, &[PathBuf::from("a.rs")]);
    // remove_entity cleans edges referencing the removed entity
    assert!(graph.edges.is_empty());
}

#[test]
fn test_apply_deletions_multiple_files() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:a", "a", "a.rs"));
    graph.insert_entity(make_entity("b.rs:b", "b", "b.rs"));
    graph.insert_entity(make_entity("c.rs:c", "c", "c.rs"));

    let removed = apply_deletions(&mut graph, &[PathBuf::from("a.rs"), PathBuf::from("c.rs")]);
    assert_eq!(removed, 2);
    assert_eq!(graph.entities.len(), 1);
    assert!(graph.entities.contains_key("b.rs:b"));
}

// --- apply_renames tests ---

#[test]
fn test_apply_renames_updates_file_path() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("old.rs:foo", "foo", "old.rs"));

    let (migrated_files, renamed) = apply_renames(
        &mut graph,
        &[(PathBuf::from("old.rs"), PathBuf::from("new.rs"))],
    );

    assert_eq!(migrated_files, 1);
    assert_eq!(renamed, 1);

    // Old ID should be gone, new ID should exist
    assert!(graph.get_entity("old.rs:foo").is_none());
    let entity = graph.get_entity("new.rs:foo").unwrap();
    assert_eq!(entity.file, PathBuf::from("new.rs"));
    assert_eq!(entity.id, "new.rs:foo");
    assert!(graph.file_index.contains_key(&PathBuf::from("new.rs")));
    assert!(!graph.file_index.contains_key(&PathBuf::from("old.rs")));
}

#[test]
fn test_apply_renames_multiple_entities() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("old.rs:foo", "foo", "old.rs"));
    graph.insert_entity(make_entity("old.rs:bar", "bar", "old.rs"));

    let (migrated_files, renamed) = apply_renames(
        &mut graph,
        &[(PathBuf::from("old.rs"), PathBuf::from("new.rs"))],
    );

    assert_eq!(migrated_files, 1);
    assert_eq!(renamed, 2);
    assert_eq!(graph.file_index[&PathBuf::from("new.rs")].len(), 2);
    // Verify new IDs exist
    assert!(graph.entities.contains_key("new.rs:foo"));
    assert!(graph.entities.contains_key("new.rs:bar"));
    // Old IDs should be gone
    assert!(!graph.entities.contains_key("old.rs:foo"));
    assert!(!graph.entities.contains_key("old.rs:bar"));
}

#[test]
fn test_apply_renames_nonexistent_file() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:foo", "foo", "a.rs"));

    let (migrated_files, renamed) = apply_renames(
        &mut graph,
        &[(PathBuf::from("nonexistent.rs"), PathBuf::from("new.rs"))],
    );

    assert_eq!(migrated_files, 0);
    assert_eq!(renamed, 0);
    assert_eq!(graph.entities.len(), 1);
}

// --- compute_drift tests ---

#[test]
fn test_compute_drift_identical() {
    let a = vec!["auth check".to_string(), "token validation".to_string()];
    assert_eq!(compute_drift(&a, &a), 0.0);
}

#[test]
fn test_compute_drift_completely_different() {
    let a = vec!["auth check".to_string()];
    let b = vec!["database query".to_string()];
    assert_eq!(compute_drift(&a, &b), 1.0);
}

#[test]
fn test_compute_drift_partial_overlap() {
    let a = vec!["auth check".to_string(), "token validation".to_string()];
    let b = vec!["auth check".to_string(), "login flow".to_string()];
    let drift = compute_drift(&a, &b);
    // Intersection = 1 ("auth check"), Union = 3 → Jaccard = 1/3, Distance = 2/3
    assert!((drift - 2.0 / 3.0).abs() < 1e-10);
}

#[test]
fn test_compute_drift_empty() {
    let empty: Vec<String> = vec![];
    let a = vec!["x".to_string()];
    assert_eq!(compute_drift(&empty, &empty), 0.0);
    assert_eq!(compute_drift(&empty, &a), 1.0);
    assert_eq!(compute_drift(&a, &empty), 1.0);
}
