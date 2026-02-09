//! Tree-sitter query engine for paradigm-specific entity and dependency extraction.
//!
//! Compiles tree-sitter queries from TOML definitions at startup and executes them
//! against source files to extract additional entities and dependencies.

use super::defs::{ParadigmDef, parse_edge_kind, parse_entity_kind};
use crate::deps::{CallDep, FunctionScope, RawDeps, find_enclosing_scope};
use crate::entities::RawEntity;
use crate::languages::Language;
use rpg_core::graph::EdgeKind;
use std::collections::HashMap;
use std::path::Path;
use tree_sitter::StreamingIterator;

/// Pre-compiled tree-sitter queries, keyed by (grammar_name, query_string).
pub struct QueryCache {
    /// Maps (language_name, query_id) → compiled tree_sitter::Query
    cache: HashMap<(String, String), tree_sitter::Query>,
}

impl QueryCache {
    /// Pre-compile ALL queries from all paradigm defs at startup.
    /// Returns errors for queries that fail to compile.
    pub fn compile_all(defs: &[ParadigmDef]) -> Result<Self, Vec<String>> {
        let mut cache = HashMap::new();
        let mut errors = Vec::new();

        for def in defs {
            // Entity queries
            for eq in &def.entity_queries {
                let languages = if eq.languages.is_empty() {
                    def.languages.clone()
                } else {
                    eq.languages.clone()
                };
                let expanded = expand_languages(&languages);
                for lang_name in &expanded {
                    let query_str = eq.query_by_language.get(lang_name).unwrap_or(&eq.query);
                    let key = (lang_name.clone(), eq.id.clone());
                    if cache.contains_key(&key) {
                        continue;
                    }
                    match compile_query(lang_name, query_str) {
                        Ok(q) => {
                            cache.insert(key, q);
                        }
                        Err(e) => errors.push(format!(
                            "[{}] entity query '{}' for lang '{}': {}",
                            def.name, eq.id, lang_name, e
                        )),
                    }
                }
            }

            // Dep queries
            for dq in &def.dep_queries {
                let languages = if dq.languages.is_empty() {
                    def.languages.clone()
                } else {
                    dq.languages.clone()
                };
                let expanded = expand_languages(&languages);
                for lang_name in &expanded {
                    let query_str = dq.query_by_language.get(lang_name).unwrap_or(&dq.query);
                    let key = (lang_name.clone(), dq.id.clone());
                    if cache.contains_key(&key) {
                        continue;
                    }
                    match compile_query(lang_name, query_str) {
                        Ok(q) => {
                            cache.insert(key, q);
                        }
                        Err(e) => errors.push(format!(
                            "[{}] dep query '{}' for lang '{}': {}",
                            def.name, dq.id, lang_name, e
                        )),
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(Self { cache })
        } else {
            Err(errors)
        }
    }

    fn get(&self, lang_name: &str, query_id: &str) -> Option<&tree_sitter::Query> {
        self.cache
            .get(&(lang_name.to_string(), query_id.to_string()))
    }
}

/// Compile a tree-sitter query string for a given language.
fn compile_query(lang_name: &str, query_str: &str) -> Result<tree_sitter::Query, String> {
    let ts_lang =
        ts_language_for(lang_name).ok_or_else(|| format!("unknown language '{lang_name}'"))?;
    tree_sitter::Query::new(&ts_lang, query_str).map_err(|e| e.to_string())
}

/// Expand language lists with grammar aliases (e.g., "typescript" implies "tsx").
/// This ensures queries declared for "typescript" also compile/run on .tsx files.
fn expand_languages(languages: &[String]) -> Vec<String> {
    crate::languages::expand_lang_aliases(languages)
}

/// Map language name to tree-sitter Language.
fn ts_language_for(name: &str) -> Option<tree_sitter::Language> {
    crate::languages::grammar_for(name)
}

/// Map our Language enum to the string used in TOML language lists.
fn language_to_name(lang: Language) -> &'static str {
    lang.name()
}

/// For languages with grammar aliases, determine which grammar to use
/// based on the file extension. E.g., TypeScript .tsx files use "tsx" grammar.
fn effective_lang_name(lang: Language, file: &Path) -> &'static str {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    crate::languages::effective_grammar_name(lang.name(), ext)
}

/// Execute entity queries from paradigm defs and return additional entities.
pub fn execute_entity_queries(
    qcache: &QueryCache,
    active_defs: &[&ParadigmDef],
    file: &Path,
    source: &str,
    language: Language,
    _base_entities: &[RawEntity],
) -> Vec<RawEntity> {
    let lang_name = language_to_name(language);
    let eff_lang = effective_lang_name(language, file);
    let mut additional = Vec::new();

    // Parse the source once
    let Some(ts_lang) = ts_language_for(eff_lang) else {
        return additional;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return additional;
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return additional;
    };

    for def in active_defs {
        for eq in &def.entity_queries {
            let query_langs = if eq.languages.is_empty() {
                &def.languages
            } else {
                &eq.languages
            };
            // Check if this query applies to this file's language
            if !query_langs.iter().any(|l| l == lang_name || l == eff_lang) {
                continue;
            }

            // Prefer effective language (matches the grammar used to parse the tree)
            let query = qcache
                .get(eff_lang, &eq.id)
                .or_else(|| qcache.get(lang_name, &eq.id));
            let Some(query) = query else {
                continue;
            };

            let Some(entity_kind) = parse_entity_kind(&eq.entity_kind) else {
                continue;
            };

            // Find the capture index for the entity name
            let name_capture = eq.entity_name.trim_start_matches('@');
            let Some(name_idx) = query.capture_index_for_name(name_capture) else {
                continue;
            };

            // If parent starts with '@', resolve it from a query capture
            let parent_capture_idx = eq.parent.as_deref().and_then(|p| {
                let cap_name = p.trim_start_matches('@');
                if p.starts_with('@') {
                    query.capture_index_for_name(cap_name)
                } else {
                    None
                }
            });

            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
            while let Some(m) = matches.next() {
                let name_node = m.captures.iter().find(|c| c.index == name_idx);
                if let Some(cap) = name_node {
                    let name = source[cap.node.byte_range()].to_string();
                    let src_range = cap.node.parent().map_or_else(
                        || cap.node.byte_range(),
                        |p: tree_sitter::Node<'_>| p.byte_range(),
                    );

                    // Resolve parent: from capture, literal string, or None
                    let parent_class = if let Some(pidx) = parent_capture_idx {
                        m.captures
                            .iter()
                            .find(|c| c.index == pidx)
                            .map(|c| source[c.node.byte_range()].to_string())
                    } else {
                        eq.parent.clone()
                    };

                    additional.push(RawEntity {
                        name,
                        kind: entity_kind,
                        file: file.to_path_buf(),
                        line_start: cap.node.start_position().row + 1,
                        line_end: cap.node.end_position().row + 1,
                        parent_class,
                        source_text: source[src_range].to_string(),
                    });
                }
            }
        }
    }

    additional
}

/// Execute dep queries from paradigm defs and append to raw_deps.
pub fn execute_dep_queries(
    qcache: &QueryCache,
    active_defs: &[&ParadigmDef],
    file: &Path,
    source: &str,
    language: Language,
    scopes: &[FunctionScope],
    raw_deps: &mut RawDeps,
) {
    let lang_name = language_to_name(language);
    let eff_lang = effective_lang_name(language, file);

    // Parse the source once
    let Some(ts_lang) = ts_language_for(eff_lang) else {
        return;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return;
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return;
    };

    for def in active_defs {
        for dq in &def.dep_queries {
            let query_langs = if dq.languages.is_empty() {
                &def.languages
            } else {
                &dq.languages
            };
            if !query_langs.iter().any(|l| l == lang_name || l == eff_lang) {
                continue;
            }

            // Prefer effective language (matches the grammar used to parse the tree)
            let query = qcache
                .get(eff_lang, &dq.id)
                .or_else(|| qcache.get(lang_name, &dq.id));
            let Some(query) = query else {
                continue;
            };

            let Some(edge_kind) = parse_edge_kind(&dq.edge_kind) else {
                continue;
            };

            let callee_capture = dq.callee.trim_start_matches('@');
            let Some(callee_idx) = query.capture_index_for_name(callee_capture) else {
                continue;
            };

            let mut cursor = tree_sitter::QueryCursor::new();
            let mut qmatches = cursor.matches(query, tree.root_node(), source.as_bytes());
            while let Some(m) = qmatches.next() {
                let callee_node = m.captures.iter().find(|c| c.index == callee_idx);
                if let Some(cap) = callee_node {
                    let callee = source[cap.node.byte_range()].to_string();

                    // Apply filter
                    if let Some(ref filter) = dq.filter_callee
                        && filter == "starts_uppercase"
                        && !callee.starts_with(|c: char| c.is_ascii_uppercase())
                    {
                        continue;
                    }

                    // Determine caller
                    let caller = if dq.caller == "enclosing_scope" {
                        let row = cap.node.start_position().row;
                        find_enclosing_scope(scopes, row).unwrap_or_else(|| "<module>".to_string())
                    } else {
                        // Try to find a capture for the caller
                        let caller_capture = dq.caller.trim_start_matches('@');
                        match query.capture_index_for_name(caller_capture) {
                            Some(idx) => m
                                .captures
                                .iter()
                                .find(|c| c.index == idx)
                                .map(|c| source[c.node.byte_range()].to_string())
                                .unwrap_or_else(|| "<module>".to_string()),
                            None => "<module>".to_string(),
                        }
                    };

                    let dep = CallDep {
                        caller_entity: caller,
                        callee,
                    };

                    // Push to appropriate dep vector
                    match edge_kind {
                        EdgeKind::Renders => raw_deps.renders.push(dep),
                        EdgeKind::ReadsState => raw_deps.reads_state.push(dep),
                        EdgeKind::WritesState => raw_deps.writes_state.push(dep),
                        EdgeKind::Dispatches => raw_deps.dispatches.push(dep),
                        EdgeKind::Invokes => raw_deps.calls.push(dep),
                        _ => {} // Other edge kinds not supported as call-like deps
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradigms::defs::load_builtin_defs;

    #[test]
    fn test_query_cache_compiles_builtins() {
        let defs = load_builtin_defs().unwrap();
        // Should compile without errors (empty if no queries defined yet)
        let result = QueryCache::compile_all(&defs);
        // Even if there are no queries, it should succeed
        assert!(
            result.is_ok(),
            "query cache compilation failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_ts_language_for() {
        assert!(ts_language_for("typescript").is_some());
        assert!(ts_language_for("tsx").is_some());
        assert!(ts_language_for("javascript").is_some());
        assert!(ts_language_for("python").is_some());
        assert!(ts_language_for("rust").is_some());
        assert!(ts_language_for("bogus").is_none());
    }
}
