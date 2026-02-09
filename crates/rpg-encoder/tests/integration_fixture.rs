//! Integration test: build an RPG from a real Python fixture project.
//!
//! Validates entity extraction, dependency resolution, hierarchy construction,
//! and graph integrity on a small but realistic multi-file project.

use rpg_core::graph::{EntityKind, RPGraph};
use rpg_parser::entities::{RawEntity, extract_entities};
use rpg_parser::languages::Language;
use std::path::{Path, PathBuf};

/// Path to the fixture project relative to the workspace root.
fn fixture_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/python_project")
}

/// Collect all .py files from the fixture directory (excluding __init__.py).
fn collect_fixture_files(root: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect_recursive(root, root, &mut files);
    files
}

fn collect_recursive(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(base, &path, out);
        } else if path.extension().is_some_and(|e| e == "py") {
            let rel = path.strip_prefix(base).unwrap().to_path_buf();
            let source = std::fs::read_to_string(&path).unwrap();
            out.push((rel, source));
        }
    }
}

/// Build a complete structural RPG from the fixture project.
fn build_fixture_graph() -> RPGraph {
    let root = fixture_root();
    let files = collect_fixture_files(&root);
    assert!(
        files.len() >= 4,
        "expected at least 4 .py files, got {}",
        files.len()
    );

    // Parse all files
    let mut all_entities: Vec<RawEntity> = Vec::new();
    for (rel_path, source) in &files {
        let raw = extract_entities(rel_path, source, Language::PYTHON);
        all_entities.extend(raw);
    }

    // Build the graph — RawEntity::into_entity() creates Entity with default deps
    let mut graph = RPGraph::new("python");
    for raw in all_entities {
        graph.insert_entity(raw.into_entity());
    }

    // Create file-level Module entities
    graph.create_module_entities();

    // Build structural hierarchy
    graph.build_file_path_hierarchy();

    // Resolve dependencies
    rpg_encoder::grounding::resolve_dependencies(&mut graph);

    // Hierarchy enrichment
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();
    rpg_encoder::grounding::ground_hierarchy(&mut graph);
    graph.refresh_metadata();

    graph
}

#[test]
fn test_fixture_entity_extraction() {
    let graph = build_fixture_graph();

    // Should have entities from all fixture files
    let entity_names: Vec<&str> = graph.entities.values().map(|e| e.name.as_str()).collect();

    // From main.py
    assert!(
        entity_names.contains(&"main"),
        "missing main() from main.py"
    );
    assert!(
        entity_names.contains(&"parse_args"),
        "missing parse_args() from main.py"
    );

    // From auth/login.py
    assert!(
        entity_names.contains(&"User"),
        "missing User class from login.py"
    );
    assert!(
        entity_names.contains(&"authenticate"),
        "missing authenticate() from login.py"
    );
    assert!(
        entity_names.contains(&"validate_token"),
        "missing validate_token() from login.py"
    );

    // From utils/config.py
    assert!(
        entity_names.contains(&"load_config"),
        "missing load_config() from config.py"
    );
    assert!(
        entity_names.contains(&"parse_toml"),
        "missing parse_toml() from config.py"
    );

    // From models.py
    assert!(
        entity_names.contains(&"Task"),
        "missing Task class from models.py"
    );
    assert!(
        entity_names.contains(&"TaskList"),
        "missing TaskList class from models.py"
    );
}

#[test]
fn test_fixture_entity_counts() {
    let graph = build_fixture_graph();

    // Count by kind (excluding Module entities)
    let functions = graph
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::Function)
        .count();
    let classes = graph
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::Class)
        .count();
    let methods = graph
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::Method)
        .count();
    let modules = graph
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::Module)
        .count();

    assert!(functions >= 5, "expected >= 5 functions, got {}", functions);
    assert!(classes >= 3, "expected >= 3 classes, got {}", classes);
    assert!(methods >= 3, "expected >= 3 methods, got {}", methods);

    // Module count should match unique source files with entities
    // (__init__.py files with no code produce no Module entities)
    assert!(
        modules >= 3,
        "expected >= 3 Module entities, got {}",
        modules,
    );
}

#[test]
fn test_fixture_hierarchy_structure() {
    let graph = build_fixture_graph();

    // Structural hierarchy should exist
    assert!(
        !graph.hierarchy.is_empty(),
        "hierarchy should not be empty after structural build"
    );
    assert!(
        !graph.metadata.semantic_hierarchy,
        "should be structural (not semantic) hierarchy"
    );

    // Every non-Module entity should have a hierarchy path
    for (id, entity) in &graph.entities {
        if entity.kind != EntityKind::Module {
            assert!(
                !entity.hierarchy_path.is_empty(),
                "entity {} has empty hierarchy_path",
                id
            );
        }
    }
}

#[test]
fn test_fixture_no_dangling_edges() {
    let graph = build_fixture_graph();

    // Non-containment edges should reference entity IDs that exist.
    // Containment edges (EdgeKind::Contains) use hierarchy node IDs as sources
    // which live in the hierarchy tree, not in graph.entities.
    for edge in &graph.edges {
        if edge.kind == rpg_core::graph::EdgeKind::Contains {
            // Containment edge: source is hierarchy node ID, target is entity or hierarchy node
            assert!(
                edge.source.starts_with("h:") || graph.entities.contains_key(&edge.source),
                "containment edge source is neither hierarchy node nor entity: {}",
                edge.source
            );
            assert!(
                edge.target.starts_with("h:") || graph.entities.contains_key(&edge.target),
                "containment edge target is neither hierarchy node nor entity: {}",
                edge.target
            );
        } else {
            assert!(
                graph.entities.contains_key(&edge.source),
                "dangling edge source: {}",
                edge.source
            );
            assert!(
                graph.entities.contains_key(&edge.target),
                "dangling edge target: {}",
                edge.target
            );
        }
    }
}

#[test]
fn test_fixture_file_index_consistency() {
    let graph = build_fixture_graph();

    // Every entity should be in the file index
    for (id, entity) in &graph.entities {
        let ids = graph.file_index.get(&entity.file);
        assert!(
            ids.is_some_and(|ids| ids.contains(id)),
            "entity {} not found in file_index for {}",
            id,
            entity.file.display()
        );
    }
}

#[test]
fn test_fixture_metadata_consistency() {
    let graph = build_fixture_graph();

    assert_eq!(graph.metadata.language, "python");
    assert_eq!(graph.metadata.total_entities, graph.entities.len());
    assert_eq!(graph.metadata.total_files, graph.file_index.len());
    assert!(
        graph.metadata.functional_areas > 0,
        "should have at least 1 functional area"
    );
}

#[test]
fn test_fixture_storage_roundtrip() {
    let graph = build_fixture_graph();
    let tmpdir = tempfile::tempdir().unwrap();

    // Save
    rpg_core::storage::save(tmpdir.path(), &graph).unwrap();
    assert!(rpg_core::storage::rpg_exists(tmpdir.path()));

    // Load
    let loaded = rpg_core::storage::load(tmpdir.path()).unwrap();

    // Verify
    assert_eq!(loaded.entities.len(), graph.entities.len());
    assert_eq!(loaded.edges.len(), graph.edges.len());
    assert_eq!(loaded.metadata.language, "python");
    assert_eq!(
        loaded.metadata.total_entities,
        graph.metadata.total_entities
    );

    // Edge indexes should be rebuilt on load
    let entity_ids: Vec<&String> = loaded.entities.keys().collect();
    if !entity_ids.is_empty() {
        let edges = loaded.edges_for(entity_ids[0]);
        // Just verify the method doesn't panic
        let _ = edges;
    }
}

#[test]
fn test_fixture_search_integration() {
    let graph = build_fixture_graph();

    // Search for auth-related entities
    let results = rpg_nav::search::search(
        &graph,
        "authenticate",
        rpg_nav::search::SearchMode::Snippets,
        None,
        10,
    );

    assert!(
        !results.is_empty(),
        "search for 'authenticate' should return results"
    );
    assert_eq!(results[0].entity_name, "authenticate");
}
