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

/// Engine that matches entities against TOML-defined auto-lift rules.
///
/// Rules are collected from paradigm definitions in priority order.
/// `core.toml` (priority 100) is always included; framework-specific rules
/// from detected paradigms are prepended (lower priority number = higher priority
/// = checked first).
pub struct AutoLiftEngine {
    rules: Vec<AutoLiftRule>,
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
                rules.extend(def.auto_lift.iter().cloned());
            }
        }
        Self { rules }
    }

    /// Try to match an entity against auto-lift rules. First match wins.
    pub fn try_lift(&self, raw: &RawEntity) -> Option<Vec<String>> {
        for rule in &self.rules {
            if matches_entity(&rule.match_rule, raw, &raw.file) {
                return Some(Self::expand_templates(rule, raw));
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
        RawEntity {
            name: name.to_string(),
            kind: EntityKind::Method,
            file: std::path::PathBuf::from("src/lib.rs"),
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
        let source = (1..=10)
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
}
