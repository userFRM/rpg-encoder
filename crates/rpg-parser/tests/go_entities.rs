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
