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
