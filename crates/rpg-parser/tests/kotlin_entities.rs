use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn kotlin_extract_class() {
    let source = "class Foo { }";
    let entities = extract_entities(Path::new("Foo.kt"), source, Language::KOTLIN);
    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);
}

#[test]
fn kotlin_extract_class_with_method() {
    let source = r"class Foo {
    fun bar() { }
}";
    let entities = extract_entities(Path::new("Foo.kt"), source, Language::KOTLIN);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let method = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn kotlin_extract_function() {
    let source = "fun hello() { }";
    let entities = extract_entities(Path::new("test.kt"), source, Language::KOTLIN);
    let func = entities.iter().find(|e| e.name == "hello").unwrap();
    assert_eq!(func.kind, EntityKind::Function);
    assert!(func.parent_class.is_none());
}

#[test]
fn kotlin_extract_object() {
    let source = "object Singleton { }";
    let entities = extract_entities(Path::new("Singleton.kt"), source, Language::KOTLIN);
    let obj = entities.iter().find(|e| e.name == "Singleton").unwrap();
    assert_eq!(obj.kind, EntityKind::Class);
}

#[test]
fn kotlin_extract_interface() {
    let source = "interface Drawable { fun draw() }";
    let entities = extract_entities(Path::new("Drawable.kt"), source, Language::KOTLIN);
    let iface = entities.iter().find(|e| e.name == "Drawable").unwrap();
    assert_eq!(iface.kind, EntityKind::Class);
}
