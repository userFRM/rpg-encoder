use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn go_single_import() {
    let source = r#"package main

import "fmt"

func main() {}
"#;
    let deps = extract_deps(Path::new("test.go"), source, Language::Go);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "fmt");
    assert!(deps.imports[0].symbols.is_empty());
}

#[test]
fn go_multiple_imports() {
    let source = r#"package main

import (
	"fmt"
	"os"
)

func main() {}
"#;
    let deps = extract_deps(Path::new("test.go"), source, Language::Go);
    assert_eq!(deps.imports.len(), 2);
    let modules: Vec<&str> = deps.imports.iter().map(|i| i.module.as_str()).collect();
    assert!(modules.contains(&"fmt"));
    assert!(modules.contains(&"os"));
}

#[test]
fn go_call_extraction() {
    let source = r#"package main

import "fmt"

func main() {
    fmt.Println("test")
}
"#;
    let deps = extract_deps(Path::new("test.go"), source, Language::Go);
    assert!(!deps.calls.is_empty());
    let callees: Vec<&str> = deps.calls.iter().map(|c| c.callee.as_str()).collect();
    assert!(callees.contains(&"Println"));
}
