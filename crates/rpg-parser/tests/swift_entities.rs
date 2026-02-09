use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn swift_extract_class() {
    let source = "class Foo { }";
    let entities = extract_entities(Path::new("Foo.swift"), source, Language::SWIFT);
    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);
}

#[test]
fn swift_extract_struct() {
    let source = "struct Point { }";
    let entities = extract_entities(Path::new("Point.swift"), source, Language::SWIFT);
    let s = entities.iter().find(|e| e.name == "Point").unwrap();
    assert_eq!(s.kind, EntityKind::Class);
}

#[test]
fn swift_extract_class_with_method() {
    let source = r"class Foo {
    func bar() { }
}";
    let entities = extract_entities(Path::new("Foo.swift"), source, Language::SWIFT);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let method = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn swift_extract_function() {
    let source = "func hello() { }";
    let entities = extract_entities(Path::new("test.swift"), source, Language::SWIFT);
    let func = entities.iter().find(|e| e.name == "hello").unwrap();
    assert_eq!(func.kind, EntityKind::Function);
    assert!(func.parent_class.is_none());
}

#[test]
fn swift_extract_protocol() {
    let source = r"protocol Drawable {
    func draw()
}";
    let entities = extract_entities(Path::new("Drawable.swift"), source, Language::SWIFT);
    let proto = entities.iter().find(|e| e.name == "Drawable").unwrap();
    assert_eq!(proto.kind, EntityKind::Class);
}

#[test]
fn swift_extract_init() {
    let source = r"class Foo {
    init() { }
}";
    let entities = extract_entities(Path::new("Foo.swift"), source, Language::SWIFT);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let init = entities.iter().find(|e| e.name == "init").unwrap();
    assert_eq!(init.kind, EntityKind::Method);
    assert_eq!(init.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn swift_extract_enum() {
    let source = "enum Color { case red, green, blue }";
    let entities = extract_entities(Path::new("Color.swift"), source, Language::SWIFT);
    let en = entities.iter().find(|e| e.name == "Color").unwrap();
    assert_eq!(en.kind, EntityKind::Class);
}
