use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_es_named_import() {
    let source = "import { foo, bar } from './module';";
    let deps = extract_deps(Path::new("test.js"), source, Language::JavaScript);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "./module");
    assert!(deps.imports[0].symbols.contains(&"foo".to_string()));
    assert!(deps.imports[0].symbols.contains(&"bar".to_string()));
    assert_eq!(deps.imports[0].symbols.len(), 2);
}

#[test]
fn test_default_import() {
    let source = "import React from 'react';";
    let deps = extract_deps(Path::new("test.js"), source, Language::JavaScript);
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
    let deps = extract_deps(Path::new("test.js"), source, Language::JavaScript);
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
    let deps = extract_deps(Path::new("test.js"), source, Language::JavaScript);
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
    let deps = extract_deps(Path::new("index.js"), source, Language::JavaScript);
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
    let deps = extract_deps(Path::new("test.jsx"), source, Language::JavaScript);
    let button_call = deps.calls.iter().find(|c| c.callee == "Button");
    assert!(
        button_call.is_some(),
        "expected JSX component call in .jsx file, got: {:?}",
        deps.calls
    );
    assert_eq!(button_call.unwrap().caller_entity, "App");
}
