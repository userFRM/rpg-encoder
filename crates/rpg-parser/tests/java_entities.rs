use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn java_extract_class() {
    let source = "public class Foo { }";
    let entities = extract_entities(Path::new("Foo.java"), source, Language::JAVA);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Foo");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn java_extract_class_with_method() {
    let source = "public class Foo { public void bar() {} }";
    let entities = extract_entities(Path::new("Foo.java"), source, Language::JAVA);
    assert_eq!(entities.len(), 2);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let method = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn java_extract_interface() {
    let source = "public interface Runnable { void run(); }";
    let entities = extract_entities(Path::new("Runnable.java"), source, Language::JAVA);
    // Interface itself should be extracted as Class
    let iface = entities.iter().find(|e| e.name == "Runnable").unwrap();
    assert_eq!(iface.kind, EntityKind::Class);
}

#[test]
fn java_extract_enum() {
    let source = "public enum Color { RED, GREEN, BLUE }";
    let entities = extract_entities(Path::new("Color.java"), source, Language::JAVA);
    let en = entities.iter().find(|e| e.name == "Color").unwrap();
    assert_eq!(en.kind, EntityKind::Class);
}

#[test]
fn java_extract_class_with_constructor() {
    let source = "public class Foo { public Foo() {} }";
    let entities = extract_entities(Path::new("Foo.java"), source, Language::JAVA);
    assert_eq!(entities.len(), 2);

    let class = entities
        .iter()
        .find(|e| e.name == "Foo" && e.kind == EntityKind::Class)
        .unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let ctor = entities
        .iter()
        .find(|e| e.kind == EntityKind::Method)
        .unwrap();
    assert_eq!(ctor.name, "Foo");
    assert_eq!(ctor.parent_class.as_deref(), Some("Foo"));
}
