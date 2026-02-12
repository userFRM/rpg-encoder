use rpg_core::graph::*;
use rpg_encoder::evolution::{
    apply_deletions, apply_renames, compute_drift, merge_features, rebuild_hierarchy_from_entities,
};
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

// --- merge_features tests ---

fn make_lifted_entity(id: &str, name: &str, file: &str, features: &[&str]) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Function,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: features.iter().map(|s| s.to_string()).collect(),
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

fn make_module_entity(id: &str, name: &str, file: &str, features: &[&str]) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Module,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 0,
        line_end: 0,
        parent_class: None,
        semantic_features: features.iter().map(|s| s.to_string()).collect(),
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_merge_features_restores_semantic_features() {
    let mut old_graph = RPGraph::new("rust");
    old_graph.insert_entity(make_lifted_entity(
        "a.rs:foo",
        "foo",
        "a.rs",
        &["validate input", "return result"],
    ));

    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_lifted_entity("a.rs:foo", "foo", "a.rs", &[]));

    let stats = merge_features(&mut new_graph, &old_graph);
    assert_eq!(stats.features_restored, 1);
    assert_eq!(
        new_graph.entities["a.rs:foo"].semantic_features,
        vec!["validate input", "return result"]
    );
}

#[test]
fn test_merge_features_restores_hierarchy_path() {
    let mut old_graph = RPGraph::new("rust");
    let mut entity = make_lifted_entity("a.rs:foo", "foo", "a.rs", &["test"]);
    entity.hierarchy_path = "Auth/sessions/validate".to_string();
    old_graph.insert_entity(entity);

    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_lifted_entity("a.rs:foo", "foo", "a.rs", &[]));

    let stats = merge_features(&mut new_graph, &old_graph);
    assert_eq!(stats.hierarchy_restored, 1);
    assert_eq!(
        new_graph.entities["a.rs:foo"].hierarchy_path,
        "Auth/sessions/validate"
    );
}

#[test]
fn test_merge_features_restores_module_features() {
    let mut old_graph = RPGraph::new("rust");
    old_graph.insert_entity(make_module_entity(
        "a.rs:(module)",
        "(module)",
        "a.rs",
        &["manage auth", "validate tokens"],
    ));

    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_module_entity("a.rs:(module)", "(module)", "a.rs", &[]));

    let stats = merge_features(&mut new_graph, &old_graph);
    assert_eq!(stats.modules_restored, 1);
    // Module features should not be counted in features_restored
    assert_eq!(stats.features_restored, 0);
}

#[test]
fn test_merge_features_counts_orphaned_and_new() {
    let mut old_graph = RPGraph::new("rust");
    old_graph.insert_entity(make_lifted_entity("a.rs:foo", "foo", "a.rs", &["test"]));
    old_graph.insert_entity(make_lifted_entity("b.rs:bar", "bar", "b.rs", &["test"]));

    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_lifted_entity("a.rs:foo", "foo", "a.rs", &[]));
    new_graph.insert_entity(make_lifted_entity("c.rs:baz", "baz", "c.rs", &[]));

    let stats = merge_features(&mut new_graph, &old_graph);
    assert_eq!(stats.orphaned, 1); // b.rs:bar was in old but not new
    assert_eq!(stats.new_entities, 1); // c.rs:baz is in new but not old
}

#[test]
fn test_merge_features_no_overwrite() {
    let mut old_graph = RPGraph::new("rust");
    old_graph.insert_entity(make_lifted_entity(
        "a.rs:foo",
        "foo",
        "a.rs",
        &["old feature"],
    ));

    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_lifted_entity(
        "a.rs:foo",
        "foo",
        "a.rs",
        &["new feature"],
    ));

    let stats = merge_features(&mut new_graph, &old_graph);
    // Should not overwrite existing features
    assert_eq!(stats.features_restored, 0);
    assert_eq!(
        new_graph.entities["a.rs:foo"].semantic_features,
        vec!["new feature"]
    );
}

// --- rebuild_hierarchy_from_entities tests ---

#[test]
fn test_rebuild_hierarchy_from_entities() {
    // Simulate the real workflow: old graph has semantic hierarchy,
    // new graph gets structural hierarchy from build_file_path_hierarchy,
    // then merge_features restores hierarchy_paths, then rebuild_hierarchy_from_entities
    // reconstructs the semantic hierarchy tree.

    // Old graph: has semantic hierarchy with lifted features + paths
    let mut old_graph = RPGraph::new("rust");
    let mut old_e1 = make_lifted_entity("a.rs:foo", "foo", "a.rs", &["validate input"]);
    old_e1.hierarchy_path = "Auth/sessions/validate".to_string();
    old_graph.insert_entity(old_e1);
    let mut old_e2 = make_lifted_entity("b.rs:bar", "bar", "b.rs", &["handle login"]);
    old_e2.hierarchy_path = "Auth/sessions/login".to_string();
    old_graph.insert_entity(old_e2);
    old_graph.metadata.semantic_hierarchy = true;

    // New graph: fresh structural build (no features, no semantic paths)
    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_lifted_entity("a.rs:foo", "foo", "a.rs", &[]));
    new_graph.insert_entity(make_lifted_entity("b.rs:bar", "bar", "b.rs", &[]));
    new_graph.build_file_path_hierarchy();
    assert!(!new_graph.metadata.semantic_hierarchy);

    // Merge restores features + hierarchy paths
    let stats = merge_features(&mut new_graph, &old_graph);
    assert_eq!(stats.features_restored, 2);
    assert_eq!(stats.hierarchy_restored, 2);

    // Rebuild semantic hierarchy from restored paths
    rebuild_hierarchy_from_entities(&mut new_graph, true);

    assert!(new_graph.metadata.semantic_hierarchy);
    assert!(new_graph.hierarchy.contains_key("Auth"));
    let auth = &new_graph.hierarchy["Auth"];
    assert!(auth.children.contains_key("sessions"));
    let sessions = &auth.children["sessions"];
    assert!(sessions.children.contains_key("validate"));
    assert!(sessions.children.contains_key("login"));
}

#[test]
fn test_rebuild_hierarchy_noop_without_semantic() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity("a.rs:foo", "foo", "a.rs"));
    graph.build_file_path_hierarchy();

    let hierarchy_before = graph.hierarchy.len();
    rebuild_hierarchy_from_entities(&mut graph, false);
    // Should be unchanged since old graph didn't have semantic hierarchy
    assert_eq!(graph.hierarchy.len(), hierarchy_before);
    assert!(!graph.metadata.semantic_hierarchy);
}

#[test]
fn test_no_semantic_hierarchy_when_zero_paths_restored() {
    // Edge case: old graph had semantic hierarchy but all entities are new
    // (no IDs match), so zero hierarchy paths get restored. The caller should
    // NOT rebuild semantic hierarchy in this case.

    let mut old_graph = RPGraph::new("rust");
    let mut old_entity = make_lifted_entity("old.rs:foo", "foo", "old.rs", &["validate input"]);
    old_entity.hierarchy_path = "Auth/sessions/validate".to_string();
    old_graph.insert_entity(old_entity);
    old_graph.metadata.semantic_hierarchy = true;

    // New graph has completely different entities (no ID overlap)
    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_lifted_entity("new.rs:bar", "bar", "new.rs", &[]));
    new_graph.build_file_path_hierarchy();

    let stats = merge_features(&mut new_graph, &old_graph);
    // No IDs match → nothing restored
    assert_eq!(stats.features_restored, 0);
    assert_eq!(stats.hierarchy_restored, 0);
    assert_eq!(stats.orphaned, 1);
    assert_eq!(stats.new_entities, 1);

    // Caller logic: only rebuild if hierarchy_restored > 0
    // (This mirrors the fix in CLI main.rs and MCP tools.rs)
    let old_had_semantic = old_graph.metadata.semantic_hierarchy;
    if old_had_semantic && stats.hierarchy_restored > 0 {
        rebuild_hierarchy_from_entities(&mut new_graph, true);
    }

    // semantic_hierarchy should NOT be set since no paths were restored
    assert!(!new_graph.metadata.semantic_hierarchy);
}

#[test]
fn test_merge_hierarchy_path_overwrites_structural() {
    // Verify that merge_features correctly overwrites structural paths
    // (from build_file_path_hierarchy) with semantic paths from old graph.

    let mut old_graph = RPGraph::new("rust");
    let mut entity = make_lifted_entity(
        "src/auth.rs:validate",
        "validate",
        "src/auth.rs",
        &["validate tokens"],
    );
    entity.hierarchy_path = "Security/auth/validate".to_string();
    old_graph.insert_entity(entity);

    let mut new_graph = RPGraph::new("rust");
    new_graph.insert_entity(make_lifted_entity(
        "src/auth.rs:validate",
        "validate",
        "src/auth.rs",
        &[],
    ));
    new_graph.build_file_path_hierarchy();

    // After structural build, entity has a file-path hierarchy (e.g., "src/auth/validate")
    let structural_path = new_graph.entities["src/auth.rs:validate"]
        .hierarchy_path
        .clone();
    assert!(!structural_path.is_empty());
    assert_ne!(structural_path, "Security/auth/validate");

    let stats = merge_features(&mut new_graph, &old_graph);
    assert_eq!(stats.hierarchy_restored, 1);
    assert_eq!(
        new_graph.entities["src/auth.rs:validate"].hierarchy_path,
        "Security/auth/validate"
    );
}
