use rpg_core::graph::*;
use rpg_nav::search::{
    SearchMode, SearchParams, search, search_with_embedding, search_with_params,
};
use std::path::PathBuf;

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

fn make_graph() -> RPGraph {
    let mut graph = RPGraph::new("rust");

    graph.insert_entity(make_entity(
        "auth.rs:validate_token",
        "validate_token",
        "auth.rs",
        vec!["JWT token validation", "authentication check"],
        "Security/auth/token",
    ));
    graph.insert_entity(make_entity(
        "db.rs:query_users",
        "query_users",
        "db.rs",
        vec!["database query", "user retrieval"],
        "DataAccess/storage/users",
    ));
    graph.insert_entity(make_entity(
        "api.rs:handle_login",
        "handle_login",
        "api.rs",
        vec!["login endpoint", "authentication flow"],
        "Security/auth/login",
    ));
    graph.insert_entity(make_entity(
        "utils.rs:parse_config",
        "parse_config",
        "utils.rs",
        vec!["configuration parsing", "TOML reader"],
        "Core/config/parsing",
    ));

    graph.insert_into_hierarchy("Security/auth/token", "auth.rs:validate_token");
    graph.insert_into_hierarchy("DataAccess/storage/users", "db.rs:query_users");
    graph.insert_into_hierarchy("Security/auth/login", "api.rs:handle_login");
    graph.insert_into_hierarchy("Core/config/parsing", "utils.rs:parse_config");

    graph
}

#[test]
fn test_feature_search_single_match() {
    let graph = make_graph();
    let results = search(&graph, "JWT", SearchMode::Features, None, 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "validate_token");
}

#[test]
fn test_feature_search_multiple_matches() {
    let graph = make_graph();
    let results = search(&graph, "authentication", SearchMode::Features, None, 10);
    assert!(results.len() >= 2);
    let names: Vec<&str> = results.iter().map(|r| r.entity_name.as_str()).collect();
    assert!(names.contains(&"validate_token"));
    assert!(names.contains(&"handle_login"));
}

#[test]
fn test_feature_search_no_match() {
    let graph = make_graph();
    let results = search(
        &graph,
        "nonexistent_feature",
        SearchMode::Features,
        None,
        10,
    );
    assert!(results.is_empty());
}

#[test]
fn test_snippet_search_by_name() {
    let graph = make_graph();
    let results = search(&graph, "parse_config", SearchMode::Snippets, None, 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "parse_config");
}

#[test]
fn test_snippet_search_by_file() {
    let graph = make_graph();
    let results = search(&graph, "db.rs", SearchMode::Snippets, None, 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "query_users");
}

#[test]
fn test_auto_mode_uses_features_first() {
    let graph = make_graph();
    let results = search(&graph, "database query", SearchMode::Auto, None, 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "query_users");
    assert!(!results[0].matched_features.is_empty());
}

#[test]
fn test_auto_mode_falls_back_to_snippets() {
    let graph = make_graph();
    // "handle_login" won't match any feature exactly but matches entity name
    let results = search(&graph, "handle_login", SearchMode::Auto, None, 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "handle_login");
}

#[test]
fn test_scope_filtering() {
    let graph = make_graph();
    let results = search(
        &graph,
        "authentication",
        SearchMode::Features,
        Some("Security"),
        10,
    );
    // Should only find entities under Security hierarchy
    for r in &results {
        assert!(
            r.entity_id == "auth.rs:validate_token" || r.entity_id == "api.rs:handle_login",
            "unexpected entity in scoped search: {}",
            r.entity_id
        );
    }
}

#[test]
fn test_scope_filtering_narrow() {
    let graph = make_graph();
    let results = search(
        &graph,
        "authentication",
        SearchMode::Features,
        Some("Security/auth/token"),
        10,
    );
    // Only validate_token is in Security/auth/token
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_name, "validate_token");
}

#[test]
fn test_empty_query() {
    let graph = make_graph();
    let results = search(&graph, "", SearchMode::Features, None, 10);
    assert!(results.is_empty());
}

#[test]
fn test_results_sorted_by_score() {
    let graph = make_graph();
    let results = search(&graph, "authentication", SearchMode::Features, None, 10);
    if results.len() >= 2 {
        for i in 0..results.len() - 1 {
            assert!(results[i].score >= results[i + 1].score);
        }
    }
}

fn make_entity_with_embedding(
    id: &str,
    name: &str,
    file: &str,
    features: Vec<&str>,
    hierarchy: &str,
    embedding: Vec<f32>,
) -> Entity {
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
        embedding: Some(embedding),
    }
}

#[test]
fn test_semantic_search_with_embeddings() {
    let mut graph = RPGraph::new("rust");

    // Entity A: embedding close to [1, 0, 0]
    graph.insert_entity(make_entity_with_embedding(
        "a.rs:alpha",
        "alpha",
        "a.rs",
        vec!["authentication"],
        "Security/auth/token",
        vec![1.0, 0.0, 0.0],
    ));
    // Entity B: embedding close to [0, 1, 0]
    graph.insert_entity(make_entity_with_embedding(
        "b.rs:beta",
        "beta",
        "b.rs",
        vec!["database query"],
        "DataAccess/storage/db",
        vec![0.0, 1.0, 0.0],
    ));

    // Query embedding close to entity A
    let query_emb = vec![0.9, 0.1, 0.0];

    let results = search_with_embedding(
        &graph,
        "",
        SearchMode::Semantic,
        None,
        10,
        Some(&query_emb),
        0.5,
    );

    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "alpha");
}

#[test]
fn test_hybrid_search_merges_results() {
    let mut graph = RPGraph::new("rust");

    // Entity that matches keyword but not embedding
    graph.insert_entity(make_entity_with_embedding(
        "a.rs:validate_token",
        "validate_token",
        "a.rs",
        vec!["JWT token validation"],
        "Security/auth/token",
        vec![0.0, 0.0, 1.0], // orthogonal to query
    ));
    // Entity that matches embedding but not keyword
    graph.insert_entity(make_entity_with_embedding(
        "b.rs:check_auth",
        "check_auth",
        "b.rs",
        vec!["session check"],
        "Security/auth/session",
        vec![1.0, 0.0, 0.0], // close to query
    ));

    let query_emb = vec![0.95, 0.05, 0.0];

    let results = search_with_embedding(
        &graph,
        "JWT",
        SearchMode::Hybrid,
        None,
        10,
        Some(&query_emb),
        0.5,
    );

    // Both should appear — one via keyword, one via embedding
    assert!(results.len() >= 2);
    let names: Vec<&str> = results.iter().map(|r| r.entity_name.as_str()).collect();
    assert!(names.contains(&"validate_token"));
    assert!(names.contains(&"check_auth"));
}

#[test]
fn test_semantic_fallback_without_embedding() {
    let graph = make_graph();
    // Semantic mode without query embedding should fall back to keyword search
    let results = search_with_embedding(&graph, "JWT", SearchMode::Semantic, None, 10, None, 0.5);
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_name, "validate_token");
}

// --- SearchParams tests ---

#[test]
fn test_search_with_file_pattern() {
    let graph = make_graph();
    let results = search_with_params(
        &graph,
        &SearchParams {
            query: "authentication",
            mode: SearchMode::Features,
            scope: None,
            limit: 10,
            line_nums: None,
            file_pattern: Some("auth*"),
            query_embedding: None,
            semantic_weight: 0.5,
            entity_type_filter: None,
        },
    );
    // Only auth.rs matches the pattern "auth*"
    assert!(!results.is_empty());
    for r in &results {
        assert!(
            r.file.starts_with("auth"),
            "file should match auth*: {}",
            r.file
        );
    }
}

#[test]
fn test_search_with_line_nums() {
    let mut graph = RPGraph::new("rust");
    // Entity at lines 1-10
    graph.insert_entity(make_entity(
        "a.rs:early",
        "early",
        "a.rs",
        vec!["database query"],
        "Core/db/query",
    ));
    // Entity at lines 50-60 — modify line range
    let mut late = make_entity(
        "a.rs:late",
        "late",
        "a.rs",
        vec!["database query"],
        "Core/db/query",
    );
    late.line_start = 50;
    late.line_end = 60;
    graph.insert_entity(late);

    let results = search_with_params(
        &graph,
        &SearchParams {
            query: "database",
            mode: SearchMode::Features,
            scope: None,
            limit: 10,
            line_nums: Some((40, 70)),
            file_pattern: None,
            query_embedding: None,
            semantic_weight: 0.5,
            entity_type_filter: None,
        },
    );
    // Only "late" should match (lines 50-60 overlaps 40-70)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_name, "late");
}

#[test]
fn test_search_params_combined() {
    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_entity(
        "auth.rs:validate",
        "validate",
        "auth.rs",
        vec!["token validation"],
        "Security/auth/token",
    ));
    let mut other = make_entity(
        "db.rs:validate",
        "validate",
        "db.rs",
        vec!["token validation"],
        "DataAccess/db/validate",
    );
    other.line_start = 100;
    other.line_end = 120;
    graph.insert_entity(other);

    // Filter by file + line range: only auth.rs at lines 1-10
    let results = search_with_params(
        &graph,
        &SearchParams {
            query: "token",
            mode: SearchMode::Features,
            scope: None,
            limit: 10,
            line_nums: Some((1, 20)),
            file_pattern: Some("auth*"),
            query_embedding: None,
            semantic_weight: 0.5,
            entity_type_filter: None,
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "auth.rs:validate");
}
