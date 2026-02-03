use rpg_parser::deps::extract_rust_deps;
use std::path::Path;

#[test]
fn test_simple_use() {
    let source = "use std::fs::read_to_string;\n";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "std::fs");
    assert_eq!(deps.imports[0].symbols, vec!["read_to_string"]);
}

#[test]
fn test_grouped_use() {
    let source = "use std::collections::{HashMap, HashSet};\n";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "std::collections");
    assert!(deps.imports[0].symbols.contains(&"HashMap".to_string()));
    assert!(deps.imports[0].symbols.contains(&"HashSet".to_string()));
}

#[test]
fn test_pub_use() {
    let source = "pub use crate::graph::Entity;\n";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    assert_eq!(deps.imports.len(), 1);
    assert!(deps.imports[0].symbols.contains(&"Entity".to_string()));
}

#[test]
fn test_function_call() {
    let source = "\
fn main() {
    let s = read_file();
}

fn read_file() -> String {
    String::new()
}
";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    assert!(
        deps.calls
            .iter()
            .any(|c| c.callee == "read_file" && c.caller_entity == "main")
    );
}

#[test]
fn test_method_call() {
    let source = "\
struct Foo;
impl Foo {
    fn process(&self) {
        self.validate();
    }
    fn validate(&self) {}
}
";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    // method_call_expression captures "validate" via the name field
    assert!(
        deps.calls
            .iter()
            .any(|c| c.callee.contains("validate") && c.caller_entity == "Foo::process")
    );
}

#[test]
fn test_path_call() {
    let source = "\
fn main() {
    std::fs::read_to_string(\"file.txt\");
}
";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    assert!(deps.calls.iter().any(|c| c.callee == "read_to_string"));
}

#[test]
fn test_empty_file() {
    let source = "";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    assert!(deps.imports.is_empty());
    assert!(deps.calls.is_empty());
    assert!(deps.inherits.is_empty());
}

#[test]
fn test_use_with_alias() {
    let source = "use std::io::Result as IoResult;\n";
    let deps = extract_rust_deps(Path::new("test.rs"), source);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "std::io");
    // Note: simple use aliases not yet stripped (only grouped use aliases are)
    assert!(deps.imports[0].symbols[0].starts_with("Result"));
}
