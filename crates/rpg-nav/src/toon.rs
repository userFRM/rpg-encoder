//! TOON (Token-Oriented Object Notation) serializer for MCP tool output.
//!
//! Uses the `toon-format` crate for spec-compliant serialization.
//! Domain-specific output structures are defined as serde-serializable types.
//!
//! TOON format: <https://github.com/toon-format/toon>

use crate::fetch::{FetchOutput, FetchResult, HierarchyFetchResult};
use crate::search::SearchResult;
use rpg_core::graph::RPGraph;
use serde::Serialize;
use toon_format::{EncodeOptions, encode};

/// Get default encoding options: pipe delimiter, 2-space indent.
fn encode_opts() -> EncodeOptions {
    EncodeOptions::default()
        .with_delimiter(toon_format::Delimiter::Pipe)
        .with_indent(toon_format::Indent::Spaces(2))
}

// ---------------------------------------------------------------------------
// Search result output
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SearchResultRow {
    name: String,
    file: String,
    line: usize,
    score: f64,
    lifted: bool,
    features: String,
}

#[derive(Serialize)]
struct SearchOutput {
    results: Vec<SearchResultRow>,
    lifted_count: usize,
    total_count: usize,
}

/// Format search results as TOON.
pub fn format_search_results(results: &[SearchResult]) -> String {
    let lifted_count = results.iter().filter(|r| r.lifted).count();
    let total_count = results.len();

    let output = SearchOutput {
        results: results
            .iter()
            .map(|r| SearchResultRow {
                name: r.entity_name.clone(),
                file: r.file.clone(),
                line: r.line_start,
                score: clean_score(r.score),
                lifted: r.lifted,
                features: r.matched_features.join(", "),
            })
            .collect(),
        lifted_count,
        total_count,
    };

    let mut toon = encode(&output, &encode_opts()).unwrap_or_else(|_| format!("{:?}", results));

    if lifted_count < total_count {
        toon.push_str(&format!(
            "\n({}/{} lifted. Use get_entities_for_lifting + submit_lift_results to add semantic features.)",
            lifted_count, total_count
        ));
    }

    toon
}

// ---------------------------------------------------------------------------
// Fetch result output
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct FetchEntityOutput {
    name: String,
    kind: String,
    file: String,
    lines: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    hierarchy: String,
    lifted: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    features: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    invokes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    invoked_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    imports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    imported_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    inherits: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    inherited_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    renders: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    rendered_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reads_state: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    state_read_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    writes_state: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    state_written_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dispatches: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dispatched_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    siblings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

/// Format a fetch result as TOON.
pub fn format_fetch_result(result: &FetchResult) -> String {
    let entity = &result.entity;

    let output = FetchEntityOutput {
        name: entity.name.clone(),
        kind: format!("{:?}", entity.kind).to_lowercase(),
        file: entity.file.display().to_string(),
        lines: format!("{}-{}", entity.line_start, entity.line_end),
        hierarchy: entity.hierarchy_path.clone(),
        lifted: !entity.semantic_features.is_empty(),
        features: entity.semantic_features.clone(),
        invokes: entity.deps.invokes.clone(),
        invoked_by: entity.deps.invoked_by.clone(),
        imports: entity.deps.imports.clone(),
        imported_by: entity.deps.imported_by.clone(),
        inherits: entity.deps.inherits.clone(),
        inherited_by: entity.deps.inherited_by.clone(),
        renders: entity.deps.renders.clone(),
        rendered_by: entity.deps.rendered_by.clone(),
        reads_state: entity.deps.reads_state.clone(),
        state_read_by: entity.deps.state_read_by.clone(),
        writes_state: entity.deps.writes_state.clone(),
        state_written_by: entity.deps.state_written_by.clone(),
        dispatches: entity.deps.dispatches.clone(),
        dispatched_by: entity.deps.dispatched_by.clone(),
        siblings: result.hierarchy_context.clone(),
        source: result.source_code.clone(),
    };

    let mut toon = encode(&output, &encode_opts()).unwrap_or_else(|_| format!("{:?}", result));

    if entity.semantic_features.is_empty() {
        toon.push_str(
            "\n(not lifted — use get_entities_for_lifting + submit_lift_results to add semantic features)",
        );
    }

    toon
}

// ---------------------------------------------------------------------------
// Hierarchy node fetch result
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct HierarchyNodeOutput {
    r#type: String,
    name: String,
    id: String,
    entities: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    grounded_paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    features: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    entity_ids: Vec<String>,
}

/// Format a hierarchy node fetch result as TOON.
pub fn format_hierarchy_fetch_result(result: &HierarchyFetchResult) -> String {
    let node = &result.node;

    let output = HierarchyNodeOutput {
        r#type: "hierarchy_node".to_string(),
        name: node.name.clone(),
        id: node.id.clone(),
        entities: result.entity_count,
        grounded_paths: node
            .grounded_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        children: result.child_names.clone(),
        features: node.semantic_features.iter().take(20).cloned().collect(),
        entity_ids: node.entities.iter().take(20).cloned().collect(),
    };

    encode(&output, &encode_opts()).unwrap_or_else(|_| format!("{:?}", result))
}

/// Format a FetchOutput (entity or hierarchy node) as TOON.
pub fn format_fetch_output(output: &FetchOutput) -> String {
    match output {
        FetchOutput::Entity(result) => format_fetch_result(result),
        FetchOutput::Hierarchy(result) => format_hierarchy_fetch_result(result),
    }
}

// ---------------------------------------------------------------------------
// RPG info output
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AreaInfo {
    name: String,
    entities: usize,
    children: Vec<String>,
}

#[derive(Serialize)]
struct CoverageInfo {
    area: String,
    lifted: usize,
    total: usize,
    pct: u32,
}

#[derive(Serialize)]
struct RpgInfoOutput {
    version: String,
    languages: String,
    updated: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    entities: usize,
    lifted: String,
    files: usize,
    areas: usize,
    edges: usize,
    hierarchy_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hierarchy: Vec<AreaInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    coverage_by_area: Vec<CoverageInfo>,
}

/// Format RPG info as TOON.
pub fn format_rpg_info(graph: &RPGraph) -> String {
    let hierarchy: Vec<AreaInfo> = graph
        .hierarchy
        .iter()
        .map(|(name, area)| AreaInfo {
            name: name.clone(),
            entities: area.entity_count(),
            children: area.children.keys().cloned().collect(),
        })
        .collect();

    let area_cov = graph.area_coverage();
    let coverage_by_area: Vec<CoverageInfo> = area_cov
        .iter()
        .map(|(name, lifted, total)| {
            #[allow(clippy::cast_sign_loss)]
            let pct = if *total > 0 {
                (*lifted as f64 / *total as f64 * 100.0) as u32
            } else {
                100
            };
            CoverageInfo {
                area: name.clone(),
                lifted: *lifted,
                total: *total,
                pct,
            }
        })
        .collect();

    let output = RpgInfoOutput {
        version: graph.version.clone(),
        languages: if graph.metadata.languages.is_empty() {
            graph.metadata.language.clone()
        } else {
            graph.metadata.languages.join(", ")
        },
        updated: graph.updated_at.to_string(),
        commit: graph
            .base_commit
            .as_ref()
            .map(|s| s[..8.min(s.len())].to_string()),
        entities: graph.metadata.total_entities,
        lifted: format!(
            "{}/{}",
            graph.metadata.lifted_entities, graph.metadata.total_entities
        ),
        files: graph.metadata.total_files,
        areas: graph.metadata.functional_areas,
        edges: graph.metadata.total_edges,
        hierarchy_type: if graph.metadata.semantic_hierarchy {
            "semantic".to_string()
        } else {
            "structural".to_string()
        },
        summary: graph.metadata.repo_summary.clone(),
        hierarchy,
        coverage_by_area,
    };

    let mut toon = encode(&output, &encode_opts()).unwrap_or_else(|_| "encoding error".to_string());

    // Add tip for lowest coverage area
    let lowest = area_cov
        .iter()
        .filter(|(_, _, total)| *total > 0)
        .min_by_key(|(_, lifted, total)| (*lifted as f64 / *total as f64 * 10000.0) as i64);

    if let Some((name, lifted, total)) = lowest
        && *lifted < *total
    {
        toon.push_str(&format!(
            "\nTip: lift area \"{}/**\" to improve coverage.",
            name
        ));
    }

    toon
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Clean a float score: NaN/Infinity → 0, -0 → 0, round to 6 decimals.
fn clean_score(v: f64) -> f64 {
    if v.is_nan() || v.is_infinite() {
        return 0.0;
    }
    if v == 0.0 {
        return 0.0;
    }
    // Round to 6 decimal places
    (v * 1_000_000.0).round() / 1_000_000.0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_search_results_empty() {
        let result = format_search_results(&[]);
        // Should contain results as empty array
        assert!(result.contains("results"));
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
        assert!(output.contains("main"));
        assert!(output.contains("src/main.rs"));
        assert!(output.contains("entry point"));
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
        // Each line should not have trailing whitespace
        for line in output.lines() {
            assert!(!line.ends_with(' '), "trailing space in: {:?}", line);
        }
    }

    #[test]
    fn test_clean_score() {
        assert_eq!(clean_score(1.0), 1.0);
        assert_eq!(clean_score(0.5), 0.5);
        assert_eq!(clean_score(f64::NAN), 0.0);
        assert_eq!(clean_score(f64::INFINITY), 0.0);
        assert_eq!(clean_score(-0.0), 0.0);
    }
}
