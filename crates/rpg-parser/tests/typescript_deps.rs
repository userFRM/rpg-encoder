use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;
use std::path::Path;

/// Helper that runs base `extract_deps` followed by the TOML-driven paradigm
/// pipeline (dep queries + built-in dep features like Redux state signals).
/// Use this for tests that exercise paradigm-specific extraction (JSX renders,
/// Redux dispatches/selectors, RTK Query hooks, state signals).
fn extract_deps_with_paradigms(
    path: &Path,
    source: &str,
    language: Language,
) -> rpg_parser::deps::RawDeps {
    let mut deps = extract_deps(path, source, language);
    let defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap();
    let qcache = rpg_parser::paradigms::query_engine::QueryCache::compile_all(&defs).unwrap();
    let active: Vec<&_> = defs.iter().collect();
    let scopes = rpg_parser::deps::build_scopes(source, language);
    rpg_parser::paradigms::query_engine::execute_dep_queries(
        &qcache, &active, path, source, language, &scopes, &mut deps,
    );
    let raw_entities: Vec<rpg_parser::entities::RawEntity> = vec![];
    rpg_parser::paradigms::features::apply_builtin_dep_features(
        &active,
        path,
        source,
        language,
        &raw_entities,
        &mut deps,
    );
    deps
}

#[test]
fn test_ts_named_import() {
    let source = "import { foo, bar } from './module';";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "./module");
    assert!(deps.imports[0].symbols.contains(&"foo".to_string()));
    assert!(deps.imports[0].symbols.contains(&"bar".to_string()));
    assert_eq!(deps.imports[0].symbols.len(), 2);
}

#[test]
fn test_ts_namespace_import() {
    let source = "import * as utils from 'utils';";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "utils");
}

#[test]
fn test_ts_default_import() {
    let source = "import React from 'react';";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "react");
}

#[test]
fn test_ts_class_inheritance() {
    let source = "class Dog extends Animal {}";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert!(
        !deps.inherits.is_empty(),
        "expected at least one inherit dep"
    );
    assert_eq!(deps.inherits[0].child_class, "Dog");
    assert_eq!(deps.inherits[0].parent_class, "Animal");
}

#[test]
fn test_ts_call_extraction() {
    let source = "\
function greet() {
    console.log(\"hello\");
}
";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert!(!deps.calls.is_empty(), "expected at least one call dep");
    let log_call = deps.calls.iter().find(|c| c.callee == "log");
    assert!(
        log_call.is_some(),
        "expected a call to 'log', got: {:?}",
        deps.calls
    );
    assert_eq!(log_call.unwrap().caller_entity, "greet");
}

#[test]
fn test_ts_empty_source() {
    let deps = extract_deps(Path::new("test.ts"), "", Language::TYPESCRIPT);
    assert!(deps.imports.is_empty());
    assert!(deps.calls.is_empty());
    assert!(deps.inherits.is_empty());
    assert!(deps.composes.is_empty());
    assert!(deps.renders.is_empty());
    assert!(deps.reads_state.is_empty());
    assert!(deps.writes_state.is_empty());
    assert!(deps.dispatches.is_empty());
}

#[test]
fn test_ts_multiple_imports() {
    let source = "\
import { useState } from 'react';
import { Router } from 'react-router';
import axios from 'axios';
";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert_eq!(deps.imports.len(), 3);
    let modules: Vec<&str> = deps.imports.iter().map(|i| i.module.as_str()).collect();
    assert!(modules.contains(&"react"));
    assert!(modules.contains(&"react-router"));
    assert!(modules.contains(&"axios"));
}

#[test]
fn test_tsx_parsing_with_jsx() {
    let source = r#"
import React from 'react';

function App(): JSX.Element {
    return <div className="app"><h1>Hello</h1></div>;
}
"#;
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    // Should parse without errors — imports and calls still extracted
    assert!(!deps.imports.is_empty());
}

#[test]
fn test_barrel_reexport_named() {
    let source = "export { Foo, Bar } from './foo';";
    let deps = extract_deps(Path::new("index.ts"), source, Language::TYPESCRIPT);
    assert_eq!(
        deps.composes.len(),
        2,
        "expected exactly 2 compose deps, got: {:?}",
        deps.composes
    );
    let names: Vec<&str> = deps
        .composes
        .iter()
        .map(|c| c.target_name.as_str())
        .collect();
    assert!(
        names.contains(&"Foo"),
        "expected Foo in composes, got: {:?}",
        names
    );
    assert!(
        names.contains(&"Bar"),
        "expected Bar in composes, got: {:?}",
        names
    );
    // Should also produce import deps for resolution
    assert!(
        !deps.imports.is_empty(),
        "expected import dep for re-export resolution"
    );
}

#[test]
fn test_barrel_reexport_star() {
    let source = "export * from './utils';";
    let deps = extract_deps(Path::new("index.ts"), source, Language::TYPESCRIPT);
    assert_eq!(
        deps.composes.len(),
        1,
        "expected exactly 1 compose dep, got: {:?}",
        deps.composes
    );
    // Star re-export target is the module path (stripped of ./ prefix)
    assert_eq!(deps.composes[0].target_name, "utils");
    // Should also produce an import dep
    assert!(!deps.imports.is_empty());
}

#[test]
fn test_jsx_component_call() {
    let source = r"
function App() {
    return <Button onClick={handleClick}>Click me</Button>;
}
";
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let button_call = deps.renders.iter().find(|c| c.callee == "Button");
    assert!(
        button_call.is_some(),
        "expected a render of 'Button' from JSX usage, got: {:?}",
        deps.renders
    );
    assert_eq!(button_call.unwrap().caller_entity, "App");
}

#[test]
fn test_jsx_self_closing_component() {
    let source = r#"
function App() {
    return <Icon name="star" />;
}
"#;
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let icon_call = deps.renders.iter().find(|c| c.callee == "Icon");
    assert!(
        icon_call.is_some(),
        "expected a render of 'Icon' from self-closing JSX, got: {:?}",
        deps.renders
    );
}

#[test]
fn test_jsx_html_element_ignored() {
    let source = r"
function App() {
    return <div><span>text</span></div>;
}
";
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let html_calls: Vec<_> = deps
        .renders
        .iter()
        .filter(|c| c.callee == "div" || c.callee == "span")
        .collect();
    assert!(
        html_calls.is_empty(),
        "HTML elements should not produce calls, got: {:?}",
        html_calls
    );
}

#[test]
fn test_arrow_function_scope_for_calls() {
    let source = r"
const App = () => {
    fetchData();
    return <Button />;
};
";
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let fetch_call = deps.calls.iter().find(|c| c.callee == "fetchData");
    assert!(
        fetch_call.is_some(),
        "expected call to fetchData, got: {:?}",
        deps.calls
    );
    assert_eq!(
        fetch_call.unwrap().caller_entity,
        "App",
        "arrow function scope should be 'App'"
    );
    let button_call = deps.renders.iter().find(|c| c.callee == "Button");
    assert!(
        button_call.is_some(),
        "expected JSX render of Button in arrow function"
    );
    assert_eq!(button_call.unwrap().caller_entity, "App");
}

#[test]
fn test_barrel_reexport_aliased() {
    let source = "export { default as Foo } from './mod';";
    let deps = extract_deps(Path::new("index.ts"), source, Language::TYPESCRIPT);
    assert_eq!(
        deps.composes.len(),
        1,
        "expected 1 compose dep for aliased re-export, got: {:?}",
        deps.composes
    );
    // Should use the alias name, not the original name
    assert_eq!(deps.composes[0].target_name, "Foo");
}

#[test]
fn test_jsx_dotted_component() {
    let source = r"
function App() {
    return <Router.Route path='/' />;
}
";
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    // Dotted component: extracts last segment for resolution
    let route_call = deps.renders.iter().find(|c| c.callee == "Route");
    assert!(
        route_call.is_some(),
        "expected render of 'Route' from <Router.Route />, got: {:?}",
        deps.renders
    );
    assert_eq!(route_call.unwrap().caller_entity, "App");
}

#[test]
fn test_jsx_nested_components() {
    let source = r"
function Layout() {
    return <Container><Header /><Content /></Container>;
}
";
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let component_calls: Vec<&str> = deps.renders.iter().map(|c| c.callee.as_str()).collect();
    assert!(
        component_calls.contains(&"Container"),
        "expected Container, got: {:?}",
        component_calls
    );
    assert!(
        component_calls.contains(&"Header"),
        "expected Header, got: {:?}",
        component_calls
    );
    assert!(
        component_calls.contains(&"Content"),
        "expected Content, got: {:?}",
        component_calls
    );
}

#[test]
fn test_frontend_render_and_state_signals() {
    let source = r"
function LoginPage() {
    const user = useSelector(selectUser);
    const [count, setCount] = useState(0);
    setCount(count + 1);
    dispatch(loginRequested());
    return <LoginForm />;
}
";

    let deps = extract_deps_with_paradigms(
        Path::new("app/login/page.tsx"),
        source,
        Language::TYPESCRIPT,
    );

    let render = deps.renders.iter().find(|d| d.callee == "LoginForm");
    assert!(
        render.is_some(),
        "expected LoginPage to render LoginForm, got: {:?}",
        deps.renders
    );
    assert_eq!(render.unwrap().caller_entity, "LoginPage");

    let reads = deps.reads_state.iter().find(|d| d.callee == "useSelector");
    assert!(
        reads.is_some(),
        "expected state read signal from useSelector, got: {:?}",
        deps.reads_state
    );

    let writes = deps.writes_state.iter().find(|d| d.callee == "setCount");
    assert!(
        writes.is_some(),
        "expected state write signal from setCount, got: {:?}",
        deps.writes_state
    );

    let dispatch = deps
        .dispatches
        .iter()
        .find(|d| d.callee == "loginRequested");
    assert!(
        dispatch.is_some(),
        "expected dispatch signal for loginRequested, got: {:?}",
        deps.dispatches
    );
}

#[test]
fn test_use_selector_extracts_selector_name() {
    let source = r"
function ProfilePage() {
    const user = useSelector(selectUser);
    return <div>{user.name}</div>;
}
";
    let deps = extract_deps_with_paradigms(
        Path::new("app/profile/page.tsx"),
        source,
        Language::TYPESCRIPT,
    );

    // Should have both useSelector and selectUser as reads_state entries
    let callees: Vec<&str> = deps.reads_state.iter().map(|d| d.callee.as_str()).collect();
    assert!(
        callees.contains(&"useSelector"),
        "expected useSelector in reads_state, got: {:?}",
        callees
    );
    assert!(
        callees.contains(&"selectUser"),
        "expected selectUser in reads_state, got: {:?}",
        callees
    );
}

#[test]
fn test_use_dispatch_not_a_state_reader() {
    let source = r"
function MyComponent() {
    const dispatch = useDispatch();
    return <div />;
}
";
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    // useDispatch acquires dispatch capability — it does NOT read state.
    // It should NOT appear in reads_state.
    let dispatch_read = deps.reads_state.iter().find(|d| d.callee == "useDispatch");
    assert!(
        dispatch_read.is_none(),
        "useDispatch should NOT be a state reader, but found in reads_state: {:?}",
        deps.reads_state
    );
}

#[test]
fn test_rtk_query_hook_as_state_reader() {
    let source = r"
function PostList() {
    const { data } = useGetPostsQuery();
    return <ul />;
}
";
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let query_read = deps
        .reads_state
        .iter()
        .find(|d| d.callee == "useGetPostsQuery");
    assert!(
        query_read.is_some(),
        "expected useGetPostsQuery as state reader, got: {:?}",
        deps.reads_state
    );
}

#[test]
fn test_dispatch_inside_callback_attributed_to_component() {
    let source = r"
function LoginForm() {
    const dispatch = useDispatch();
    const handleSubmit = async (e) => {
        e.preventDefault();
        dispatch(loginUser({ email, password }));
    };
    return <form />;
}
";
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);

    // dispatch(loginUser(...)) is inside handleSubmit arrow, but since handleSubmit
    // is not an entity, it should bubble up to the enclosing LoginForm scope.
    let login_dispatch = deps.dispatches.iter().find(|d| d.callee == "loginUser");
    assert!(
        login_dispatch.is_some(),
        "expected dispatch of loginUser, got: {:?}",
        deps.dispatches
    );
    assert_eq!(
        login_dispatch.unwrap().caller_entity,
        "LoginForm",
        "dispatch inside handleSubmit should be attributed to LoginForm, not handleSubmit"
    );
}

#[test]
fn test_use_selector_inside_use_effect_attributed_to_component() {
    let source = r"
function Dashboard() {
    const user = useSelector(selectUser);
    useEffect(() => {
        dispatch(fetchPosts());
    }, []);
    return <PostList />;
}
";
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);

    // useSelector at component level should be attributed to Dashboard
    let selector_read = deps.reads_state.iter().find(|d| d.callee == "useSelector");
    assert!(
        selector_read.is_some(),
        "expected useSelector, got: {:?}",
        deps.reads_state
    );
    assert_eq!(selector_read.unwrap().caller_entity, "Dashboard");

    // dispatch inside useEffect callback should also be attributed to Dashboard
    let fetch_dispatch = deps.dispatches.iter().find(|d| d.callee == "fetchPosts");
    assert!(
        fetch_dispatch.is_some(),
        "expected dispatch of fetchPosts, got: {:?}",
        deps.dispatches
    );
    assert_eq!(
        fetch_dispatch.unwrap().caller_entity,
        "Dashboard",
        "dispatch inside useEffect callback should be attributed to Dashboard"
    );
}

#[test]
fn test_dispatch_object_literal_not_extracted() {
    let source = r#"
function MyComponent() {
    dispatch({ type: "INCREMENT", payload: 1 });
    return <div />;
}
"#;
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    // Object literal arguments should NOT produce a dispatch target —
    // only action creator calls (dispatch(actionCreator())) or identifier
    // references (dispatch(someAction)) are valid targets.
    let noisy = deps.dispatches.iter().find(|d| d.callee == "type");
    assert!(
        noisy.is_none(),
        "object literal dispatch should not extract 'type' as target, got: {:?}",
        deps.dispatches
    );
}

#[test]
fn test_use_selector_import_alias_extracts_selector_name() {
    let source = r#"
import { useSelector as useReduxSelector } from "react-redux";

function ProfilePage() {
    const user = useReduxSelector(selectUser);
    return <div>{user.name}</div>;
}
"#;
    let deps = extract_deps_with_paradigms(
        Path::new("app/profile/page.tsx"),
        source,
        Language::TYPESCRIPT,
    );
    let callees: Vec<&str> = deps.reads_state.iter().map(|d| d.callee.as_str()).collect();
    assert!(
        callees.contains(&"useReduxSelector"),
        "expected aliased selector hook in reads_state, got: {:?}",
        callees
    );
    assert!(
        callees.contains(&"selectUser"),
        "expected selector function in reads_state, got: {:?}",
        callees
    );
}

#[test]
fn test_dispatch_alias_variable_from_import_alias() {
    let source = r#"
import { useDispatch as useReduxDispatch } from "react-redux";

function LoginForm() {
    const appDispatch = useReduxDispatch();
    appDispatch(loginUser({ email, password }));
    return <form />;
}
"#;
    let deps = extract_deps_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let login_dispatch = deps.dispatches.iter().find(|d| d.callee == "loginUser");
    assert!(
        login_dispatch.is_some(),
        "expected dispatch target loginUser from aliased dispatch variable, got: {:?}",
        deps.dispatches
    );
    assert_eq!(login_dispatch.unwrap().caller_entity, "LoginForm");
}

#[test]
fn test_destructured_hooks_require_identifier_rhs() {
    // Destructuring from a function call should NOT produce hook entities —
    // only destructuring from a plain identifier (API object) should.
    let source = r"
export const { useFoo, useBar } = createSomething();
";
    let entities =
        rpg_parser::entities::extract_entities(Path::new("test.ts"), source, Language::TYPESCRIPT);
    let hooks: Vec<&str> = entities
        .iter()
        .filter(|e| e.kind == rpg_core::graph::EntityKind::Hook)
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        hooks.is_empty(),
        "destructuring from function call should not produce hook entities, got: {:?}",
        hooks
    );
}
