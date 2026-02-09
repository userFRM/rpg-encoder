use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_es_named_import() {
    let source = "import { foo, bar } from './module';";
    let deps = extract_deps(Path::new("test.js"), source, Language::JAVASCRIPT);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "./module");
    assert!(deps.imports[0].symbols.contains(&"foo".to_string()));
    assert!(deps.imports[0].symbols.contains(&"bar".to_string()));
    assert_eq!(deps.imports[0].symbols.len(), 2);
}

#[test]
fn test_default_import() {
    let source = "import React from 'react';";
    let deps = extract_deps(Path::new("test.js"), source, Language::JAVASCRIPT);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "react");
}

#[test]
fn test_call_extraction() {
    let source = "\
function main() {
    console.log(\"hi\");
}
";
    let deps = extract_deps(Path::new("test.js"), source, Language::JAVASCRIPT);
    assert!(!deps.calls.is_empty(), "expected at least one call dep");
    let log_call = deps.calls.iter().find(|c| c.callee == "log");
    assert!(
        log_call.is_some(),
        "expected a call to 'log', got: {:?}",
        deps.calls
    );
    assert_eq!(log_call.unwrap().caller_entity, "main");
}

#[test]
fn test_class_inheritance() {
    let source = "class Dog extends Animal {}";
    let deps = extract_deps(Path::new("test.js"), source, Language::JAVASCRIPT);
    assert!(
        !deps.inherits.is_empty(),
        "expected at least one inherit dep"
    );
    assert_eq!(deps.inherits[0].child_class, "Dog");
    assert_eq!(deps.inherits[0].parent_class, "Animal");
}

#[test]
fn test_js_barrel_reexport() {
    let source = "export { Foo } from './foo';";
    let deps = extract_deps(Path::new("index.js"), source, Language::JAVASCRIPT);
    assert_eq!(deps.composes.len(), 1);
    assert_eq!(deps.composes[0].target_name, "Foo");
}

#[test]
fn test_jsx_component_usage() {
    let source = r"
function App() {
    return <Button />;
}
";
    // JSX renders are now extracted by the TOML paradigm dep pipeline, not the base extractor.
    // Wire the paradigm engine to verify end-to-end.
    let mut deps = extract_deps(Path::new("test.jsx"), source, Language::JAVASCRIPT);
    let defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap();
    let qcache = rpg_parser::paradigms::query_engine::QueryCache::compile_all(&defs).unwrap();
    let active: Vec<&_> = defs.iter().collect(); // all paradigms active for unit test
    let scopes = rpg_parser::deps::build_scopes(source, Language::JAVASCRIPT);
    rpg_parser::paradigms::query_engine::execute_dep_queries(
        &qcache,
        &active,
        Path::new("test.jsx"),
        source,
        Language::JAVASCRIPT,
        &scopes,
        &mut deps,
    );
    let button_call = deps.renders.iter().find(|c| c.callee == "Button");
    assert!(
        button_call.is_some(),
        "expected JSX component render in .jsx file, got: {:?}",
        deps.renders
    );
    assert_eq!(button_call.unwrap().caller_entity, "App");
}
