//! TOON (Token-Oriented Object Notation) serializer for MCP tool output.
//!
//! Produces compact, LLM-optimized output following the TOON format spec:
//! indentation-based objects, tabular arrays, canonical numbers, minimal quoting.
//!
//! TOON format: <https://github.com/toon-format/toon>

use crate::fetch::{FetchOutput, FetchResult, HierarchyFetchResult};
use crate::search::SearchResult;
use rpg_core::graph::RPGraph;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Escape a string value for TOON. Returns unquoted if safe, quoted otherwise.
fn toon_escape(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_string();
    }

    let needs_quote = s.starts_with(' ')
        || s.ends_with(' ')
        || s.contains(':')
        || s.contains('|')
        || s.contains(',')
        || s.contains('\\')
        || s.contains('"')
        || s.contains('\n')
        || s.contains('\r')
        || s.contains('\t')
        || s == "true"
        || s == "false"
        || s == "null";

    if !needs_quote {
        return s.to_string();
    }

    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escape a value for use inside a delimited tabular row.
/// Quotes if the value contains the active delimiter character.
fn toon_cell(s: &str, delimiter: char) -> String {
    // If the string contains the delimiter, it must be quoted regardless
    if s.contains(delimiter) {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        return format!("\"{}\"", escaped);
    }
    toon_escape(s)
}

/// Format a float score in canonical TOON decimal.
/// NaN and Infinity become `null`, -0 becomes `0`, no trailing zeros.
fn toon_score(v: f64) -> String {
    if v.is_nan() || v.is_infinite() {
        return "null".to_string();
    }

    // Handle negative zero
    let v = if v == 0.0 { 0.0 } else { v };

    if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.6}", v);
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    }
}

/// Format a TOON list: `key[N]:` header followed by indented items.
fn toon_list(indent: usize, key: &str, items: &[String]) -> String {
    let prefix = "  ".repeat(indent);
    let child_prefix = "  ".repeat(indent + 1);

    if items.is_empty() {
        return format!("{}{}[0]:", prefix, key);
    }

    let mut out = format!("{}{}[{}]:\n", prefix, key, items.len());
    for item in items {
        out.push_str(&child_prefix);
        out.push_str(&toon_escape(item));
        out.push('\n');
    }
    // Remove trailing newline (will be added by caller or final trim)
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Public formatters
// ---------------------------------------------------------------------------

/// Format search results as a TOON tabular array with auto-selected delimiter.
pub fn format_search_results(results: &[SearchResult]) -> String {
    let header = "results";
    let fields = "{name,file,line,score,lifted,features}";

    if results.is_empty() {
        return format!("{}[0]{}:", header, fields);
    }

    // Check if any cell data contains a pipe to choose delimiter
    let has_pipe = results.iter().any(|r| {
        r.entity_name.contains('|')
            || r.file.contains('|')
            || r.matched_features.iter().any(|f| f.contains('|'))
    });
    let delim = if has_pipe { ',' } else { '|' };
    let mut out = format!("{}[{}{}]{fields}:\n", header, results.len(), delim);

    let lifted_count = results.iter().filter(|r| r.lifted).count();

    for r in results {
        let features = r.matched_features.join(", ");
        let lifted_str = if r.lifted { "yes" } else { "no" };
        let row = format!(
            "  {}{}{}{}{}{}{}{}{}{}{}",
            toon_cell(&r.entity_name, delim),
            delim,
            toon_cell(&r.file, delim),
            delim,
            r.line_start,
            delim,
            toon_score(r.score),
            delim,
            lifted_str,
            delim,
            toon_cell(&features, delim),
        );
        out.push_str(&row);
        out.push('\n');
    }

    if lifted_count < results.len() {
        out.push_str(&format!(
            "  ({}/{} lifted. Use get_entities_for_lifting or lift_area to add semantic features.)\n",
            lifted_count,
            results.len()
        ));
    }

    // Remove trailing newline per TOON spec §13.1
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Format a fetch result as a TOON indented object.
pub fn format_fetch_result(result: &FetchResult) -> String {
    let entity = &result.entity;
    let mut lines: Vec<String> = Vec::new();

    // Scalar fields
    lines.push(format!("name: {}", toon_escape(&entity.name)));
    lines.push(format!(
        "kind: {}",
        format!("{:?}", entity.kind).to_lowercase()
    ));
    lines.push(format!(
        "file: {}",
        toon_escape(&entity.file.display().to_string())
    ));
    lines.push(format!("lines: {}-{}", entity.line_start, entity.line_end));

    if !entity.hierarchy_path.is_empty() {
        lines.push(format!(
            "hierarchy: {}",
            toon_escape(&entity.hierarchy_path)
        ));
    }

    // Lifting status + features
    if entity.semantic_features.is_empty() {
        lines.push(
            "lifted: no (use get_entities_for_lifting or lift_area to add semantic features)"
                .to_string(),
        );
    } else {
        lines.push("lifted: yes".to_string());
        lines.push(toon_list(0, "features", &entity.semantic_features));
    }

    // Dependency lists (omit empty ones to save tokens)
    let dep_fields: &[(&str, &Vec<String>)] = &[
        ("invokes", &entity.deps.invokes),
        ("invoked_by", &entity.deps.invoked_by),
        ("imports", &entity.deps.imports),
        ("imported_by", &entity.deps.imported_by),
        ("inherits", &entity.deps.inherits),
        ("inherited_by", &entity.deps.inherited_by),
    ];

    for (key, vals) in dep_fields {
        if !vals.is_empty() {
            lines.push(toon_list(0, key, vals));
        }
    }

    // Siblings
    if !result.hierarchy_context.is_empty() {
        lines.push(toon_list(0, "siblings", &result.hierarchy_context));
    }

    // Source code (always last field)
    match &result.source_code {
        Some(code) => {
            lines.push("source:".to_string());
            for line in code.lines() {
                lines.push(format!("  {}", line));
            }
        }
        None => {
            lines.push("source: null".to_string());
        }
    }

    lines.join("\n")
}

/// Format a hierarchy node fetch result as a TOON indented object.
pub fn format_hierarchy_fetch_result(result: &HierarchyFetchResult) -> String {
    let node = &result.node;
    let mut lines: Vec<String> = Vec::new();

    lines.push("type: hierarchy_node".to_string());
    lines.push(format!("name: {}", toon_escape(&node.name)));
    lines.push(format!("id: {}", toon_escape(&node.id)));
    lines.push(format!("entities: {}", result.entity_count));

    if !node.grounded_paths.is_empty() {
        let paths: Vec<String> = node
            .grounded_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        lines.push(toon_list(0, "grounded_paths", &paths));
    }

    if !result.child_names.is_empty() {
        lines.push(toon_list(0, "children", &result.child_names));
    }

    if !node.semantic_features.is_empty() {
        let top_features: Vec<String> = node.semantic_features.iter().take(20).cloned().collect();
        lines.push(toon_list(0, "features", &top_features));
    }

    if !node.entities.is_empty() {
        let entity_ids: Vec<String> = node.entities.iter().take(20).cloned().collect();
        lines.push(toon_list(0, "entity_ids", &entity_ids));
    }

    lines.join("\n")
}

/// Format a FetchOutput (entity or hierarchy node) as TOON.
pub fn format_fetch_output(output: &FetchOutput) -> String {
    match output {
        FetchOutput::Entity(result) => format_fetch_result(result),
        FetchOutput::Hierarchy(result) => format_hierarchy_fetch_result(result),
    }
}

/// Format RPG info as a TOON indented object.
pub fn format_rpg_info(graph: &RPGraph) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("version: {}", graph.version));
    lines.push(format!("language: {}", graph.metadata.language));
    lines.push(format!("updated: {}", graph.updated_at));

    if let Some(sha) = &graph.base_commit {
        let short = &sha[..8.min(sha.len())];
        lines.push(format!("commit: {}", short));
    }

    lines.push(format!("entities: {}", graph.metadata.total_entities));
    lines.push(format!(
        "lifted: {}/{}",
        graph.metadata.lifted_entities, graph.metadata.total_entities
    ));
    lines.push(format!("files: {}", graph.metadata.total_files));
    lines.push(format!("areas: {}", graph.metadata.functional_areas));
    lines.push(format!("edges: {}", graph.metadata.total_edges));
    lines.push(format!(
        "hierarchy_type: {}",
        if graph.metadata.semantic_hierarchy {
            "semantic"
        } else {
            "structural"
        }
    ));

    if let Some(summary) = &graph.metadata.repo_summary {
        lines.push(format!("summary: {}", summary));
    }

    if !graph.hierarchy.is_empty() {
        lines.push("hierarchy:".to_string());
        for (name, area) in &graph.hierarchy {
            lines.push(format!("  {} ({} entities)", name, area.entity_count()));
            for (cat_name, cat) in &area.children {
                lines.push(format!(
                    "    {} ({} entities)",
                    cat_name,
                    cat.entity_count()
                ));
            }
        }
    }

    // Per-area lifting coverage
    let area_cov = graph.area_coverage();
    if !area_cov.is_empty() {
        lines.push("coverage_by_area:".to_string());
        let mut lowest_pct = 100.0f64;
        let mut lowest_area = String::new();
        for (name, lifted, total) in &area_cov {
            let pct = if *total > 0 {
                *lifted as f64 / *total as f64 * 100.0
            } else {
                100.0
            };
            lines.push(format!("  {}: {}/{} ({:.0}%)", name, lifted, total, pct));
            if pct < lowest_pct && *total > 0 {
                lowest_pct = pct;
                lowest_area = name.clone();
            }
        }
        if lowest_pct < 100.0 {
            lines.push(format!(
                "Tip: lift area \"{}/**\" to improve coverage.",
                lowest_area
            ));
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toon_escape_safe() {
        assert_eq!(toon_escape("hello"), "hello");
        assert_eq!(toon_escape("foo_bar"), "foo_bar");
        assert_eq!(toon_escape("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn test_toon_escape_empty() {
        assert_eq!(toon_escape(""), "\"\"");
    }

    #[test]
    fn test_toon_escape_special() {
        assert_eq!(toon_escape("key: value"), "\"key: value\"");
        assert_eq!(toon_escape("a|b"), "\"a|b\"");
        assert_eq!(toon_escape("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(toon_escape("line1\nline2"), "\"line1\\nline2\"");
        assert_eq!(toon_escape("path\\to"), "\"path\\\\to\"");
        assert_eq!(toon_escape("true"), "\"true\"");
        assert_eq!(toon_escape("null"), "\"null\"");
    }

    #[test]
    fn test_toon_escape_comma() {
        assert_eq!(toon_escape("a,b"), "\"a,b\"");
    }

    #[test]
    fn test_toon_escape_whitespace() {
        assert_eq!(toon_escape(" padded"), "\" padded\"");
        assert_eq!(toon_escape("padded "), "\"padded \"");
    }

    #[test]
    fn test_toon_cell_pipe_delimiter() {
        assert_eq!(toon_cell("hello", '|'), "hello");
        assert_eq!(toon_cell("a|b", '|'), "\"a|b\"");
    }

    #[test]
    fn test_toon_cell_comma_delimiter() {
        assert_eq!(toon_cell("hello", ','), "hello");
        assert_eq!(toon_cell("a,b", ','), "\"a,b\"");
        // Pipe is safe when comma is the delimiter
        assert_eq!(toon_cell("a|b", ','), "\"a|b\"");
    }

    #[test]
    fn test_toon_score() {
        assert_eq!(toon_score(1.0), "1");
        assert_eq!(toon_score(0.5), "0.5");
        assert_eq!(toon_score(2.33), "2.33");
        assert_eq!(toon_score(0.0), "0");
        assert_eq!(toon_score(1.500000), "1.5");
        assert_eq!(toon_score(42.0), "42");
    }

    #[test]
    fn test_toon_score_nan() {
        assert_eq!(toon_score(f64::NAN), "null");
    }

    #[test]
    fn test_toon_score_infinity() {
        assert_eq!(toon_score(f64::INFINITY), "null");
        assert_eq!(toon_score(f64::NEG_INFINITY), "null");
    }

    #[test]
    fn test_toon_score_negative_zero() {
        assert_eq!(toon_score(-0.0), "0");
    }

    #[test]
    fn test_toon_list_empty() {
        assert_eq!(toon_list(0, "items", &[]), "items[0]:");
    }

    #[test]
    fn test_toon_list_values() {
        let items = vec!["alpha".to_string(), "beta".to_string()];
        let expected = "items[2]:\n  alpha\n  beta";
        assert_eq!(toon_list(0, "items", &items), expected);
    }

    #[test]
    fn test_toon_list_indented() {
        let items = vec!["x".to_string()];
        let expected = "  nested[1]:\n    x";
        assert_eq!(toon_list(1, "nested", &items), expected);
    }

    #[test]
    fn test_format_search_results_empty() {
        let result = format_search_results(&[]);
        assert_eq!(result, "results[0]{name,file,line,score,lifted,features}:");
    }

    #[test]
    fn test_format_search_results_single() {
        let results = vec![SearchResult {
            entity_id: "src/main.rs:main".to_string(),
            entity_name: "main".to_string(),
            file: "src/main.rs".to_string(),
            line_start: 1,
            score: 1.5,
            matched_features: vec!["entry point".to_string()],
            lifted: true,
        }];
        let output = format_search_results(&results);
        // Header should include delimiter indicator and lifted field
        assert!(output.starts_with("results[1|]{name,file,line,score,lifted,features}:"));
        assert!(output.contains("main|src/main.rs|1|1.5|yes|entry point"));
        // No trailing newline
        assert!(!output.ends_with('\n'));
    }

    #[test]
    fn test_format_search_results_no_trailing_whitespace() {
        let results = vec![SearchResult {
            entity_id: "a.rs:foo".to_string(),
            entity_name: "foo".to_string(),
            file: "a.rs".to_string(),
            line_start: 5,
            score: 0.8,
            matched_features: vec![],
            lifted: false,
        }];
        let output = format_search_results(&results);
        assert!(!output.ends_with('\n'));
        assert!(!output.ends_with(' '));
        // Each line should not have trailing whitespace
        for line in output.lines() {
            assert!(!line.ends_with(' '), "trailing space in: {:?}", line);
        }
    }
}
