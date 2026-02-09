use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn bash_source_import() {
    let source = r"source ./utils.sh
hello";
    let deps = extract_deps(Path::new("test.sh"), source, Language::BASH);
    assert!(!deps.imports.is_empty());
    let import = &deps.imports[0];
    assert_eq!(import.module, "./utils.sh");
}

#[test]
fn bash_dot_source_import() {
    let source = r". ./lib.sh
world";
    let deps = extract_deps(Path::new("test.sh"), source, Language::BASH);
    assert!(!deps.imports.is_empty());
    let import = &deps.imports[0];
    assert_eq!(import.module, "./lib.sh");
}

#[test]
fn bash_function_call() {
    let source = r#"function greet() {
  echo "hi"
}
greet"#;
    let deps = extract_deps(Path::new("test.sh"), source, Language::BASH);
    assert!(!deps.calls.is_empty());
    let callees: Vec<&str> = deps.calls.iter().map(|c| c.callee.as_str()).collect();
    assert!(callees.contains(&"greet"));
}

#[test]
fn bash_quoted_source() {
    let source = r#"#!/bin/bash
source "./utils.sh"
. './helpers.sh'
"#;
    let deps = extract_deps(Path::new("main.sh"), source, Language::BASH);
    // Should strip quotes from paths
    let modules: Vec<&str> = deps.imports.iter().map(|i| i.module.as_str()).collect();
    assert!(
        modules.contains(&"./utils.sh"),
        "should strip double quotes from source path, got: {:?}",
        modules
    );
    assert!(
        modules.contains(&"./helpers.sh"),
        "should strip single quotes from dot-source path, got: {:?}",
        modules
    );
}

#[test]
fn bash_dot_source_absolute_path() {
    let source = ". /etc/profile.d/custom.sh\necho done";
    let deps = extract_deps(Path::new("init.sh"), source, Language::BASH);
    assert!(
        !deps.imports.is_empty(),
        "dot-source with absolute path should produce an import"
    );
    let import = &deps.imports[0];
    assert_eq!(import.module, "/etc/profile.d/custom.sh");
}
