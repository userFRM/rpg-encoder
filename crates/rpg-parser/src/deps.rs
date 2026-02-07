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
struct FunctionScope {
    name: String,
    start_row: usize,
    end_row: usize,
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
fn find_enclosing_scope(scopes: &[FunctionScope], row: usize) -> Option<String> {
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
    match language {
        Language::Python => extract_python_deps(path, source),
        Language::Rust => extract_rust_deps(path, source),
        Language::TypeScript | Language::JavaScript => extract_js_deps(path, source, language),
        Language::Go => extract_go_deps(path, source),
        Language::Java => extract_java_deps(path, source),
        Language::C | Language::Cpp => extract_c_deps(path, source, language),
    }
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

    // Collect imports, inheritance, calls, and JSX usage
    collect_js_imports(&root, source, &mut deps);
    collect_js_calls(&root, source, &scopes, &mut deps.calls);
    collect_js_jsx_usage(&root, source, &scopes, &mut deps.calls);

    deps
}

fn collect_js_scopes(
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
                        }
                    }
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

fn collect_js_jsx_usage(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    calls: &mut Vec<CallDep>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "jsx_self_closing_element" | "jsx_opening_element" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let tag = &source[name_node.byte_range()];
                    // Uppercase = component, lowercase = HTML element.
                    // For dotted names like Foo.Bar, extract the last segment
                    // so it can resolve to entity "Bar".
                    if tag.starts_with(|c: char| c.is_uppercase()) {
                        let callee = tag.rsplit('.').next().unwrap_or(tag).to_string();
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
            _ => {}
        }
        collect_js_jsx_usage(&child, source, scopes, calls);
    }
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

/// Extract deps from Go source.
pub fn extract_go_deps(_path: &Path, source: &str) -> RawDeps {
    let lang = Language::Go.ts_language();
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
    let lang = Language::Java.ts_language();
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
    if language == Language::Cpp {
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
