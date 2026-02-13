//! Integration tests for rpg-cli functionality.
//! Tests the underlying library functions that the CLI commands invoke.

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
        hierarchy_path: "Core/test".to_string(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_storage_load_nonexistent() {
    let tmpdir = tempfile::tempdir().unwrap();
    let result = rpg_core::storage::load(tmpdir.path());
    assert!(result.is_err(), "loading from empty dir should fail");
}

#[test]
fn test_storage_roundtrip() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("main.rs:main", "main", "main.rs"));
    graph.refresh_metadata();

    rpg_core::storage::save(tmpdir.path(), &graph).unwrap();
    assert!(rpg_core::storage::rpg_exists(tmpdir.path()));

    let loaded = rpg_core::storage::load(tmpdir.path()).unwrap();
    assert_eq!(loaded.metadata.total_entities, 1);
    assert!(loaded.entities.contains_key("main.rs:main"));
}

#[test]
fn test_rpg_exists_false() {
    let tmpdir = tempfile::tempdir().unwrap();
    assert!(!rpg_core::storage::rpg_exists(tmpdir.path()));
}

#[test]
fn test_search_on_saved_graph() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("main.rs:main", "main", "main.rs"));
    graph.refresh_metadata();
    rpg_core::storage::save(tmpdir.path(), &graph).unwrap();

    let loaded = rpg_core::storage::load(tmpdir.path()).unwrap();
    let results = rpg_nav::search::search(
        &loaded,
        "main",
        rpg_nav::search::SearchMode::Snippets,
        None,
        10,
    );
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "main");
}

#[test]
fn test_info_displays_metadata() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:foo", "foo", "a.rs"));
    graph.insert_entity(make_entity("b.rs:bar", "bar", "b.rs"));
    graph.refresh_metadata();

    assert_eq!(graph.metadata.total_entities, 2);
    assert_eq!(graph.metadata.total_files, 2);
    assert_eq!(graph.metadata.language, "rust");
}

#[test]
fn test_config_defaults_without_file() {
    let tmpdir = tempfile::tempdir().unwrap();
    let config = rpg_core::config::RpgConfig::load(tmpdir.path()).unwrap();
    assert_eq!(config.navigation.search_result_limit, 10);
    assert_eq!(config.encoding.batch_size, 50);
    assert_eq!(config.encoding.max_batch_tokens, 8000);
}
