//! On-demand semantic lifting: scope resolution, batching, and incremental update utilities.

use anyhow::Result;
use rpg_core::graph::RPGraph;
use rpg_parser::entities::RawEntity;
use rpg_parser::languages::Language;
use std::collections::{HashMap, HashSet};
use std::path::Path;

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

/// Try to auto-lift a trivial entity based on heuristics.
///
/// Returns `Some(features)` for entities that are obviously trivial:
/// - ≤3 lines + getter/setter/new/default/from/into name pattern
/// - Returns `None` for anything it can't confidently classify
pub fn try_auto_lift(raw: &RawEntity) -> Option<Vec<String>> {
    let line_count = raw.source_text.lines().count();
    if line_count > 3 {
        return None;
    }

    let name_lower = raw.name.to_lowercase();
    let source_lower = raw.source_text.to_lowercase();

    // Getter patterns: get_*, is_*, has_*
    if name_lower.starts_with("get_")
        || name_lower.starts_with("is_")
        || name_lower.starts_with("has_")
    {
        let field = name_lower
            .strip_prefix("get_")
            .or_else(|| name_lower.strip_prefix("is_"))
            .or_else(|| name_lower.strip_prefix("has_"))
            .unwrap_or(&name_lower);
        let verb = if name_lower.starts_with("get_") {
            "return"
        } else {
            "check"
        };
        return Some(vec![format!("{} {}", verb, field.replace('_', " "))]);
    }

    // Setter patterns: set_*, with_*
    if name_lower.starts_with("set_") || name_lower.starts_with("with_") {
        let field = name_lower
            .strip_prefix("set_")
            .or_else(|| name_lower.strip_prefix("with_"))
            .unwrap_or(&name_lower);
        return Some(vec![format!("set {}", field.replace('_', " "))]);
    }

    // Constructor: new, default, from, into
    if name_lower == "new" || name_lower == "default" {
        // Infer type from parent class if available
        let type_name = raw
            .parent_class
            .as_deref()
            .unwrap_or("instance")
            .to_lowercase();
        let verb = if name_lower == "default" {
            "create default"
        } else {
            "create"
        };
        return Some(vec![format!("{} {}", verb, type_name)]);
    }

    // From/Into trait implementations
    if name_lower == "from" || name_lower == "into" {
        let type_name = raw
            .parent_class
            .as_deref()
            .unwrap_or("value")
            .to_lowercase();
        return Some(vec![format!("convert to {}", type_name)]);
    }

    // Display/Debug/ToString
    if name_lower == "fmt" && (source_lower.contains("display") || source_lower.contains("debug")) {
        let type_name = raw
            .parent_class
            .as_deref()
            .unwrap_or("value")
            .to_lowercase();
        return Some(vec![format!("format {} for display", type_name)]);
    }

    // Clone/Drop
    if name_lower == "clone" {
        let type_name = raw
            .parent_class
            .as_deref()
            .unwrap_or("value")
            .to_lowercase();
        return Some(vec![format!("clone {}", type_name)]);
    }
    if name_lower == "drop" {
        let type_name = raw
            .parent_class
            .as_deref()
            .unwrap_or("value")
            .to_lowercase();
        return Some(vec![format!("clean up {}", type_name)]);
    }

    None
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
}
