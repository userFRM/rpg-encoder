//! Built-in analyzers for paradigm-specific features that require
//! cross-statement analysis beyond what tree-sitter queries can express.
//!
//! These are activated by feature flags in paradigm TOML files.

use super::defs::ParadigmDef;
use crate::deps::{self, CallDep, FunctionScope, RawDeps, find_enclosing_scope};
use crate::entities::RawEntity;
use crate::languages::Language;
use rpg_core::graph::EntityKind;
use std::collections::HashSet;
use std::path::Path;

/// Apply built-in entity feature extractors for active paradigms.
///
/// Currently supports:
/// - Redux: extract createSlice reducer keys and destructured RTK Query hooks
pub fn apply_builtin_entity_features(
    active_defs: &[&ParadigmDef],
    file: &Path,
    source: &str,
    language: Language,
    entities: &mut Vec<RawEntity>,
) {
    if !(language == Language::TYPESCRIPT || language == Language::JAVASCRIPT) {
        return;
    }

    for def in active_defs {
        if def.features.redux_state_signals {
            let ts_lang = language.ts_language();
            let mut parser = tree_sitter::Parser::new();
            if parser.set_language(&ts_lang).is_err() {
                return;
            }
            let Some(tree) = parser.parse(source.as_bytes(), None) else {
                return;
            };
            let root = tree.root_node();

            // Extract createSlice reducer keys as child entities
            let base_entities = entities.clone();
            for entity in &base_entities {
                if entity.source_text.contains("createSlice(") {
                    extract_create_slice_reducers(&root, file, source, &entity.name, entities);
                }
            }

            // Extract destructured RTK Query hooks
            extract_destructured_hooks(&root, file, source, entities);
        }
    }
}

/// Apply built-in dependency feature extractors for active paradigms.
///
/// Currently supports:
/// - Redux: state signal collection (reads_state, writes_state, dispatches)
pub fn apply_builtin_dep_features(
    active_defs: &[&ParadigmDef],
    _file: &Path,
    source: &str,
    language: Language,
    _entities: &[RawEntity],
    raw_deps: &mut RawDeps,
) {
    if !(language == Language::TYPESCRIPT || language == Language::JAVASCRIPT) {
        return;
    }

    for def in active_defs {
        if def.features.redux_state_signals {
            let ts_lang = language.ts_language();
            let mut parser = tree_sitter::Parser::new();
            if parser.set_language(&ts_lang).is_err() {
                return;
            }
            let Some(tree) = parser.parse(source.as_bytes(), None) else {
                return;
            };
            let root = tree.root_node();
            let mut scopes = Vec::new();
            deps::collect_js_scopes(&root, source, &mut scopes, None);

            let state_context = build_state_signal_context(&root, source);
            collect_state_signals(
                &root,
                source,
                &scopes,
                &state_context,
                &mut raw_deps.reads_state,
                &mut raw_deps.writes_state,
                &mut raw_deps.dispatches,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Redux state signal helpers (moved from redux.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct StateSignalContext {
    selector_hooks: HashSet<String>,
    dispatch_hooks: HashSet<String>,
    dispatch_call_aliases: HashSet<String>,
}

fn build_state_signal_context(node: &tree_sitter::Node, source: &str) -> StateSignalContext {
    let mut ctx = StateSignalContext::default();
    ctx.selector_hooks.insert("useSelector".to_string());
    ctx.selector_hooks.insert("useAppSelector".to_string());
    ctx.dispatch_hooks.insert("useDispatch".to_string());
    ctx.dispatch_hooks.insert("useAppDispatch".to_string());
    ctx.dispatch_call_aliases.insert("dispatch".to_string());

    collect_redux_hook_aliases(
        node,
        source,
        &mut ctx.selector_hooks,
        &mut ctx.dispatch_hooks,
    );
    collect_dispatch_call_aliases(
        node,
        source,
        &ctx.dispatch_hooks,
        &mut ctx.dispatch_call_aliases,
    );
    ctx
}

fn collect_redux_hook_aliases(
    node: &tree_sitter::Node,
    source: &str,
    selector_hooks: &mut HashSet<String>,
    dispatch_hooks: &mut HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_statement" {
            let Some(src_node) = child.child_by_field_name("source") else {
                continue;
            };
            let module =
                source[src_node.byte_range()].trim_matches(|c: char| c == '\'' || c == '"');
            if module != "react-redux" {
                continue;
            }
            collect_redux_hook_aliases_from_import_clause(
                &child,
                source,
                selector_hooks,
                dispatch_hooks,
            );
        }
        collect_redux_hook_aliases(&child, source, selector_hooks, dispatch_hooks);
    }
}

fn collect_redux_hook_aliases_from_import_clause(
    node: &tree_sitter::Node,
    source: &str,
    selector_hooks: &mut HashSet<String>,
    dispatch_hooks: &mut HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_specifier" {
            let spec_text = source[child.byte_range()].trim();
            let (imported, local) = parse_import_specifier_alias(spec_text);
            if imported == "useSelector" || imported == "useAppSelector" {
                selector_hooks.insert(local.clone());
            }
            if imported == "useDispatch" || imported == "useAppDispatch" {
                dispatch_hooks.insert(local);
            }
        }
        collect_redux_hook_aliases_from_import_clause(
            &child,
            source,
            selector_hooks,
            dispatch_hooks,
        );
    }
}

fn parse_import_specifier_alias(spec_text: &str) -> (String, String) {
    let mut parts = spec_text.splitn(2, " as ");
    let first = parts.next().unwrap_or("").trim().to_string();
    let second = parts.next().map(|s| s.trim().to_string());
    match second {
        Some(local) if !local.is_empty() => (first, local),
        _ => (first.clone(), first),
    }
}

fn collect_dispatch_call_aliases(
    node: &tree_sitter::Node,
    source: &str,
    dispatch_hooks: &HashSet<String>,
    dispatch_call_aliases: &mut HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if (child.kind() == "lexical_declaration" || child.kind() == "variable_declaration")
            && let Some(alias) =
                extract_dispatch_alias_from_declaration(&child, source, dispatch_hooks)
        {
            dispatch_call_aliases.insert(alias);
        }
        collect_dispatch_call_aliases(&child, source, dispatch_hooks, dispatch_call_aliases);
    }
}

fn extract_dispatch_alias_from_declaration(
    decl_node: &tree_sitter::Node,
    source: &str,
    dispatch_hooks: &HashSet<String>,
) -> Option<String> {
    let mut cursor = decl_node.walk();
    for child in decl_node.children(&mut cursor) {
        if child.kind() != "variable_declarator" {
            continue;
        }
        let name_node = child.child_by_field_name("name")?;
        if name_node.kind() != "identifier" {
            continue;
        }
        let value_node = child.child_by_field_name("value")?;
        if value_node.kind() != "call_expression" {
            continue;
        }
        let func_node = value_node.child_by_field_name("function")?;
        let callee = extract_callee_name(&func_node, source);
        if dispatch_hooks.contains(&callee) {
            return Some(source[name_node.byte_range()].to_string());
        }
    }
    None
}

fn collect_state_signals(
    node: &tree_sitter::Node,
    source: &str,
    scopes: &[FunctionScope],
    state_context: &StateSignalContext,
    reads_state: &mut Vec<CallDep>,
    writes_state: &mut Vec<CallDep>,
    dispatches: &mut Vec<CallDep>,
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

                if state_context.selector_hooks.contains(&callee) || is_state_reader_name(&callee) {
                    reads_state.push(CallDep {
                        caller_entity: caller.clone(),
                        callee: callee.clone(),
                    });
                    if state_context.selector_hooks.contains(&callee)
                        && let Some(sel) = extract_selector_argument(&child, source)
                    {
                        reads_state.push(CallDep {
                            caller_entity: caller.clone(),
                            callee: sel,
                        });
                    }
                }

                if is_state_setter_name(&callee) {
                    writes_state.push(CallDep {
                        caller_entity: caller.clone(),
                        callee: callee.clone(),
                    });
                    if let Some(normalized_store) = normalized_store_target_from_setter(&callee) {
                        writes_state.push(CallDep {
                            caller_entity: caller.clone(),
                            callee: normalized_store,
                        });
                    }
                }

                if state_context.dispatch_call_aliases.contains(&callee) {
                    let target = extract_dispatch_target(&child, source)
                        .unwrap_or_else(|| "dispatch".to_string());
                    dispatches.push(CallDep {
                        caller_entity: caller,
                        callee: target,
                    });
                }
            }
        }
        collect_state_signals(
            &child,
            source,
            scopes,
            state_context,
            reads_state,
            writes_state,
            dispatches,
        );
    }
}

fn is_state_reader_name(name: &str) -> bool {
    if matches!(
        name,
        "useSelector" | "useAppSelector" | "useStore" | "getState" | "useAtomValue"
    ) {
        return true;
    }
    if name.starts_with("use")
        && name.len() > 3
        && name.chars().nth(3).is_some_and(|c| c.is_ascii_uppercase())
        && (name.ends_with("Query") || name.ends_with("Mutation"))
    {
        return true;
    }
    false
}

fn is_state_setter_name(name: &str) -> bool {
    if name == "setState" {
        return true;
    }
    if !name.starts_with("set") || name.len() <= 3 {
        return false;
    }
    if matches!(
        name,
        "setTimeout" | "setInterval" | "setImmediate" | "setPrototypeOf"
    ) {
        return false;
    }
    name.chars().nth(3).is_some_and(|c| c.is_ascii_uppercase())
}

fn normalized_store_target_from_setter(setter_name: &str) -> Option<String> {
    if !(setter_name.ends_with("Store") || setter_name.ends_with("Slice")) {
        return None;
    }
    if !setter_name.starts_with("set") || setter_name.len() <= 3 {
        return None;
    }
    let mut chars = setter_name[3..].chars();
    let first = chars.next()?;
    let mut normalized = String::new();
    normalized.push(first.to_ascii_lowercase());
    normalized.extend(chars);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn extract_dispatch_target(call_expr: &tree_sitter::Node, source: &str) -> Option<String> {
    let args_node = call_expr.child_by_field_name("arguments")?;
    let mut cursor = args_node.walk();
    for arg in args_node.children(&mut cursor) {
        if !arg.is_named() {
            continue;
        }
        if arg.kind() == "call_expression"
            && let Some(func_node) = arg.child_by_field_name("function")
        {
            let callee = extract_callee_name(&func_node, source);
            if !callee.is_empty() {
                return Some(callee);
            }
        }
        if arg.kind() == "identifier" {
            let text = source[arg.byte_range()].trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if arg.kind() == "member_expression"
            && let Some(prop) = arg.child_by_field_name("property")
        {
            let text = source[prop.byte_range()].trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        break;
    }
    None
}

fn extract_selector_argument(call_expr: &tree_sitter::Node, source: &str) -> Option<String> {
    let args_node = call_expr.child_by_field_name("arguments")?;
    let mut cursor = args_node.walk();
    for arg in args_node.children(&mut cursor) {
        if !arg.is_named() {
            continue;
        }
        if arg.kind() == "identifier" {
            let text = source[arg.byte_range()].trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if arg.kind() == "member_expression"
            && let Some(prop) = arg.child_by_field_name("property")
        {
            let text = source[prop.byte_range()].trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        break;
    }
    None
}

/// Extract a callee name from a call's function node.
fn extract_callee_name(node: &tree_sitter::Node, source: &str) -> String {
    match node.kind() {
        "identifier" => source[node.byte_range()].to_string(),
        "attribute" | "member_expression" => {
            if let Some(attr) = node
                .child_by_field_name("attribute")
                .or_else(|| node.child_by_field_name("property"))
            {
                source[attr.byte_range()].to_string()
            } else {
                source[node.byte_range()].to_string()
            }
        }
        _ => {
            let text = &source[node.byte_range()];
            text.rsplit('.').next().unwrap_or("").trim().to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Redux entity extraction helpers (moved from redux.rs)
// ---------------------------------------------------------------------------

/// Extract reducer keys from createSlice({ reducers: { key1, key2 } }) as child entities.
fn extract_create_slice_reducers(
    root: &tree_sitter::Node,
    file: &Path,
    source: &str,
    slice_name: &str,
    entities: &mut Vec<RawEntity>,
) {
    walk_for_reducers(root, source, slice_name, file, entities);
}

fn walk_for_reducers(
    node: &tree_sitter::Node,
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
            });
        }
    }
}

/// Extract destructured hooks from object_pattern: const { useGetPostsQuery } = postsApi;
fn extract_destructured_hooks(
    node: &tree_sitter::Node,
    file: &Path,
    source: &str,
    entities: &mut Vec<RawEntity>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "lexical_declaration" || child.kind() == "variable_declaration" {
            let mut inner = child.walk();
            for decl in child.children(&mut inner) {
                if decl.kind() == "variable_declarator"
                    && let Some(name_node) = decl.child_by_field_name("name")
                    && name_node.kind() == "object_pattern"
                {
                    // Validate RHS is a plain identifier (API object)
                    let has_identifier_rhs = decl
                        .child_by_field_name("value")
                        .is_some_and(|val| val.kind() == "identifier");
                    if !has_identifier_rhs {
                        continue;
                    }

                    let mut pc = name_node.walk();
                    for prop in name_node.children(&mut pc) {
                        let name = match prop.kind() {
                            "shorthand_property_identifier_pattern"
                            | "shorthand_property_identifier" => Some(&source[prop.byte_range()]),
                            "pair_pattern" => prop
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
                                file: file.to_path_buf(),
                                line_start: child.start_position().row + 1,
                                line_end: child.end_position().row + 1,
                                parent_class: None,
                                source_text: source[child.byte_range()].to_string(),
                            });
                        }
                    }
                }
            }
        }
        // Recurse into export statements
        if child.kind() == "export_statement" {
            extract_destructured_hooks(&child, file, source, entities);
        }
    }
}

/// Check if a name looks like a custom React hook (use* prefix + 4th char uppercase).
fn looks_like_custom_hook(name: &str) -> bool {
    if !name.starts_with("use") || name.len() <= 3 {
        return false;
    }
    name.chars().nth(3).is_some_and(|c| c.is_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_state_reader_name() {
        assert!(is_state_reader_name("useSelector"));
        assert!(is_state_reader_name("useAppSelector"));
        assert!(is_state_reader_name("useStore"));
        assert!(is_state_reader_name("getState"));
        assert!(is_state_reader_name("useGetPostsQuery"));
        assert!(is_state_reader_name("useUpdateUserMutation"));
        assert!(!is_state_reader_name("useState"));
        assert!(!is_state_reader_name("doSomething"));
    }

    #[test]
    fn test_is_state_setter_name() {
        assert!(is_state_setter_name("setState"));
        assert!(is_state_setter_name("setUser"));
        assert!(is_state_setter_name("setAuthStore"));
        assert!(!is_state_setter_name("setTimeout"));
        assert!(!is_state_setter_name("setInterval"));
        assert!(!is_state_setter_name("set"));
        assert!(!is_state_setter_name("setup"));
    }

    #[test]
    fn test_normalized_store_target() {
        assert_eq!(
            normalized_store_target_from_setter("setAuthStore"),
            Some("authStore".to_string())
        );
        assert_eq!(
            normalized_store_target_from_setter("setUserSlice"),
            Some("userSlice".to_string())
        );
        assert_eq!(normalized_store_target_from_setter("setUser"), None);
        assert_eq!(normalized_store_target_from_setter("setState"), None);
    }

    #[test]
    fn test_looks_like_custom_hook() {
        assert!(looks_like_custom_hook("useAuth"));
        assert!(looks_like_custom_hook("useState"));
        assert!(!looks_like_custom_hook("use"));
        assert!(!looks_like_custom_hook("useless"));
    }
}
