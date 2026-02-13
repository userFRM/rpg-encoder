use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn go_extract_function() {
    let source = r#"package main

import "fmt"

func main() { fmt.Println("hello") }
"#;
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "main");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn go_extract_function_with_params() {
    let source = r"package main

func add(a int, b int) int { return a + b }
";
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "add");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn go_signature_typed_params_and_return() {
    let source = r"package main

func compute(x int, y string) bool { return true }
";
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.parameters[0].name, "x");
    assert_eq!(sig.parameters[0].type_annotation.as_deref(), Some("int"));
    assert_eq!(sig.parameters[1].name, "y");
    assert_eq!(sig.parameters[1].type_annotation.as_deref(), Some("string"));
    assert_eq!(sig.return_type.as_deref(), Some("bool"));
}

#[test]
fn go_signature_no_return_type() {
    let source = r"package main

func greet(name string) { }
";
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    assert_eq!(sig.parameters.len(), 1);
    assert_eq!(sig.parameters[0].name, "name");
    assert!(sig.return_type.is_none());
}

#[test]
fn go_signature_grouped_params() {
    let source = r"package main

func add(a, b int) int { return a + b }
";
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    // Grouped params: `a, b int` should yield 2 params both with type `int`
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.parameters[0].name, "a");
    assert_eq!(sig.parameters[0].type_annotation.as_deref(), Some("int"));
    assert_eq!(sig.parameters[1].name, "b");
    assert_eq!(sig.parameters[1].type_annotation.as_deref(), Some("int"));
}

#[test]
fn go_extract_method_with_receiver() {
    let source = r"package main

func (s *Server) Start() error { return nil }
";
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Start");
    assert_eq!(entities[0].kind, EntityKind::Method);
    assert_eq!(entities[0].parent_class.as_deref(), Some("Server"));
}

#[test]
fn go_extract_struct_type() {
    let source = r"package main

type Config struct {
    Port int
}
";
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Config");
    assert_eq!(entities[0].kind, EntityKind::Class);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn go_extract_interface_type() {
    let source = r"package main

type Reader interface {
    Read(p []byte) (n int, err error)
}
";
    let entities = extract_entities(Path::new("test.go"), source, Language::GO);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Reader");
    assert_eq!(entities[0].kind, EntityKind::Class);
    assert!(entities[0].parent_class.is_none());
}
