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
        signature: None,
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

#[test]
fn test_signature_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let mut graph = RPGraph::new("rust");
    let mut entity = make_entity("f.rs:compute", "compute", "f.rs");
    entity.signature = Some(Signature {
        parameters: vec![
            Param {
                name: "x".to_string(),
                type_annotation: Some("i32".to_string()),
            },
            Param {
                name: "y".to_string(),
                type_annotation: None,
            },
        ],
        return_type: Some("bool".to_string()),
    });
    graph.insert_entity(entity);
    graph.refresh_metadata();

    storage::save(root, &graph).unwrap();
    let loaded = storage::load(root).unwrap();

    let e = loaded.entities.get("f.rs:compute").unwrap();
    let sig = e
        .signature
        .as_ref()
        .expect("signature should survive roundtrip");
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.parameters[0].name, "x");
    assert_eq!(sig.parameters[0].type_annotation.as_deref(), Some("i32"));
    assert_eq!(sig.parameters[1].name, "y");
    assert!(sig.parameters[1].type_annotation.is_none());
    assert_eq!(sig.return_type.as_deref(), Some("bool"));
}

#[test]
fn test_signature_none_backward_compat() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Save an entity without a signature
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("f.rs:main", "main", "f.rs"));
    graph.refresh_metadata();
    storage::save(root, &graph).unwrap();

    // Load it back — signature should be None (serde(default))
    let loaded = storage::load(root).unwrap();
    let e = loaded.entities.get("f.rs:main").unwrap();
    assert!(e.signature.is_none());
}

#[test]
fn test_data_flow_deps_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let mut graph = RPGraph::new("rust");
    let mut entity = make_entity("f.rs:caller", "caller", "f.rs");
    entity.deps.data_flows_to = vec!["f.rs:callee".to_string()];
    entity.deps.data_flows_from = vec!["f.rs:source".to_string()];
    graph.insert_entity(entity);
    graph.edges.push(DependencyEdge {
        source: "f.rs:caller".to_string(),
        target: "f.rs:callee".to_string(),
        kind: EdgeKind::DataFlow,
    });
    graph.refresh_metadata();

    storage::save(root, &graph).unwrap();
    let loaded = storage::load(root).unwrap();

    let e = loaded.entities.get("f.rs:caller").unwrap();
    assert_eq!(e.deps.data_flows_to, vec!["f.rs:callee".to_string()]);
    assert_eq!(e.deps.data_flows_from, vec!["f.rs:source".to_string()]);
    assert_eq!(loaded.edges[0].kind, EdgeKind::DataFlow);
}
