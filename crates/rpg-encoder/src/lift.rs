//! On-demand semantic lifting: scope resolution, batching, and incremental update utilities.

use anyhow::Result;
use rpg_core::graph::RPGraph;
use rpg_parser::entities::RawEntity;
use rpg_parser::languages::Language;
use rpg_parser::paradigms::classify::matches_entity;
use rpg_parser::paradigms::defs::{AutoLiftRule, ParadigmDef};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

/// A resolved set of entity IDs to lift.
pub struct LiftScope {
    pub entity_ids: Vec<String>,
}

/// Result of a lift operation.
pub struct LiftResult {
    pub entities_lifted: usize,
    pub entities_failed: usize,
    pub entities_repaired: usize,
    pub hierarchy_updated: bool,
}

/// Resolve a scope specification into concrete entity IDs.
///
/// Supports:
/// - File globs: `src/auth/**` or `*.rs` — matched against entity file paths
/// - Hierarchy path prefix: `Auth/login` — collects via hierarchy subtree
/// - Comma-separated entity IDs: `src/foo.rs:bar,src/baz.rs:qux`
/// - `*` or `all` — all unlifted entities
pub fn resolve_scope(graph: &RPGraph, scope: &str) -> LiftScope {
    let scope = scope.trim();

    // "all" or "*" → all unlifted entities
    if scope == "*" || scope.eq_ignore_ascii_case("all") {
        let entity_ids = graph
            .entities
            .iter()
            .filter(|(_, e)| {
                e.semantic_features.is_empty() && e.kind != rpg_core::graph::EntityKind::Module
            })
            .map(|(id, _)| id.clone())
            .collect();
        return LiftScope { entity_ids };
    }

    // Try as glob pattern (contains * or ?)
    if (scope.contains('*') || scope.contains('?'))
        && let Ok(glob) = globset::Glob::new(scope)
    {
        let matcher = glob.compile_matcher();
        let entity_ids = graph
            .entities
            .iter()
            .filter(|(_, e)| matcher.is_match(&e.file))
            .map(|(id, _)| id.clone())
            .collect();
        return LiftScope { entity_ids };
    }

    // Try as hierarchy path prefix
    let mut hierarchy_ids: Vec<String> = Vec::new();
    for (area_name, area_node) in &graph.hierarchy {
        if area_name == scope || scope.starts_with(&format!("{}/", area_name)) {
            // Collect from this subtree
            if area_name == scope {
                hierarchy_ids.extend(area_node.all_entity_ids());
            } else {
                // Walk deeper into the path
                let remainder = &scope[area_name.len() + 1..];
                let parts: Vec<&str> = remainder.split('/').collect();
                let mut current = area_node;
                let mut found = true;
                for part in &parts {
                    if let Some(child) = current.children.get(*part) {
                        current = child;
                    } else {
                        found = false;
                        break;
                    }
                }
                if found {
                    hierarchy_ids.extend(current.all_entity_ids());
                }
            }
        }
    }
    if !hierarchy_ids.is_empty() {
        return LiftScope {
            entity_ids: hierarchy_ids,
        };
    }

    // Try as comma-separated entity IDs
    let entity_ids: Vec<String> = scope
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|id| graph.entities.contains_key(id))
        .collect();

    LiftScope { entity_ids }
}

// ---------------------------------------------------------------------------
// TOML-driven auto-lift engine
// ---------------------------------------------------------------------------

/// Convert identifier fragment to human-readable field text.
/// Handles camelCase ("MaxCount" → "max count"), PascalCase, snake_case,
/// and acronyms ("HTTPServer" → "http server", "getHTTPServer" → "get http server").
fn normalize_field(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' {
            if !result.ends_with(' ') {
                result.push(' ');
            }
        } else {
            if i > 0 && ch.is_ascii_uppercase() {
                let prev = chars[i - 1];
                // Insert space at word boundaries:
                // 1. lowercase/digit → Uppercase  (camelCase: "getName" → "get Name")
                // 2. Uppercase → Uppercase+lowercase (acronym end: "HTTPServer" → "HTTP Server")
                if (prev.is_ascii_lowercase() || prev.is_ascii_digit()) && !result.ends_with(' ') {
                    result.push(' ');
                } else if prev.is_ascii_uppercase()
                    && let Some(&next) = chars.get(i + 1)
                    && next.is_ascii_lowercase()
                    && !result.ends_with(' ')
                {
                    result.push(' ');
                }
            }
            result.push(ch.to_ascii_lowercase());
        }
    }
    result
}

/// An auto-lift rule tagged with its source paradigm's languages.
/// Rules with empty `languages` (core rules) match any file.
struct TaggedRule {
    rule: AutoLiftRule,
    languages: Vec<String>,
}

/// Confidence level for auto-lifted features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiftConfidence {
    /// High confidence — apply features directly.
    Accept,
    /// Medium confidence — include in batch 0 with pre-filled features for LLM verification.
    Review,
    /// Low confidence — pattern matched but structural complexity too high; needs full LLM lifting.
    Reject,
}

/// Engine that matches entities against TOML-defined auto-lift rules.
///
/// Rules are collected from paradigm definitions in priority order.
/// `core.toml` (priority 100) is always included; framework-specific rules
/// from detected paradigms are prepended (lower priority number = higher priority
/// = checked first). Language-specific rules only match entities whose file
/// extension belongs to the rule's source language.
pub struct AutoLiftEngine {
    rules: Vec<TaggedRule>,
}

impl AutoLiftEngine {
    /// Build an engine from all paradigm definitions.
    ///
    /// `active_paradigm_names` controls which framework TOMLs contribute rules.
    /// `core.toml` (identified by `languages = []`) is always included regardless.
    pub fn new(all_defs: &[ParadigmDef], active_paradigm_names: &[String]) -> Self {
        let mut rules = Vec::new();
        // Defs are already sorted by priority (ascending = highest priority first).
        // Framework rules come before core rules, so framework patterns win.
        for def in all_defs {
            let is_core = def.languages.is_empty();
            let is_active = active_paradigm_names.iter().any(|n| n == &def.name);
            if is_core || is_active {
                for auto_rule in &def.auto_lift {
                    rules.push(TaggedRule {
                        rule: auto_rule.clone(),
                        languages: def.languages.clone(),
                    });
                }
            }
        }
        Self { rules }
    }

    /// Try to match an entity against auto-lift rules with confidence scoring.
    ///
    /// Returns `Some((features, confidence))` if a rule matches:
    /// - `Accept`: entity is structurally simple — apply features directly
    /// - `Review`: entity has moderate complexity — show pre-filled features for LLM verification
    ///
    /// Returns `None` if no rule matches (entity needs full LLM lifting).
    ///
    /// Rules without structural gate fields (max_branches/max_loops/max_calls)
    /// always produce `Accept` confidence when matched.
    pub fn try_lift_with_confidence(
        &self,
        raw: &RawEntity,
    ) -> Option<(Vec<String>, LiftConfidence)> {
        let (features, has_structural_gates) = self.try_lift_internal(raw)?;

        if !has_structural_gates {
            // Rules without structural fields → binary match → Accept
            return Some((features, LiftConfidence::Accept));
        }

        // Determine confidence from structural signals:
        // - Accept: simple (0 branches, 0 loops, ≤2 calls)
        // - Review: moderate (exactly 1 branch, or 3+ calls, but no loops)
        // - Reject: complex (2+ branches OR any loop) — needs full LLM lifting
        let signals = rpg_parser::signals::analyze(&raw.source_text);
        let confidence = if signals.branch_count > 1 || signals.loop_count > 0 {
            LiftConfidence::Reject
        } else if signals.branch_count == 0 && signals.loop_count == 0 && signals.call_count <= 2 {
            LiftConfidence::Accept
        } else {
            LiftConfidence::Review
        };

        Some((features, confidence))
    }

    /// Try to match an entity against auto-lift rules. First match wins.
    /// Language-specific rules are skipped when the entity's file extension
    /// doesn't belong to the rule's source language.
    pub fn try_lift(&self, raw: &RawEntity) -> Option<Vec<String>> {
        self.try_lift_internal(raw).map(|(features, _)| features)
    }

    /// Internal match that also reports whether the matching rule had structural gates.
    fn try_lift_internal(&self, raw: &RawEntity) -> Option<(Vec<String>, bool)> {
        let file_lang = raw
            .file
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);

        for tagged in &self.rules {
            // Skip language-specific rules that don't match the entity's file language.
            // When the file language is unknown, language-specific rules are skipped
            // (only core rules with empty languages apply).
            if !tagged.languages.is_empty()
                && !file_lang.is_some_and(|lang| tagged.languages.iter().any(|l| l == lang.name()))
            {
                continue;
            }
            if matches_entity(&tagged.rule.match_rule, raw, &raw.file) {
                let has_structural_gates = tagged.rule.match_rule.max_branches.is_some()
                    || tagged.rule.match_rule.max_loops.is_some()
                    || tagged.rule.match_rule.max_calls.is_some();
                return Some((
                    Self::expand_templates(&tagged.rule, raw),
                    has_structural_gates,
                ));
            }
        }
        None
    }

    /// Expand template strings using entity context.
    fn expand_templates(rule: &AutoLiftRule, raw: &RawEntity) -> Vec<String> {
        let parent = raw.parent_class.as_deref().unwrap_or("instance");
        let parent_lower = parent.to_lowercase();
        let name_lower = raw.name.to_lowercase();
        // Pass original name (not lowercased) so camelCase boundaries are preserved.
        let (field, matched_prefix) = Self::compute_field(&raw.name, &rule.strip_prefix);
        let verb = matched_prefix
            .as_deref()
            .and_then(|p| rule.prefix_verb.get(p))
            .map(|s| s.as_str())
            .unwrap_or("");
        let kind = format!("{:?}", raw.kind).to_lowercase();

        rule.features
            .iter()
            .map(|tmpl| {
                tmpl.replace("{name}", &raw.name)
                    .replace("{name_lower}", &name_lower)
                    .replace("{parent}", parent)
                    .replace("{parent_lower}", &parent_lower)
                    .replace("{field}", &field)
                    .replace("{verb}", verb)
                    .replace("{kind}", &kind)
            })
            .collect()
    }

    /// Strip the first matching prefix and normalize to human-readable text.
    /// Handles camelCase, PascalCase, and snake_case boundaries.
    /// Returns `(field_text, matched_prefix)`.
    fn compute_field(name: &str, prefixes: &[String]) -> (String, Option<String>) {
        for prefix in prefixes {
            if let Some(rest) = name.strip_prefix(prefix.as_str()) {
                return (normalize_field(rest), Some(prefix.clone()));
            }
        }
        (normalize_field(name), None)
    }
}

/// Try to auto-lift a trivial entity using the default core rules.
///
/// This is a backward-compatible wrapper around `AutoLiftEngine`. For MCP
/// server usage, prefer constructing an `AutoLiftEngine` directly with
/// detected paradigms for framework-specific auto-lift coverage.
pub fn try_auto_lift(raw: &RawEntity) -> Option<Vec<String>> {
    static ENGINE: OnceLock<AutoLiftEngine> = OnceLock::new();
    let engine = ENGINE.get_or_init(|| {
        let defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
        // No active paradigms — only core rules (languages = [])
        AutoLiftEngine::new(&defs, &[])
    });
    engine.try_lift(raw)
}

/// Generate a compact repo overview from graph metadata (paper's `repo_info` context).
/// Wraps output in `<repo_name>` and `<repo_info>` tags per paper §A.1.1.
pub fn generate_repo_info(graph: &RPGraph, project_name: &str) -> String {
    let lang = &graph.metadata.language;
    let total = graph.entities.len();
    let files = graph.metadata.total_files;

    let areas: Vec<&String> = graph.hierarchy.keys().collect();
    let area_list = if areas.len() <= 8 {
        areas
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("{} functional areas", areas.len())
    };

    let (lifted, _) = graph.lifting_coverage();
    let info = if lifted > 0 {
        format!(
            "{} repository with {} entities across {} files ({} semantically lifted). Top-level modules: {}.",
            lang, total, files, lifted, area_list
        )
    } else {
        format!(
            "{} repository with {} entities across {} files. Top-level modules: {}.",
            lang, total, files, area_list
        )
    };

    format!(
        "<repo_name>\n{}\n</repo_name>\n\n<repo_info>\n{}\n</repo_info>",
        project_name, info
    )
}

/// Re-read source files and collect RawEntity objects for the scoped entities.
/// This is needed because the graph Entity doesn't store source text.
pub fn collect_raw_entities(
    graph: &RPGraph,
    scope: &LiftScope,
    project_root: &Path,
) -> Result<Vec<RawEntity>> {
    let mut files_to_read: HashMap<std::path::PathBuf, Vec<String>> = HashMap::new();
    for id in &scope.entity_ids {
        if let Some(entity) = graph.entities.get(id) {
            // Skip Module entities — they get features via aggregation, not lifting
            if entity.kind == rpg_core::graph::EntityKind::Module {
                continue;
            }
            files_to_read
                .entry(entity.file.clone())
                .or_default()
                .push(id.clone());
        }
    }

    let mut raw_entities: Vec<RawEntity> = Vec::new();
    for (rel_path, entity_ids) in &files_to_read {
        // Per-file language detection (multi-language graph support)
        let file_lang = rel_path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);
        let Some(language) = file_lang else {
            continue;
        };

        let abs_path = project_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  Warning: could not read {}: {}", rel_path.display(), e);
                continue;
            }
        };

        let file_raws = rpg_parser::entities::extract_entities(rel_path, &source, language);

        let wanted: HashSet<&String> = entity_ids.iter().collect();
        for raw in file_raws {
            let raw_id = raw.id();
            if wanted.contains(&raw_id) {
                raw_entities.push(raw);
            }
        }
    }

    Ok(raw_entities)
}

/// Build token-budget-aware batches from a list of raw entities.
///
/// Per the paper's batching strategy: "accommodate repositories of varying scales
/// while respecting model context limits." Each batch is filled until either the
/// token budget or entity count cap is reached.
///
/// Returns a list of `(start, end)` index ranges into the input slice.
pub fn build_token_aware_batches(
    entities: &[RawEntity],
    max_count: usize,
    max_tokens: usize,
) -> Vec<(usize, usize)> {
    let mut batches = Vec::new();
    let mut batch_start = 0;
    let mut batch_tokens = 0usize;
    let mut batch_count = 0usize;

    for (i, entity) in entities.iter().enumerate() {
        // Estimate tokens: ~4 characters per token is a reasonable heuristic
        let est_tokens = entity.source_text.len() / 4 + 1;

        // Flush if adding this entity would exceed budget (but always include at least 1)
        if batch_count > 0 && (batch_tokens + est_tokens > max_tokens || batch_count >= max_count) {
            batches.push((batch_start, i));
            batch_start = i;
            batch_tokens = 0;
            batch_count = 0;
        }

        batch_tokens += est_tokens;
        batch_count += 1;
    }

    // Flush remaining
    if batch_count > 0 {
        batches.push((batch_start, entities.len()));
    }

    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::EntityKind;

    fn make_raw(name: &str, parent: Option<&str>, source: &str) -> RawEntity {
        make_raw_file(name, parent, source, "src/lib.rs")
    }

    fn make_raw_file(name: &str, parent: Option<&str>, source: &str, file: &str) -> RawEntity {
        RawEntity {
            name: name.to_string(),
            kind: EntityKind::Method,
            file: std::path::PathBuf::from(file),
            line_start: 1,
            line_end: source.lines().count(),
            parent_class: parent.map(|s| s.to_string()),
            source_text: source.to_string(),
        }
    }

    #[test]
    fn test_auto_lift_getter() {
        let raw = make_raw(
            "get_name",
            None,
            "fn get_name(&self) -> &str { &self.name }",
        );
        let features = try_auto_lift(&raw);
        assert!(features.is_some());
        let f = features.unwrap();
        assert_eq!(f.len(), 1);
        assert!(f[0].contains("name"));
        assert!(f[0].starts_with("return"));
    }

    #[test]
    fn test_auto_lift_setter() {
        let raw = make_raw(
            "set_name",
            None,
            "fn set_name(&mut self, n: &str) { self.name = n; }",
        );
        let features = try_auto_lift(&raw);
        assert!(features.is_some());
        assert!(features.unwrap()[0].starts_with("set"));
    }

    #[test]
    fn test_auto_lift_new_with_parent() {
        let raw = make_raw("new", Some("Server"), "fn new() -> Self { Self {} }");
        let features = try_auto_lift(&raw);
        assert!(features.is_some());
        let f = features.unwrap();
        assert!(f[0].contains("server"));
        assert!(f[0].starts_with("create"));
    }

    #[test]
    fn test_auto_lift_default() {
        let raw = make_raw(
            "default",
            Some("Config"),
            "fn default() -> Self { Self {} }",
        );
        let features = try_auto_lift(&raw);
        assert!(features.is_some());
        assert!(features.unwrap()[0].contains("default"));
    }

    #[test]
    fn test_auto_lift_rejects_complex_function() {
        let source = (1..=50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let raw = make_raw("process_data", None, &source);
        let features = try_auto_lift(&raw);
        assert!(
            features.is_none(),
            "complex functions should not be auto-lifted"
        );
    }

    #[test]
    fn test_auto_lift_is_check() {
        let raw = make_raw(
            "is_empty",
            None,
            "fn is_empty(&self) -> bool { self.len == 0 }",
        );
        let features = try_auto_lift(&raw);
        assert!(features.is_some());
        assert!(features.unwrap()[0].starts_with("check"));
    }

    #[test]
    fn test_auto_lift_debug_not_false_positive() {
        // Regression: a non-fmt function whose source contains "debug" should NOT be auto-lifted
        // as display formatting. This catches the boolean precedence bug.
        let raw = make_raw("process", None, "fn process(&self) { debug!(\"hi\"); }");
        let features = try_auto_lift(&raw);
        assert!(
            features.is_none(),
            "non-fmt function containing 'debug' should not be auto-lifted as display"
        );
    }

    #[test]
    fn test_auto_lift_fmt_display() {
        let raw = make_raw(
            "fmt",
            Some("MyStruct"),
            "fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, \"display\") }",
        );
        let features = try_auto_lift(&raw);
        assert!(features.is_some());
        assert!(features.unwrap()[0].contains("format"));
    }

    #[test]
    fn test_auto_lift_from() {
        let raw = make_raw(
            "from",
            Some("MyType"),
            "fn from(v: i32) -> Self { Self(v) }",
        );
        let features = try_auto_lift(&raw);
        assert!(features.is_some());
        assert!(features.unwrap()[0].contains("convert"));
    }

    // --- AutoLiftEngine tests ---

    fn make_engine() -> AutoLiftEngine {
        let defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
        // Core-only engine (no framework paradigms)
        AutoLiftEngine::new(&defs, &[])
    }

    #[test]
    fn test_engine_getter() {
        let engine = make_engine();
        let raw = make_raw(
            "get_name",
            None,
            "fn get_name(&self) -> &str { &self.name }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        let f = features.unwrap();
        assert_eq!(f, vec!["return name"]);
    }

    #[test]
    fn test_engine_setter() {
        let engine = make_engine();
        let raw = make_raw(
            "set_name",
            None,
            "fn set_name(&mut self, n: &str) { self.name = n; }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["set name"]);
    }

    #[test]
    fn test_engine_constructor() {
        let engine = make_engine();
        let raw = make_raw("new", Some("Server"), "fn new() -> Self { Self {} }");
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["create server"]);
    }

    #[test]
    fn test_engine_conversion() {
        let engine = make_engine();
        let raw = make_raw(
            "from",
            Some("MyType"),
            "fn from(v: i32) -> Self { Self(v) }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["convert to mytype"]);
    }

    #[test]
    fn test_engine_fmt() {
        let engine = make_engine();
        let raw = make_raw(
            "fmt",
            Some("MyStruct"),
            "fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, \"display\") }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["format mystruct for display"]);
    }

    #[test]
    fn test_engine_clone() {
        let engine = make_engine();
        let raw = make_raw(
            "clone",
            Some("Config"),
            "fn clone(&self) -> Self { Self {} }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["clone config"]);
    }

    #[test]
    fn test_engine_drop() {
        let engine = make_engine();
        let raw = make_raw("drop", Some("Handle"), "fn drop(&mut self) {}");
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["clean up handle"]);
    }

    #[test]
    fn test_engine_rejects_long() {
        let engine = make_engine();
        let source = (1..=15)
            .map(|i| format!("    line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let raw = make_raw("get_name", None, &source);
        let features = engine.try_lift(&raw);
        assert!(features.is_none(), "long getter should not be auto-lifted");
    }

    #[test]
    fn test_engine_priority() {
        // Framework rules (lower priority number) should match before core rules.
        // Create a framework def with a custom auto_lift rule for "new".
        let framework_def = rpg_parser::paradigms::defs::ParadigmDef {
            schema_version: 1,
            name: "myframework".to_string(),
            priority: 5,
            languages: vec!["rust".to_string()],
            detect: rpg_parser::paradigms::defs::DetectRules::default(),
            classify: Vec::new(),
            entity_queries: Vec::new(),
            dep_queries: Vec::new(),
            auto_lift: vec![rpg_parser::paradigms::defs::AutoLiftRule {
                id: "myframework.new".to_string(),
                match_rule: rpg_parser::paradigms::defs::EntityMatch {
                    name_exact: Some("new".to_string()),
                    max_lines: Some(5),
                    ..rpg_parser::paradigms::defs::EntityMatch::default()
                },
                features: vec!["instantiate {parent_lower} widget".to_string()],
                strip_prefix: Vec::new(),
                prefix_verb: std::collections::HashMap::new(),
            }],
            features: rpg_parser::paradigms::defs::FeatureFlags::default(),
            prompt_hints: rpg_parser::paradigms::defs::PromptHints::default(),
        };
        let mut all_defs = vec![framework_def];
        let core_defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
        all_defs.extend(core_defs);
        // Sort by priority like load_builtin_defs does
        all_defs.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.name.cmp(&b.name))
        });

        let engine = AutoLiftEngine::new(&all_defs, &["myframework".to_string()]);
        let raw = make_raw("new", Some("Button"), "fn new() -> Self { Self {} }");
        let features = engine.try_lift(&raw).unwrap();
        assert_eq!(features, vec!["instantiate button widget"]);
    }

    #[test]
    fn test_engine_template_expansion() {
        let engine = make_engine();

        // Test {field} expansion (strip_prefix removes "get_", underscores become spaces)
        let raw = make_raw(
            "get_max_count",
            None,
            "fn get_max_count(&self) -> usize { self.max_count }",
        );
        let features = engine.try_lift(&raw).unwrap();
        assert_eq!(features, vec!["return max count"]);

        // Test {verb} expansion (is_ → "check")
        let raw = make_raw("is_valid", None, "fn is_valid(&self) -> bool { true }");
        let features = engine.try_lift(&raw).unwrap();
        assert_eq!(features, vec!["check valid"]);

        // Test {parent_lower} for constructor
        let raw = make_raw(
            "default",
            Some("AppConfig"),
            "fn default() -> Self { Self {} }",
        );
        let features = engine.try_lift(&raw).unwrap();
        assert_eq!(features, vec!["create default appconfig"]);
    }

    // --- normalize_field tests ---

    #[test]
    fn test_normalize_field_camel() {
        assert_eq!(super::normalize_field("MaxCount"), "max count");
    }

    #[test]
    fn test_normalize_field_snake() {
        assert_eq!(super::normalize_field("max_count"), "max count");
    }

    #[test]
    fn test_normalize_field_pascal() {
        assert_eq!(super::normalize_field("GetName"), "get name");
    }

    #[test]
    fn test_normalize_field_single() {
        assert_eq!(super::normalize_field("name"), "name");
        assert_eq!(super::normalize_field("Name"), "name");
    }

    #[test]
    fn test_normalize_field_empty() {
        assert_eq!(super::normalize_field(""), "");
    }

    #[test]
    fn test_normalize_field_acronym() {
        // Consecutive uppercase is grouped as an acronym, split before the
        // last uppercase when followed by lowercase.
        assert_eq!(super::normalize_field("HTTPServer"), "http server");
        assert_eq!(super::normalize_field("getHTTPServer"), "get http server");
        assert_eq!(super::normalize_field("XMLParser"), "xml parser");
        assert_eq!(super::normalize_field("IOError"), "io error");
        assert_eq!(super::normalize_field("ID"), "id");
    }

    // --- camelCase / PascalCase engine tests ---

    #[test]
    fn test_engine_camel_getter() {
        let engine = make_engine();
        let raw = make_raw("getName", None, "String getName() { return this.name; }");
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["return name"]);
    }

    #[test]
    fn test_engine_camel_setter() {
        let engine = make_engine();
        let raw = make_raw(
            "setMaxCount",
            None,
            "void setMaxCount(int n) { this.max = n; }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["set max count"]);
    }

    #[test]
    fn test_engine_pascal_getter() {
        let engine = make_engine();
        let raw = make_raw(
            "GetName",
            None,
            "func (s *S) GetName() string { return s.name }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["return name"]);
    }

    #[test]
    fn test_engine_go_constructor() {
        let engine = make_engine();
        let raw = make_raw(
            "NewServer",
            None,
            "func NewServer() *Server { return &Server{} }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(features.unwrap(), vec!["create server"]);
    }

    #[test]
    fn test_engine_java_tostring() {
        let engine = make_engine();
        let raw = make_raw(
            "toString",
            Some("User"),
            "String toString() { return name; }",
        );
        let features = engine.try_lift(&raw);
        assert!(features.is_some());
        assert_eq!(
            features.unwrap(),
            vec!["return string representation of user"]
        );
    }

    // --- Language scoping tests ---

    fn make_mixed_engine() -> AutoLiftEngine {
        let defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
        // Activate both C and Go paradigms (simulates a mixed-language repo)
        AutoLiftEngine::new(&defs, &["c".to_string(), "go".to_string()])
    }

    #[test]
    fn test_engine_language_scoping_go_init() {
        // Regression: Go `init()` in a .go file must match go.init ("initialize package"),
        // NOT c.init ("initialize init") — even though C paradigm is also active.
        let engine = make_mixed_engine();
        let raw = make_raw_file("init", None, "func init() { setup() }", "cmd/main.go");
        let features = engine.try_lift(&raw).unwrap();
        assert_eq!(
            features,
            vec!["initialize package"],
            "Go init() should match go.init, not c.init"
        );
    }

    #[test]
    fn test_engine_language_scoping_c_init() {
        // C `init` in a .c file should match c.init
        let engine = make_mixed_engine();
        let raw = make_raw_file("init", None, "void init() { setup(); }", "src/module.c");
        let features = engine.try_lift(&raw).unwrap();
        assert_eq!(
            features,
            vec!["initialize init"],
            "C init() should match c.init, not go.init"
        );
    }

    #[test]
    fn test_engine_core_rules_match_any_language() {
        // Core rules (languages = []) should match regardless of file extension
        let engine = make_mixed_engine();
        let raw = make_raw_file(
            "getName",
            None,
            "String getName() { return this.name; }",
            "src/User.java",
        );
        let features = engine.try_lift(&raw);
        assert!(
            features.is_some(),
            "core camelCase getter should match .java files"
        );
        assert_eq!(features.unwrap(), vec!["return name"]);
    }

    #[test]
    fn test_engine_unknown_extension_skips_language_rules() {
        // Language-specific rules should NOT match files with unknown extensions.
        // Only core rules (languages = []) should apply.
        let engine = make_mixed_engine();
        let raw = make_raw_file("init", None, "func init() { }", "src/module.xyz");
        // go.init and c.init should both be skipped (unknown extension).
        // No core rule matches bare "init", so no auto-lift.
        let features = engine.try_lift(&raw);
        assert!(
            features.is_none(),
            "language-specific rules should not match unknown file extensions"
        );
    }

    // --- Confidence-gated auto-lift tests ---

    #[test]
    fn test_confidence_accept_simple_getter() {
        let engine = make_engine();
        let raw = make_raw(
            "get_name",
            None,
            "fn get_name(&self) -> &str { &self.name }",
        );
        let result = engine.try_lift_with_confidence(&raw);
        assert!(result.is_some());
        let (features, confidence) = result.unwrap();
        assert_eq!(features, vec!["return name"]);
        assert_eq!(confidence, LiftConfidence::Accept);
    }

    #[test]
    fn test_confidence_accept_8_line_getter() {
        // An 8-line getter with 0 branches, 0 loops should Accept
        let source = "fn get_value(&self) -> Value {\n    let a = self.a;\n    let b = self.b;\n    let c = self.c;\n    let d = self.d;\n    let e = self.e;\n    let f = self.f;\n    Value(a + b + c + d + e + f)\n}";
        let raw = make_raw("get_value", None, source);
        let result = make_engine().try_lift_with_confidence(&raw);
        assert!(result.is_some());
        let (_, confidence) = result.unwrap();
        assert_eq!(confidence, LiftConfidence::Accept);
    }

    #[test]
    fn test_confidence_review_moderate() {
        // A getter with exactly 1 branch (if, no else) → passes TOML max_branches=1,
        // but confidence logic sees branch_count >= 1 → Review
        let source = "fn get_name(&self) -> &str {\n    if self.name.is_empty() {\n        return &self.default_name;\n    }\n    &self.name\n}";
        let raw = make_raw("get_name", None, source);
        let result = make_engine().try_lift_with_confidence(&raw);
        assert!(result.is_some());
        let (features, confidence) = result.unwrap();
        assert_eq!(features, vec!["return name"]);
        assert_eq!(confidence, LiftConfidence::Review);
    }

    #[test]
    fn test_confidence_reject_complex() {
        // A getter with 3 branches → doesn't match TOML (max_branches=1), returns None
        let source = "fn get_name(&self) -> &str {\n    if self.a { return \"a\"; }\n    if self.b { return \"b\"; }\n    if self.c { return \"c\"; }\n    &self.name\n}";
        let raw = make_raw("get_name", None, source);
        let result = make_engine().try_lift_with_confidence(&raw);
        assert!(
            result.is_none(),
            "3+ branches should fail TOML match (max_branches=1)"
        );
    }

    #[test]
    fn test_confidence_reject_high_complexity() {
        // A fmt_display rule allows max_branches=2, so a source with 2 branches
        // passes TOML but has branch_count > 1 → Reject
        let source = "fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {\n    if self.verbose {\n        write!(f, \"Verbose Display\")\n    } else {\n        write!(f, \"Display\")\n    }\n}";
        let raw = make_raw("fmt", Some("Widget"), source);
        let result = make_engine().try_lift_with_confidence(&raw);
        assert!(result.is_some(), "should match fmt_display rule");
        let (_, confidence) = result.unwrap();
        assert_eq!(confidence, LiftConfidence::Reject);
    }

    #[test]
    fn test_confidence_no_structural_gate() {
        // Rules without structural fields (clone, drop) → always Accept
        let engine = make_engine();
        let raw = make_raw(
            "clone",
            Some("Config"),
            "fn clone(&self) -> Self { Self {} }",
        );
        let result = engine.try_lift_with_confidence(&raw);
        assert!(result.is_some());
        let (_, confidence) = result.unwrap();
        assert_eq!(confidence, LiftConfidence::Accept);
    }

    #[test]
    fn test_feature_source_auto() {
        // Auto-accepted entities should be tagged with "auto" feature_source
        // This tests the LiftConfidence enum is available and correct
        let engine = make_engine();
        let raw = make_raw("get_id", None, "fn get_id(&self) -> u64 { self.id }");
        let (_, confidence) = engine.try_lift_with_confidence(&raw).unwrap();
        assert_eq!(confidence, LiftConfidence::Accept);
    }

    #[test]
    fn test_feature_source_llm() {
        // Entities that don't match any auto-lift rule need LLM lifting.
        // try_lift_with_confidence returns None, and the MCP layer sets feature_source="llm".
        let engine = make_engine();
        // A function name that doesn't match any auto-lift pattern
        let raw = make_raw(
            "calculate_total",
            None,
            "fn calculate_total(items: &[Item]) -> f64 {\n    items.iter().map(|i| i.price).sum()\n}",
        );
        let result = engine.try_lift_with_confidence(&raw);
        assert!(
            result.is_none(),
            "non-matching entities return None (need LLM lifting → feature_source=\"llm\")"
        );
    }
}
