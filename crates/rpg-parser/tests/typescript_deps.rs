use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_ts_named_import() {
    let source = "import { foo, bar } from './module';";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TypeScript);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "./module");
    assert!(deps.imports[0].symbols.contains(&"foo".to_string()));
    assert!(deps.imports[0].symbols.contains(&"bar".to_string()));
    assert_eq!(deps.imports[0].symbols.len(), 2);
}

#[test]
fn test_ts_namespace_import() {
    let source = "import * as utils from 'utils';";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TypeScript);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "utils");
}

#[test]
fn test_ts_default_import() {
    let source = "import React from 'react';";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TypeScript);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "react");
}

#[test]
fn test_ts_class_inheritance() {
    let source = "class Dog extends Animal {}";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TypeScript);
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
    let deps = extract_deps(Path::new("test.ts"), source, Language::TypeScript);
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
    let deps = extract_deps(Path::new("test.ts"), "", Language::TypeScript);
    assert!(deps.imports.is_empty());
    assert!(deps.calls.is_empty());
    assert!(deps.inherits.is_empty());
    assert!(deps.composes.is_empty());
}

#[test]
fn test_ts_multiple_imports() {
    let source = "\
import { useState } from 'react';
import { Router } from 'react-router';
import axios from 'axios';
";
    let deps = extract_deps(Path::new("test.ts"), source, Language::TypeScript);
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
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TypeScript);
    // Should parse without errors — imports and calls still extracted
    assert!(!deps.imports.is_empty());
}

#[test]
fn test_barrel_reexport_named() {
    let source = "export { Foo, Bar } from './foo';";
    let deps = extract_deps(Path::new("index.ts"), source, Language::TypeScript);
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
    let deps = extract_deps(Path::new("index.ts"), source, Language::TypeScript);
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
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TypeScript);
    let button_call = deps.calls.iter().find(|c| c.callee == "Button");
    assert!(
        button_call.is_some(),
        "expected a call to 'Button' from JSX usage, got: {:?}",
        deps.calls
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
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TypeScript);
    let icon_call = deps.calls.iter().find(|c| c.callee == "Icon");
    assert!(
        icon_call.is_some(),
        "expected a call to 'Icon' from self-closing JSX, got: {:?}",
        deps.calls
    );
}

#[test]
fn test_jsx_html_element_ignored() {
    let source = r"
function App() {
    return <div><span>text</span></div>;
}
";
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TypeScript);
    let html_calls: Vec<_> = deps
        .calls
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
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TypeScript);
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
    let button_call = deps.calls.iter().find(|c| c.callee == "Button");
    assert!(
        button_call.is_some(),
        "expected JSX call to Button in arrow function"
    );
    assert_eq!(button_call.unwrap().caller_entity, "App");
}

#[test]
fn test_barrel_reexport_aliased() {
    let source = "export { default as Foo } from './mod';";
    let deps = extract_deps(Path::new("index.ts"), source, Language::TypeScript);
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
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TypeScript);
    // Dotted component: extracts last segment for resolution
    let route_call = deps.calls.iter().find(|c| c.callee == "Route");
    assert!(
        route_call.is_some(),
        "expected call to 'Route' from <Router.Route />, got: {:?}",
        deps.calls
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
    let deps = extract_deps(Path::new("test.tsx"), source, Language::TypeScript);
    let component_calls: Vec<&str> = deps.calls.iter().map(|c| c.callee.as_str()).collect();
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
