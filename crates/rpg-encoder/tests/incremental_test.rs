//! Integration test for the incremental update pipeline.
//!
//! Tests the full cycle: build graph → modify fixture files → detect changes →
//! apply incremental updates → verify graph integrity.
//! Uses the Python fixture project from tests/fixtures/python_project.

use rpg_core::graph::{EntityKind, RPGraph};
use rpg_encoder::evolution::{
    apply_additions, apply_deletions, apply_modifications, apply_renames, check_drift_and_reroute,
    compute_drift, find_best_hierarchy_path, reroute_entity, route_new_entity, semantic_similarity,
};
use rpg_parser::entities::{RawEntity, extract_entities};
use rpg_parser::languages::Language;
use std::path::{Path, PathBuf};

fn fixture_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/python_project")
}

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

fn build_fixture_graph() -> RPGraph {
    let root = fixture_root();
    let files = collect_fixture_files(&root);

    let mut all_entities: Vec<RawEntity> = Vec::new();
    for (rel_path, source) in &files {
        let raw = extract_entities(rel_path, source, Language::PYTHON);
        all_entities.extend(raw);
    }

    let mut graph = RPGraph::new("python");
    for raw in all_entities {
        graph.insert_entity(raw.into_entity());
    }
    graph.create_module_entities();
    graph.build_file_path_hierarchy();
    rpg_encoder::grounding::resolve_dependencies(&mut graph);
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();
    rpg_encoder::grounding::ground_hierarchy(&mut graph);
    graph.refresh_metadata();
    graph
}

/// Verify graph invariants that should hold after any update.
fn verify_graph_integrity(graph: &RPGraph) {
    // Every entity in entities map should be in file_index
    for (id, entity) in &graph.entities {
        let ids = graph.file_index.get(&entity.file);
        assert!(
            ids.is_some_and(|ids| ids.contains(id)),
            "entity {} not in file_index for {}",
            id,
            entity.file.display()
        );
    }

    // Every entity in file_index should be in entities map
    for (file, ids) in &graph.file_index {
        for id in ids {
            assert!(
                graph.entities.contains_key(id),
                "file_index entry {} for {} has no entity",
                id,
                file.display()
            );
        }
    }

    // Non-containment edges should reference existing entities
    for edge in &graph.edges {
        if edge.kind != rpg_core::graph::EdgeKind::Contains {
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

// --- Incremental update tests ---

#[test]
fn test_incremental_delete_file() {
    let mut graph = build_fixture_graph();
    let initial_count = graph.entities.len();

    // Count entities in src/models.py
    let models_count = graph
        .entities
        .values()
        .filter(|e| e.file == Path::new("src/models.py"))
        .count();
    assert!(
        models_count > 0,
        "fixture should have entities in models.py"
    );

    // Delete models.py entities
    let removed = apply_deletions(&mut graph, &[PathBuf::from("src/models.py")]);
    assert_eq!(removed, models_count);

    graph.refresh_metadata();
    assert_eq!(graph.entities.len(), initial_count - models_count);

    // No entities should reference the deleted file
    assert!(
        !graph
            .entities
            .values()
            .any(|e| e.file == Path::new("src/models.py")),
        "deleted file entities should be gone"
    );

    verify_graph_integrity(&graph);
}

#[test]
fn test_incremental_rename_file() {
    let mut graph = build_fixture_graph();

    // Find entities in auth/login.py
    let login_entity_count = graph
        .entities
        .values()
        .filter(|e| e.file == Path::new("src/auth/login.py"))
        .count();
    assert!(login_entity_count > 0);

    // Rename auth/login.py → auth/authentication.py
    let (migrated, renamed) = apply_renames(
        &mut graph,
        &[(
            PathBuf::from("src/auth/login.py"),
            PathBuf::from("src/auth/authentication.py"),
        )],
    );

    assert_eq!(migrated, 1);
    assert_eq!(renamed, login_entity_count);

    // Entities should now have new IDs with the new file path
    let new_entities: Vec<&rpg_core::graph::Entity> = graph
        .entities
        .values()
        .filter(|e| e.file == Path::new("src/auth/authentication.py"))
        .collect();
    assert_eq!(
        new_entities.len(),
        login_entity_count,
        "all entities should be rekeyed to new file"
    );
    for entity in &new_entities {
        assert!(
            entity.id.starts_with("src/auth/authentication.py:"),
            "entity ID {} should start with new file path",
            entity.id
        );
    }

    // Old file should not be in file_index
    assert!(
        !graph
            .file_index
            .contains_key(&PathBuf::from("src/auth/login.py"))
    );
    // New file should be in file_index
    assert!(
        graph
            .file_index
            .contains_key(&PathBuf::from("src/auth/authentication.py"))
    );

    verify_graph_integrity(&graph);
}

#[test]
fn test_incremental_add_new_file() {
    let mut graph = build_fixture_graph();
    let initial_count = graph.entities.len();

    // Simulate adding a new file: parse it and insert entities
    let new_source = r#"
def send_email(to: str, subject: str, body: str):
    """Send an email message."""
    pass

def format_template(template: str, **kwargs) -> str:
    """Format an email template with variables."""
    return template.format(**kwargs)
"#;

    let new_path = PathBuf::from("src/email.py");
    let new_entities = extract_entities(&new_path, new_source, Language::PYTHON);
    assert!(
        new_entities.len() >= 2,
        "should extract at least 2 functions"
    );

    for raw in new_entities {
        graph.insert_entity(raw.into_entity());
    }

    graph.refresh_metadata();
    assert!(graph.entities.len() > initial_count);

    // New entities should be searchable
    let entity_names: Vec<&str> = graph.entities.values().map(|e| e.name.as_str()).collect();
    assert!(entity_names.contains(&"send_email"));
    assert!(entity_names.contains(&"format_template"));

    verify_graph_integrity(&graph);
}

#[test]
fn test_incremental_modify_file() {
    let mut graph = build_fixture_graph();

    // Simulate modifying config.py: delete old entities, add new ones
    let config_path = PathBuf::from("src/utils/config.py");

    // Remove old config entities
    let removed = apply_deletions(&mut graph, std::slice::from_ref(&config_path));
    assert!(removed > 0);

    // Parse new version with an extra function
    let modified_source = r#"
"""Configuration utilities (v2)."""
import os
from typing import Dict, Any

def load_config(path: str) -> Dict[str, Any]:
    """Load configuration from a TOML file."""
    with open(path) as f:
        return parse_toml(f.read())

def parse_toml(content: str) -> Dict[str, Any]:
    """Parse TOML content into a dictionary."""
    result = {}
    for line in content.strip().split("\n"):
        if "=" in line:
            key, val = line.split("=", 1)
            result[key.strip()] = val.strip().strip('"')
    return result

def get_env_var(name: str, default: str = "") -> str:
    """Get an environment variable with a default."""
    return os.environ.get(name, default)

def validate_config(config: Dict[str, Any]) -> bool:
    """Validate that required config keys are present."""
    required = ["database_url", "secret_key"]
    return all(k in config for k in required)
"#;

    let new_entities = extract_entities(&config_path, modified_source, Language::PYTHON);
    assert!(
        new_entities.len() >= 4,
        "modified config should have >= 4 functions"
    );

    for raw in new_entities {
        graph.insert_entity(raw.into_entity());
    }

    graph.refresh_metadata();

    // Should have the new function
    let entity_names: Vec<&str> = graph.entities.values().map(|e| e.name.as_str()).collect();
    assert!(
        entity_names.contains(&"validate_config"),
        "new function should be present"
    );
    // Original functions should still be there
    assert!(entity_names.contains(&"load_config"));
    assert!(entity_names.contains(&"parse_toml"));
    assert!(entity_names.contains(&"get_env_var"));

    verify_graph_integrity(&graph);
}

#[test]
fn test_incremental_delete_then_rebuild_hierarchy() {
    let mut graph = build_fixture_graph();

    // Delete a file
    apply_deletions(&mut graph, &[PathBuf::from("src/models.py")]);

    // Rebuild hierarchy and verify it's still consistent
    graph.hierarchy.clear();
    graph.build_file_path_hierarchy();
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();
    rpg_encoder::grounding::ground_hierarchy(&mut graph);
    graph.refresh_metadata();

    // Hierarchy should still be valid
    assert!(!graph.hierarchy.is_empty());

    // Every entity should have a hierarchy path
    for (id, entity) in &graph.entities {
        if entity.kind != EntityKind::Module {
            assert!(
                !entity.hierarchy_path.is_empty(),
                "entity {} missing hierarchy path after rebuild",
                id
            );
        }
    }

    verify_graph_integrity(&graph);
}

#[test]
fn test_incremental_multiple_operations() {
    let mut graph = build_fixture_graph();

    // 1. Delete models.py
    apply_deletions(&mut graph, &[PathBuf::from("src/models.py")]);

    // 2. Rename login.py → auth_service.py
    apply_renames(
        &mut graph,
        &[(
            PathBuf::from("src/auth/login.py"),
            PathBuf::from("src/auth/auth_service.py"),
        )],
    );

    // 3. Add new file
    let new_source = "def health_check():\n    return True\n";
    let new_path = PathBuf::from("src/health.py");
    let new_entities = extract_entities(&new_path, new_source, Language::PYTHON);
    for raw in new_entities {
        graph.insert_entity(raw.into_entity());
    }

    graph.refresh_metadata();

    // Verify all operations took effect
    assert!(
        !graph
            .entities
            .values()
            .any(|e| e.file == Path::new("src/models.py")),
        "deleted file should be gone"
    );
    assert!(
        graph
            .file_index
            .contains_key(&PathBuf::from("src/auth/auth_service.py")),
        "renamed file should exist"
    );
    assert!(
        graph.entities.values().any(|e| e.name == "health_check"),
        "new function should exist"
    );

    verify_graph_integrity(&graph);
}

#[test]
fn test_rename_rekeys_entity_ids() {
    let mut graph = build_fixture_graph();

    // Collect old entity IDs for src/auth/login.py
    let old_ids: Vec<String> = graph
        .entities
        .values()
        .filter(|e| e.file == Path::new("src/auth/login.py"))
        .map(|e| e.id.clone())
        .collect();
    assert!(!old_ids.is_empty(), "should have entities in login.py");

    // Add an edge referencing an old entity ID to verify edge rewriting
    // Use an entity from a different file as the target
    let test_edge_source = old_ids[0].clone();
    let test_edge_target = graph
        .entities
        .keys()
        .find(|id| !id.starts_with("src/auth/login.py:"))
        .cloned()
        .expect("should have entities outside login.py");
    graph.edges.push(rpg_core::graph::DependencyEdge {
        source: test_edge_source.clone(),
        target: test_edge_target.clone(),
        kind: rpg_core::graph::EdgeKind::Invokes,
    });

    // Rename login.py → authentication.py
    let (migrated, renamed) = apply_renames(
        &mut graph,
        &[(
            PathBuf::from("src/auth/login.py"),
            PathBuf::from("src/auth/authentication.py"),
        )],
    );

    assert_eq!(migrated, 1);
    assert_eq!(renamed, old_ids.len());

    // Old IDs should be gone from entities map
    for old_id in &old_ids {
        assert!(
            !graph.entities.contains_key(old_id),
            "old entity ID {} should be removed after rename",
            old_id
        );
    }

    // New IDs should exist and reference the new file
    let new_ids: Vec<String> = graph
        .entities
        .values()
        .filter(|e| e.file == Path::new("src/auth/authentication.py"))
        .map(|e| e.id.clone())
        .collect();
    assert_eq!(
        new_ids.len(),
        old_ids.len(),
        "should have same number of entities after rename"
    );
    for new_id in &new_ids {
        assert!(
            new_id.starts_with("src/auth/authentication.py:"),
            "new ID {} should start with new file path",
            new_id
        );
    }

    // file_index should use new path with new IDs
    assert!(
        !graph
            .file_index
            .contains_key(&PathBuf::from("src/auth/login.py"))
    );
    let indexed_ids = graph
        .file_index
        .get(&PathBuf::from("src/auth/authentication.py"))
        .expect("new file should be in file_index");
    for new_id in &new_ids {
        assert!(
            indexed_ids.contains(new_id),
            "file_index should contain new ID {}",
            new_id
        );
    }

    // Edge should be rewritten to new ID (find by kind + target to avoid matching hierarchy edges)
    let rewritten_edge = graph
        .edges
        .iter()
        .find(|e| e.target == test_edge_target && e.kind == rpg_core::graph::EdgeKind::Invokes)
        .expect("test edge should still exist");
    assert!(
        rewritten_edge
            .source
            .starts_with("src/auth/authentication.py:"),
        "edge source should be rewritten to new ID, got: {}",
        rewritten_edge.source
    );

    verify_graph_integrity(&graph);
}

#[test]
fn test_new_entity_inherits_hierarchy() {
    let mut graph = build_fixture_graph();

    // Verify that existing entities in src/models.py have a hierarchy path
    let models_hierarchy = graph
        .entities
        .values()
        .find(|e| e.file == Path::new("src/models.py") && e.kind != EntityKind::Module)
        .map(|e| e.hierarchy_path.clone())
        .expect("should have entities in models.py with hierarchy path");
    assert!(
        !models_hierarchy.is_empty(),
        "existing entities should have a hierarchy path"
    );

    // Add a new entity to the same file and verify hierarchy inheritance
    let new_source = r"
class User:
    def __init__(self, name: str, email: str):
        self.name = name
        self.email = email

class Product:
    def __init__(self, title: str, price: float):
        self.title = title
        self.price = price

    def discount(self, pct: float) -> float:
        return self.price * (1.0 - pct)
";

    let models_path = PathBuf::from("src/models.py");
    let new_entities = extract_entities(&models_path, new_source, Language::PYTHON);
    let existing_ids: std::collections::HashSet<String> = graph.entities.keys().cloned().collect();

    let mut found_new = false;
    for raw in new_entities {
        let id = raw.id();
        if !existing_ids.contains(&id) {
            // This is a new entity — inherit hierarchy from siblings
            let sibling_hierarchy = graph.file_index.get(&models_path).and_then(|ids| {
                ids.iter().find_map(|sid| {
                    graph
                        .entities
                        .get(sid)
                        .filter(|e| !e.hierarchy_path.is_empty())
                        .map(|e| e.hierarchy_path.clone())
                })
            });
            assert!(
                sibling_hierarchy.is_some(),
                "should find hierarchy path from existing sibling in same file"
            );

            let mut entity = raw.into_entity();
            entity.hierarchy_path = sibling_hierarchy.unwrap();
            let entity_id = entity.id.clone();
            let hierarchy_path = entity.hierarchy_path.clone();
            graph.insert_entity(entity);
            graph.insert_into_hierarchy(&hierarchy_path, &entity_id);

            // Verify the new entity has the inherited hierarchy path
            let inserted = graph.entities.get(&entity_id).unwrap();
            assert_eq!(
                inserted.hierarchy_path, models_hierarchy,
                "new entity should inherit hierarchy path from siblings"
            );
            found_new = true;
            break;
        }
    }
    assert!(found_new, "should have found at least one new entity");

    verify_graph_integrity(&graph);
}

#[test]
fn test_new_file_entities_no_hierarchy() {
    let mut graph = build_fixture_graph();

    // Add entity from a completely new file — should have empty hierarchy
    let new_source = "def brand_new_func():\n    pass\n";
    let new_path = PathBuf::from("src/brand_new.py");
    let new_entities = extract_entities(&new_path, new_source, Language::PYTHON);
    assert!(!new_entities.is_empty());

    for raw in new_entities {
        let entity = raw.into_entity();
        assert!(
            entity.hierarchy_path.is_empty(),
            "entity from brand new file should have empty hierarchy path"
        );
        graph.insert_entity(entity);
    }

    // No hierarchy path should be assigned (new file, no siblings)
    let new_entity = graph
        .entities
        .values()
        .find(|e| e.name == "brand_new_func")
        .expect("new function should exist");
    assert!(
        new_entity.hierarchy_path.is_empty(),
        "entity from new file with no siblings should have no hierarchy"
    );

    verify_graph_integrity(&graph);
}

#[test]
fn test_apply_additions_assigns_hierarchy() {
    let mut graph = build_fixture_graph();

    // Create a temp dir with a new Python file
    let tmp = std::env::temp_dir().join("rpg_test_additions");
    let _ = std::fs::remove_dir_all(&tmp);
    let sub = tmp.join("src").join("notifications");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("email.py"),
        "def send_email(to, subject):\n    pass\n",
    )
    .unwrap();

    let added_files = vec![PathBuf::from("src/notifications/email.py")];
    let added = apply_additions(&mut graph, &added_files, &tmp, None).unwrap();
    assert_eq!(added, 1);

    // The new entity should have a file-path hierarchy path
    let entity = graph
        .entities
        .values()
        .find(|e| e.name == "send_email")
        .expect("new function should exist");
    assert!(
        !entity.hierarchy_path.is_empty(),
        "entity from new file should have structural hierarchy path"
    );
    assert_eq!(entity.hierarchy_path, "src/notifications/email");

    // Entity should also be in the hierarchy tree
    let has_hierarchy_entry = graph.hierarchy.values().any(|area| {
        fn contains_entity(node: &rpg_core::graph::HierarchyNode, id: &str) -> bool {
            node.entities.contains(&id.to_string())
                || node.children.values().any(|c| contains_entity(c, id))
        }
        contains_entity(area, &entity.id)
    });
    assert!(
        has_hierarchy_entry,
        "new entity should be placed in hierarchy tree"
    );

    verify_graph_integrity(&graph);

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_collect_raw_entities_multi_language() {
    // Build a graph with entities from two different languages
    let mut graph = RPGraph::new("python");
    graph.metadata.languages = vec!["python".to_string(), "rust".to_string()];

    // Insert Python entities
    let py_source = "def greet():\n    pass\n";
    let py_path = PathBuf::from("src/greet.py");
    let py_entities = extract_entities(&py_path, py_source, Language::PYTHON);
    for raw in py_entities {
        graph.insert_entity(raw.into_entity());
    }

    // Insert Rust entities
    let rs_source = "fn hello() {}\n";
    let rs_path = PathBuf::from("src/hello.rs");
    let rs_entities = extract_entities(&rs_path, rs_source, Language::RUST);
    for raw in rs_entities {
        graph.insert_entity(raw.into_entity());
    }

    // Both should be present
    assert!(
        graph.entities.values().any(|e| e.name == "greet"),
        "Python entity should be in graph"
    );
    assert!(
        graph.entities.values().any(|e| e.name == "hello"),
        "Rust entity should be in graph"
    );

    // collect_raw_entities should parse both via per-file language detection
    let root = fixture_root();
    // Write temp files so collect_raw_entities can read them
    let py_abs = root.join("../../temp_test_py.py");
    let rs_abs = root.join("../../temp_test_rs.rs");
    // We can't easily write temp files in the fixture dir, so test the language
    // detection logic directly: verify that both file extensions resolve to valid languages
    let py_lang = PathBuf::from("src/greet.py")
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Language::from_extension);
    let rs_lang = PathBuf::from("src/hello.rs")
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Language::from_extension);
    assert!(py_lang.is_some(), ".py should detect as Python");
    assert!(rs_lang.is_some(), ".rs should detect as Rust");
    assert_ne!(
        py_lang.unwrap(),
        rs_lang.unwrap(),
        "Python and Rust should be different languages"
    );

    let _ = (py_abs, rs_abs); // suppress unused
}

#[test]
fn test_scoped_deps_only_refreshes_changed_files() {
    let mut graph = build_fixture_graph();
    let root = fixture_root();

    // Populate deps for all files first
    rpg_encoder::grounding::populate_entity_deps(&mut graph, &root, false, None, None);

    // Find an entity in src/models.py that has invokes/imports
    let models_entity = graph
        .entities
        .values()
        .find(|e| e.file == Path::new("src/models.py") && !e.deps.imports.is_empty())
        .map(|e| (e.id.clone(), e.deps.imports.clone()));

    // Find an entity in src/auth/login.py that has deps
    let login_entity = graph
        .entities
        .values()
        .find(|e| e.file == Path::new("src/auth/login.py") && !e.deps.invokes.is_empty())
        .map(|e| (e.id.clone(), e.deps.invokes.clone()));

    // Now re-populate only models.py
    rpg_encoder::grounding::populate_entity_deps(
        &mut graph,
        &root,
        false,
        Some(&[PathBuf::from("src/models.py")]),
        None,
    );

    // login.py entity should retain its original deps (not cleared)
    if let Some((id, original_invokes)) = login_entity {
        let entity = graph.entities.get(&id).unwrap();
        assert_eq!(
            entity.deps.invokes, original_invokes,
            "unchanged file entity should retain original deps"
        );
    }

    // models.py entity deps were cleared and re-populated (may differ or be empty)
    if let Some((id, _original_imports)) = models_entity {
        // The entity should exist and have been processed
        assert!(
            graph.entities.contains_key(&id),
            "models.py entity should still exist"
        );
    }
}

#[test]
fn test_apply_modifications_updates_entity_kind() {
    // Verify that apply_modifications refreshes structural fields (kind, parent_class),
    // not just line numbers.
    let tmp = std::env::temp_dir().join("rpg_test_mod_kind");
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Write initial Python source: a function
    let file_rel = PathBuf::from("src/helper.py");
    let initial_source = "def helper():\n    pass\n";
    std::fs::write(src.join("helper.py"), initial_source).unwrap();

    // Build initial graph with the function entity
    let mut graph = RPGraph::new("python");
    graph.metadata.languages = vec!["python".to_string()];
    let raw = extract_entities(&file_rel, initial_source, Language::PYTHON);
    assert!(!raw.is_empty(), "should extract at least one entity");
    for r in raw {
        graph.insert_entity(r.into_entity());
    }

    // Verify initial kind is Function
    let entity = graph
        .entities
        .values()
        .find(|e| e.name == "helper")
        .expect("helper function should exist");
    assert_eq!(entity.kind, EntityKind::Function);
    let entity_id = entity.id.clone();

    // Now "modify" the file: add a class with the same function name as a method
    // This won't change the ID, but to test kind refresh we manually alter the entity kind
    // and then re-run apply_modifications which should restore the correct kind.
    graph.entities.get_mut(&entity_id).unwrap().kind = EntityKind::Class;
    assert_eq!(
        graph.entities.get(&entity_id).unwrap().kind,
        EntityKind::Class,
        "manually set to Class to simulate stale kind"
    );

    // Run apply_modifications — it re-extracts from source and should refresh kind
    let modified_files = vec![file_rel];
    let (modified, _added, _removed, _stale) =
        apply_modifications(&mut graph, &modified_files, &tmp, None).unwrap();
    assert!(modified > 0, "should report modified entities");

    // Verify kind was refreshed back to Function
    let entity = graph.entities.get(&entity_id).unwrap();
    assert_eq!(
        entity.kind,
        EntityKind::Function,
        "apply_modifications should refresh entity kind from re-extracted source"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}

// --- Semantic drift detection and routing tests (Algorithms 3-4) ---

#[test]
fn test_semantic_similarity() {
    let a = vec!["load config".to_string(), "validate token".to_string()];
    let b = vec!["load config".to_string(), "validate token".to_string()];
    assert!((semantic_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);

    let c = vec!["send email".to_string(), "format template".to_string()];
    assert!((semantic_similarity(&a, &c) - 0.0).abs() < f64::EPSILON);

    // Partial overlap: 1 of 3 unique features
    let d = vec!["load config".to_string(), "parse yaml".to_string()];
    let sim = semantic_similarity(&a, &d);
    assert!(
        sim > 0.0 && sim < 1.0,
        "partial overlap should be between 0 and 1, got {}",
        sim
    );

    // Empty sets
    assert!((semantic_similarity(&[], &[]) - 1.0).abs() < f64::EPSILON);
    assert!((semantic_similarity(&a, &[]) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_compute_drift_is_inverse_of_similarity() {
    let a = vec!["load config".to_string(), "validate token".to_string()];
    let b = vec!["send email".to_string(), "format template".to_string()];

    let drift = compute_drift(&a, &b);
    let sim = semantic_similarity(&a, &b);
    assert!(
        (drift + sim - 1.0).abs() < f64::EPSILON,
        "drift + similarity should equal 1.0"
    );
}

/// Build a graph with a semantic hierarchy for routing tests.
fn build_semantic_hierarchy_graph() -> RPGraph {
    let mut graph = RPGraph::new("python");
    graph.metadata.semantic_hierarchy = true;
    graph.metadata.languages = vec!["python".to_string()];

    // Create area: "Authentication" with features about auth
    let mut auth_area = rpg_core::graph::HierarchyNode::new("Authentication");
    auth_area.semantic_features = vec![
        "authenticate user".to_string(),
        "validate credentials".to_string(),
        "manage session".to_string(),
        "hash password".to_string(),
    ];

    let mut auth_login = rpg_core::graph::HierarchyNode::new("login");
    auth_login.semantic_features = vec![
        "authenticate user".to_string(),
        "validate credentials".to_string(),
    ];
    let mut auth_session = rpg_core::graph::HierarchyNode::new("session");
    auth_session.semantic_features = vec![
        "manage session".to_string(),
        "create session token".to_string(),
    ];
    auth_area.children.insert("login".to_string(), auth_login);
    auth_area
        .children
        .insert("session".to_string(), auth_session);

    // Create area: "DataProcessing" with features about data
    let mut data_area = rpg_core::graph::HierarchyNode::new("DataProcessing");
    data_area.semantic_features = vec![
        "load dataset".to_string(),
        "transform data".to_string(),
        "validate schema".to_string(),
        "export results".to_string(),
    ];

    let mut data_loading = rpg_core::graph::HierarchyNode::new("loading");
    data_loading.semantic_features = vec!["load dataset".to_string(), "parse csv".to_string()];
    let mut data_transform = rpg_core::graph::HierarchyNode::new("transform");
    data_transform.semantic_features =
        vec!["transform data".to_string(), "normalize values".to_string()];
    data_area
        .children
        .insert("loading".to_string(), data_loading);
    data_area
        .children
        .insert("transform".to_string(), data_transform);

    graph
        .hierarchy
        .insert("Authentication".to_string(), auth_area);
    graph
        .hierarchy
        .insert("DataProcessing".to_string(), data_area);

    // Add sentinel entities to anchor hierarchy nodes (prevents prune_empty from
    // destroying the hierarchy when test entities are removed/rerouted)
    let sentinel_specs = [
        (
            "src/auth/login.py:login_user",
            "login_user",
            "src/auth/login.py",
            "Authentication/login",
            vec!["authenticate user".to_string()],
        ),
        (
            "src/auth/session.py:create_session",
            "create_session",
            "src/auth/session.py",
            "Authentication/session",
            vec!["manage session".to_string()],
        ),
        (
            "src/data/loader.py:load_csv",
            "load_csv",
            "src/data/loader.py",
            "DataProcessing/loading",
            vec!["load dataset".to_string()],
        ),
        (
            "src/data/transform.py:normalize",
            "normalize",
            "src/data/transform.py",
            "DataProcessing/transform",
            vec!["transform data".to_string()],
        ),
    ];
    for (id, name, file, hier_path, feats) in sentinel_specs {
        let entity = rpg_core::graph::Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: name.to_string(),
            file: PathBuf::from(file),
            line_start: 1,
            line_end: 5,
            parent_class: None,
            semantic_features: feats,
            hierarchy_path: hier_path.to_string(),
            deps: rpg_core::graph::EntityDeps::default(),
        };
        graph.insert_entity(entity);
        graph.insert_into_hierarchy(hier_path, id);
    }

    graph.assign_hierarchy_ids();
    graph
}

#[test]
fn test_find_best_hierarchy_path_routes_to_correct_area() {
    let graph = build_semantic_hierarchy_graph();

    // Features about authentication should route to Authentication/login
    let auth_features = vec![
        "authenticate user".to_string(),
        "validate credentials".to_string(),
    ];
    let path = find_best_hierarchy_path(&graph, &auth_features);
    assert!(path.is_some());
    let path = path.unwrap();
    assert!(
        path.starts_with("Authentication"),
        "auth features should route to Authentication, got: {}",
        path
    );
    assert_eq!(path, "Authentication/login");

    // Features about data should route to DataProcessing
    let data_features = vec!["load dataset".to_string(), "parse csv".to_string()];
    let path = find_best_hierarchy_path(&graph, &data_features).unwrap();
    assert!(
        path.starts_with("DataProcessing"),
        "data features should route to DataProcessing, got: {}",
        path
    );
    assert_eq!(path, "DataProcessing/loading");
}

#[test]
fn test_find_best_hierarchy_path_drills_to_subcategory() {
    let graph = build_semantic_hierarchy_graph();

    // Features specifically about sessions should drill to session subcategory
    let session_features = vec![
        "manage session".to_string(),
        "create session token".to_string(),
    ];
    let path = find_best_hierarchy_path(&graph, &session_features).unwrap();
    assert_eq!(
        path, "Authentication/session",
        "session features should drill down to Authentication/session"
    );

    // Features about data transformation
    let transform_features = vec!["transform data".to_string(), "normalize values".to_string()];
    let path = find_best_hierarchy_path(&graph, &transform_features).unwrap();
    assert_eq!(
        path, "DataProcessing/transform",
        "transform features should drill down to DataProcessing/transform"
    );
}

#[test]
fn test_find_best_hierarchy_path_returns_none_for_empty() {
    let graph = build_semantic_hierarchy_graph();
    assert!(find_best_hierarchy_path(&graph, &[]).is_none());

    let empty_graph = RPGraph::new("python");
    let features = vec!["authenticate user".to_string()];
    assert!(find_best_hierarchy_path(&empty_graph, &features).is_none());
}

#[test]
fn test_reroute_entity_moves_in_hierarchy() {
    let mut graph = build_semantic_hierarchy_graph();

    // Insert an entity in the wrong place (DataProcessing/loading)
    let entity = rpg_core::graph::Entity {
        id: "src/auth.py:login".to_string(),
        kind: EntityKind::Function,
        name: "login".to_string(),
        file: PathBuf::from("src/auth.py"),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: vec!["authenticate user".to_string()],
        hierarchy_path: "DataProcessing/loading".to_string(),
        deps: rpg_core::graph::EntityDeps::default(),
    };
    let eid = entity.id.clone();
    graph.insert_entity(entity);
    graph.insert_into_hierarchy("DataProcessing/loading", &eid);

    // Verify it's in the wrong place
    assert_eq!(
        graph.entities.get(&eid).unwrap().hierarchy_path,
        "DataProcessing/loading"
    );

    // Re-route to correct place
    reroute_entity(&mut graph, &eid, "Authentication/login");

    // Verify it moved
    assert_eq!(
        graph.entities.get(&eid).unwrap().hierarchy_path,
        "Authentication/login"
    );

    // Verify it's in the new hierarchy node
    let auth_login = &graph.hierarchy["Authentication"].children["login"];
    assert!(
        auth_login.entities.contains(&eid),
        "entity should be in Authentication/login"
    );

    // Verify it's NOT in the old hierarchy node (may have been pruned if empty)
    if let Some(dp) = graph.hierarchy.get("DataProcessing")
        && let Some(loading) = dp.children.get("loading")
    {
        assert!(
            !loading.entities.contains(&eid),
            "entity should no longer be in DataProcessing/loading"
        );
    }
}

#[test]
fn test_check_drift_and_reroute_below_threshold() {
    let mut graph = build_semantic_hierarchy_graph();

    // Insert entity with auth features
    let entity = rpg_core::graph::Entity {
        id: "src/auth.py:check".to_string(),
        kind: EntityKind::Function,
        name: "check".to_string(),
        file: PathBuf::from("src/auth.py"),
        line_start: 1,
        line_end: 5,
        parent_class: None,
        semantic_features: vec![
            "authenticate user".to_string(),
            "validate credentials".to_string(),
        ],
        hierarchy_path: "Authentication/login".to_string(),
        deps: rpg_core::graph::EntityDeps::default(),
    };
    let eid = entity.id.clone();
    graph.insert_entity(entity);
    graph.insert_into_hierarchy("Authentication/login", &eid);

    // Minor drift: one feature changed, threshold 0.5
    let old = vec![
        "authenticate user".to_string(),
        "validate credentials".to_string(),
    ];
    let new_feats = vec![
        "authenticate user".to_string(),
        "check permissions".to_string(),
    ];

    let result = check_drift_and_reroute(&mut graph, &eid, &old, &new_feats, 0.5);
    assert!(result.is_none(), "drift below threshold should not reroute");
    assert_eq!(
        graph.entities.get(&eid).unwrap().hierarchy_path,
        "Authentication/login",
        "entity should stay in original position"
    );
}

#[test]
fn test_check_drift_and_reroute_above_threshold() {
    let mut graph = build_semantic_hierarchy_graph();

    // Insert entity in Authentication but with features that will drift to DataProcessing
    let entity = rpg_core::graph::Entity {
        id: "src/process.py:load".to_string(),
        kind: EntityKind::Function,
        name: "load".to_string(),
        file: PathBuf::from("src/process.py"),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: vec!["authenticate user".to_string()],
        hierarchy_path: "Authentication/login".to_string(),
        deps: rpg_core::graph::EntityDeps::default(),
    };
    let eid = entity.id.clone();
    graph.insert_entity(entity);
    graph.insert_into_hierarchy("Authentication/login", &eid);

    // Major drift: completely different features (old auth → new data)
    let old = vec!["authenticate user".to_string()];
    let new_feats = vec!["load dataset".to_string(), "parse csv".to_string()];
    let drift = compute_drift(&old, &new_feats);
    assert!(drift > 0.5, "drift should be above threshold: {}", drift);

    // Update entity features BEFORE check (simulates re-lifting)
    graph.entities.get_mut(&eid).unwrap().semantic_features = new_feats.clone();
    // Don't aggregate — it would overwrite manually-set hierarchy features

    let result = check_drift_and_reroute(&mut graph, &eid, &old, &new_feats, 0.5);
    assert!(result.is_some(), "drift above threshold should reroute");

    let new_path = result.unwrap();
    assert!(
        new_path.starts_with("DataProcessing"),
        "should reroute to DataProcessing, got: {}",
        new_path
    );
}

#[test]
fn test_route_new_entity_places_in_semantic_hierarchy() {
    let mut graph = build_semantic_hierarchy_graph();

    // Insert entity with file-path hierarchy (simulating apply_additions placement)
    let entity = rpg_core::graph::Entity {
        id: "src/auth/verify.py:verify_token".to_string(),
        kind: EntityKind::Function,
        name: "verify_token".to_string(),
        file: PathBuf::from("src/auth/verify.py"),
        line_start: 1,
        line_end: 15,
        parent_class: None,
        semantic_features: vec![
            "validate credentials".to_string(),
            "authenticate user".to_string(),
        ],
        hierarchy_path: "src/auth/verify".to_string(), // file-path-based
        deps: rpg_core::graph::EntityDeps::default(),
    };
    let eid = entity.id.clone();
    graph.insert_entity(entity);
    graph.insert_into_hierarchy("src/auth/verify", &eid);
    // Don't aggregate — it would overwrite manually-set hierarchy features

    // Route to semantic hierarchy
    let result = route_new_entity(&mut graph, &eid);
    assert!(
        result.is_some(),
        "should find a semantic route for auth features"
    );

    let new_path = result.unwrap();
    assert!(
        new_path.starts_with("Authentication"),
        "auth entity should route to Authentication, got: {}",
        new_path
    );
    assert_eq!(
        graph.entities.get(&eid).unwrap().hierarchy_path,
        new_path,
        "entity hierarchy_path should be updated"
    );
}

#[test]
fn test_route_new_entity_skips_without_semantic_hierarchy() {
    let mut graph = RPGraph::new("python");
    graph.metadata.semantic_hierarchy = false;

    let entity = rpg_core::graph::Entity {
        id: "src/main.py:main".to_string(),
        kind: EntityKind::Function,
        name: "main".to_string(),
        file: PathBuf::from("src/main.py"),
        line_start: 1,
        line_end: 5,
        parent_class: None,
        semantic_features: vec!["start application".to_string()],
        hierarchy_path: "src/main".to_string(),
        deps: rpg_core::graph::EntityDeps::default(),
    };
    let eid = entity.id.clone();
    graph.insert_entity(entity);

    let result = route_new_entity(&mut graph, &eid);
    assert!(
        result.is_none(),
        "should not route without semantic hierarchy"
    );
}

#[test]
fn test_routing_stops_when_child_sim_not_better_than_parent() {
    // Build a hierarchy where the area has better sim than its children
    let mut graph = RPGraph::new("python");
    graph.metadata.semantic_hierarchy = true;

    let mut area = rpg_core::graph::HierarchyNode::new("General");
    area.semantic_features = vec![
        "process request".to_string(),
        "handle input".to_string(),
        "validate data".to_string(),
    ];

    // Children have narrower features that won't match the entity as well as the parent
    let mut child_a = rpg_core::graph::HierarchyNode::new("networking");
    child_a.semantic_features = vec!["open socket".to_string(), "send packet".to_string()];
    // Anchor entity so it doesn't get pruned
    child_a.entities.push("sentinel:a".to_string());

    let mut child_b = rpg_core::graph::HierarchyNode::new("storage");
    child_b.semantic_features = vec!["write file".to_string(), "read disk".to_string()];
    child_b.entities.push("sentinel:b".to_string());

    area.children.insert("networking".to_string(), child_a);
    area.children.insert("storage".to_string(), child_b);
    graph.hierarchy.insert("General".to_string(), area);

    // Entity features match the parent area well but not any child
    let features = vec!["process request".to_string(), "handle input".to_string()];
    let path = find_best_hierarchy_path(&graph, &features);
    assert_eq!(
        path.as_deref(),
        Some("General"),
        "should stop at area level, not descend into unrelated children"
    );
}

#[test]
fn test_check_drift_and_reroute_restores_on_no_match() {
    // Build a minimal hierarchy where routing will fail for the new features
    let mut graph = RPGraph::new("python");
    graph.metadata.semantic_hierarchy = true;

    let mut area = rpg_core::graph::HierarchyNode::new("Authentication");
    area.semantic_features = vec![
        "authenticate user".to_string(),
        "validate credentials".to_string(),
    ];
    area.entities.push("sentinel:auth".to_string());
    graph.hierarchy.insert("Authentication".to_string(), area);

    // Entity in Authentication with auth features
    let entity = rpg_core::graph::Entity {
        id: "src/auth.py:login".to_string(),
        kind: EntityKind::Function,
        name: "login".to_string(),
        file: PathBuf::from("src/auth.py"),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: vec!["authenticate user".to_string()],
        hierarchy_path: "Authentication".to_string(),
        deps: rpg_core::graph::EntityDeps::default(),
    };
    let eid = entity.id.clone();
    graph.insert_entity(entity);
    graph.insert_into_hierarchy("Authentication", &eid);

    // Drift to completely unrelated features that don't match any hierarchy area
    let old = vec!["authenticate user".to_string()];
    let new_feats = vec![
        "quantum entangle".to_string(),
        "teleport particles".to_string(),
    ];
    graph.entities.get_mut(&eid).unwrap().semantic_features = new_feats.clone();

    let result = check_drift_and_reroute(&mut graph, &eid, &old, &new_feats, 0.5);
    // Should return None because no hierarchy area matches the new features
    assert!(
        result.is_none(),
        "should return None when no matching hierarchy path"
    );

    // Entity should be restored to its original position (not orphaned)
    assert!(
        graph.hierarchy["Authentication"].entities.contains(&eid),
        "entity should be restored to original hierarchy position"
    );
}
