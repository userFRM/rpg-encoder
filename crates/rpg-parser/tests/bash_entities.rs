use std::path::Path;

use rpg_core::graph::EntityKind;
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;

#[test]
fn bash_extract_function_with_keyword() {
    let source = r#"function hello() {
  echo "hello"
}"#;
    let entities = extract_entities(Path::new("test.sh"), source, Language::BASH);
    let func = entities.iter().find(|e| e.name == "hello").unwrap();
    assert_eq!(func.kind, EntityKind::Function);
    assert!(func.parent_class.is_none());
}

#[test]
fn bash_extract_function_without_keyword() {
    let source = r#"hello() {
  echo "hello"
}"#;
    let entities = extract_entities(Path::new("test.sh"), source, Language::BASH);
    let func = entities.iter().find(|e| e.name == "hello").unwrap();
    assert_eq!(func.kind, EntityKind::Function);
    assert!(func.parent_class.is_none());
}
