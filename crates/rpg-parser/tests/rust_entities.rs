use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_rust_entities;
use std::path::Path;

#[test]
fn test_simple_function() {
    let source = "fn main() {\n    println!(\"hello\");\n}\n";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "main");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_struct() {
    let source = "\
pub struct Config {
    pub name: String,
    pub value: i32,
}
";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Config");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn test_enum() {
    let source = "\
enum Color {
    Red,
    Green,
    Blue,
}
";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Color");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn test_impl_with_methods() {
    let source = "\
struct Foo;

impl Foo {
    fn new() -> Self {
        Foo
    }

    fn do_work(&self) {
        // work
    }
}
";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert_eq!(entities.len(), 3); // struct + 2 methods
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Foo" && e.kind == EntityKind::Class)
    );
    assert!(entities.iter().any(|e| e.name == "new"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Foo")));
    assert!(entities.iter().any(|e| e.name == "do_work"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Foo")));
}

#[test]
fn test_trait_with_default_method() {
    let source = "\
trait Greetable {
    fn greet(&self) {
        println!(\"hello\");
    }
}
";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert_eq!(entities.len(), 2); // trait + default method
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Greetable" && e.kind == EntityKind::Class)
    );
    assert!(entities.iter().any(|e| e.name == "greet"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Greetable")));
}

#[test]
fn test_empty_file() {
    let source = "";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert!(entities.is_empty());
}

#[test]
fn test_multiple_top_level_functions() {
    let source = "\
fn alpha() {}
fn beta() {}
fn gamma() {}
";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert_eq!(entities.len(), 3);
    assert!(entities.iter().all(|e| e.kind == EntityKind::Function));
}

#[test]
fn test_mixed_types() {
    let source = "\
struct Point { x: f64, y: f64 }
enum Shape { Circle, Square }
trait Drawable { fn draw(&self); }
fn render() {}
";
    let entities = extract_rust_entities(Path::new("test.rs"), source);
    assert_eq!(entities.len(), 4);
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Point" && e.kind == EntityKind::Class)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Shape" && e.kind == EntityKind::Class)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Drawable" && e.kind == EntityKind::Class)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "render" && e.kind == EntityKind::Function)
    );
}
