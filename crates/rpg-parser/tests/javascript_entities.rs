use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_function_declaration() {
    let source = "function hello() { return 1; }";
    let entities = extract_entities(Path::new("test.js"), source, Language::JavaScript);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "hello");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert_eq!(entities[0].line_start, 1);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_class_with_method() {
    let source = "\
class Dog {
    bark() {}
}
";
    let entities = extract_entities(Path::new("test.js"), source, Language::JavaScript);
    assert!(
        entities.len() >= 2,
        "expected at least 2 entities, got {}",
        entities.len()
    );
    let class_entity = entities
        .iter()
        .find(|e| e.name == "Dog")
        .expect("missing Dog class");
    assert_eq!(class_entity.kind, EntityKind::Class);

    let method_entity = entities
        .iter()
        .find(|e| e.name == "bark")
        .expect("missing bark method");
    assert_eq!(method_entity.kind, EntityKind::Method);
    assert_eq!(method_entity.parent_class.as_deref(), Some("Dog"));
}

#[test]
fn test_const_arrow_function() {
    let source = "const square = (x) => x * x;";
    let entities = extract_entities(Path::new("test.js"), source, Language::JavaScript);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "square");
    assert_eq!(entities[0].kind, EntityKind::Function);
}

#[test]
fn test_multiple_functions() {
    let source = "\
function alpha() {}

function beta() { return 2; }

function gamma(x) { return x; }
";
    let entities = extract_entities(Path::new("test.js"), source, Language::JavaScript);
    assert_eq!(entities.len(), 3);
    let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));
    assert!(entities.iter().all(|e| e.kind == EntityKind::Function));
}

#[test]
fn test_class_with_constructor() {
    let source = "\
class Person {
    constructor(name) {
        this.name = name;
    }
    greet() {
        return this.name;
    }
}
";
    let entities = extract_entities(Path::new("test.js"), source, Language::JavaScript);
    assert!(
        entities.len() >= 3,
        "expected at least 3 entities, got {}",
        entities.len()
    );
    let class_entity = entities
        .iter()
        .find(|e| e.name == "Person")
        .expect("missing Person class");
    assert_eq!(class_entity.kind, EntityKind::Class);

    let ctor = entities
        .iter()
        .find(|e| e.name == "constructor")
        .expect("missing constructor");
    assert_eq!(ctor.kind, EntityKind::Method);
    assert_eq!(ctor.parent_class.as_deref(), Some("Person"));

    let greet = entities
        .iter()
        .find(|e| e.name == "greet")
        .expect("missing greet method");
    assert_eq!(greet.kind, EntityKind::Method);
    assert_eq!(greet.parent_class.as_deref(), Some("Person"));
}
