use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn php_extract_class() {
    let source = "<?php class Foo { }";
    let entities = extract_entities(Path::new("Foo.php"), source, Language::PHP);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Foo");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn php_extract_class_with_method() {
    let source = "<?php class Foo { public function bar() { } }";
    let entities = extract_entities(Path::new("Foo.php"), source, Language::PHP);
    assert_eq!(entities.len(), 2);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let method = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn php_extract_function() {
    let source = "<?php function hello() { }";
    let entities = extract_entities(Path::new("hello.php"), source, Language::PHP);
    let func = entities.iter().find(|e| e.name == "hello").unwrap();
    assert_eq!(func.kind, EntityKind::Function);
}

#[test]
fn php_extract_trait() {
    let source = "<?php trait Loggable { }";
    let entities = extract_entities(Path::new("Loggable.php"), source, Language::PHP);
    let tr = entities.iter().find(|e| e.name == "Loggable").unwrap();
    assert_eq!(tr.kind, EntityKind::Class);
}

#[test]
fn php_extract_interface() {
    let source = "<?php interface Printable { public function printIt(); }";
    let entities = extract_entities(Path::new("Printable.php"), source, Language::PHP);
    let iface = entities.iter().find(|e| e.name == "Printable").unwrap();
    assert_eq!(iface.kind, EntityKind::Class);
}
