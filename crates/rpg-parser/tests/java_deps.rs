use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn java_import_declaration() {
    let source = r"import java.util.List;

public class Foo {}
";
    let deps = extract_deps(Path::new("Foo.java"), source, Language::JAVA);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "java.util");
    assert_eq!(deps.imports[0].symbols, vec!["List"]);
}

#[test]
fn java_class_inheritance() {
    let source = "public class Dog extends Animal {}";
    let deps = extract_deps(Path::new("Dog.java"), source, Language::JAVA);
    assert!(!deps.inherits.is_empty());
    let inherit = &deps.inherits[0];
    assert_eq!(inherit.child_class, "Dog");
    assert_eq!(inherit.parent_class, "Animal");
}

#[test]
fn java_interface_implementation() {
    let source = "public class Foo implements Bar {}";
    let deps = extract_deps(Path::new("Foo.java"), source, Language::JAVA);
    assert!(!deps.inherits.is_empty());
    let inherit = deps
        .inherits
        .iter()
        .find(|i| i.parent_class == "Bar")
        .unwrap();
    assert_eq!(inherit.child_class, "Foo");
    assert_eq!(inherit.parent_class, "Bar");
}
