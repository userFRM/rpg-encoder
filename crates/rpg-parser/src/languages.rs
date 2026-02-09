//! Language detection and tree-sitter grammar loading.
//!
//! All types and methods are auto-generated from `languages/defs/*.toml`
//! by build.rs. Adding a new language requires only a TOML file +
//! the tree-sitter grammar dependency — zero Rust source changes.

// Generated: LangId struct, Language alias, from_extension, from_name,
// name, glob_pattern, ts_language, detect_primary, detect_all,
// grammar_for, expand_lang_aliases, effective_grammar_name,
// builtin_entity_extractor, builtin_dep_extractor_name,
// builtin_entity_extractor_name
include!(concat!(env!("OUT_DIR"), "/lang_registry.rs"));
