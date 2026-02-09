//! Extract dependencies (imports, calls, inheritance) from AST.

use crate::languages::Language;
use std::path::Path;

/// Raw dependency information extracted from a single file.
#[derive(Debug, Clone, Default)]
pub struct RawDeps {
    pub imports: Vec<ImportDep>,
    pub calls: Vec<CallDep>,
    pub inherits: Vec<InheritDep>,
    pub composes: Vec<ComposeDep>,
    pub renders: Vec<CallDep>,
    pub reads_state: Vec<CallDep>,
    pub writes_state: Vec<CallDep>,
    pub dispatches: Vec<CallDep>,
}

impl RawDeps {
    /// All call-like dep vectors with their corresponding edge kinds.
    /// Used by grounding to generically map caller→entity deps.
    pub fn call_dep_vectors(&self) -> Vec<(rpg_core::graph::EdgeKind, &Vec<CallDep>)> {
        use rpg_core::graph::EdgeKind;
        vec![
            (EdgeKind::Invokes, &self.calls),
            (EdgeKind::Renders, &self.renders),
            (EdgeKind::ReadsState, &self.reads_state),
            (EdgeKind::WritesState, &self.writes_state),
            (EdgeKind::Dispatches, &self.dispatches),
        ]
    }
}

/// A raw import dependency (module + imported symbols).
#[derive(Debug, Clone)]
pub struct ImportDep {
    pub module: String,
    pub symbols: Vec<String>,
}

/// A raw function call dependency (caller → callee).
#[derive(Debug, Clone)]
pub struct CallDep {
    pub caller_entity: String,
    pub callee: String,
}

/// A raw inheritance dependency (child class → parent class).
#[derive(Debug, Clone)]
pub struct InheritDep {
    pub child_class: String,
    pub parent_class: String,
}

/// A raw composition dependency (re-export or aggregation).
#[derive(Debug, Clone)]
pub struct ComposeDep {
    pub source_entity: String,
    pub target_name: String,
}

/// A scope (function or method) that can contain call sites.
#[derive(Debug, Clone)]
pub struct FunctionScope {
    pub name: String,
    pub start_row: usize,
    pub end_row: usize,
}

/// Extract dependency info from a Python source file.
pub fn extract_python_deps(_path: &Path, source: &str) -> RawDeps {
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();
    let mut cursor = root.walk();

    // First pass: collect imports, inheritance, and function scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_python_scopes(&root, source, &mut scopes, None);

    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" | "import_from_statement" => {
                let text = &source[child.byte_range()];
                if let Some(import) = parse_python_import(text) {
                    deps.imports.push(import);
                }
            }
            "class_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    if let Some(bases) = child.child_by_field_name("superclasses") {
                        let bases_text = &source[bases.byte_range()];
                        for base in parse_python_bases(bases_text) {
                            deps.inherits.push(InheritDep {
                                child_class: class_name.clone(),
                                parent_class: base,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Second pass: collect call expressions
    collect_python_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

/// Recursively collect function/method scopes from Python AST.
fn collect_python_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    collect_python_scopes(&child, source, scopes, Some(&class_name));
                }
                continue; // Don't recurse again below
            }
            _ => {}
        }
        collect_python_scopes(&child, source, scopes, parent_class);
    }
}

/// Recursively collect call expressions from Python AST.
fn collect_python_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call"
            && let Some(func_node) = child.child_by_field_name("function")
        {
            let callee = extract_callee_name(&func_node, source);
            if !callee.is_empty() {
                let call_row = child.start_position().row;
                let caller = find_enclosing_scope(scopes, call_row)
                    .unwrap_or_else(|| "<module>".to_string());
                calls.push(CallDep {
                    caller_entity: caller,
                    callee,
                });
            }
        }
        collect_python_calls(&child, source, scopes, calls);
    }
}

/// Extract a callee name from a call's function node.
fn extract_callee_name(node: &tree_sitter::Node, source: &str) -> String {
    match node.kind() {
        "identifier" => source[node.byte_range()].to_string(),
        "attribute" => {
            // obj.method -> extract just the method name
            if let Some(attr) = node.child_by_field_name("attribute") {
                source[attr.byte_range()].to_string()
            } else {
                source[node.byte_range()].to_string()
            }
        }
        _ => {
            // For complex expressions, take the rightmost identifier
            let text = &source[node.byte_range()];
            text.rsplit('.').next().unwrap_or("").trim().to_string()
        }
    }
}

/// Find which function scope encloses a given line.
pub fn find_enclosing_scope(scopes: &[FunctionScope], row: usize) -> Option<String> {
    // Find the innermost (smallest range) scope containing this row
    scopes
        .iter()
        .filter(|s| row >= s.start_row && row <= s.end_row)
        .min_by_key(|s| s.end_row - s.start_row)
        .map(|s| s.name.clone())
}

/// Extract callee name from a Rust call expression's function node.
fn extract_rust_callee(node: &tree_sitter::Node, source: &str) -> String {
    let text = &source[node.byte_range()];
    // For paths like foo::bar::baz(), extract "baz"
    // For simple identifiers, return as-is
    text.rsplit("::").next().unwrap_or("").trim().to_string()
}

fn parse_python_import(text: &str) -> Option<ImportDep> {
    let text = text.trim();
    if text.starts_with("from ") {
        // from module import symbols
        let parts: Vec<&str> = text.splitn(2, " import ").collect();
        if parts.len() == 2 {
            let module = parts[0].trim_start_matches("from ").trim().to_string();
            let symbols: Vec<String> = parts[1]
                .split(',')
                .map(|s| {
                    s.trim()
                        .split(" as ")
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                })
                .filter(|s| !s.is_empty() && s != "*")
                .collect();
            return Some(ImportDep { module, symbols });
        }
    } else if text.starts_with("import ") {
        let module = text
            .trim_start_matches("import ")
            .split(" as ")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        return Some(ImportDep {
            module,
            symbols: Vec::new(),
        });
    }
    None
}

fn parse_python_bases(text: &str) -> Vec<String> {
    let text = text.trim_start_matches('(').trim_end_matches(')');
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Extract dependency info from a Rust source file.
pub fn extract_rust_deps(_path: &Path, source: &str) -> RawDeps {
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();
    let mut cursor = root.walk();

    // Collect function scopes and use declarations
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_rust_scopes(&root, source, &mut scopes, None);

    for child in root.children(&mut cursor) {
        if child.kind() == "use_declaration" {
            let text = &source[child.byte_range()];
            let import = parse_rust_use(text);
            deps.imports.push(import);
        }
    }

    // Collect call expressions
    collect_rust_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

/// Recursively collect function/method scopes from Rust AST.
fn collect_rust_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_type: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_type {
                        Some(typ) => format!("{}::{}", typ, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "impl_item" => {
                let type_name = child
                    .child_by_field_name("type")
                    .map(|n| source[n.byte_range()].to_string());
                collect_rust_scopes(&child, source, scopes, type_name.as_deref());
                continue;
            }
            _ => {}
        }
        collect_rust_scopes(&child, source, scopes, parent_type);
    }
}

/// Recursively collect call expressions from Rust AST.
fn collect_rust_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                // foo::bar() or foo()
                if let Some(func_node) = child.child_by_field_name("function") {
                    let callee = extract_rust_callee(&func_node, source);
                    if !callee.is_empty() {
                        let call_row = child.start_position().row;
                        let caller = find_enclosing_scope(scopes, call_row)
                            .unwrap_or_else(|| "<module>".to_string());
                        calls.push(CallDep {
                            caller_entity: caller,
                            callee,
                        });
                    }
                }
            }
            "method_call_expression" | "field_expression" => {
                // obj.method() — tree-sitter-rust uses "method_call_expression" for x.foo()
                // but some versions nest it differently
                if child.kind() == "method_call_expression" {
                    // The method name is in the "name" field
                    if let Some(method_node) = child.child_by_field_name("name") {
                        let callee = source[method_node.byte_range()].to_string();
                        if !callee.is_empty() {
                            let call_row = child.start_position().row;
                            let caller = find_enclosing_scope(scopes, call_row)
                                .unwrap_or_else(|| "<module>".to_string());
                            calls.push(CallDep {
                                caller_entity: caller,
                                callee,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        collect_rust_calls(&child, source, scopes, calls);
    }
}

fn parse_rust_use(text: &str) -> ImportDep {
    let mut text = text.trim();
    // Strip visibility modifiers: pub, pub(crate), pub(super), pub(in path)
    if text.starts_with("pub") {
        if let Some(rest) = text.strip_prefix("pub(") {
            if let Some(idx) = rest.find(')') {
                text = rest[idx + 1..].trim();
            }
        } else {
            text = text.strip_prefix("pub ").unwrap_or(text);
        }
    }
    let text = text.trim_start_matches("use ").trim_end_matches(';');
    // Simple case: use foo::bar::Baz;
    let parts: Vec<&str> = text.rsplitn(2, "::").collect();
    if parts.len() == 2 {
        let module = parts[1].to_string();
        let symbol = parts[0].trim().to_string();
        if symbol.starts_with('{') {
            // use foo::{A, B, C}
            let symbols: Vec<String> = symbol
                .trim_start_matches('{')
                .trim_end_matches('}')
                .split(',')
                .map(|s| {
                    s.trim()
                        .split(" as ")
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                })
                .filter(|s| !s.is_empty())
                .collect();
            ImportDep { module, symbols }
        } else {
            ImportDep {
                module,
                symbols: vec![symbol],
            }
        }
    } else {
        ImportDep {
            module: text.to_string(),
            symbols: Vec::new(),
        }
    }
}

/// Generic dependency extraction dispatching to the correct language extractor.
pub fn extract_deps(path: &Path, source: &str, language: Language) -> RawDeps {
    if let Some(name) = crate::languages::builtin_dep_extractor_name(language) {
        match name {
            "extract_python_deps" => return extract_python_deps(path, source),
            "extract_rust_deps" => return extract_rust_deps(path, source),
            "extract_js_deps" => return extract_js_deps(path, source, language),
            "extract_go_deps" => return extract_go_deps(path, source),
            "extract_java_deps" => return extract_java_deps(path, source),
            "extract_c_deps" => return extract_c_deps(path, source, language),
            "extract_csharp_deps" => return extract_csharp_deps(path, source),
            "extract_php_deps" => return extract_php_deps(path, source),
            "extract_ruby_deps" => return extract_ruby_deps(path, source),
            "extract_kotlin_deps" => return extract_kotlin_deps(path, source),
            "extract_swift_deps" => return extract_swift_deps(path, source),
            "extract_scala_deps" => return extract_scala_deps(path, source),
            "extract_bash_deps" => return extract_bash_deps(path, source),
            other => {
                eprintln!(
                    "warning: unrecognized dep extractor '{}' for {:?}",
                    other, language
                );
            }
        }
    }
    RawDeps::default()
}

// ---------------------------------------------------------------------------
// TypeScript / JavaScript
// ---------------------------------------------------------------------------

/// Extract deps from TypeScript or JavaScript source.
pub fn extract_js_deps(_path: &Path, source: &str, language: Language) -> RawDeps {
    let ts_lang = language.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_js_scopes(&root, source, &mut scopes, None);

    // Core deps: imports, inheritance, and calls.
    // Paradigm-specific deps (JSX renders, Redux state signals) are handled
    // by the TOML-driven paradigm engine (query_engine.rs + features.rs).
    collect_js_imports(&root, source, &mut deps);
    collect_js_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

/// Build function scopes from source for a given language.
///
/// Convenience wrapper that handles tree-sitter parsing internally.
/// Only produces scopes for JS/TS languages; returns empty for others.
pub fn build_scopes(source: &str, language: Language) -> Vec<FunctionScope> {
    if language != Language::TYPESCRIPT && language != Language::JAVASCRIPT {
        return Vec::new();
    }
    let ts_lang = language.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let mut scopes = Vec::new();
    collect_js_scopes(&root, source, &mut scopes, None);
    scopes
}

pub fn collect_js_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    scopes.push(FunctionScope {
                        name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                    // Don't recurse into function bodies — nested arrow functions
                    // (handleSubmit, useEffect callbacks) are not entities and should
                    // not become scopes. Deps inside them bubble up to this scope.
                    continue;
                }
            }
            "method_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                    // Don't recurse into method bodies (same rationale as above).
                    continue;
                }
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_js_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            // Arrow functions: const Foo = () => {}
            "lexical_declaration" | "variable_declaration" => {
                let mut found_scope = false;
                let mut inner = child.walk();
                for decl in child.children(&mut inner) {
                    if decl.kind() == "variable_declarator" {
                        let has_arrow = has_child_kind(&decl, "arrow_function");
                        let has_func = has_child_kind(&decl, "function");
                        if (has_arrow || has_func)
                            && let Some(name_node) = decl.child_by_field_name("name")
                        {
                            let name = source[name_node.byte_range()].to_string();
                            scopes.push(FunctionScope {
                                name,
                                start_row: child.start_position().row,
                                end_row: child.end_position().row,
                            });
                            found_scope = true;
                        }
                    }
                }
                // Don't recurse into arrow/function bodies (same rationale).
                if found_scope {
                    continue;
                }
            }
            _ => {}
        }
        collect_js_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_js_imports(node: &tree_sitter::Node, source: &str, deps: &mut RawDeps) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                if let Some(src_node) = child.child_by_field_name("source") {
                    let module = source[src_node.byte_range()]
                        .trim_matches(|c: char| c == '\'' || c == '"')
                        .to_string();
                    // Collect named imports
                    let mut symbols = Vec::new();
                    let mut ic = child.walk();
                    for import_child in child.children(&mut ic) {
                        if import_child.kind() == "import_clause" {
                            collect_js_import_names(&import_child, source, &mut symbols);
                        }
                    }
                    deps.imports.push(ImportDep { module, symbols });
                }
            }
            "class_declaration" => {
                // Check for extends
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut ic = child.walk();
                    for c in child.children(&mut ic) {
                        if c.kind() == "class_heritage" {
                            let heritage_text = &source[c.byte_range()];
                            // "extends Foo" or "extends Foo implements Bar"
                            for part in heritage_text.split_whitespace() {
                                if part != "extends" && part != "implements" {
                                    let parent = part.trim_end_matches(',').to_string();
                                    if !parent.is_empty() {
                                        deps.inherits.push(InheritDep {
                                            child_class: class_name.clone(),
                                            parent_class: parent,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "export_statement" => {
                // Detect barrel re-exports: export { X } from './Y' or export * from './Y'
                if let Some(src_node) = child.child_by_field_name("source") {
                    let module = source[src_node.byte_range()]
                        .trim_matches(|c: char| c == '\'' || c == '"')
                        .to_string();

                    let mut ic = child.walk();
                    let mut specifier_names = Vec::new();
                    let mut has_star = false;

                    for ec in child.children(&mut ic) {
                        match ec.kind() {
                            "export_clause" => {
                                let mut sc = ec.walk();
                                for spec in ec.children(&mut sc) {
                                    if spec.kind() == "export_specifier" {
                                        // Use alias if present (export { default as Foo }),
                                        // otherwise use the name field
                                        let name_node = spec
                                            .child_by_field_name("alias")
                                            .or_else(|| spec.child_by_field_name("name"));
                                        if let Some(n) = name_node {
                                            specifier_names
                                                .push(source[n.byte_range()].to_string());
                                        }
                                    }
                                }
                            }
                            "namespace_export" | "*" => {
                                has_star = true;
                            }
                            _ => {}
                        }
                    }

                    if has_star {
                        // export * from './module' — compose the module file path
                        // Use the module path so resolve_dep can match the Module entity
                        let target = module
                            .trim_start_matches("./")
                            .trim_start_matches("../")
                            .to_string();
                        deps.composes.push(ComposeDep {
                            source_entity: "<module>".to_string(),
                            target_name: target,
                        });
                        deps.imports.push(ImportDep {
                            module: module.clone(),
                            symbols: Vec::new(),
                        });
                    }

                    for name in &specifier_names {
                        deps.composes.push(ComposeDep {
                            source_entity: "<module>".to_string(),
                            target_name: name.clone(),
                        });
                    }

                    if !specifier_names.is_empty() {
                        deps.imports.push(ImportDep {
                            module,
                            symbols: specifier_names,
                        });
                    }
                } else {
                    // No source → regular export wrapping a declaration
                    collect_js_imports(&child, source, deps);
                }
            }
            _ => {}
        }
    }
}

fn collect_js_import_names(node: &tree_sitter::Node, source: &str, symbols: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                symbols.push(source[child.byte_range()].to_string());
            }
            "import_specifier" => {
                if let Some(name) = child.child_by_field_name("name") {
                    symbols.push(source[name.byte_range()].to_string());
                }
            }
            _ => {
                collect_js_import_names(&child, source, symbols);
            }
        }
    }
}

fn collect_js_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression"
            && let Some(func_node) = child.child_by_field_name("function")
        {
            let callee = extract_callee_name(&func_node, source);
            if !callee.is_empty() {
                let call_row = child.start_position().row;
                let caller = find_enclosing_scope(scopes, call_row)
                    .unwrap_or_else(|| "<module>".to_string());
                calls.push(CallDep {
                    caller_entity: caller,
                    callee,
                });
            }
        }
        collect_js_calls(&child, source, scopes, calls);
    }
}

fn has_child_kind(node: &tree_sitter::Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|c| c.kind() == kind)
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

/// Extract deps from Go source.
pub fn extract_go_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::GO.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_go_scopes(&root, source, &mut scopes);

    // Collect imports
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            collect_go_imports(&child, source, &mut deps.imports);
        }
    }

    // Collect calls
    collect_go_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_go_scopes(node: &tree_sitter::Node, source: &str, scopes: &mut Vec<FunctionScope>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    scopes.push(FunctionScope {
                        name: source[name_node.byte_range()].to_string(),
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let receiver = child
                        .child_by_field_name("receiver")
                        .and_then(|r| {
                            let mut c = r.walk();
                            r.children(&mut c)
                                .find(|n| n.kind() == "parameter_declaration")
                        })
                        .and_then(|pd| pd.child_by_field_name("type"))
                        .map(|t| source[t.byte_range()].trim_start_matches('*').to_string());

                    let scope_name = match receiver {
                        Some(ref r) => format!("{}.{}", r, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            _ => {}
        }
    }
}

fn collect_go_imports(node: &tree_sitter::Node, source: &str, imports: &mut Vec<ImportDep>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_spec" {
            let path_node = child.child_by_field_name("path");
            if let Some(pn) = path_node {
                let module = source[pn.byte_range()].trim_matches('"').to_string();
                imports.push(ImportDep {
                    module,
                    symbols: Vec::new(),
                });
            }
        } else if child.kind() == "import_spec_list" {
            collect_go_imports(&child, source, imports);
        } else if child.kind() == "interpreted_string_literal" {
            // Single import without parens
            let module = source[child.byte_range()].trim_matches('"').to_string();
            imports.push(ImportDep {
                module,
                symbols: Vec::new(),
            });
        }
    }
}

fn collect_go_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression"
            && let Some(func_node) = child.child_by_field_name("function")
        {
            let callee = extract_callee_name(&func_node, source);
            if !callee.is_empty() {
                let caller = find_enclosing_scope(scopes, child.start_position().row)
                    .unwrap_or_else(|| "<module>".to_string());
                calls.push(CallDep {
                    caller_entity: caller,
                    callee,
                });
            }
        }
        collect_go_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

/// Extract deps from Java source.
pub fn extract_java_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::JAVA.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_java_scopes(&root, source, &mut scopes, None);

    // Collect imports and inheritance
    collect_java_imports_and_inheritance(&root, source, &mut deps);

    // Collect calls
    collect_java_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_java_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" | "constructor_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_java_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            _ => {}
        }
        collect_java_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_java_imports_and_inheritance(
    node: &tree_sitter::Node,
    source: &str,
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                let text = source[child.byte_range()].trim().to_string();
                let module = text
                    .trim_start_matches("import ")
                    .trim_start_matches("static ")
                    .trim_end_matches(';')
                    .trim()
                    .to_string();

                let parts: Vec<&str> = module.rsplitn(2, '.').collect();
                if parts.len() == 2 {
                    deps.imports.push(ImportDep {
                        module: parts[1].to_string(),
                        symbols: vec![parts[0].to_string()],
                    });
                } else {
                    deps.imports.push(ImportDep {
                        module,
                        symbols: Vec::new(),
                    });
                }
            }
            "class_declaration" | "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();

                    // Check superclass
                    if let Some(sc) = child.child_by_field_name("superclass") {
                        // superclass node contains "extends Type"
                        let text = source[sc.byte_range()].trim().to_string();
                        let parent = text.trim_start_matches("extends ").trim().to_string();
                        if !parent.is_empty() {
                            deps.inherits.push(InheritDep {
                                child_class: class_name.clone(),
                                parent_class: parent,
                            });
                        }
                    }

                    // Check interfaces
                    if let Some(ifaces) = child.child_by_field_name("interfaces") {
                        let text = source[ifaces.byte_range()].trim().to_string();
                        let text = text
                            .trim_start_matches("implements ")
                            .trim_start_matches("extends ");
                        for iface in text.split(',') {
                            let iface = iface.trim().to_string();
                            if !iface.is_empty() {
                                deps.inherits.push(InheritDep {
                                    child_class: class_name.clone(),
                                    parent_class: iface,
                                });
                            }
                        }
                    }
                }
                // Recurse for nested classes
                collect_java_imports_and_inheritance(&child, source, deps);
            }
            _ => {
                collect_java_imports_and_inheritance(&child, source, deps);
            }
        }
    }
}

fn collect_java_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "method_invocation"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let callee = source[name_node.byte_range()].to_string();
            if !callee.is_empty() {
                let caller = find_enclosing_scope(scopes, child.start_position().row)
                    .unwrap_or_else(|| "<module>".to_string());
                calls.push(CallDep {
                    caller_entity: caller,
                    callee,
                });
            }
        }
        collect_java_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// C / C++
// ---------------------------------------------------------------------------

/// Extract deps from C or C++ source.
pub fn extract_c_deps(_path: &Path, source: &str, language: Language) -> RawDeps {
    let ts_lang = language.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_c_scopes(&root, source, &mut scopes);

    // Collect #include directives
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "preproc_include"
            && let Some(path_node) = child.child_by_field_name("path")
        {
            let include_path = source[path_node.byte_range()]
                .trim_matches(|c: char| c == '"' || c == '<' || c == '>')
                .to_string();
            deps.imports.push(ImportDep {
                module: include_path,
                symbols: Vec::new(),
            });
        }
    }

    // C++: collect inheritance from class_specifier
    if language == Language::CPP {
        collect_cpp_inheritance(&root, source, &mut deps.inherits);
    }

    // Collect calls
    collect_c_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_c_scopes(node: &tree_sitter::Node, source: &str, scopes: &mut Vec<FunctionScope>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition"
            && let Some(decl) = child.child_by_field_name("declarator")
            && let Some(name) = super::entities::extract_c_declarator_name(&decl, source)
        {
            scopes.push(FunctionScope {
                name,
                start_row: child.start_position().row,
                end_row: child.end_position().row,
            });
        }
        collect_c_scopes(&child, source, scopes);
    }
}

fn collect_cpp_inheritance(node: &tree_sitter::Node, source: &str, inherits: &mut Vec<InheritDep>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if (child.kind() == "class_specifier" || child.kind() == "struct_specifier")
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let class_name = source[name_node.byte_range()].to_string();
            // Look for base_class_clause
            let mut ic = child.walk();
            for c in child.children(&mut ic) {
                if c.kind() == "base_class_clause" {
                    let text = &source[c.byte_range()];
                    // Parse ": public Base, private Other"
                    let text = text.trim_start_matches(':').trim();
                    for base in text.split(',') {
                        let base = base.trim();
                        // Skip access specifiers
                        let parent = base
                            .trim_start_matches("public ")
                            .trim_start_matches("protected ")
                            .trim_start_matches("private ")
                            .trim()
                            .to_string();
                        if !parent.is_empty() {
                            inherits.push(InheritDep {
                                child_class: class_name.clone(),
                                parent_class: parent,
                            });
                        }
                    }
                }
            }
        }
        collect_cpp_inheritance(&child, source, inherits);
    }
}

fn collect_c_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression"
            && let Some(func_node) = child.child_by_field_name("function")
        {
            let callee = extract_callee_name(&func_node, source);
            if !callee.is_empty() {
                let caller = find_enclosing_scope(scopes, child.start_position().row)
                    .unwrap_or_else(|| "<module>".to_string());
                calls.push(CallDep {
                    caller_entity: caller,
                    callee,
                });
            }
        }
        collect_c_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// C#
// ---------------------------------------------------------------------------

/// Extract deps from C# source.
pub fn extract_csharp_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::CSHARP.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_csharp_scopes(&root, source, &mut scopes, None);

    // Collect imports and inheritance
    collect_csharp_imports_and_inheritance(&root, source, &mut deps);

    // Collect calls
    collect_csharp_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_csharp_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" | "constructor_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class_declaration" | "struct_declaration" | "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_csharp_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            _ => {}
        }
        collect_csharp_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_csharp_imports_and_inheritance(
    node: &tree_sitter::Node,
    source: &str,
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "using_directive" => {
                let text = source[child.byte_range()].trim().to_string();
                let module = text
                    .trim_start_matches("global ")
                    .trim_start_matches("using ")
                    .trim_start_matches("static ")
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                // Skip alias usings like "using Foo = Bar.Baz"
                if !module.contains('=') {
                    deps.imports.push(ImportDep {
                        module,
                        symbols: Vec::new(),
                    });
                }
            }
            "class_declaration" | "struct_declaration" | "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    // Check for base_list (: BaseClass, IInterface)
                    let mut ic = child.walk();
                    for c in child.children(&mut ic) {
                        if c.kind() == "base_list" {
                            let text = &source[c.byte_range()];
                            // Strip leading ":"
                            let text = text.trim_start_matches(':').trim();
                            for base in text.split(',') {
                                let parent = base.trim().to_string();
                                // Strip generic type parameters
                                let parent =
                                    parent.split('<').next().unwrap_or("").trim().to_string();
                                if !parent.is_empty() {
                                    deps.inherits.push(InheritDep {
                                        child_class: class_name.clone(),
                                        parent_class: parent,
                                    });
                                }
                            }
                        }
                    }
                }
                collect_csharp_imports_and_inheritance(&child, source, deps);
            }
            _ => {
                collect_csharp_imports_and_inheritance(&child, source, deps);
            }
        }
    }
}

fn collect_csharp_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "invocation_expression" {
            // The function part is the first child
            if let Some(func_node) = child.child(0) {
                let callee = match func_node.kind() {
                    "member_access_expression" => {
                        // obj.Method() — extract method name
                        func_node
                            .child_by_field_name("name")
                            .map(|n| source[n.byte_range()].to_string())
                            .unwrap_or_default()
                    }
                    "identifier" => source[func_node.byte_range()].to_string(),
                    _ => {
                        let text = &source[func_node.byte_range()];
                        text.rsplit('.').next().unwrap_or("").trim().to_string()
                    }
                };
                if !callee.is_empty() {
                    let caller = find_enclosing_scope(scopes, child.start_position().row)
                        .unwrap_or_else(|| "<module>".to_string());
                    calls.push(CallDep {
                        caller_entity: caller,
                        callee,
                    });
                }
            }
        }
        collect_csharp_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// PHP
// ---------------------------------------------------------------------------

/// Extract deps from PHP source.
pub fn extract_php_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::PHP.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_php_scopes(&root, source, &mut scopes, None);

    // Collect imports and inheritance
    collect_php_imports_and_inheritance(&root, source, &mut deps);

    // Collect calls
    collect_php_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_php_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_php_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            _ => {}
        }
        collect_php_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_php_imports_and_inheritance(node: &tree_sitter::Node, source: &str, deps: &mut RawDeps) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_declaration" => {
                // use Foo\Bar\Baz; or use Foo\Bar\{Baz, Qux};
                let text = source[child.byte_range()].trim().to_string();
                let module = text
                    .trim_start_matches("use ")
                    .trim_start_matches("function ")
                    .trim_start_matches("const ")
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                deps.imports.push(ImportDep {
                    module,
                    symbols: Vec::new(),
                });
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    // Check for extends (base_clause)
                    let mut ic = child.walk();
                    for c in child.children(&mut ic) {
                        match c.kind() {
                            "base_clause" => {
                                // extends ParentClass
                                let text = &source[c.byte_range()];
                                let text = text.trim_start_matches("extends").trim();
                                for parent in text.split(',') {
                                    let parent = parent.trim().to_string();
                                    if !parent.is_empty() {
                                        deps.inherits.push(InheritDep {
                                            child_class: class_name.clone(),
                                            parent_class: parent,
                                        });
                                    }
                                }
                            }
                            "class_interface_clause" => {
                                // implements Interface1, Interface2
                                let text = &source[c.byte_range()];
                                let text = text.trim_start_matches("implements").trim();
                                for iface in text.split(',') {
                                    let iface = iface.trim().to_string();
                                    if !iface.is_empty() {
                                        deps.inherits.push(InheritDep {
                                            child_class: class_name.clone(),
                                            parent_class: iface,
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                collect_php_imports_and_inheritance(&child, source, deps);
            }
            _ => {
                collect_php_imports_and_inheritance(&child, source, deps);
            }
        }
    }
}

fn collect_php_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_call_expression" => {
                if let Some(func_node) = child.child_by_field_name("function") {
                    let callee = extract_callee_name(&func_node, source);
                    if !callee.is_empty() {
                        let caller = find_enclosing_scope(scopes, child.start_position().row)
                            .unwrap_or_else(|| "<module>".to_string());
                        calls.push(CallDep {
                            caller_entity: caller,
                            callee,
                        });
                    }
                }
            }
            "member_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = source[name_node.byte_range()].to_string();
                    if !callee.is_empty() {
                        let caller = find_enclosing_scope(scopes, child.start_position().row)
                            .unwrap_or_else(|| "<module>".to_string());
                        calls.push(CallDep {
                            caller_entity: caller,
                            callee,
                        });
                    }
                }
            }
            "scoped_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = source[name_node.byte_range()].to_string();
                    if !callee.is_empty() {
                        let caller = find_enclosing_scope(scopes, child.start_position().row)
                            .unwrap_or_else(|| "<module>".to_string());
                        calls.push(CallDep {
                            caller_entity: caller,
                            callee,
                        });
                    }
                }
            }
            _ => {}
        }
        collect_php_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// Ruby
// ---------------------------------------------------------------------------

/// Extract deps from Ruby source.
pub fn extract_ruby_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::RUBY.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_ruby_scopes(&root, source, &mut scopes, None);

    // Collect imports, inheritance, and calls
    collect_ruby_imports_and_inheritance(&root, source, &mut deps);
    collect_ruby_calls(&root, source, &scopes, &mut deps);

    deps
}

fn collect_ruby_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "singleton_method" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_ruby_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            _ => {}
        }
        collect_ruby_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_ruby_imports_and_inheritance(
    node: &tree_sitter::Node,
    source: &str,
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                // require 'foo' or require_relative 'bar'
                if let Some(method_node) = child.child_by_field_name("method") {
                    let method_name = &source[method_node.byte_range()];
                    if (method_name == "require" || method_name == "require_relative")
                        && let Some(args) = child.child_by_field_name("arguments")
                    {
                        let text = source[args.byte_range()]
                            .trim_matches(|c: char| {
                                c == '(' || c == ')' || c == '\'' || c == '"' || c == ' '
                            })
                            .to_string();
                        if !text.is_empty() {
                            deps.imports.push(ImportDep {
                                module: text,
                                symbols: Vec::new(),
                            });
                        }
                    }
                }
            }
            "class" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    // Check for superclass
                    if let Some(superclass) = child.child_by_field_name("superclass") {
                        let parent = source[superclass.byte_range()].trim().to_string();
                        if !parent.is_empty() {
                            deps.inherits.push(InheritDep {
                                child_class: class_name.clone(),
                                parent_class: parent,
                            });
                        }
                    }
                    // Check for include/extend inside class body
                    collect_ruby_mixins(&child, source, &class_name, deps);
                }
                collect_ruby_imports_and_inheritance(&child, source, deps);
            }
            _ => {
                collect_ruby_imports_and_inheritance(&child, source, deps);
            }
        }
    }
}

/// Collect `include Module` and `extend Module` calls inside a class body.
fn collect_ruby_mixins(
    node: &tree_sitter::Node,
    source: &str,
    class_name: &str,
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call"
            && let Some(method_node) = child.child_by_field_name("method")
        {
            let method_name = &source[method_node.byte_range()];
            if (method_name == "include" || method_name == "extend")
                && let Some(args) = child.child_by_field_name("arguments")
            {
                let mut ac = args.walk();
                for arg in args.children(&mut ac) {
                    if arg.kind() == "constant" || arg.kind() == "scope_resolution" {
                        let mixin = source[arg.byte_range()].trim().to_string();
                        if !mixin.is_empty() {
                            deps.inherits.push(InheritDep {
                                child_class: class_name.to_string(),
                                parent_class: mixin,
                            });
                        }
                    }
                }
            }
        }
        collect_ruby_mixins(&child, source, class_name, deps);
    }
}

fn collect_ruby_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call"
            && let Some(method_node) = child.child_by_field_name("method")
        {
            let callee = source[method_node.byte_range()].to_string();
            // Skip require/require_relative/include/extend — already handled
            if !callee.is_empty()
                && callee != "require"
                && callee != "require_relative"
                && callee != "include"
                && callee != "extend"
            {
                let caller = find_enclosing_scope(scopes, child.start_position().row)
                    .unwrap_or_else(|| "<module>".to_string());
                deps.calls.push(CallDep {
                    caller_entity: caller,
                    callee,
                });
            }
        }
        collect_ruby_calls(&child, source, scopes, deps);
    }
}

// ---------------------------------------------------------------------------
// Kotlin
// ---------------------------------------------------------------------------

/// Extract deps from Kotlin source.
pub fn extract_kotlin_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::KOTLIN.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_kotlin_scopes(&root, source, &mut scopes, None);

    // Collect imports and inheritance
    collect_kotlin_imports_and_inheritance(&root, source, &mut deps);

    // Collect calls
    collect_kotlin_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_kotlin_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class_declaration" | "object_declaration" | "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_kotlin_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            _ => {}
        }
        collect_kotlin_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_kotlin_imports_and_inheritance(
    node: &tree_sitter::Node,
    source: &str,
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // "import_header" (old grammar) / "import" (kotlin-ng grammar)
            "import_header" | "import" if child.is_named() => {
                let text = source[child.byte_range()].trim().to_string();
                let module = text.trim_start_matches("import ").trim().to_string();
                if !module.is_empty() {
                    let parts: Vec<&str> = module.rsplitn(2, '.').collect();
                    if parts.len() == 2 {
                        deps.imports.push(ImportDep {
                            module: parts[1].to_string(),
                            symbols: vec![parts[0].to_string()],
                        });
                    } else {
                        deps.imports.push(ImportDep {
                            module,
                            symbols: Vec::new(),
                        });
                    }
                }
            }
            "class_declaration" | "object_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    // Look for delegation_specifier nodes in the class
                    collect_kotlin_supertypes(&child, source, &class_name, &mut deps.inherits);
                }
                collect_kotlin_imports_and_inheritance(&child, source, deps);
            }
            _ => {
                collect_kotlin_imports_and_inheritance(&child, source, deps);
            }
        }
    }
}

fn collect_kotlin_supertypes(
    node: &tree_sitter::Node,
    source: &str,
    class_name: &str,
    inherits: &mut Vec<InheritDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "delegation_specifier" {
            let text = source[child.byte_range()].trim().to_string();
            // Strip constructor args and generic params
            let parent = text
                .split('(')
                .next()
                .unwrap_or("")
                .split('<')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !parent.is_empty() {
                inherits.push(InheritDep {
                    child_class: class_name.to_string(),
                    parent_class: parent,
                });
            }
        } else {
            collect_kotlin_supertypes(&child, source, class_name, inherits);
        }
    }
}

fn collect_kotlin_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            // The function reference is typically the first child
            if let Some(func_node) = child.child(0) {
                let callee = match func_node.kind() {
                    "simple_identifier" => source[func_node.byte_range()].to_string(),
                    "navigation_expression" => {
                        // obj.method — take the last identifier
                        let text = &source[func_node.byte_range()];
                        text.rsplit('.').next().unwrap_or("").trim().to_string()
                    }
                    _ => {
                        let text = &source[func_node.byte_range()];
                        text.rsplit('.').next().unwrap_or("").trim().to_string()
                    }
                };
                if !callee.is_empty() {
                    let caller = find_enclosing_scope(scopes, child.start_position().row)
                        .unwrap_or_else(|| "<module>".to_string());
                    calls.push(CallDep {
                        caller_entity: caller,
                        callee,
                    });
                }
            }
        }
        collect_kotlin_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// Swift
// ---------------------------------------------------------------------------

/// Extract deps from Swift source.
pub fn extract_swift_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::SWIFT.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_swift_scopes(&root, source, &mut scopes, None);

    // Collect imports and inheritance
    collect_swift_imports_and_inheritance(&root, source, &mut deps);

    // Collect calls
    collect_swift_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_swift_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "init_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                } else if child.kind() == "init_declaration" {
                    // init doesn't always have a "name" field
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.init", cls),
                        None => "init".to_string(),
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class_declaration"
            | "struct_declaration"
            | "protocol_declaration"
            | "enum_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_swift_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            "extension_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let ext_name = source[name_node.byte_range()].to_string();
                    collect_swift_scopes(&child, source, scopes, Some(&ext_name));
                }
                continue;
            }
            _ => {}
        }
        collect_swift_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_swift_imports_and_inheritance(
    node: &tree_sitter::Node,
    source: &str,
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                let text = source[child.byte_range()].trim().to_string();
                let module = text.trim_start_matches("import ").trim().to_string();
                if !module.is_empty() {
                    deps.imports.push(ImportDep {
                        module,
                        symbols: Vec::new(),
                    });
                }
            }
            "class_declaration" | "struct_declaration" | "enum_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    // Look for inheritance specifiers (type_identifier after ":")
                    collect_swift_supertypes(&child, source, &class_name, &mut deps.inherits);
                }
                collect_swift_imports_and_inheritance(&child, source, deps);
            }
            _ => {
                collect_swift_imports_and_inheritance(&child, source, deps);
            }
        }
    }
}

fn collect_swift_supertypes(
    node: &tree_sitter::Node,
    source: &str,
    class_name: &str,
    inherits: &mut Vec<InheritDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "inheritance_specifier" || child.kind() == "type_identifier" {
            // Only top-level type_identifiers that are direct children of the class
            // header are supertypes. Check if this is within an inheritance clause.
            if child.kind() == "inheritance_specifier" {
                // inheritance_specifier wraps a type; split by comma in case
                // the grammar bundles multiple conformances into one node
                let text = source[child.byte_range()].trim().to_string();
                for part in text.split(',') {
                    let parent = part.split('<').next().unwrap_or("").trim().to_string();
                    if !parent.is_empty() {
                        inherits.push(InheritDep {
                            child_class: class_name.to_string(),
                            parent_class: parent,
                        });
                    }
                }
            }
        } else {
            collect_swift_supertypes(&child, source, class_name, inherits);
        }
    }
}

fn collect_swift_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            // The callee is the first child
            if let Some(func_node) = child.child(0) {
                let callee = match func_node.kind() {
                    "simple_identifier" => source[func_node.byte_range()].to_string(),
                    "navigation_expression" => {
                        // obj.method — extract method name
                        let text = &source[func_node.byte_range()];
                        text.rsplit('.').next().unwrap_or("").trim().to_string()
                    }
                    _ => {
                        let text = &source[func_node.byte_range()];
                        text.rsplit('.').next().unwrap_or("").trim().to_string()
                    }
                };
                if !callee.is_empty() {
                    let caller = find_enclosing_scope(scopes, child.start_position().row)
                        .unwrap_or_else(|| "<module>".to_string());
                    calls.push(CallDep {
                        caller_entity: caller,
                        callee,
                    });
                }
            }
        }
        collect_swift_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// Scala
// ---------------------------------------------------------------------------

/// Extract deps from Scala source.
pub fn extract_scala_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::SCALA.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_scala_scopes(&root, source, &mut scopes, None);

    // Collect imports and inheritance
    collect_scala_imports_and_inheritance(&root, source, &mut deps);

    // Collect calls
    collect_scala_calls(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_scala_scopes(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &mut Vec<FunctionScope>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    let scope_name = match parent_class {
                        Some(cls) => format!("{}.{}", cls, name),
                        None => name,
                    };
                    scopes.push(FunctionScope {
                        name: scope_name,
                        start_row: child.start_position().row,
                        end_row: child.end_position().row,
                    });
                }
            }
            "class_definition" | "trait_definition" | "object_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls = source[name_node.byte_range()].to_string();
                    collect_scala_scopes(&child, source, scopes, Some(&cls));
                    continue;
                }
            }
            _ => {}
        }
        collect_scala_scopes(&child, source, scopes, parent_class);
    }
}

fn collect_scala_imports_and_inheritance(
    node: &tree_sitter::Node,
    source: &str,
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                let text = source[child.byte_range()].trim().to_string();
                let module = text.trim_start_matches("import ").trim().to_string();
                if !module.is_empty() {
                    deps.imports.push(ImportDep {
                        module,
                        symbols: Vec::new(),
                    });
                }
            }
            "class_definition" | "trait_definition" | "object_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    // Look for extends_clause
                    let mut ic = child.walk();
                    for c in child.children(&mut ic) {
                        if c.kind() == "extends_clause" {
                            let text = &source[c.byte_range()];
                            let text = text.trim_start_matches("extends").trim();
                            // May contain "with" for trait mixing
                            for part in text.split(" with ") {
                                let parent = part
                                    .trim()
                                    .split('(')
                                    .next()
                                    .unwrap_or("")
                                    .split('[')
                                    .next()
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                                if !parent.is_empty() {
                                    deps.inherits.push(InheritDep {
                                        child_class: class_name.clone(),
                                        parent_class: parent,
                                    });
                                }
                            }
                        }
                    }
                }
                collect_scala_imports_and_inheritance(&child, source, deps);
            }
            _ => {
                collect_scala_imports_and_inheritance(&child, source, deps);
            }
        }
    }
}

fn collect_scala_calls(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let callee = extract_callee_name(&func_node, source);
                if !callee.is_empty() {
                    let caller = find_enclosing_scope(scopes, child.start_position().row)
                        .unwrap_or_else(|| "<module>".to_string());
                    calls.push(CallDep {
                        caller_entity: caller,
                        callee,
                    });
                }
            } else if let Some(func_node) = child.child(0) {
                // Fallback: first child is the callee
                let callee = extract_callee_name(&func_node, source);
                if !callee.is_empty() {
                    let caller = find_enclosing_scope(scopes, child.start_position().row)
                        .unwrap_or_else(|| "<module>".to_string());
                    calls.push(CallDep {
                        caller_entity: caller,
                        callee,
                    });
                }
            }
        }
        collect_scala_calls(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// Bash
// ---------------------------------------------------------------------------

/// Extract deps from Bash/Shell source.
pub fn extract_bash_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::BASH.ts_language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return RawDeps::default();
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return RawDeps::default();
    };

    let mut deps = RawDeps::default();
    let root = tree.root_node();

    // Collect function scopes
    let mut scopes: Vec<FunctionScope> = Vec::new();
    collect_bash_scopes(&root, source, &mut scopes);

    // Collect source/. imports and command calls
    collect_bash_deps_recursive(&root, source, &scopes, &mut deps);

    deps
}

fn collect_bash_scopes(node: &tree_sitter::Node, source: &str, scopes: &mut Vec<FunctionScope>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            scopes.push(FunctionScope {
                name: source[name_node.byte_range()].to_string(),
                start_row: child.start_position().row,
                end_row: child.end_position().row,
            });
        }
        collect_bash_scopes(&child, source, scopes);
    }
}

fn collect_bash_deps_recursive(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    deps: &mut RawDeps,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "command"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let cmd_name = source[name_node.byte_range()].to_string();
            match cmd_name.as_str() {
                "source" | "." => {
                    // source ./file.sh or . ./file.sh
                    // Get the first argument
                    let mut ac = child.walk();
                    let arg = child.children(&mut ac).find(|c| {
                        (c.kind() == "word" || c.kind() == "string" || c.kind() == "raw_string")
                            && c.id() != name_node.id()
                    });
                    if let Some(arg_node) = arg {
                        let sourced_file = source[arg_node.byte_range()]
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                        if !sourced_file.is_empty() {
                            deps.imports.push(ImportDep {
                                module: sourced_file,
                                symbols: Vec::new(),
                            });
                        }
                    }
                }
                _ => {
                    // Regular command call
                    if !cmd_name.is_empty() {
                        let caller = find_enclosing_scope(scopes, child.start_position().row)
                            .unwrap_or_else(|| "<module>".to_string());
                        deps.calls.push(CallDep {
                            caller_entity: caller,
                            callee: cmd_name,
                        });
                    }
                }
            }
        }
        collect_bash_deps_recursive(&child, source, scopes, deps);
    }
}
