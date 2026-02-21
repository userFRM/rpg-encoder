//! Extract code entities (functions, classes, methods) from AST.

use crate::languages::Language;
use rpg_core::graph::{Entity, EntityDeps, EntityKind, Param, Signature, normalize_path};
use std::path::Path;

/// A raw parameter extracted from AST.
#[derive(Debug, Clone)]
pub struct RawParam {
    pub name: String,
    pub type_annotation: Option<String>,
}

/// A raw function/method signature extracted from AST.
#[derive(Debug, Clone, Default)]
pub struct RawSignature {
    pub parameters: Vec<RawParam>,
    pub return_type: Option<String>,
}

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
    pub signature: Option<RawSignature>,
}

impl RawEntity {
    /// Generate a unique entity ID.
    pub fn id(&self) -> String {
        match &self.parent_class {
            Some(class) => format!("{}:{}::{}", normalize_path(&self.file), class, self.name),
            None => format!("{}:{}", normalize_path(&self.file), self.name),
        }
    }

    /// Convert to a full Entity (with empty semantic features and deps).
    pub fn into_entity(self) -> Entity {
        let id = self.id();
        let signature = self.signature.map(|sig| Signature {
            parameters: sig
                .parameters
                .into_iter()
                .map(|p| Param {
                    name: p.name,
                    type_annotation: p.type_annotation,
                })
                .collect(),
            return_type: sig.return_type,
        });
        Entity {
            id,
            kind: self.kind,
            name: self.name,
            file: self.file,
            line_start: self.line_start,
            line_end: self.line_end,
            parent_class: self.parent_class,
            semantic_features: Vec::new(),
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
            signature,
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
                        signature: extract_python_signature(&child, source),
                    });
                }
            }
            // Python: decorated definitions (@property, @staticmethod, etc.)
            "decorated_definition" => {
                // Recurse into the decorated definition to extract the inner function/class
                extract_python_node(&child, path, source, parent_class, entities);
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
                        signature: None,
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
                        signature: extract_rust_signature(&child, source),
                    });
                }
            }
            "struct_item" | "enum_item" | "type_item" => {
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
                        signature: None,
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
                        signature: None,
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
///
/// Tries the builtin extractor registered in the language TOML first.
/// Falls back to an empty result for languages without a builtin extractor.
pub fn extract_entities(path: &Path, source: &str, language: Language) -> Vec<RawEntity> {
    if let Some(extractor_name) = crate::languages::builtin_entity_extractor_name(language)
        && let Some(extractor) = crate::languages::builtin_entity_extractor(extractor_name)
    {
        return extractor(path, source);
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// TypeScript / JavaScript
// ---------------------------------------------------------------------------

/// Extract entities from a TypeScript source file.
pub fn extract_typescript_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    extract_js_like_entities(path, source, Language::TYPESCRIPT)
}

/// Extract entities from a JavaScript source file.
pub fn extract_javascript_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    extract_js_like_entities(path, source, Language::JAVASCRIPT)
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
                    let default_kind = if parent_class.is_some() {
                        EntityKind::Method
                    } else {
                        EntityKind::Function
                    };
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: classify_js_entity_kind(
                            path,
                            name,
                            &source[child.byte_range()],
                            default_kind,
                            parent_class,
                        ),
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                        signature: extract_js_signature(&child, source),
                    });
                }
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: class_name.to_string(),
                        kind: classify_js_entity_kind(
                            path,
                            class_name,
                            &source[child.byte_range()],
                            EntityKind::Class,
                            None,
                        ),
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: None,
                        source_text: source[child.byte_range()].to_string(),
                        signature: None,
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_js_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            // TS: interface_declaration / type_alias_declaration
            "interface_declaration" | "type_alias_declaration" => {
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
                        signature: None,
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
                        signature: extract_js_signature(&child, source),
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
                        let decl_source = &source[decl.byte_range()];

                        if (has_arrow || has_func)
                            && let Some(name_node) = decl.child_by_field_name("name")
                        {
                            let name = &source[name_node.byte_range()];
                            // createAsyncThunk wraps an async function but should
                            // be classified as Function (callable thunk), not Store
                            let kind = if looks_like_async_thunk(decl_source) {
                                EntityKind::Function
                            } else {
                                classify_js_entity_kind(
                                    path,
                                    name,
                                    decl_source,
                                    EntityKind::Function,
                                    parent_class,
                                )
                            };
                            entities.push(RawEntity {
                                name: name.to_string(),
                                kind,
                                file: path.to_path_buf(),
                                line_start: child.start_position().row + 1,
                                line_end: child.end_position().row + 1,
                                parent_class: parent_class.map(String::from),
                                source_text: source[child.byte_range()].to_string(),
                                signature: None,
                            });
                        } else if let Some(name_node) = decl.child_by_field_name("name") {
                            let name_kind = name_node.kind();
                            // Destructured RTK Query hooks:
                            // const { useGetPostsQuery, useGetUserQuery } = postsApi;
                            if name_kind == "object_pattern" {
                                extract_destructured_hooks(
                                    &name_node,
                                    path,
                                    source,
                                    &child,
                                    parent_class,
                                    entities,
                                );
                            } else if looks_like_async_thunk(decl_source) {
                                // createAsyncThunk wraps a call_expression (not
                                // a direct arrow/function child), so handle it here
                                let name = &source[name_node.byte_range()];
                                entities.push(RawEntity {
                                    name: name.to_string(),
                                    kind: EntityKind::Function,
                                    file: path.to_path_buf(),
                                    line_start: child.start_position().row + 1,
                                    line_end: child.end_position().row + 1,
                                    parent_class: parent_class.map(String::from),
                                    source_text: source[child.byte_range()].to_string(),
                                    signature: None,
                                });
                            } else {
                                let name = &source[name_node.byte_range()];
                                if looks_like_store_entity(name, decl_source) {
                                    entities.push(RawEntity {
                                        name: name.to_string(),
                                        kind: EntityKind::Store,
                                        file: path.to_path_buf(),
                                        line_start: decl.start_position().row + 1,
                                        line_end: decl.end_position().row + 1,
                                        parent_class: parent_class.map(String::from),
                                        source_text: decl_source.to_string(),
                                        signature: None,
                                    });
                                    // Extract createSlice reducer keys as child entities
                                    if decl_source.contains("createSlice(") {
                                        extract_create_slice_reducers(
                                            &decl, path, source, name, entities,
                                        );
                                    }
                                }
                            }
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

fn classify_js_entity_kind(
    path: &Path,
    name: &str,
    source_snippet: &str,
    default_kind: EntityKind,
    parent_class: Option<&str>,
) -> EntityKind {
    if parent_class.is_some() || default_kind == EntityKind::Method {
        return EntityKind::Method;
    }

    if is_next_app_file(path, "page") && looks_like_react_component(name, source_snippet) {
        return EntityKind::Page;
    }

    if is_next_app_file(path, "layout") && looks_like_react_component(name, source_snippet) {
        return EntityKind::Layout;
    }

    if looks_like_custom_hook(name) {
        return EntityKind::Hook;
    }

    if default_kind == EntityKind::Class {
        if source_snippet.contains("extends React.Component")
            || source_snippet.contains("extends React.PureComponent")
            || source_snippet.contains("extends Component")
            || source_snippet.contains("extends PureComponent")
        {
            return EntityKind::Component;
        }
        return EntityKind::Class;
    }

    if looks_like_react_component(name, source_snippet) {
        return EntityKind::Component;
    }

    default_kind
}

fn is_next_app_file(path: &Path, stem: &str) -> bool {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if !file_name.starts_with(&format!("{}.", stem)) {
        return false;
    }
    path.iter().any(|seg| seg.to_string_lossy() == "app")
}

fn looks_like_react_component(name: &str, source_snippet: &str) -> bool {
    let starts_upper = name.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    if !starts_upper {
        return false;
    }
    source_snippet.contains("return <")
        || (source_snippet.contains("return (") && source_snippet.contains('<'))
        || source_snippet.contains("=> <")
        || source_snippet.contains("React.FC")
        || source_snippet.contains("<>")
}

fn looks_like_custom_hook(name: &str) -> bool {
    if !name.starts_with("use") || name.len() <= 3 {
        return false;
    }
    name.chars().nth(3).is_some_and(|c| c.is_ascii_uppercase())
}

fn looks_like_store_entity(name: &str, source_snippet: &str) -> bool {
    let lname = name.to_ascii_lowercase();
    if lname.starts_with("set") {
        // Setter-like functions should not be classified as store entities.
        return source_snippet.contains("configureStore(")
            || source_snippet.contains("createStore(")
            || source_snippet.contains("createSlice(");
    }
    // Match "store" as a word via camelCase: capital "Store" in the original name,
    // or the lowered name equals/starts with "store".
    // This rejects "restore" (no capital S, no prefix) and "localStorage"/"sessionStorage"
    // (which contain "Storag" not "Store").
    let has_store_word = lname == "store" || lname.starts_with("store") || name.contains("Store");
    has_store_word
        || lname.ends_with("slice")
        || source_snippet.contains("configureStore(")
        || source_snippet.contains("createStore(")
        || source_snippet.contains("createSlice(")
        || source_snippet.contains("createApi(")
}

fn looks_like_async_thunk(source_snippet: &str) -> bool {
    source_snippet.contains("createAsyncThunk(")
}

/// Extract reducer keys from a createSlice({ reducers: { key1, key2 } }) call as child entities.
fn extract_create_slice_reducers(
    decl: &tree_sitter::Node,
    path: &Path,
    source: &str,
    slice_name: &str,
    entities: &mut Vec<RawEntity>,
) {
    // Walk into: call_expression > arguments > object > pair[key=reducers] > object > pair*
    fn walk_for_reducers<'a>(
        node: &'a tree_sitter::Node<'a>,
        source: &str,
        slice_name: &str,
        path: &Path,
        entities: &mut Vec<RawEntity>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if (child.kind() == "pair" || child.kind() == "property_signature")
                && let Some(key) = child.child_by_field_name("key")
            {
                let key_text = &source[key.byte_range()];
                if key_text == "reducers" {
                    // Found the reducers object — extract each key
                    if let Some(value) = child.child_by_field_name("value") {
                        extract_reducer_keys(&value, source, slice_name, path, entities);
                    }
                    return;
                }
            }
            walk_for_reducers(&child, source, slice_name, path, entities);
        }
    }

    fn extract_reducer_keys(
        node: &tree_sitter::Node,
        source: &str,
        slice_name: &str,
        path: &Path,
        entities: &mut Vec<RawEntity>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if (child.kind() == "pair"
                || child.kind() == "method_definition"
                || child.kind() == "shorthand_property_identifier_pattern"
                || child.kind() == "shorthand_property_identifier")
                && let Some(key) = child
                    .child_by_field_name("key")
                    .or_else(|| child.child_by_field_name("name"))
            {
                let reducer_name = &source[key.byte_range()];
                entities.push(RawEntity {
                    name: reducer_name.to_string(),
                    kind: EntityKind::Function,
                    file: path.to_path_buf(),
                    line_start: child.start_position().row + 1,
                    line_end: child.end_position().row + 1,
                    parent_class: Some(slice_name.to_string()),
                    source_text: source[child.byte_range()].to_string(),
                    signature: None,
                });
            }
        }
    }

    walk_for_reducers(decl, source, slice_name, path, entities);
}

/// Extract destructured hooks from object_pattern: const { useGetPostsQuery } = postsApi;
///
/// Only fires when the RHS of the variable_declarator is a plain identifier (the API
/// object) — this avoids false positives from unrelated object destructuring like
/// `const { useFoo } = someRandomFunction()`.
fn extract_destructured_hooks(
    object_pattern: &tree_sitter::Node,
    path: &Path,
    source: &str,
    outer_decl: &tree_sitter::Node,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    // The object_pattern is the LHS name of a variable_declarator.
    // Validate the RHS (initializer) is a plain identifier — this indicates
    // destructuring from a known API object (e.g., `postsApi`), not an arbitrary call.
    let var_declarator = object_pattern.parent();
    let has_identifier_rhs = var_declarator
        .as_ref()
        .and_then(|vd| vd.child_by_field_name("value"))
        .is_some_and(|val| val.kind() == "identifier");
    if !has_identifier_rhs {
        return;
    }

    let mut cursor = object_pattern.walk();
    for child in object_pattern.children(&mut cursor) {
        // shorthand_property_identifier_pattern is the node kind for `{ foo }` in destructuring
        let name = match child.kind() {
            "shorthand_property_identifier_pattern" | "shorthand_property_identifier" => {
                Some(&source[child.byte_range()])
            }
            "pair_pattern" => child
                .child_by_field_name("value")
                .map(|v| &source[v.byte_range()]),
            _ => None,
        };
        if let Some(name) = name
            && looks_like_custom_hook(name)
        {
            entities.push(RawEntity {
                name: name.to_string(),
                kind: EntityKind::Hook,
                file: path.to_path_buf(),
                line_start: outer_decl.start_position().row + 1,
                line_end: outer_decl.end_position().row + 1,
                parent_class: parent_class.map(String::from),
                source_text: source[outer_decl.byte_range()].to_string(),
                signature: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

/// Extract entities from a Go source file.
pub fn extract_go_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::GO.ts_language();
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
                        signature: extract_go_signature(&child, source),
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
                        signature: extract_go_signature(&child, source),
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
                            signature: None,
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
    let lang = Language::JAVA.ts_language();
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
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration" => {
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
                        signature: None,
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
                        signature: extract_java_signature(&child, source),
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
    extract_c_like_entities(path, source, Language::CPP)
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
                        signature: extract_c_signature(&child, source),
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
                        signature: None,
                    });
                    // C++: recurse into class/struct body for methods
                    if lang == Language::CPP
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

// ---------------------------------------------------------------------------
// C#
// ---------------------------------------------------------------------------

/// Extract entities from a C# source file.
pub fn extract_csharp_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::CSHARP.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    extract_csharp_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_csharp_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "struct_declaration"
            | "enum_declaration"
            | "record_declaration" => {
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
                        signature: None,
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_csharp_node(&body, path, source, Some(class_name), entities);
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
                        signature: extract_csharp_signature(&child, source),
                    });
                }
            }
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                extract_csharp_node(&child, path, source, parent_class, entities);
            }
            _ => {
                if parent_class.is_none() {
                    extract_csharp_node(&child, path, source, None, entities);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PHP
// ---------------------------------------------------------------------------

/// Extract entities from a PHP source file.
pub fn extract_php_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::PHP.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    extract_php_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_php_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => {
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
                        signature: None,
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_php_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    entities.push(RawEntity {
                        name: name.to_string(),
                        kind: EntityKind::Function,
                        file: path.to_path_buf(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_class: parent_class.map(String::from),
                        source_text: source[child.byte_range()].to_string(),
                        signature: None,
                    });
                }
            }
            "method_declaration" => {
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
                        signature: None,
                    });
                }
            }
            _ => {
                extract_php_node(&child, path, source, parent_class, entities);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ruby
// ---------------------------------------------------------------------------

/// Extract entities from a Ruby source file.
pub fn extract_ruby_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::RUBY.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    extract_ruby_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_ruby_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" | "module" => {
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
                        signature: None,
                    });
                    // Recurse into class/module body for methods
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_ruby_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            "method" => {
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
                        signature: None,
                    });
                }
            }
            "singleton_method" => {
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
                        signature: None,
                    });
                }
            }
            _ => {
                extract_ruby_node(&child, path, source, parent_class, entities);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Kotlin
// ---------------------------------------------------------------------------

/// Extract entities from a Kotlin source file.
pub fn extract_kotlin_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::KOTLIN.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    extract_kotlin_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_kotlin_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "object_declaration" | "interface_declaration" => {
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
                        signature: None,
                    });
                    // kotlin-ng uses "class_body" / "enum_class_body" child nodes (not a "body" field)
                    let body = child.child_by_field_name("body").or_else(|| {
                        let mut c = child.walk();
                        child
                            .children(&mut c)
                            .find(|n| n.kind() == "class_body" || n.kind() == "enum_class_body")
                    });
                    if let Some(body) = body {
                        extract_kotlin_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            "function_declaration" => {
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
                        signature: None,
                    });
                }
            }
            _ => {
                extract_kotlin_node(&child, path, source, parent_class, entities);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Swift
// ---------------------------------------------------------------------------

/// Extract entities from a Swift source file.
pub fn extract_swift_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::SWIFT.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    extract_swift_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_swift_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration"
            | "struct_declaration"
            | "protocol_declaration"
            | "enum_declaration" => {
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
                        signature: None,
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_swift_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
            "function_declaration" => {
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
                        signature: None,
                    });
                }
            }
            "init_declaration" => {
                // Swift initializer — use "init" as the method name
                entities.push(RawEntity {
                    name: "init".to_string(),
                    kind: EntityKind::Method,
                    file: path.to_path_buf(),
                    line_start: child.start_position().row + 1,
                    line_end: child.end_position().row + 1,
                    parent_class: parent_class.map(String::from),
                    source_text: source[child.byte_range()].to_string(),
                    signature: None,
                });
            }
            "extension_declaration" => {
                // Recurse into extension body, treating it like a class extension
                let ext_name = child
                    .child_by_field_name("name")
                    .map(|n| &source[n.byte_range()]);
                let effective_parent = ext_name.or(parent_class);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_swift_node(&body, path, source, effective_parent, entities);
                }
            }
            _ => {
                extract_swift_node(&child, path, source, parent_class, entities);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scala
// ---------------------------------------------------------------------------

/// Extract entities from a Scala source file.
pub fn extract_scala_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::SCALA.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    extract_scala_node(&tree.root_node(), path, source, None, &mut entities);
    entities
}

fn extract_scala_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    parent_class: Option<&str>,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" | "object_definition" | "trait_definition" => {
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
                        signature: None,
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_scala_node(&body, path, source, Some(class_name), entities);
                    }
                }
            }
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
                        signature: None,
                    });
                }
            }
            _ => {
                extract_scala_node(&child, path, source, parent_class, entities);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Bash
// ---------------------------------------------------------------------------

/// Extract entities from a Bash/shell script source file.
pub fn extract_bash_entities(path: &Path, source: &str) -> Vec<RawEntity> {
    let lang = Language::BASH.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    extract_bash_node(&tree.root_node(), path, source, &mut entities);
    entities
}

fn extract_bash_node(
    node: &tree_sitter::Node,
    path: &Path,
    source: &str,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            // Try field name "name" first, then fall back to finding a "word" child
            let name = child
                .child_by_field_name("name")
                .or_else(|| {
                    let mut c = child.walk();
                    child.children(&mut c).find(|n| n.kind() == "word")
                })
                .map(|n| source[n.byte_range()].to_string());
            if let Some(name) = name {
                entities.push(RawEntity {
                    name,
                    kind: EntityKind::Function,
                    file: path.to_path_buf(),
                    line_start: child.start_position().row + 1,
                    line_end: child.end_position().row + 1,
                    parent_class: None,
                    source_text: source[child.byte_range()].to_string(),
                    signature: None,
                });
            }
        } else {
            extract_bash_node(&child, path, source, entities);
        }
    }
}

// ---------------------------------------------------------------------------
// Signature extraction helpers
// ---------------------------------------------------------------------------

/// Extract signature from a Rust function_item node.
pub fn extract_rust_signature(node: &tree_sitter::Node, source: &str) -> Option<RawSignature> {
    if !matches!(node.kind(), "function_item") {
        return None;
    }
    let params_node = node.child_by_field_name("parameters")?;
    let mut params = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "parameter" {
            let text = &source[child.byte_range()];
            // Skip self/&self/&mut self
            let trimmed = text.trim();
            if trimmed == "self"
                || trimmed == "&self"
                || trimmed == "&mut self"
                || trimmed.starts_with("self:")
            {
                continue;
            }
            // pattern: type  or  pattern  (Rust uses name: Type)
            if let Some((name_part, type_part)) = text.split_once(':') {
                params.push(RawParam {
                    name: name_part.trim().to_string(),
                    type_annotation: Some(type_part.trim().to_string()),
                });
            } else {
                params.push(RawParam {
                    name: text.trim().to_string(),
                    type_annotation: None,
                });
            }
        }
    }
    let return_type = node.child_by_field_name("return_type").map(|rt| {
        source[rt.byte_range()]
            .trim_start_matches("->")
            .trim()
            .to_string()
    });
    Some(RawSignature {
        parameters: params,
        return_type,
    })
}

/// Extract signature from a Python function_definition node.
pub fn extract_python_signature(node: &tree_sitter::Node, source: &str) -> Option<RawSignature> {
    if !matches!(node.kind(), "function_definition") {
        return None;
    }
    let params_node = node.child_by_field_name("parameters")?;
    let mut params = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = &source[child.byte_range()];
                if name == "self" || name == "cls" {
                    continue;
                }
                params.push(RawParam {
                    name: name.to_string(),
                    type_annotation: None,
                });
            }
            "typed_parameter" => {
                // Python typed_parameter: name is first identifier child (no "name" field)
                let name = child
                    .child(0)
                    .filter(|n| n.kind() == "identifier")
                    .map(|n| source[n.byte_range()].to_string());
                if let Some(ref n) = name
                    && (n == "self" || n == "cls")
                {
                    continue;
                }
                let type_ann = child
                    .child_by_field_name("type")
                    .map(|t| source[t.byte_range()].to_string());
                if let Some(name) = name {
                    params.push(RawParam {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            "default_parameter" | "typed_default_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .or_else(|| child.child(0).filter(|n| n.kind() == "identifier"))
                    .map(|n| source[n.byte_range()].to_string());
                if let Some(ref n) = name
                    && (n == "self" || n == "cls")
                {
                    continue;
                }
                let type_ann = child
                    .child_by_field_name("type")
                    .map(|t| source[t.byte_range()].to_string());
                if let Some(name) = name {
                    params.push(RawParam {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            // *args
            "list_splat_pattern" => {
                let name = child
                    .child(1)
                    .filter(|n| n.kind() == "identifier")
                    .map(|n| format!("*{}", &source[n.byte_range()]));
                if let Some(name) = name {
                    params.push(RawParam {
                        name,
                        type_annotation: None,
                    });
                }
            }
            // **kwargs
            "dictionary_splat_pattern" => {
                let name = child
                    .child(1)
                    .filter(|n| n.kind() == "identifier")
                    .map(|n| format!("**{}", &source[n.byte_range()]));
                if let Some(name) = name {
                    params.push(RawParam {
                        name,
                        type_annotation: None,
                    });
                }
            }
            _ => {}
        }
    }
    let return_type = node
        .child_by_field_name("return_type")
        .map(|rt| source[rt.byte_range()].to_string());
    Some(RawSignature {
        parameters: params,
        return_type,
    })
}

/// Extract signature from a JS/TS function_declaration, method_definition, or arrow_function.
pub fn extract_js_signature(node: &tree_sitter::Node, source: &str) -> Option<RawSignature> {
    // Find formal_parameters (may be direct child or via field)
    let params_node = node.child_by_field_name("parameters").or_else(|| {
        let mut c = node.walk();
        node.children(&mut c)
            .find(|n| n.kind() == "formal_parameters")
    })?;
    let mut params = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                params.push(RawParam {
                    name: source[child.byte_range()].to_string(),
                    type_annotation: None,
                });
            }
            "required_parameter" | "optional_parameter" => {
                let name = child
                    .child_by_field_name("pattern")
                    .or_else(|| child.child_by_field_name("name"))
                    .map(|n| source[n.byte_range()].to_string());
                let type_ann = child.child_by_field_name("type").map(|t| {
                    source[t.byte_range()]
                        .trim_start_matches(':')
                        .trim()
                        .to_string()
                });
                if let Some(name) = name {
                    params.push(RawParam {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            // Simple parameter with type annotation
            "assignment_pattern" => {
                if let Some(left) = child.child_by_field_name("left") {
                    params.push(RawParam {
                        name: source[left.byte_range()].to_string(),
                        type_annotation: None,
                    });
                }
            }
            // ...rest parameter (JS: rest_pattern, TS: rest_parameter)
            "rest_parameter" | "rest_pattern" => {
                let name = child
                    .child_by_field_name("pattern")
                    .or_else(|| child.child_by_field_name("name"))
                    .or_else(|| {
                        // Fallback: find first identifier child after "..."
                        let mut gc = child.walk();
                        child.children(&mut gc).find(|n| n.kind() == "identifier")
                    })
                    .map(|n| format!("...{}", &source[n.byte_range()]));
                let type_ann = child.child_by_field_name("type").map(|t| {
                    source[t.byte_range()]
                        .trim_start_matches(':')
                        .trim()
                        .to_string()
                });
                if let Some(name) = name {
                    params.push(RawParam {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            _ => {}
        }
    }
    // Return type (TS only — type_annotation child on the function)
    let return_type = node.child_by_field_name("return_type").map(|rt| {
        source[rt.byte_range()]
            .trim_start_matches(':')
            .trim()
            .to_string()
    });
    Some(RawSignature {
        parameters: params,
        return_type,
    })
}

/// Extract signature from a Go function_declaration or method_declaration.
pub fn extract_go_signature(node: &tree_sitter::Node, source: &str) -> Option<RawSignature> {
    let params_node = node.child_by_field_name("parameters")?;
    let mut params = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            let type_ann = child
                .child_by_field_name("type")
                .map(|t| source[t.byte_range()].to_string());
            // Go allows grouped params: `func foo(a, b int)` → multiple name children
            let mut gc = child.walk();
            let names: Vec<String> = child
                .children(&mut gc)
                .filter(|n| n.kind() == "identifier")
                .map(|n| source[n.byte_range()].to_string())
                .collect();
            if names.is_empty() {
                // Fallback to field-based access
                if let Some(name) = child
                    .child_by_field_name("name")
                    .map(|n| source[n.byte_range()].to_string())
                {
                    params.push(RawParam {
                        name,
                        type_annotation: type_ann,
                    });
                }
            } else {
                for name in names {
                    params.push(RawParam {
                        name,
                        type_annotation: type_ann.clone(),
                    });
                }
            }
        }
    }
    let return_type = node
        .child_by_field_name("result")
        .map(|rt| source[rt.byte_range()].to_string());
    Some(RawSignature {
        parameters: params,
        return_type,
    })
}

/// Extract signature from a Java method_declaration or constructor_declaration.
pub fn extract_java_signature(node: &tree_sitter::Node, source: &str) -> Option<RawSignature> {
    let params_node = node.child_by_field_name("parameters")?;
    let mut params = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "formal_parameter" || child.kind() == "spread_parameter" {
            let name = child
                .child_by_field_name("name")
                .map(|n| source[n.byte_range()].to_string());
            let type_ann = child
                .child_by_field_name("type")
                .map(|t| source[t.byte_range()].to_string());
            if let Some(name) = name {
                params.push(RawParam {
                    name,
                    type_annotation: type_ann,
                });
            }
        }
    }
    // Return type: the `type` field on the method_declaration
    let return_type = node
        .child_by_field_name("type")
        .map(|t| source[t.byte_range()].to_string());
    Some(RawSignature {
        parameters: params,
        return_type,
    })
}

/// Extract signature from a C/C++ function_definition.
pub fn extract_c_signature(node: &tree_sitter::Node, source: &str) -> Option<RawSignature> {
    if node.kind() != "function_definition" {
        return None;
    }
    // Return type is the `type` field on the function_definition
    let return_type = node
        .child_by_field_name("type")
        .map(|t| source[t.byte_range()].to_string());
    // Parameters: declarator → parameter_list
    let declarator = node.child_by_field_name("declarator")?;
    let param_list = find_child_kind(&declarator, "parameter_list")?;
    let mut params = Vec::new();
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            let type_ann = child
                .child_by_field_name("type")
                .map(|t| source[t.byte_range()].to_string());
            let name = child.child_by_field_name("declarator").map(|d| {
                source[d.byte_range()]
                    .trim_start_matches('*')
                    .trim()
                    .to_string()
            });
            if let Some(name) = name {
                params.push(RawParam {
                    name,
                    type_annotation: type_ann,
                });
            }
        }
    }
    Some(RawSignature {
        parameters: params,
        return_type,
    })
}

/// Extract signature from a C# method_declaration or constructor_declaration.
pub fn extract_csharp_signature(node: &tree_sitter::Node, source: &str) -> Option<RawSignature> {
    let params_node = node.child_by_field_name("parameters")?;
    let mut params = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "parameter" {
            let name = child
                .child_by_field_name("name")
                .map(|n| source[n.byte_range()].to_string());
            let type_ann = child
                .child_by_field_name("type")
                .map(|t| source[t.byte_range()].to_string());
            if let Some(name) = name {
                params.push(RawParam {
                    name,
                    type_annotation: type_ann,
                });
            }
        }
    }
    // C# grammar uses "returns" field, not "type", for method return type
    let return_type = node
        .child_by_field_name("returns")
        .or_else(|| node.child_by_field_name("type"))
        .map(|t| source[t.byte_range()].to_string());
    Some(RawSignature {
        parameters: params,
        return_type,
    })
}

/// Find the first child of a specific kind (helper for C signature extraction).
fn find_child_kind<'a>(node: &tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
        if let Some(found) = find_child_kind(&child, kind) {
            return Some(found);
        }
    }
    None
}
