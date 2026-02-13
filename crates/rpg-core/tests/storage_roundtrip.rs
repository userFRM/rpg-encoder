use rpg_core::graph::*;
use rpg_core::storage;
use std::path::PathBuf;
use tempfile::TempDir;

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
        feature_source: None,
        hierarchy_path: "Area/cat/sub".to_string(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_save_and_load_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:main", "main", "f.rs"));
    graph.edges.push(DependencyEdge {
        source: "f.rs:main".to_string(),
        target: "f.rs:helper".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.insert_into_hierarchy("Core/parsing/ast", "f.rs:main");
    graph.refresh_metadata();

    storage::save(root, &graph).unwrap();
    assert!(storage::rpg_exists(root));

    let loaded = storage::load(root).unwrap();
    assert_eq!(loaded.entities.len(), 1);
    assert_eq!(loaded.edges.len(), 1);
    assert_eq!(loaded.metadata.total_entities, 1);
    assert!(loaded.hierarchy.contains_key("Core"));
}

#[test]
fn test_composes_edge_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let mut graph = RPGraph::new("rust");
    let mut entity = make_entity("f.rs:main", "main", "f.rs");
    entity.deps.composes = vec!["helper".to_string()];
    entity.deps.composed_by = vec!["caller".to_string()];
    graph.insert_entity(entity);
    graph.edges.push(DependencyEdge {
        source: "f.rs:main".to_string(),
        target: "f.rs:helper".to_string(),
        kind: EdgeKind::Composes,
    });
    graph.refresh_metadata();

    storage::save(root, &graph).unwrap();
    let loaded = storage::load(root).unwrap();

    // Verify Composes edge survives serialization
    assert_eq!(loaded.edges.len(), 1);
    assert_eq!(loaded.edges[0].kind, EdgeKind::Composes);

    // Verify composes/composed_by deps survive serialization
    let e = loaded.entities.get("f.rs:main").unwrap();
    assert_eq!(e.deps.composes, vec!["helper".to_string()]);
    assert_eq!(e.deps.composed_by, vec!["caller".to_string()]);
}

#[test]
fn test_rpg_exists_false() {
    let tmp = TempDir::new().unwrap();
    assert!(!storage::rpg_exists(tmp.path()));
}

#[test]
fn test_rpg_dir_and_file_paths() {
    let root = PathBuf::from("/project");
    assert_eq!(storage::rpg_dir(&root), PathBuf::from("/project/.rpg"));
    assert_eq!(
        storage::rpg_file(&root),
        PathBuf::from("/project/.rpg/graph.json")
    );
}
