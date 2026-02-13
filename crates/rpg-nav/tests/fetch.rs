use rpg_core::graph::*;
use rpg_nav::fetch::{FetchOutput, fetch};
use std::path::PathBuf;
use tempfile::TempDir;

fn make_entity(id: &str, name: &str, file: &str, hierarchy: &str) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Function,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 1,
        line_end: 3,
        parent_class: None,
        semantic_features: vec!["test feature".to_string()],
        feature_source: None,
        hierarchy_path: hierarchy.to_string(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_fetch_existing_entity() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create a source file
    std::fs::write(
        root.join("main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity(
        "main.rs:main",
        "main",
        "main.rs",
        "Core/entry/main",
    ));

    let output = fetch(&graph, "main.rs:main", root).unwrap();
    match output {
        FetchOutput::Entity(result) => {
            assert_eq!(result.entity.name, "main");
            assert!(result.source_code.is_some());
            let source = result.source_code.unwrap();
            assert!(source.contains("fn main()"));
            assert!(source.contains("println!"));
        }
        FetchOutput::Hierarchy(_) => panic!("expected Entity result"),
    }
}

#[test]
fn test_fetch_nonexistent_entity() {
    let tmp = TempDir::new().unwrap();
    let graph = RPGraph::new("rust");

    let result = fetch(&graph, "nonexistent", tmp.path());
    assert!(result.is_err());
}

#[test]
fn test_fetch_missing_source_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("gone.rs:foo", "foo", "gone.rs", ""));

    let output = fetch(&graph, "gone.rs:foo", root).unwrap();
    match output {
        FetchOutput::Entity(result) => {
            assert_eq!(result.entity.name, "foo");
            // File doesn't exist, source_code should be None
            assert!(result.source_code.is_none());
        }
        FetchOutput::Hierarchy(_) => panic!("expected Entity result"),
    }
}

#[test]
fn test_fetch_finds_siblings() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    std::fs::write(root.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("lib.rs:a", "a", "lib.rs", "Core/utils/helpers"));
    graph.insert_entity(make_entity("lib.rs:b", "b", "lib.rs", "Core/utils/helpers"));

    let output = fetch(&graph, "lib.rs:a", root).unwrap();
    match output {
        FetchOutput::Entity(result) => {
            assert!(result.hierarchy_context.contains(&"lib.rs:b".to_string()));
        }
        FetchOutput::Hierarchy(_) => panic!("expected Entity result"),
    }
}

#[test]
fn test_fetch_no_siblings_different_paths() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    std::fs::write(root.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("lib.rs:a", "a", "lib.rs", "Core/utils/a"));
    graph.insert_entity(make_entity("lib.rs:b", "b", "lib.rs", "Core/utils/b"));

    let output = fetch(&graph, "lib.rs:a", root).unwrap();
    match output {
        FetchOutput::Entity(result) => {
            assert!(result.hierarchy_context.is_empty());
        }
        FetchOutput::Hierarchy(_) => panic!("expected Entity result"),
    }
}

#[test]
fn test_fetch_hierarchy_node() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity(
        "auth.rs:login",
        "login",
        "auth.rs",
        "Auth/login/validate",
    ));
    graph.insert_into_hierarchy("Auth/login/validate", "auth.rs:login");
    graph.assign_hierarchy_ids();

    let output = fetch(&graph, "h:Auth", root).unwrap();
    match output {
        FetchOutput::Hierarchy(result) => {
            assert_eq!(result.node.name, "Auth");
            assert_eq!(result.entity_count, 1);
            assert!(result.child_names.contains(&"login".to_string()));
        }
        FetchOutput::Entity(_) => panic!("expected Hierarchy result"),
    }
}
