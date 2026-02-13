use rpg_core::graph::*;
use rpg_core::storage;
use rpg_encoder::grounding;
use rpg_nav::explore::{Direction, explore, format_tree};
use rpg_nav::fetch::{FetchOutput, fetch};
use rpg_nav::search::{SearchMode, search};
use rpg_nav::toon;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;
use std::path::{Path, PathBuf};
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
        feature_source: None,
        hierarchy_path: hierarchy.to_string(),
        deps: EntityDeps::default(),
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

fn nextjs_fixture_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/nextjs_project")
}

fn collect_ts_fixture_files(root: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    collect_ts_recursive(root, root, &mut out);
    out
}

fn collect_ts_recursive(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ts_recursive(base, &path, out);
            continue;
        }

        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext != "ts" && ext != "tsx" {
            continue;
        }

        let rel = path.strip_prefix(base).unwrap().to_path_buf();
        let source = std::fs::read_to_string(&path).unwrap();
        out.push((rel, source));
    }
}

fn build_nextjs_fixture_graph() -> RPGraph {
    let root = nextjs_fixture_root();
    let files = collect_ts_fixture_files(&root);
    assert!(
        !files.is_empty(),
        "expected fixture TS/TSX files in {}",
        root.display()
    );

    // Load TOML paradigm defs + compile queries
    let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap();
    let qcache =
        rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs).unwrap();
    let languages = vec![Language::TYPESCRIPT];
    let active_defs =
        rpg_parser::paradigms::detect_paradigms_toml(&root, &languages, &paradigm_defs);

    let mut graph = RPGraph::new("typescript");
    for (rel_path, source) in &files {
        let mut raw_entities = extract_entities(rel_path, source, Language::TYPESCRIPT);

        // TOML-driven paradigm pipeline: classify + entity queries + builtin features
        rpg_parser::paradigms::classify::classify_entities(
            &active_defs,
            rel_path,
            &mut raw_entities,
        );
        let extra = rpg_parser::paradigms::query_engine::execute_entity_queries(
            &qcache,
            &active_defs,
            rel_path,
            source,
            Language::TYPESCRIPT,
            &raw_entities,
        );
        raw_entities.extend(extra);
        rpg_parser::paradigms::features::apply_builtin_entity_features(
            &active_defs,
            rel_path,
            source,
            Language::TYPESCRIPT,
            &mut raw_entities,
        );

        for entity in raw_entities {
            graph.insert_entity(entity.into_entity());
        }
    }

    graph.create_module_entities();
    graph.build_file_path_hierarchy();
    let paradigm_ctx = grounding::ParadigmContext {
        active_defs: active_defs.clone(),
        qcache: &qcache,
    };
    grounding::populate_entity_deps(&mut graph, &root, false, None, Some(&paradigm_ctx));
    grounding::resolve_dependencies(&mut graph);
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();
    grounding::ground_hierarchy(&mut graph);
    graph.refresh_metadata();
    graph
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
    let FetchOutput::Entity(result) = &output else {
        panic!("expected Entity result")
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
    let FetchOutput::Entity(result2) = &output2 else {
        panic!("expected Entity result")
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
        info.contains("languages: rust"),
        "info should contain languages; got:\n{}",
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
        info.contains("hierarchy"),
        "info should contain hierarchy section; got:\n{}",
        info
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

#[test]
fn test_nextjs_page_component_store_traversal() {
    let graph = build_nextjs_fixture_graph();

    let page_id = "app/login/page.tsx:Page";
    let component_id = "src/components/LoginForm.tsx:LoginForm";
    let store_id = "src/state/store.ts:store";

    let page = graph.get_entity(page_id).expect("missing page entity");
    assert_eq!(page.kind, EntityKind::Page);
    assert!(
        page.deps.renders.contains(&"LoginForm".to_string()),
        "Page should render LoginForm, got: {:?}",
        page.deps.renders
    );

    let component = graph
        .get_entity(component_id)
        .expect("missing component entity");
    assert_eq!(component.kind, EntityKind::Component);
    // LoginForm reads state via useSelector(selectAuthLoading)
    assert!(
        component
            .deps
            .reads_state
            .contains(&"useSelector".to_string()),
        "LoginForm should read state via useSelector, got: {:?}",
        component.deps.reads_state
    );
    // LoginForm dispatches loginUser thunk
    assert!(
        component.deps.dispatches.contains(&"loginUser".to_string()),
        "LoginForm should dispatch loginUser, got: {:?}",
        component.deps.dispatches
    );

    let store = graph.get_entity(store_id).expect("missing store entity");
    assert_eq!(store.kind, EntityKind::Store);

    let has_page_to_component = graph
        .edges
        .iter()
        .any(|e| e.source == page_id && e.target == component_id && e.kind == EdgeKind::Renders);
    assert!(
        has_page_to_component,
        "expected renders edge page -> component"
    );

    let tree = explore(&graph, page_id, Direction::Downstream, 3, None)
        .expect("expected traversal tree from page");
    let tree_output = format_tree(&tree, 0);
    assert!(
        tree_output.contains("LoginForm"),
        "traversal should include component node:\n{}",
        tree_output
    );
}

#[test]
fn test_rtk_full_cycle_traversal() {
    let graph = build_nextjs_fixture_graph();

    // Verify key RTK entities exist
    let slice_id = "src/state/authSlice.ts:authSlice";
    let thunk_id = "src/state/thunks.ts:loginUser";
    let selector_id = "src/state/selectors.ts:selectAuthLoading";
    let api_id = "src/state/api.ts:postsApi";
    let hook_id = "src/hooks/useAuth.ts:useAuth";
    let login_page_id = "app/login/page.tsx:Page";
    let dashboard_page_id = "app/dashboard/page.tsx:Page";
    let login_form_id = "src/components/LoginForm.tsx:LoginForm";
    let post_list_id = "src/components/PostList.tsx:PostList";

    // authSlice is a Store entity
    let slice = graph.get_entity(slice_id).expect("missing authSlice");
    assert_eq!(slice.kind, EntityKind::Store);

    // loginUser is a Function (thunk)
    let thunk = graph.get_entity(thunk_id).expect("missing loginUser thunk");
    assert_eq!(thunk.kind, EntityKind::Function);

    // selectAuthLoading is a Function (selector)
    let selector = graph
        .get_entity(selector_id)
        .expect("missing selectAuthLoading");
    assert_eq!(selector.kind, EntityKind::Function);

    // postsApi is a Store entity (createApi)
    let api = graph.get_entity(api_id).expect("missing postsApi");
    assert_eq!(api.kind, EntityKind::Store);

    // useAuth is a Hook
    let hook = graph.get_entity(hook_id).expect("missing useAuth hook");
    assert_eq!(hook.kind, EntityKind::Hook);

    // Verify slice contains reducer entities
    let reducer_id = format!("{}::loginStarted", slice_id);
    let reducer = graph
        .get_entity(&reducer_id)
        .expect("missing loginStarted reducer entity");
    assert_eq!(reducer.kind, EntityKind::Function);
    assert_eq!(reducer.parent_class.as_deref(), Some("authSlice"));

    // LoginForm dispatches loginUser thunk
    let login_form = graph.get_entity(login_form_id).expect("missing LoginForm");
    assert!(
        login_form
            .deps
            .dispatches
            .contains(&"loginUser".to_string()),
        "LoginForm should dispatch loginUser"
    );
    // LoginForm reads state via selector
    assert!(
        login_form
            .deps
            .reads_state
            .contains(&"selectAuthLoading".to_string()),
        "LoginForm should read selectAuthLoading, got: {:?}",
        login_form.deps.reads_state
    );

    // Dispatches edge: LoginForm -> loginUser thunk
    let has_dispatch_edge = graph.edges.iter().any(|e| {
        e.source == login_form_id && e.target == thunk_id && e.kind == EdgeKind::Dispatches
    });
    assert!(
        has_dispatch_edge,
        "expected dispatches edge LoginForm -> loginUser"
    );

    // ReadsState edge: LoginForm -> selectAuthLoading
    let has_reads_edge = graph.edges.iter().any(|e| {
        e.source == login_form_id && e.target == selector_id && e.kind == EdgeKind::ReadsState
    });
    assert!(
        has_reads_edge,
        "expected reads_state edge LoginForm -> selectAuthLoading"
    );

    // Login page renders LoginForm
    assert!(graph.get_entity(login_page_id).is_some());
    let has_page_render = graph.edges.iter().any(|e| {
        e.source == login_page_id && e.target == login_form_id && e.kind == EdgeKind::Renders
    });
    assert!(has_page_render, "expected login page -> LoginForm render");

    // Dashboard page renders PostList
    assert!(graph.get_entity(dashboard_page_id).is_some());
    let has_dashboard_render = graph.edges.iter().any(|e| {
        e.source == dashboard_page_id && e.target == post_list_id && e.kind == EdgeKind::Renders
    });
    assert!(
        has_dashboard_render,
        "expected dashboard page -> PostList render"
    );

    // PostList reads state via RTK Query hook
    let post_list = graph.get_entity(post_list_id).expect("missing PostList");
    assert!(
        post_list
            .deps
            .reads_state
            .contains(&"useGetPostsQuery".to_string()),
        "PostList should read state via useGetPostsQuery, got: {:?}",
        post_list.deps.reads_state
    );

    // Full traversal from login page
    let tree = explore(&graph, login_page_id, Direction::Downstream, 4, None)
        .expect("expected traversal tree from login page");
    let tree_output = format_tree(&tree, 0);
    assert!(
        tree_output.contains("LoginForm"),
        "traversal should reach LoginForm:\n{}",
        tree_output
    );
    assert!(
        tree_output.contains("loginUser"),
        "traversal should reach loginUser thunk:\n{}",
        tree_output
    );
}

/// End-to-end test for the restore-only routing path: all drifted entities
/// restore to their original positions, but hierarchy IDs and containment
/// edges must still be refreshed (not stale).
#[test]
fn test_restore_only_routing_refreshes_hierarchy_ids_and_containment() {
    // Build a graph with semantic hierarchy and two areas
    let mut graph = RPGraph::new("python");
    graph.metadata.semantic_hierarchy = true;

    // Area "Auth" with one entity
    let auth_entity = Entity {
        id: "src/auth.py:login".to_string(),
        kind: EntityKind::Function,
        name: "login".to_string(),
        file: PathBuf::from("src/auth.py"),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: vec![
            "authenticate user".to_string(),
            "validate token".to_string(),
        ],
        feature_source: None,
        hierarchy_path: "Auth/login".to_string(),
        deps: EntityDeps::default(),
    };
    graph.insert_entity(auth_entity);
    graph.insert_into_hierarchy("Auth/login", "src/auth.py:login");

    // Area "Data" with one entity
    let data_entity = Entity {
        id: "src/data.py:load".to_string(),
        kind: EntityKind::Function,
        name: "load".to_string(),
        file: PathBuf::from("src/data.py"),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: vec!["load dataset".to_string(), "parse csv".to_string()],
        feature_source: None,
        hierarchy_path: "Data/loading".to_string(),
        deps: EntityDeps::default(),
    };
    graph.insert_entity(data_entity);
    graph.insert_into_hierarchy("Data/loading", "src/data.py:load");

    // Initial structural setup
    graph.aggregate_hierarchy_features();
    graph.assign_hierarchy_ids();
    graph.materialize_containment_edges();
    graph.refresh_metadata();

    // Verify initial containment edges exist
    let initial_contains = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .count();
    assert!(
        initial_contains > 0,
        "should have containment edges initially"
    );

    // Record initial hierarchy IDs
    let initial_auth_id = graph.hierarchy.get("Auth").map(|n| n.id.clone());
    assert!(
        initial_auth_id.is_some(),
        "Auth area should have a hierarchy_id"
    );

    // --- Simulate the MCP submit_lift_results restore-only path ---
    // Entity gets "drifted" features that are still in the same domain.
    // The entity should restore to its original position after routing.
    let drifted_ids = vec!["src/auth.py:login".to_string()];
    let newly_lifted_ids: Vec<String> = vec![];

    // Simulate the MCP routing loop for drifted entities
    for eid in &drifted_ids {
        let original_path = graph
            .entities
            .get(eid)
            .map(|e| e.hierarchy_path.clone())
            .unwrap_or_default();

        graph.remove_entity_from_hierarchy(eid);
        graph.aggregate_hierarchy_features();

        // Entity features slightly changed but still auth-domain
        let features = vec![
            "authenticate user".to_string(),
            "verify credentials".to_string(),
        ];
        if let Some(entity) = graph.entities.get_mut(eid) {
            entity.semantic_features = features.clone();
        }

        match rpg_encoder::evolution::find_best_hierarchy_path(&graph, &features) {
            Some(new_path) if new_path != original_path => {
                if let Some(entity) = graph.entities.get_mut(eid) {
                    entity.hierarchy_path = new_path.clone();
                }
                graph.insert_into_hierarchy(&new_path, eid);
            }
            _ => {
                // Restore to original — this is the "restore-only" path
                if !original_path.is_empty() {
                    graph.insert_into_hierarchy(&original_path, eid);
                }
            }
        }
    }

    // The MCP handler should always rebuild if hierarchy_mutated, even on restore-only
    let hierarchy_mutated = (!drifted_ids.is_empty() || !newly_lifted_ids.is_empty())
        && graph.metadata.semantic_hierarchy;
    assert!(hierarchy_mutated, "should detect hierarchy mutations");

    // Apply the final rebuild (this is what the MCP handler does)
    graph.aggregate_hierarchy_features();
    graph.assign_hierarchy_ids();
    graph.materialize_containment_edges();
    graph.refresh_metadata();

    // Verify hierarchy IDs are still assigned (not stale/missing)
    let final_auth_id = graph.hierarchy.get("Auth").map(|n| n.id.clone());
    assert!(
        final_auth_id.is_some() && !final_auth_id.as_ref().unwrap().is_empty(),
        "Auth area should still have a hierarchy_id after restore-only routing"
    );

    // Verify containment edges are still present
    let final_contains = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Contains)
        .count();
    assert!(
        final_contains > 0,
        "containment edges should be rebuilt after restore-only routing"
    );

    // Verify the entity is back in its original hierarchy position
    let login = graph.get_entity("src/auth.py:login").unwrap();
    assert_eq!(
        login.hierarchy_path, "Auth/login",
        "entity should be restored to original position"
    );
    let auth_login = &graph.hierarchy["Auth"].children["login"];
    assert!(
        auth_login
            .entities
            .contains(&"src/auth.py:login".to_string()),
        "entity should be present in Auth/login hierarchy node"
    );
}
