//! Data-driven paradigm detection and adapter system.
//!
//! A "paradigm" is a framework or convention layer above the language syntax:
//! React, Next.js, Redux, Django, Spring Boot, etc.
//!
//! Paradigms are defined entirely in TOML files (`paradigms/defs/*.toml`).
//! The Rust engine is generic — it loads TOML definitions, applies classification
//! rules, executes tree-sitter queries, and runs built-in analyzers.
//!
//! Adding a new paradigm = drop a TOML file + `cargo build`. No Rust changes needed.

pub mod classify;
pub mod defs;
pub mod features;
pub mod helpers;
pub mod query_engine;

use crate::languages::Language;
use std::path::Path;

// ---------------------------------------------------------------------------
// TOML-driven detection (data-driven paradigm engine)
// ---------------------------------------------------------------------------

/// Detect paradigms using TOML definitions. Returns references to active defs
/// in priority order (lowest priority number = highest priority = first).
///
/// A paradigm is active if:
/// 1. At least one of its declared languages is present in the project
/// 2. Its detection rules match (deps in manifest, config files exist, etc.)
pub fn detect_paradigms_toml<'a>(
    root: &Path,
    languages: &[Language],
    paradigm_defs: &'a [defs::ParadigmDef],
) -> Vec<&'a defs::ParadigmDef> {
    let manifest = helpers::read_manifest(root);
    paradigm_defs
        .iter()
        .filter(|def| {
            def.languages
                .iter()
                .any(|dl| languages.iter().any(|l| l.name() == dl))
        })
        .filter(|def| matches_detect_rules(root, &manifest, &def.detect))
        .collect()
    // Already sorted by priority from load_builtin_defs()
}

/// Check if detection rules from a TOML paradigm definition match.
fn matches_detect_rules(root: &Path, manifest: &str, rules: &defs::DetectRules) -> bool {
    // deps: any listed dependency found in manifest
    if !rules.deps.is_empty() && rules.deps.iter().any(|d| helpers::has_dep(manifest, d)) {
        return true;
    }

    // config_files: any config file exists
    if rules.config_files.iter().any(|f| root.join(f).exists()) {
        return true;
    }

    // files: any matching file exists in project (glob patterns)
    if !rules.files.is_empty() && has_matching_files(root, &rules.files) {
        return true;
    }

    // dir_with_files: directory exists with matching files
    for dwf in &rules.dir_with_files {
        if has_dir_with_pattern(root, &dwf.dir, &dwf.pattern) {
            return true;
        }
    }

    false
}

/// Check if project has files matching any of the glob patterns.
fn has_matching_files(root: &Path, patterns: &[String]) -> bool {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .add_custom_ignore_filename(".rpgignore")
        .max_depth(Some(5))
        .build();

    for entry in walker.flatten() {
        if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
            for pattern in patterns {
                // Simple glob: *.ext matching
                if let Some(ext_pattern) = pattern.strip_prefix("*.") {
                    if name.ends_with(&format!(".{ext_pattern}")) {
                        return true;
                    }
                } else if name == pattern {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a directory exists and contains files matching a pattern.
fn has_dir_with_pattern(root: &Path, dir: &str, pattern: &str) -> bool {
    let dir_path = root.join(dir);
    if !dir_path.is_dir() {
        return false;
    }
    let walker = ignore::WalkBuilder::new(&dir_path)
        .hidden(true)
        .git_ignore(true)
        .max_depth(Some(4))
        .build();
    for entry in walker.flatten() {
        if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
            // Simple pattern matching: "page.*" matches "page.tsx", "page.js", etc.
            if let Some(stem) = pattern.strip_suffix(".*") {
                if name.starts_with(&format!("{stem}.")) {
                    return true;
                }
            } else if name == pattern {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_paradigms_toml_nextjs_fixture() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/nextjs_project");
        if !fixture.exists() {
            return;
        }
        let languages = vec![Language::TYPESCRIPT];
        let paradigm_defs = defs::load_builtin_defs().unwrap();
        let active = detect_paradigms_toml(&fixture, &languages, &paradigm_defs);
        let names: Vec<&str> = active.iter().map(|d| d.name.as_str()).collect();
        assert!(
            names.contains(&"nextjs"),
            "expected nextjs, got: {:?}",
            names
        );
    }

    #[test]
    fn test_detect_paradigms_toml_rust_only() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let languages = vec![Language::RUST];
        let paradigm_defs = defs::load_builtin_defs().unwrap();
        let active = detect_paradigms_toml(fixture, &languages, &paradigm_defs);
        assert!(
            active.is_empty(),
            "Rust-only project should have no paradigms, got: {:?}",
            active.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_detect_paradigms_toml_priority_order() {
        let paradigm_defs = defs::load_builtin_defs().unwrap();
        // With all JS languages present, the order should match priority
        let priorities: Vec<i32> = paradigm_defs.iter().map(|d| d.priority).collect();
        let mut sorted = priorities.clone();
        sorted.sort_unstable();
        assert_eq!(priorities, sorted, "defs should be in priority order");
    }
}
