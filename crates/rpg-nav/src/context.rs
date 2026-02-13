//! ContextPack: single-call retrieval that searches, fetches, and explores in one operation.

use crate::explore::{Direction, get_neighbors};
use crate::search::{SearchMode, SearchParams, search_with_params};
use rpg_core::graph::RPGraph;
use std::collections::{HashMap, HashSet};

/// Request parameters for building a context pack.
pub struct ContextPackRequest<'a> {
    pub query: &'a str,
    pub scope: Option<&'a str>,
    pub token_budget: usize,
    pub include_source: bool,
    pub depth: usize,
}

/// A single entity in the packed context.
#[derive(Debug, Clone)]
pub struct PackedEntity {
    pub entity_id: String,
    pub name: String,
    pub file: String,
    pub kind: String,
    pub features: Vec<String>,
    pub source: Option<String>,
    pub deps_summary: String,
    pub relevance: f64,
}

/// The result of a context pack operation.
#[derive(Debug, Clone)]
pub struct ContextPackResult {
    pub primary_entities: Vec<PackedEntity>,
    pub neighborhood_entities: Vec<PackedEntity>,
    pub token_estimate: usize,
}

/// Build a context pack: search → fetch → expand neighbors → budget-trim.
pub fn build_context_pack(
    graph: &RPGraph,
    project_root: &std::path::Path,
    request: &ContextPackRequest,
    embedding_scores: Option<&HashMap<String, f64>>,
) -> ContextPackResult {
    // Step 1: Search for primary candidates
    let results = search_with_params(
        graph,
        &SearchParams {
            query: request.query,
            mode: SearchMode::Auto,
            scope: request.scope,
            limit: 10,
            line_nums: None,
            file_pattern: None,
            entity_type_filter: None,
            embedding_scores,
        },
    );

    let mut primary: Vec<PackedEntity> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for r in &results {
        if seen_ids.contains(&r.entity_id) {
            continue;
        }
        seen_ids.insert(r.entity_id.clone());

        let Some(entity) = graph.get_entity(&r.entity_id) else {
            continue;
        };

        let source = if request.include_source {
            read_source_truncated(project_root, entity, 30)
        } else {
            None
        };

        let deps_summary = format_deps_summary(&entity.deps);

        primary.push(PackedEntity {
            entity_id: r.entity_id.clone(),
            name: entity.name.clone(),
            file: entity.file.display().to_string(),
            kind: format!("{:?}", entity.kind).to_lowercase(),
            features: entity.semantic_features.clone(),
            source,
            deps_summary,
            relevance: r.score,
        });
    }

    // Step 2: Expand neighbors via BFS up to requested depth
    let mut neighborhood: Vec<PackedEntity> = Vec::new();
    if request.depth > 0 {
        use std::collections::VecDeque;
        // BFS frontier: (entity_id, current_depth)
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        for pe in &primary {
            queue.push_back((pe.entity_id.clone(), 0));
        }

        while let Some((current_id, current_depth)) = queue.pop_front() {
            if current_depth >= request.depth {
                continue;
            }
            let neighbors = get_neighbors(graph, &current_id, Direction::Both, None);
            for (nid, _edge_kind, _dir) in neighbors {
                if seen_ids.contains(&nid) {
                    continue;
                }
                seen_ids.insert(nid.clone());

                let Some(entity) = graph.get_entity(&nid) else {
                    continue;
                };

                neighborhood.push(PackedEntity {
                    entity_id: nid.clone(),
                    name: entity.name.clone(),
                    file: entity.file.display().to_string(),
                    kind: format!("{:?}", entity.kind).to_lowercase(),
                    features: entity.semantic_features.clone(),
                    source: None, // No source for neighbors to save tokens
                    deps_summary: String::new(),
                    relevance: 0.0,
                });

                queue.push_back((nid, current_depth + 1));
            }
        }
    }

    // Step 3: Token budgeting — estimate and trim
    let mut token_estimate = estimate_tokens(&primary, &neighborhood);

    // Drop lowest-relevance neighborhood entities first, then primary
    while token_estimate > request.token_budget && !neighborhood.is_empty() {
        neighborhood.pop();
        token_estimate = estimate_tokens(&primary, &neighborhood);
    }
    while token_estimate > request.token_budget && primary.len() > 1 {
        primary.pop();
        token_estimate = estimate_tokens(&primary, &neighborhood);
    }
    // If still over budget with 1 primary entity, truncate its source
    if token_estimate > request.token_budget && primary.len() == 1 {
        primary[0].source = None;
        token_estimate = estimate_tokens(&primary, &neighborhood);
    }

    ContextPackResult {
        primary_entities: primary,
        neighborhood_entities: neighborhood,
        token_estimate,
    }
}

fn read_source_truncated(
    project_root: &std::path::Path,
    entity: &rpg_core::graph::Entity,
    max_lines: usize,
) -> Option<String> {
    let file_path = project_root.join(&entity.file);
    let content = std::fs::read_to_string(&file_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = entity.line_start.saturating_sub(1);
    let end = entity.line_end.min(lines.len());
    if start >= end || start >= lines.len() {
        return None;
    }
    let entity_lines = &lines[start..end];

    if entity_lines.len() > max_lines {
        let mut truncated = entity_lines[..max_lines].join("\n");
        truncated.push_str(&format!(
            "\n// ... ({} more lines)",
            entity_lines.len() - max_lines
        ));
        Some(truncated)
    } else {
        Some(entity_lines.join("\n"))
    }
}

/// Format a compact dependency summary line for an entity.
/// Returns empty string if the entity has no deps.
/// Format: `Called by: a, b | Calls: c, d | Inherits: e`
/// Each dep list is capped at 5 entries.
pub fn format_deps_summary(deps: &rpg_core::graph::EntityDeps) -> String {
    let mut parts = Vec::new();
    if !deps.invoked_by.is_empty() {
        let names: Vec<&str> = deps.invoked_by.iter().take(5).map(|s| s.as_str()).collect();
        parts.push(format!("Called by: {}", names.join(", ")));
    }
    if !deps.invokes.is_empty() {
        let names: Vec<&str> = deps.invokes.iter().take(5).map(|s| s.as_str()).collect();
        parts.push(format!("Calls: {}", names.join(", ")));
    }
    if !deps.inherits.is_empty() {
        let names: Vec<&str> = deps.inherits.iter().take(5).map(|s| s.as_str()).collect();
        parts.push(format!("Inherits: {}", names.join(", ")));
    }
    if !deps.renders.is_empty() {
        let names: Vec<&str> = deps.renders.iter().take(5).map(|s| s.as_str()).collect();
        parts.push(format!("Renders: {}", names.join(", ")));
    }
    parts.join(" | ")
}

fn estimate_tokens(primary: &[PackedEntity], neighborhood: &[PackedEntity]) -> usize {
    let mut chars = 0usize;
    for p in primary {
        chars += p.entity_id.len() + p.name.len() + p.file.len() + p.kind.len();
        chars += p.features.iter().map(|f| f.len()).sum::<usize>();
        chars += p.source.as_ref().map_or(0, |s| s.len());
        chars += p.deps_summary.len();
    }
    for n in neighborhood {
        chars += n.entity_id.len() + n.name.len() + n.file.len();
        chars += n.features.iter().map(|f| f.len()).sum::<usize>();
    }
    // ~4 chars per token + overhead for formatting
    chars / 4 + primary.len() * 10 + neighborhood.len() * 5
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, EdgeKind, Entity, EntityDeps, EntityKind, RPGraph};
    use std::path::PathBuf;

    fn make_entity(id: &str, name: &str, features: Vec<&str>) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: name.to_string(),
            file: PathBuf::from("src/lib.rs"),
            line_start: 1,
            line_end: 3,
            parent_class: None,
            semantic_features: features.into_iter().map(|s| s.to_string()).collect(),
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
        }
    }

    fn make_test_graph() -> RPGraph {
        // A -> B -> C (invokes chain)
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/lib.rs:a".to_string(),
            make_entity("src/lib.rs:a", "a", vec!["do stuff"]),
        );
        graph.entities.insert(
            "src/lib.rs:b".to_string(),
            make_entity("src/lib.rs:b", "b", vec!["help stuff"]),
        );
        graph.entities.insert(
            "src/lib.rs:c".to_string(),
            make_entity("src/lib.rs:c", "c", vec!["finish stuff"]),
        );
        graph.edges = vec![
            DependencyEdge {
                source: "src/lib.rs:a".to_string(),
                target: "src/lib.rs:b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "src/lib.rs:b".to_string(),
                target: "src/lib.rs:c".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();
        graph
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(&[], &[]), 0);
    }

    #[test]
    fn test_estimate_tokens_with_entities() {
        let primary = vec![PackedEntity {
            entity_id: "src/lib.rs:foo".to_string(),
            name: "foo".to_string(),
            file: "src/lib.rs".to_string(),
            kind: "function".to_string(),
            features: vec!["do stuff".to_string()],
            source: Some("fn foo() {}".to_string()),
            deps_summary: String::new(),
            relevance: 1.0,
        }];
        let estimate = estimate_tokens(&primary, &[]);
        assert!(estimate > 0, "should produce non-zero token estimate");
        // source has 11 chars ~= 2-3 tokens, plus overhead
        assert!(estimate < 100, "should be reasonable for one small entity");
    }

    #[test]
    fn test_format_deps_summary_empty() {
        let deps = EntityDeps::default();
        assert!(format_deps_summary(&deps).is_empty());
    }

    #[test]
    fn test_format_deps_summary_populated() {
        let deps = EntityDeps {
            invoked_by: vec!["main".to_string()],
            invokes: vec!["helper".to_string()],
            inherits: vec!["Base".to_string()],
            renders: vec!["Widget".to_string()],
            ..Default::default()
        };
        let summary = format_deps_summary(&deps);
        assert!(summary.contains("Called by: main"));
        assert!(summary.contains("Calls: helper"));
        assert!(summary.contains("Inherits: Base"));
        assert!(summary.contains("Renders: Widget"));
    }

    #[test]
    fn test_format_deps_summary_caps_at_five() {
        let deps = EntityDeps {
            invokes: (1..=10).map(|i| format!("fn{}", i)).collect(),
            ..Default::default()
        };
        let summary = format_deps_summary(&deps);
        assert!(summary.contains("fn5"));
        assert!(!summary.contains("fn6"));
    }

    #[test]
    fn test_build_context_pack_no_results() {
        let graph = RPGraph::new("rust");
        let request = ContextPackRequest {
            query: "nonexistent",
            scope: None,
            token_budget: 4000,
            include_source: false,
            depth: 1,
        };
        let result = build_context_pack(&graph, std::path::Path::new("/tmp"), &request, None);
        assert!(result.primary_entities.is_empty());
        assert!(result.neighborhood_entities.is_empty());
    }

    #[test]
    fn test_build_context_pack_finds_entities_by_feature() {
        let graph = make_test_graph();
        let request = ContextPackRequest {
            query: "do stuff",
            scope: None,
            token_budget: 10000,
            include_source: false,
            depth: 0,
        };
        let result = build_context_pack(&graph, std::path::Path::new("/tmp"), &request, None);
        // Should find at least one entity matching "do stuff"
        assert!(
            !result.primary_entities.is_empty(),
            "should find entities matching query"
        );
    }

    #[test]
    fn test_build_context_pack_depth_expands_neighbors() {
        let graph = make_test_graph();
        // Search for "a" with depth=1, should expand to find neighbors
        let request_d0 = ContextPackRequest {
            query: "do stuff",
            scope: None,
            token_budget: 10000,
            include_source: false,
            depth: 0,
        };
        let result_d0 = build_context_pack(&graph, std::path::Path::new("/tmp"), &request_d0, None);

        let request_d1 = ContextPackRequest {
            query: "do stuff",
            scope: None,
            token_budget: 10000,
            include_source: false,
            depth: 1,
        };
        let result_d1 = build_context_pack(&graph, std::path::Path::new("/tmp"), &request_d1, None);

        let total_d0 = result_d0.primary_entities.len() + result_d0.neighborhood_entities.len();
        let total_d1 = result_d1.primary_entities.len() + result_d1.neighborhood_entities.len();
        assert!(
            total_d1 >= total_d0,
            "depth=1 should find at least as many entities as depth=0"
        );
    }

    #[test]
    fn test_build_context_pack_budget_trimming() {
        let graph = make_test_graph();
        // Tiny budget should trim results
        let request = ContextPackRequest {
            query: "stuff",
            scope: None,
            token_budget: 1,
            include_source: false,
            depth: 1,
        };
        let result = build_context_pack(&graph, std::path::Path::new("/tmp"), &request, None);
        // With budget=1, should keep at most 1 primary
        assert!(
            result.primary_entities.len() <= 1,
            "tiny budget should trim to at most 1 primary"
        );
        assert!(
            result.neighborhood_entities.is_empty(),
            "tiny budget should trim all neighborhood entities"
        );
    }
}
