use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_function_declaration() {
    let source = r#"function greet(name: string): string { return "hi"; }"#;
    let entities = extract_entities(Path::new("test.ts"), source, Language::TypeScript);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "greet");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert_eq!(entities[0].line_start, 1);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_class_with_method() {
    let source = "\
class Foo {
    bar(): void {}
}
";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TypeScript);
    assert!(
        entities.len() >= 2,
        "expected at least 2 entities, got {}",
        entities.len()
    );
    let class_entity = entities
        .iter()
        .find(|e| e.name == "Foo")
        .expect("missing Foo class");
    assert_eq!(class_entity.kind, EntityKind::Class);
    assert!(class_entity.parent_class.is_none());

    let method_entity = entities
        .iter()
        .find(|e| e.name == "bar")
        .expect("missing bar method");
    assert_eq!(method_entity.kind, EntityKind::Method);
    assert_eq!(method_entity.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn test_interface_declaration() {
    let source = "\
interface Animal {
    name: string;
    speak(): void;
}
";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TypeScript);
    let iface = entities
        .iter()
        .find(|e| e.name == "Animal")
        .expect("missing Animal interface");
    assert_eq!(iface.kind, EntityKind::Class);
    assert!(iface.parent_class.is_none());
}

#[test]
fn test_named_arrow_function() {
    let source = "const add = (a: number, b: number): number => a + b;";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TypeScript);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "add");
    assert_eq!(entities[0].kind, EntityKind::Function);
}

#[test]
fn test_exported_function() {
    let source = "export function doStuff() {}";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TypeScript);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "doStuff");
    assert_eq!(entities[0].kind, EntityKind::Function);
}

#[test]
fn test_tsx_entity_extraction() {
    let source = r#"
import React from 'react';

interface ButtonProps {
    label: string;
    onClick: () => void;
}

const Button: React.FC<ButtonProps> = ({ label, onClick }) => {
    return <button onClick={onClick}>{label}</button>;
};

export function App() {
    return (
        <div>
            <Button label="Click me" onClick={() => {}} />
        </div>
    );
}
"#;
    let entities = extract_entities(Path::new("test.tsx"), source, Language::TypeScript);
    let iface = entities.iter().find(|e| e.name == "ButtonProps");
    assert!(
        iface.is_some(),
        "expected ButtonProps interface, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(iface.unwrap().kind, EntityKind::Class);

    let button = entities.iter().find(|e| e.name == "Button");
    assert!(
        button.is_some(),
        "expected Button arrow function, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(button.unwrap().kind, EntityKind::Function);

    let app = entities.iter().find(|e| e.name == "App");
    assert!(
        app.is_some(),
        "expected App function, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(app.unwrap().kind, EntityKind::Function);
}
