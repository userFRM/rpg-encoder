use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn ruby_extract_class() {
    let source = "class Foo\nend";
    let entities = extract_entities(Path::new("foo.rb"), source, Language::RUBY);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Foo");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn ruby_extract_class_with_method() {
    let source = "class Foo\n  def bar\n  end\nend";
    let entities = extract_entities(Path::new("foo.rb"), source, Language::RUBY);
    assert_eq!(entities.len(), 2);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let method = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn ruby_extract_module() {
    let source = "module MyModule\nend";
    let entities = extract_entities(Path::new("my_module.rb"), source, Language::RUBY);
    let module = entities.iter().find(|e| e.name == "MyModule").unwrap();
    assert_eq!(module.kind, EntityKind::Class);
}

#[test]
fn ruby_extract_top_level_method() {
    let source = "def hello\nend";
    let entities = extract_entities(Path::new("hello.rb"), source, Language::RUBY);
    let func = entities.iter().find(|e| e.name == "hello").unwrap();
    assert_eq!(func.kind, EntityKind::Function);
}

#[test]
fn ruby_extract_singleton_method() {
    let source = "class Foo\n  def self.bar\n  end\nend";
    let entities = extract_entities(Path::new("foo.rb"), source, Language::RUBY);
    let singleton = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(singleton.kind, EntityKind::Method);
    assert_eq!(singleton.parent_class.as_deref(), Some("Foo"));
}
