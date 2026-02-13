use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn csharp_extract_class() {
    let source = "public class Foo { }";
    let entities = extract_entities(Path::new("Foo.cs"), source, Language::CSHARP);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Foo");
    assert_eq!(entities[0].kind, EntityKind::Class);
}

#[test]
fn csharp_extract_class_with_method() {
    let source = "public class Foo { public void Bar() { } }";
    let entities = extract_entities(Path::new("Foo.cs"), source, Language::CSHARP);
    assert_eq!(entities.len(), 2);

    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);

    let method = entities.iter().find(|e| e.name == "Bar").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn csharp_extract_interface() {
    let source = "public interface IFoo { void Bar(); }";
    let entities = extract_entities(Path::new("IFoo.cs"), source, Language::CSHARP);
    let iface = entities.iter().find(|e| e.name == "IFoo").unwrap();
    assert_eq!(iface.kind, EntityKind::Class);
}

#[test]
fn csharp_extract_namespace_traversal() {
    let source = "namespace MyApp { public class Foo { } }";
    let entities = extract_entities(Path::new("Foo.cs"), source, Language::CSHARP);
    let class = entities.iter().find(|e| e.name == "Foo").unwrap();
    assert_eq!(class.kind, EntityKind::Class);
}

#[test]
fn csharp_extract_enum() {
    let source = "public enum Color { Red, Green, Blue }";
    let entities = extract_entities(Path::new("Color.cs"), source, Language::CSHARP);
    let en = entities.iter().find(|e| e.name == "Color").unwrap();
    assert_eq!(en.kind, EntityKind::Class);
}

#[test]
fn csharp_nested_class_inside_namespace() {
    let source = r"namespace MyApp.Models {
    public class Outer {
        public class Inner {
            public void DoStuff() { }
        }
    }
}";
    let entities = extract_entities(Path::new("Outer.cs"), source, Language::CSHARP);

    let outer = entities.iter().find(|e| e.name == "Outer").unwrap();
    assert_eq!(outer.kind, EntityKind::Class);
    assert_eq!(outer.parent_class, None);

    let inner = entities.iter().find(|e| e.name == "Inner").unwrap();
    assert_eq!(inner.kind, EntityKind::Class);
    assert_eq!(inner.parent_class.as_deref(), Some("Outer"));

    let method = entities.iter().find(|e| e.name == "DoStuff").unwrap();
    assert_eq!(method.kind, EntityKind::Method);
    assert_eq!(method.parent_class.as_deref(), Some("Inner"));
}

#[test]
fn csharp_record_declaration() {
    let source = "public record Person(string Name, int Age);";
    let entities = extract_entities(Path::new("Person.cs"), source, Language::CSHARP);
    let record = entities.iter().find(|e| e.name == "Person").unwrap();
    assert_eq!(record.kind, EntityKind::Class);
}

#[test]
fn csharp_signature_typed_params_and_return() {
    let source = "public class Foo { public bool Compute(int x, string y) { return true; } }";
    let entities = extract_entities(Path::new("Foo.cs"), source, Language::CSHARP);
    let method = entities
        .iter()
        .find(|e| e.name == "Compute")
        .expect("should find Compute");
    let sig = method.signature.as_ref().expect("should have signature");
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.parameters[0].name, "x");
    assert_eq!(sig.parameters[0].type_annotation.as_deref(), Some("int"));
    assert_eq!(sig.parameters[1].name, "y");
    assert_eq!(sig.parameters[1].type_annotation.as_deref(), Some("string"));
    assert_eq!(sig.return_type.as_deref(), Some("bool"));
}

#[test]
fn csharp_signature_void_return() {
    let source = "public class Foo { public void Greet(string name) { } }";
    let entities = extract_entities(Path::new("Foo.cs"), source, Language::CSHARP);
    let method = entities
        .iter()
        .find(|e| e.name == "Greet")
        .expect("should find Greet");
    let sig = method.signature.as_ref().expect("should have signature");
    assert_eq!(sig.parameters.len(), 1);
    assert_eq!(sig.parameters[0].name, "name");
    assert_eq!(sig.return_type.as_deref(), Some("void"));
}
