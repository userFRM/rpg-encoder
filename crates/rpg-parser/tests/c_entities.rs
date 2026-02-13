use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_extract_c_function() {
    let source = "int main() { return 0; }\n";
    let entities = extract_entities(Path::new("test.c"), source, Language::C);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "main");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_extract_c_function_with_params() {
    let source = "int add(int a, int b) { return a + b; }\n";
    let entities = extract_entities(Path::new("test.c"), source, Language::C);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "add");
    assert_eq!(entities[0].kind, EntityKind::Function);
}

#[test]
fn test_signature_typed_params_and_return() {
    let source = "int add(int a, int b) { return a + b; }\n";
    let entities = extract_entities(Path::new("test.c"), source, Language::C);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.parameters[0].name, "a");
    assert_eq!(sig.parameters[0].type_annotation.as_deref(), Some("int"));
    assert_eq!(sig.parameters[1].name, "b");
    assert_eq!(sig.parameters[1].type_annotation.as_deref(), Some("int"));
    assert_eq!(sig.return_type.as_deref(), Some("int"));
}

#[test]
fn test_signature_void_return() {
    let source = "void greet(char* name) { printf(\"%s\", name); }\n";
    let entities = extract_entities(Path::new("test.c"), source, Language::C);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    assert_eq!(sig.parameters.len(), 1);
    assert_eq!(sig.parameters[0].name, "name");
    assert_eq!(sig.return_type.as_deref(), Some("void"));
}

#[test]
fn test_extract_c_struct() {
    let source = "struct Point { int x; int y; };\n";
    let entities = extract_entities(Path::new("test.c"), source, Language::C);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Point");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn test_extract_c_multiple_functions() {
    let source = "\
int foo() { return 1; }
int bar() { return 2; }
int baz() { return 3; }
";
    let entities = extract_entities(Path::new("test.c"), source, Language::C);
    assert_eq!(entities.len(), 3);
    assert!(entities.iter().all(|e| e.kind == EntityKind::Function));
    assert!(entities.iter().any(|e| e.name == "foo"));
    assert!(entities.iter().any(|e| e.name == "bar"));
    assert!(entities.iter().any(|e| e.name == "baz"));
}

#[test]
fn test_extract_c_pointer_returning_function() {
    let source = "char* getName() { return \"test\"; }\n";
    let entities = extract_entities(Path::new("test.c"), source, Language::C);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "getName");
    assert_eq!(entities[0].kind, EntityKind::Function);
}
