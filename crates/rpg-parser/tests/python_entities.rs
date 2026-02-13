use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_python_entities;
use std::path::Path;

#[test]
fn test_signature_typed_params_and_return() {
    let source = "def compute(x: int, y: str) -> bool:\n    return True\n";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.parameters[0].name, "x");
    assert_eq!(sig.parameters[0].type_annotation.as_deref(), Some("int"));
    assert_eq!(sig.parameters[1].name, "y");
    assert_eq!(sig.parameters[1].type_annotation.as_deref(), Some("str"));
    assert_eq!(sig.return_type.as_deref(), Some("bool"));
}

#[test]
fn test_signature_untyped_params_no_return() {
    let source = "def greet(name, greeting):\n    print(greeting, name)\n";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.parameters[0].name, "name");
    assert!(sig.parameters[0].type_annotation.is_none());
    assert_eq!(sig.parameters[1].name, "greeting");
    assert!(sig.parameters[1].type_annotation.is_none());
    assert!(sig.return_type.is_none());
}

#[test]
fn test_signature_self_skipped() {
    let source = "\
class Foo:
    def bar(self, x: int) -> str:
        return str(x)
";
    let entities = extract_python_entities(Path::new("test.py"), source);
    let method = entities
        .iter()
        .find(|e| e.name == "bar")
        .expect("should find bar");
    let sig = method.signature.as_ref().expect("should have signature");
    // self should be skipped
    assert_eq!(sig.parameters.len(), 1);
    assert_eq!(sig.parameters[0].name, "x");
    assert_eq!(sig.return_type.as_deref(), Some("str"));
}

#[test]
fn test_signature_cls_skipped() {
    let source = "\
class Foo:
    @classmethod
    def create(cls, name: str) -> 'Foo':
        return Foo()
";
    let entities = extract_python_entities(Path::new("test.py"), source);
    let method = entities
        .iter()
        .find(|e| e.name == "create")
        .expect("should find create");
    let sig = method.signature.as_ref().expect("should have signature");
    // cls should be skipped
    assert_eq!(sig.parameters.len(), 1);
    assert_eq!(sig.parameters[0].name, "name");
}

#[test]
fn test_signature_args_kwargs() {
    let source = "def variadic(a, *args, **kwargs):\n    pass\n";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 1);
    let sig = entities[0]
        .signature
        .as_ref()
        .expect("should have signature");
    assert_eq!(sig.parameters.len(), 3);
    assert_eq!(sig.parameters[0].name, "a");
    assert_eq!(sig.parameters[1].name, "*args");
    assert_eq!(sig.parameters[2].name, "**kwargs");
}

#[test]
fn test_simple_function() {
    let source = "def greet(name):\n    print(name)\n";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "greet");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert_eq!(entities[0].line_start, 1);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_multiple_functions() {
    let source = "\
def foo():
    pass

def bar():
    pass

def baz():
    pass
";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 3);
    let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"foo"));
    assert!(names.contains(&"bar"));
    assert!(names.contains(&"baz"));
    assert!(entities.iter().all(|e| e.kind == EntityKind::Function));
}

#[test]
fn test_class_with_methods() {
    let source = "\
class Calculator:
    def add(self, a, b):
        return a + b

    def subtract(self, a, b):
        return a - b
";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 3); // class + 2 methods
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Calculator" && e.kind == EntityKind::Class)
    );
    assert!(entities.iter().any(|e| e.name == "add"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Calculator")));
    assert!(entities.iter().any(|e| e.name == "subtract"
        && e.kind == EntityKind::Method
        && e.parent_class.as_deref() == Some("Calculator")));
}

#[test]
fn test_empty_file() {
    let source = "";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert!(entities.is_empty());
}

#[test]
fn test_class_no_methods() {
    let source = "\
class Empty:
    pass
";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Empty");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn test_mixed_functions_and_classes() {
    let source = "\
def helper():
    pass

class Service:
    def run(self):
        pass

def main():
    pass
";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 4); // helper, Service, run, main
    assert!(
        entities
            .iter()
            .any(|e| e.name == "helper" && e.kind == EntityKind::Function)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Service" && e.kind == EntityKind::Class)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "run" && e.kind == EntityKind::Method)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "main" && e.kind == EntityKind::Function)
    );
}

#[test]
fn test_entity_id_generation() {
    let source = "\
class Foo:
    def bar(self):
        pass
";
    let entities = extract_python_entities(Path::new("src/module.py"), source);
    let method = entities.iter().find(|e| e.name == "bar").unwrap();
    assert_eq!(method.id(), "src/module.py:Foo::bar");

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.id(), "src/module.py:Foo");
}

#[test]
fn test_source_text_captured() {
    let source = "def hello():\n    return 42\n";
    let entities = extract_python_entities(Path::new("test.py"), source);
    assert_eq!(entities.len(), 1);
    assert!(entities[0].source_text.contains("def hello()"));
    assert!(entities[0].source_text.contains("return 42"));
}
