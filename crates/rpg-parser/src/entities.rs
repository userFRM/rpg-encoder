//! Extract code entities (functions, classes, methods) from AST.

use crate::languages::Language;
use rpg_core::graph::{Entity, EntityDeps, EntityKind};
use std::path::Path;

/// A raw extracted entity before semantic enrichment.
#[derive(Debug, Clone)]
pub struct RawEntity {
    pub name: String,
    pub kind: EntityKind,
    pub file: std::path::PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub parent_class: Option<String>,
    pub source_text: String,
}

impl RawEntity {
    /// Generate a unique entity ID.
    pub fn id(&self) -> String {
        match &self.parent_class {
            Some(class) => format!("{}:{}::{}", self.file.display(), class, self.name),
            None => format!("{}:{}", self.file.display(), self.name),
        }
    }

    /// Convert to a full Entity (with empty semantic features and deps).
    pub fn into_entity(self) -> Entity {
        let id = self.id();
        Entity {
            id,
            kind: self.kind,
            name: self.name,
            file: self.file,
            line_start: self.line_start,
            line_end: self.line_end,
            parent_class: self.parent_class,
            semantic_features: Vec::new(),
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
            embedding: None,
        }
    }
}

/// Extract entities from a Python source file using tree-sitter.
pub fn extract_python_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };

    let mut entities = Vec::new();
    extract_python_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_python_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    let kind = if parent_class.is_some() {
                        EntityKind::Method
                    } else {
                        EntityKind::Function
                    };
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "class_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: class_name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                    // Recurse into class body for methods
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_python_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            _ => {
                // Recurse into other nodes at top level
                if parent_class.is_none() {
                    extract_python_node(&child, path, source, None, entities);
                }
            }
        }
    }
}

/// Extract entities from a Rust source file using tree-sitter.
pub fn extract_rust_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };

    let mut entities = Vec::new();
    extract_rust_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_rust_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_struct: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: if parent_struct.is_some() {
                            EntityKind::Method
                        } else {
                            EntityKind::Function
                        },
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_struct.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "struct_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "impl_item" => {
                // Find the type name being impl'd
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = &source[type_node.byte_range()];
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_rust_node(&body, path, source, Some(type_name), entities);
                    }
                }
            }
            "enum_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "trait_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                    // Recurse into trait body for default method implementations
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_rust_node(&body, path, source, Some(name), entities);
                    }
                }
            }
            _ => {
                if parent_struct.is_none() {
                    extract_rust_node(&child, path, source, None, entities);
                }
            }
        }
    }
}

/// Generic entity extraction dispatching to the correct language extractor.
pub fn extract_entities(path: &Path, source: &str, language: Language) -> Vec<RawEntity> {
    match language {
        Language::Python => extract_python_entities(path, source),
        Language::Rust => extract_rust_entities(path, source),
        Language::TypeScript => extract_typescript_entities(path, source),
        Language::JavaScript => extract_javascript_entities(path, source),
        Language::Go => extract_go_entities(path, source),
        Language::Java => extract_java_entities(path, source),
        Language::C => extract_c_entities(path, source),
        Language::Cpp => extract_cpp_entities(path, source),
    }
}

// ---------------------------------------------------------------------------
// TypeScript / JavaScript
// ---------------------------------------------------------------------------

/// Extract entities from a TypeScript source file.
pub fn extract_typescript_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    extract_js_like_entities(path, source, Language::TypeScript)
}

/// Extract entities from a JavaScript source file.
pub fn extract_javascript_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    extract_js_like_entities(path, source, Language::JavaScript)
}

fn extract_js_like_entities(path: &Path, source: &str, lang: Language) -> Vec<RawEntity> {
    let ts_lang = lang.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };

    let mut entities = Vec::new();
    extract_js_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_js_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: if parent_class.is_some() {
                            EntityKind::Method
                        } else {
                            EntityKind::Function
                        },
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: class_name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_js_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            // TS: interface_declaration
            "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            // TS: type_alias_declaration
            "type_alias_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "method_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Method,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            // Named arrow function in variable: const foo = () => {}
            "lexical_declaration" | "variable_declaration" => {
                let mut inner = child.walk();
                for decl in child.children(&mut inner) {
                    if decl.kind() == "variable_declarator" {
                        let has_arrow = has_child_kind(&decl, "arrow_function");
                        let has_func = has_child_kind(&decl, "function");
                        if (has_arrow || has_func)
                            && let Some(name_node) = decl.child_by_field_name("name")
                        {
                            let name = &source[name_node.byte_range()];
                            entities.push(RawEntity {
                                name: name.to_string(),
                                kind: EntityKind::Function,
                                file: path.to_path_buf(),
                                line_start: child.start_position().row + 1,
                                line_end: child.end_position().row + 1,
                                parent_class: parent_class.map(String::from),
                                source_text: source[child.byte_range()].to_string(),
                            });
                        }
                    }
                }
            }
            "export_statement" => {
                // Recurse into export statements to find declarations
                extract_js_node(&child, path, source, parent_class, entities);
            }
            _ => {
                if parent_class.is_none() {
                    extract_js_node(&child, path, source, None, entities);
                }
            }
        }
    }
}

fn has_child_kind(node: &tree_sitter::Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|c| c.kind() == kind)
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

/// Extract entities from a Go source file.
pub fn extract_go_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::Go.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };

    let mut entities = Vec::new();
    let mut cursor = tree.root_node().walk();
    for child in tree.root_node().children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Function,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    // Extract receiver type
                    let receiver = child
                        .child_by_field_name("receiver")
                        .and_then(|r| {
                            // parameter_list -> parameter_declaration -> type
                            let mut c = r.walk();
                            r.children(&mut c)
                                .find(|n| n.kind() == "parameter_declaration")
                        })
                        .and_then(|pd| pd.child_by_field_name("type"))
                        .map(|t| {
                            let text = &source[t.byte_range()];
                            text.trim_start_matches('*').to_string()
                        });

                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Method,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: receiver,
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "type_declaration" => {
                // type_declaration contains type_spec children
                let mut tc = child.walk();
                for spec in child.children(&mut tc) {
                    if spec.kind() == "type_spec"
                        && let Some(name_node) = spec.child_by_field_name("name")
                    {
                        let name = &source[name_node.byte_range()];
                        entities.push(RawEntity {
                            name: name.to_string(),
                            kind: EntityKind::Class,
                            file: path.to_path_buf(),
                            line_start: spec.start_position().row + 1,
                            line_end: spec.end_position().row + 1,
                            parent_class: None,
                            source_text: source[spec.byte_range()].to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    entities
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

/// Extract entities from a Java source file.
pub fn extract_java_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::Java.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };

    let mut entities = Vec::new();
    extract_java_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_java_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: class_name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_java_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            "method_declaration" | "constructor_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Method,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            _ => {
                extract_java_node(&child, path, source, parent_class, entities);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// C
// ---------------------------------------------------------------------------

/// Extract entities from a C source file.
pub fn extract_c_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    extract_c_like_entities(path, source, Language::C)
}

/// Extract entities from a C++ source file.
pub fn extract_cpp_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    extract_c_like_entities(path, source, Language::Cpp)
}

fn extract_c_like_entities(path: &Path, source: &str, lang: Language) -> Vec<RawEntity> {
    let ts_lang = lang.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };

    let mut entities = Vec::new();
    extract_c_node(&tree.root_node(), path, source, None, &mut entities, lang);
    entities
}

fn extract_c_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
    lang: Language,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                // The declarator contains the function name
                if let Some(decl) = child.child_by_field_name("declarator")
                    && let Some(name) = extract_c_declarator_name(&decl, source)
                {
                    entities.push(RawEntity {
                        name,
                        kind: if parent_class.is_some() {
                            EntityKind::Method
                        } else {
                            EntityKind::Function
                        },
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                    });
                }
            }
            "struct_specifier" | "class_specifier" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Class,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                    });
                    // C++: recurse into class/struct body for methods
                    if lang == Language::Cpp
                        && let Some(body) = child.child_by_field_name("body")
                    {
                        extract_c_node(&body, path, source, Some(name), entities, lang);
                    }
                }
            }
            _ => {
                if parent_class.is_none() {
                    extract_c_node(&child, path, source, None, entities, lang);
                }
            }
        }
    }
}

/// Extract function name from a C/C++ declarator (handles nested function_declarator).
pub fn extract_c_declarator_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_declarator" => {
            // function_declarator has a declarator child which is the name (or pointer)
            node.child_by_field_name("declarator")
                .and_then(|d| extract_c_declarator_name(&d, source))
        }
        "pointer_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|d| extract_c_declarator_name(&d, source)),
        "identifier" | "field_identifier" => Some(source[node.byte_range()].to_string()),
        // C++: qualified_identifier like ClassName::method
        "qualified_identifier" => {
            // Take the last name segment
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .filter(|c| c.kind() == "identifier" || c.kind() == "destructor_name")
                .last()
                .map(|n| source[n.byte_range()].to_string())
        }
        _ => None,
    }
}
