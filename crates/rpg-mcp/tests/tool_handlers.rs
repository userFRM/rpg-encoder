use rpg_core::graph::*;
use rpg_core::storage;
use rpg_nav::explore::{Direction, explore, format_tree};
use rpg_nav::fetch::{FetchOutput, fetch};
use rpg_nav::search::{SearchMode, search};
use rpg_nav::toon;
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_entity(id: &str, name: &str, file: &str, features: Vec<&str>, hierarchy: &str) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Function,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: features.into_iter().map(String::from).collect(),
        hierarchy_path: hierarchy.to_string(),
        deps: EntityDeps::default(),
        embedding: None,
    }
}

fn make_test_graph() -> RPGraph {
    let mut graph = RPGraph::new("rust");

    let e1 = make_entity(
        "main.rs:main",
        "main",
        "main.rs",
        vec!["entry point", "application startup"],
        "Core/entry",
    );
    let e2 = make_entity(
        "lib.rs:process",
        "process",
        "lib.rs",
        vec!["data processing", "transform pipeline"],
        "Core/processing",
    );
    let e3 = make_entity(
        "lib.rs:validate",
        "validate",
        "lib.rs",
        vec!["input validation", "error handling"],
        "Core/processing",
    );
    let e4 = make_entity(
        "utils.rs:helper",
        "helper",
        "utils.rs",
        vec!["utility function", "string formatting"],
        "Utils/helpers",
    );

    graph.insert_entity(e1);
    graph.insert_entity(e2);
    graph.insert_entity(e3);
    graph.insert_entity(e4);

    graph.insert_into_hierarchy("Core/entry", "main.rs:main");
    graph.insert_into_hierarchy("Core/processing", "lib.rs:process");
    graph.insert_into_hierarchy("Core/processing", "lib.rs:validate");
    graph.insert_into_hierarchy("Utils/helpers", "utils.rs:helper");

    graph.edges.push(DependencyEdge {
        source: "main.rs:main".to_string(),
        target: "lib.rs:process".to_string(),
        kind: EdgeKind::Invokes,
    });
    graph.edges.push(DependencyEdge {
        source: "lib.rs:process".to_string(),
        target: "lib.rs:validate".to_string(),
        kind: EdgeKind::Invokes,
    });

    graph.refresh_metadata();
    graph
}

fn make_temp_project(graph: &RPGraph) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // Write source files that entities reference
    std::fs::write(
        root.join("main.rs"),
        "fn main() {\n    let data = read_input();\n    process(data);\n}\n\n\n\n\n\n\n",
    )
    .unwrap();
    std::fs::write(
        root.join("lib.rs"),
        "fn process(data: &str) {\n    validate(data);\n    transform(data);\n}\n\nfn validate(data: &str) {\n    if data.is_empty() {\n        panic!(\"empty\");\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("utils.rs"),
        "fn helper(s: &str) -> String {\n    s.trim().to_uppercase()\n}\n\n\n\n\n\n\n\n",
    )
    .unwrap();

    // Save the graph into .rpg/
    storage::save(&root, graph).unwrap();

    (tmp, root)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_search_node() {
    let graph = make_test_graph();

    // Search by feature keyword "processing"
    let results = search(&graph, "processing", SearchMode::Features, None, 10);
    assert!(!results.is_empty(), "search should find matching entities");

    // The "process" entity has "data processing" as a feature
    let names: Vec<&str> = results.iter().map(|r| r.entity_name.as_str()).collect();
    assert!(
        names.contains(&"process"),
        "should find 'process' entity; got: {:?}",
        names
    );

    // Format as TOON and verify output structure
    let toon_output = toon::format_search_results(&results);
    assert!(
        toon_output.contains("process"),
        "TOON output should contain entity name 'process'"
    );
    assert!(
        toon_output.contains("results["),
        "TOON output should have results header"
    );

    // Also test snippets mode with entity name match
    let snippet_results = search(&graph, "validate", SearchMode::Snippets, None, 10);
    assert!(
        !snippet_results.is_empty(),
        "snippets search should match entity name"
    );
    assert_eq!(snippet_results[0].entity_name, "validate");
}

#[test]
fn test_fetch_node() {
    let graph = make_test_graph();
    let (_tmp, root) = make_temp_project(&graph);

    // Fetch the "main" entity
    let output = fetch(&graph, "main.rs:main", &root).unwrap();
    let result = match &output {
        FetchOutput::Entity(r) => r,
        _ => panic!("expected Entity result"),
    };
    assert_eq!(result.entity.name, "main");
    assert_eq!(result.entity.kind, EntityKind::Function);
    assert!(
        result.source_code.is_some(),
        "source code should be read from disk"
    );
    let source = result.source_code.as_ref().unwrap();
    assert!(
        source.contains("fn main()"),
        "source should contain function definition"
    );

    // Format as TOON and verify output structure
    let toon_output = toon::format_fetch_result(result);
    assert!(
        toon_output.contains("name: main"),
        "TOON output should contain 'name: main'"
    );
    assert!(
        toon_output.contains("kind: function"),
        "TOON output should contain 'kind: function'"
    );
    assert!(
        toon_output.contains("file: main.rs"),
        "TOON output should contain 'file: main.rs'"
    );
    assert!(
        toon_output.contains("source:"),
        "TOON output should contain 'source:' section"
    );
    assert!(
        toon_output.contains("entry point"),
        "TOON output should contain semantic feature"
    );

    // Fetch entity with siblings
    let output2 = fetch(&graph, "lib.rs:process", &root).unwrap();
    let result2 = match &output2 {
        FetchOutput::Entity(r) => r,
        _ => panic!("expected Entity result"),
    };
    assert!(
        result2
            .hierarchy_context
            .contains(&"lib.rs:validate".to_string()),
        "process and validate share the same hierarchy path and should be siblings"
    );
}

#[test]
fn test_explore_rpg() {
    let graph = make_test_graph();

    // Explore downstream from "main"
    let tree = explore(&graph, "main.rs:main", Direction::Downstream, 3, None);
    assert!(tree.is_some(), "explore should return a traversal tree");
    let tree = tree.unwrap();

    assert_eq!(tree.entity_name, "main");
    assert!(
        !tree.children.is_empty(),
        "main should have downstream children (it invokes process)"
    );

    // The first child should be "process"
    let child_names: Vec<&str> = tree
        .children
        .iter()
        .map(|c| c.entity_name.as_str())
        .collect();
    assert!(
        child_names.contains(&"process"),
        "downstream children should include 'process'; got: {:?}",
        child_names
    );

    // Format tree and verify output
    let tree_output = format_tree(&tree, 0);
    assert!(
        tree_output.contains("main"),
        "tree output should contain 'main'"
    );
    assert!(
        tree_output.contains("process"),
        "tree output should contain 'process'"
    );
    assert!(
        tree_output.contains("validate"),
        "tree output should contain 'validate' (process -> validate)"
    );

    // Explore upstream from "validate"
    let upstream = explore(&graph, "lib.rs:validate", Direction::Upstream, 3, None);
    assert!(upstream.is_some());
    let upstream = upstream.unwrap();
    let upstream_child_names: Vec<&str> = upstream
        .children
        .iter()
        .map(|c| c.entity_name.as_str())
        .collect();
    assert!(
        upstream_child_names.contains(&"process"),
        "upstream from validate should include 'process'; got: {:?}",
        upstream_child_names
    );
}

#[test]
fn test_rpg_info() {
    let graph = make_test_graph();

    let info = toon::format_rpg_info(&graph);

    // Verify key metadata fields
    assert!(
        info.contains("language: rust"),
        "info should contain language; got:\n{}",
        info
    );
    assert!(
        info.contains("entities: 4"),
        "info should show 4 entities; got:\n{}",
        info
    );
    assert!(
        info.contains("edges: 2"),
        "info should show 2 edges; got:\n{}",
        info
    );
    assert!(
        info.contains("version:"),
        "info should contain version field"
    );
    assert!(
        info.contains("hierarchy:"),
        "info should contain hierarchy section"
    );
}

#[test]
fn test_reload_rpg() {
    let graph = make_test_graph();
    let (_tmp, root) = make_temp_project(&graph);

    // The graph was already saved by make_temp_project; now load it
    let loaded = storage::load(&root).unwrap();

    assert_eq!(
        loaded.entities.len(),
        graph.entities.len(),
        "loaded graph should have same entity count"
    );
    assert_eq!(
        loaded.edges.len(),
        graph.edges.len(),
        "loaded graph should have same edge count"
    );
    assert_eq!(loaded.metadata.language, "rust");

    // Verify a specific entity survived the roundtrip
    let entity = loaded.get_entity("main.rs:main").unwrap();
    assert_eq!(entity.name, "main");
    assert_eq!(entity.semantic_features.len(), 2);
    assert!(
        entity
            .semantic_features
            .contains(&"entry point".to_string())
    );
}
