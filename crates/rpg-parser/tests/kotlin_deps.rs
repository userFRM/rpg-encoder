use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn kotlin_import_declaration() {
    let source = r"import kotlin.collections.List

class Foo { }
";
    let deps = extract_deps(Path::new("Foo.kt"), source, Language::KOTLIN);
    assert!(!deps.imports.is_empty());
    let import = &deps.imports[0];
    assert_eq!(import.module, "kotlin.collections");
    assert_eq!(import.symbols, vec!["List"]);
}

#[test]
fn kotlin_class_inheritance() {
    let source = "class Dog : Animal() { }";
    let deps = extract_deps(Path::new("Dog.kt"), source, Language::KOTLIN);
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
fn kotlin_interface_method_call() {
    let source = r#"interface Validator {
    fun validate() {
        println("validating")
    }
}"#;
    let deps = extract_deps(Path::new("Validator.kt"), source, Language::KOTLIN);
    let call = deps.calls.iter().find(|c| c.callee == "println");
    assert!(
        call.is_some(),
        "should find println call inside interface method"
    );
    assert_eq!(call.unwrap().caller_entity, "Validator.validate");
}
