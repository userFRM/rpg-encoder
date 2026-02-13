use rpg_core::graph::*;
use rpg_encoder::grounding::resolve_dependencies;
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
        semantic_features: Vec::new(),
        feature_source: None,
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_resolve_invokes() {
    let mut graph = RPGraph::new("rust");
    let mut caller = make_entity("a.rs:caller", "caller", "a.rs");
    caller.deps.invokes.push("callee".to_string());
    graph.insert_entity(caller);
    graph.insert_entity(make_entity("b.rs:callee", "callee", "b.rs"));

    resolve_dependencies(&mut graph);

    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].source, "a.rs:caller");
    assert_eq!(graph.edges[0].target, "b.rs:callee");
    assert_eq!(graph.edges[0].kind, EdgeKind::Invokes);

    // Reverse edge should be populated
    let callee = graph.get_entity("b.rs:callee").unwrap();
    assert!(callee.deps.invoked_by.contains(&"a.rs:caller".to_string()));
}

#[test]
fn test_resolve_inherits() {
    let mut graph = RPGraph::new("python");
    let mut child = make_entity("a.py:Dog", "Dog", "a.py");
    child.kind = EntityKind::Class;
    child.deps.inherits.push("Animal".to_string());
    graph.insert_entity(child);

    let mut parent = make_entity("b.py:Animal", "Animal", "b.py");
    parent.kind = EntityKind::Class;
    graph.insert_entity(parent);

    resolve_dependencies(&mut graph);

    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].kind, EdgeKind::Inherits);
    assert_eq!(graph.edges[0].source, "a.py:Dog");
    assert_eq!(graph.edges[0].target, "b.py:Animal");

    let animal = graph.get_entity("b.py:Animal").unwrap();
    assert!(animal.deps.inherited_by.contains(&"a.py:Dog".to_string()));
}

#[test]
fn test_resolve_imports() {
    let mut graph = RPGraph::new("rust");
    let mut importer = make_entity("a.rs:main", "main", "a.rs");
    importer.deps.imports.push("Config".to_string());
    graph.insert_entity(importer);
    graph.insert_entity(make_entity("b.rs:Config", "Config", "b.rs"));

    resolve_dependencies(&mut graph);

    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].kind, EdgeKind::Imports);
    assert_eq!(graph.edges[0].source, "a.rs:main");
    assert_eq!(graph.edges[0].target, "b.rs:Config");

    let config = graph.get_entity("b.rs:Config").unwrap();
    assert!(config.deps.imported_by.contains(&"a.rs:main".to_string()));
}

#[test]
fn test_resolve_no_match() {
    let mut graph = RPGraph::new("rust");
    let mut caller = make_entity("a.rs:caller", "caller", "a.rs");
    caller.deps.invokes.push("nonexistent".to_string());
    graph.insert_entity(caller);

    resolve_dependencies(&mut graph);

    assert!(graph.edges.is_empty());
}

#[test]
fn test_resolve_multiple_edges() {
    let mut graph = RPGraph::new("rust");

    let mut a = make_entity("a.rs:a", "a", "a.rs");
    a.deps.invokes.push("b".to_string());
    a.deps.invokes.push("c".to_string());
    graph.insert_entity(a);
    graph.insert_entity(make_entity("b.rs:b", "b", "b.rs"));
    graph.insert_entity(make_entity("c.rs:c", "c", "c.rs"));

    resolve_dependencies(&mut graph);

    assert_eq!(graph.edges.len(), 2);
    let targets: Vec<&str> = graph.edges.iter().map(|e| e.target.as_str()).collect();
    assert!(targets.contains(&"b.rs:b"));
    assert!(targets.contains(&"c.rs:c"));
}

#[test]
fn test_resolve_composes() {
    let mut graph = RPGraph::new("rust");
    let mut composer = make_entity("mod.rs:facade", "facade", "mod.rs");
    composer.deps.composes.push("impl_detail".to_string());
    graph.insert_entity(composer);
    graph.insert_entity(make_entity("impl.rs:impl_detail", "impl_detail", "impl.rs"));

    resolve_dependencies(&mut graph);

    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].source, "mod.rs:facade");
    assert_eq!(graph.edges[0].target, "impl.rs:impl_detail");
    assert_eq!(graph.edges[0].kind, EdgeKind::Composes);

    // Reverse edge should be populated
    let target = graph.get_entity("impl.rs:impl_detail").unwrap();
    assert!(
        target
            .deps
            .composed_by
            .contains(&"mod.rs:facade".to_string())
    );
}

#[test]
fn test_resolve_mixed_dep_types() {
    let mut graph = RPGraph::new("python");

    let mut entity = make_entity("a.py:MyClass", "MyClass", "a.py");
    entity.kind = EntityKind::Class;
    entity.deps.inherits.push("BaseClass".to_string());
    entity.deps.imports.push("helper".to_string());
    entity.deps.invokes.push("utility".to_string());
    graph.insert_entity(entity);

    let mut base = make_entity("b.py:BaseClass", "BaseClass", "b.py");
    base.kind = EntityKind::Class;
    graph.insert_entity(base);
    graph.insert_entity(make_entity("c.py:helper", "helper", "c.py"));
    graph.insert_entity(make_entity("d.py:utility", "utility", "d.py"));

    resolve_dependencies(&mut graph);

    assert_eq!(graph.edges.len(), 3);
    let edge_kinds: Vec<EdgeKind> = graph.edges.iter().map(|e| e.kind).collect();
    assert!(edge_kinds.contains(&EdgeKind::Inherits));
    assert!(edge_kinds.contains(&EdgeKind::Imports));
    assert!(edge_kinds.contains(&EdgeKind::Invokes));
}

#[test]
fn test_resolve_dependencies_clears_reverse_deps_on_re_resolve() {
    let mut graph = RPGraph::new("rust");
    let mut caller = make_entity("a.rs:caller", "caller", "a.rs");
    caller.deps.invokes.push("callee".to_string());
    graph.insert_entity(caller);
    graph.insert_entity(make_entity("b.rs:callee", "callee", "b.rs"));

    // Resolve twice — reverse deps should NOT accumulate
    resolve_dependencies(&mut graph);
    resolve_dependencies(&mut graph);

    let callee = graph.get_entity("b.rs:callee").unwrap();
    assert_eq!(
        callee.deps.invoked_by.len(),
        1,
        "invoked_by should have exactly 1 entry after double resolve, got: {:?}",
        callee.deps.invoked_by
    );

    assert_eq!(graph.edges.len(), 1, "edges should not duplicate");
}

#[test]
fn test_resolve_frontend_dependency_edges() {
    let mut graph = RPGraph::new("typescript");

    let mut page = make_entity(
        "app/login/page.tsx:LoginPage",
        "LoginPage",
        "app/login/page.tsx",
    );
    page.kind = EntityKind::Page;
    page.deps.renders.push("LoginForm".to_string());
    page.deps.reads_state.push("authStore".to_string());
    page.deps.writes_state.push("setSession".to_string());
    page.deps.dispatches.push("loginRequested".to_string());
    graph.insert_entity(page);

    let mut component = make_entity(
        "src/ui/LoginForm.tsx:LoginForm",
        "LoginForm",
        "src/ui/LoginForm.tsx",
    );
    component.kind = EntityKind::Component;
    graph.insert_entity(component);

    let mut store = make_entity(
        "src/state/store.ts:authStore",
        "authStore",
        "src/state/store.ts",
    );
    store.kind = EntityKind::Store;
    graph.insert_entity(store);

    graph.insert_entity(make_entity(
        "src/state/session.ts:setSession",
        "setSession",
        "src/state/session.ts",
    ));
    graph.insert_entity(make_entity(
        "src/state/actions.ts:loginRequested",
        "loginRequested",
        "src/state/actions.ts",
    ));

    resolve_dependencies(&mut graph);

    let kinds: Vec<EdgeKind> = graph.edges.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&EdgeKind::Renders));
    assert!(kinds.contains(&EdgeKind::ReadsState));
    assert!(kinds.contains(&EdgeKind::WritesState));
    assert!(kinds.contains(&EdgeKind::Dispatches));

    let login_form = graph.get_entity("src/ui/LoginForm.tsx:LoginForm").unwrap();
    assert!(
        login_form
            .deps
            .rendered_by
            .contains(&"app/login/page.tsx:LoginPage".to_string())
    );

    let auth_store = graph.get_entity("src/state/store.ts:authStore").unwrap();
    assert!(
        auth_store
            .deps
            .state_read_by
            .contains(&"app/login/page.tsx:LoginPage".to_string())
    );
}

#[test]
fn test_resolve_dispatch_to_slice_reducer() {
    let mut graph = RPGraph::new("typescript");

    let mut component = make_entity(
        "src/components/LoginForm.tsx:LoginForm",
        "LoginForm",
        "src/components/LoginForm.tsx",
    );
    component.kind = EntityKind::Component;
    component.deps.dispatches.push("loginStarted".to_string());
    graph.insert_entity(component);

    let mut reducer = make_entity(
        "src/state/authSlice.ts:authSlice::loginStarted",
        "loginStarted",
        "src/state/authSlice.ts",
    );
    reducer.parent_class = Some("authSlice".to_string());
    graph.insert_entity(reducer);

    resolve_dependencies(&mut graph);

    let has_dispatch = graph.edges.iter().any(|e| {
        e.source == "src/components/LoginForm.tsx:LoginForm"
            && e.target == "src/state/authSlice.ts:authSlice::loginStarted"
            && e.kind == EdgeKind::Dispatches
    });
    assert!(
        has_dispatch,
        "expected Dispatches edge from component to reducer, edges: {:?}",
        graph.edges
    );

    let reducer_entity = graph
        .get_entity("src/state/authSlice.ts:authSlice::loginStarted")
        .unwrap();
    assert!(
        reducer_entity
            .deps
            .dispatched_by
            .contains(&"src/components/LoginForm.tsx:LoginForm".to_string()),
        "reducer should have dispatched_by reverse edge"
    );
}

#[test]
fn test_resolve_reads_state_to_selector() {
    let mut graph = RPGraph::new("typescript");

    let mut component = make_entity(
        "src/components/LoginForm.tsx:LoginForm",
        "LoginForm",
        "src/components/LoginForm.tsx",
    );
    component.kind = EntityKind::Component;
    component
        .deps
        .reads_state
        .push("selectAuthLoading".to_string());
    graph.insert_entity(component);

    graph.insert_entity(make_entity(
        "src/state/selectors.ts:selectAuthLoading",
        "selectAuthLoading",
        "src/state/selectors.ts",
    ));

    resolve_dependencies(&mut graph);

    let has_reads = graph.edges.iter().any(|e| {
        e.source == "src/components/LoginForm.tsx:LoginForm"
            && e.target == "src/state/selectors.ts:selectAuthLoading"
            && e.kind == EdgeKind::ReadsState
    });
    assert!(
        has_reads,
        "expected ReadsState edge from component to selector, edges: {:?}",
        graph.edges
    );

    let selector = graph
        .get_entity("src/state/selectors.ts:selectAuthLoading")
        .unwrap();
    assert!(
        selector
            .deps
            .state_read_by
            .contains(&"src/components/LoginForm.tsx:LoginForm".to_string()),
        "selector should have state_read_by reverse edge"
    );
}
