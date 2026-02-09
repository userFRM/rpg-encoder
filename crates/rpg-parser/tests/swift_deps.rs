use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn swift_import_declaration() {
    let source = r"import Foundation

class Foo { }
";
    let deps = extract_deps(Path::new("Foo.swift"), source, Language::SWIFT);
    assert!(!deps.imports.is_empty());
    let import = &deps.imports[0];
    assert_eq!(import.module, "Foundation");
}

#[test]
fn swift_class_inheritance() {
    let source = "class Dog: Animal { }";
    let deps = extract_deps(Path::new("Dog.swift"), source, Language::SWIFT);
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
fn swift_extension_method_call() {
    let source = r#"extension String {
    func greet() {
        print("hello")
    }
}"#;
    let deps = extract_deps(Path::new("Ext.swift"), source, Language::SWIFT);
    let call = deps.calls.iter().find(|c| c.callee == "print");
    assert!(
        call.is_some(),
        "should find print call inside extension method"
    );
    assert_eq!(call.unwrap().caller_entity, "String.greet");
}

#[test]
fn swift_multi_protocol_conformance() {
    let source = "class MyView: UIView, UITableViewDelegate, UITableViewDataSource { }";
    let deps = extract_deps(Path::new("MyView.swift"), source, Language::SWIFT);
    assert!(
        deps.inherits.len() >= 2,
        "should find at least 2 parent types, got: {:?}",
        deps.inherits
    );
}
