use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_extract_cpp_function() {
    let source = "int main() { return 0; }\n";
    let entities = extract_entities(Path::new("test.cpp"), source, Language::CPP);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "main");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_extract_cpp_class_with_method() {
    let source = "class Foo { public: void bar() {} };\n";
    let entities = extract_entities(Path::new("test.cpp"), source, Language::CPP);
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Foo" && e.kind == EntityKind::Class)
    );
    assert!(entities.iter().any(|e| e.name == "bar"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Foo")));
}

#[test]
fn test_extract_cpp_struct() {
    let source = "struct Point { int x; int y; };\n";
    let entities = extract_entities(Path::new("test.cpp"), source, Language::CPP);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Point");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn test_extract_cpp_standalone_function() {
    let source = "void greet() {}\n";
    let entities = extract_entities(Path::new("test.cpp"), source, Language::CPP);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "greet");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_extract_cpp_class_with_multiple_methods() {
    let source = "\
class Calculator {
public:
    int add(int a, int b) { return a + b; }
    int sub(int a, int b) { return a - b; }
};
";
    let entities = extract_entities(Path::new("test.cpp"), source, Language::CPP);
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Calculator" && e.kind == EntityKind::Class)
    );
    assert!(entities.iter().any(|e| e.name == "add"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Calculator")));
    assert!(entities.iter().any(|e| e.name == "sub"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Calculator")));
}
