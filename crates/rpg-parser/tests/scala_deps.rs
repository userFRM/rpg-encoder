use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn scala_import_declaration() {
    let source = r"import scala.collection.mutable.ListBuffer

class Foo { }
";
    let deps = extract_deps(Path::new("Foo.scala"), source, Language::SCALA);
    assert!(!deps.imports.is_empty());
    let import = &deps.imports[0];
    assert_eq!(import.module, "scala.collection.mutable.ListBuffer");
}

#[test]
fn scala_class_extends() {
    let source = "class Dog extends Animal { }";
    let deps = extract_deps(Path::new("Dog.scala"), source, Language::SCALA);
    assert!(!deps.inherits.is_empty());
    let inherit = deps
        .inherits
        .iter()
        .find(|i| i.parent_class == "Animal")
        .unwrap();
    assert_eq!(inherit.child_class, "Dog");
    assert_eq!(inherit.parent_class, "Animal");
}

#[test]
fn scala_multi_trait_inheritance() {
    let source = "class MyService extends Base with Logging with Serializable { }";
    let deps = extract_deps(Path::new("MyService.scala"), source, Language::SCALA);
    assert!(
        deps.inherits.len() >= 3,
        "expected at least 3 inheritance entries for extends + 2 with traits, got: {:?}",
        deps.inherits
    );
    let parents: Vec<&str> = deps
        .inherits
        .iter()
        .map(|i| i.parent_class.as_str())
        .collect();
    assert!(
        parents.contains(&"Base"),
        "expected 'Base' in parents, got: {:?}",
        parents
    );
    assert!(
        parents.contains(&"Logging"),
        "expected 'Logging' in parents, got: {:?}",
        parents
    );
    assert!(
        parents.contains(&"Serializable"),
        "expected 'Serializable' in parents, got: {:?}",
        parents
    );
    // All should be for MyService
    for inherit in &deps.inherits {
        assert_eq!(inherit.child_class, "MyService");
    }
}
