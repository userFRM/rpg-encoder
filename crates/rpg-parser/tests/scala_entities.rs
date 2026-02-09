use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn scala_extract_class() {
    let source = "class Foo { }";
    let entities = extract_entities(Path::new("Foo.scala"), source, Language::SCALA);
    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);
}

#[test]
fn scala_extract_object() {
    let source = "object Foo { }";
    let entities = extract_entities(Path::new("Foo.scala"), source, Language::SCALA);
    let obj = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(obj.kind, EntityKind::Class);
}

#[test]
fn scala_extract_trait() {
    let source = "trait Drawable { }";
    let entities = extract_entities(Path::new("Drawable.scala"), source, Language::SCALA);
    let tr = entities.iter().find(|e| e.name == "Drawable").unwrap();
    assert_eq!(tr.kind, EntityKind::Class);
}

#[test]
fn scala_extract_class_with_method() {
    let source = r"class Foo {
  def bar(): Unit = { }
}";
    let entities = extract_entities(Path::new("Foo.scala"), source, Language::SCALA);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let method = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn scala_extract_function() {
    let source = r"object Main {
  def hello(): Unit = { }
}";
    let entities = extract_entities(Path::new("Main.scala"), source, Language::SCALA);
    // Top-level def in Scala must be inside an object; it becomes a Method
    let func = entities.iter().find(|e| e.name == "hello").unwrap();
    assert_eq!(func.kind, EntityKind::Method);
    assert_eq!(func.parent_class.as_deref(), Some("Main"));
}
