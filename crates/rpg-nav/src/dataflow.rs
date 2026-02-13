//! Area-level connectivity analysis from entity dependency edges.

use rpg_core::graph::{EdgeKind, RPGraph};
use std::collections::BTreeMap;

/// An aggregated invocation relationship between two L1 hierarchy areas.
#[derive(Debug, Clone)]
pub struct AreaInvocation {
    pub source_area: String,
    pub target_area: String,
    pub edge_count: usize,
    /// Representative callee names (up to 3) for context.
    pub sample_callees: Vec<String>,
}

/// Aggregate Invokes edges by L1 hierarchy area.
///
/// For each Invokes edge, determines the source and target entity's L1 area
/// from their `hierarchy_path`. Skips intra-area edges (same area).
/// Returns results sorted by edge count descending.
pub fn compute_area_invocations(graph: &RPGraph) -> Vec<AreaInvocation> {
    // (source_area, target_area) → (count, sample callees)
    let mut agg: BTreeMap<(String, String), (usize, Vec<String>)> = BTreeMap::new();

    for edge in &graph.edges {
        if edge.kind != EdgeKind::Invokes {
            continue;
        }
        let source_entity = graph.entities.get(&edge.source);
        let target_entity = graph.entities.get(&edge.target);

        let (Some(src), Some(tgt)) = (source_entity, target_entity) else {
            continue;
        };

        let src_area = l1_area(&src.hierarchy_path);
        let tgt_area = l1_area(&tgt.hierarchy_path);

        if src_area.is_empty() || tgt_area.is_empty() || src_area == tgt_area {
            continue;
        }

        let key = (src_area.to_string(), tgt_area.to_string());
        let entry = agg.entry(key).or_insert_with(|| (0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 3 && !entry.1.contains(&tgt.name) {
            entry.1.push(tgt.name.clone());
        }
    }

    let mut result: Vec<AreaInvocation> = agg
        .into_iter()
        .map(|((src, tgt), (count, samples))| AreaInvocation {
            source_area: src,
            target_area: tgt,
            edge_count: count,
            sample_callees: samples,
        })
        .collect();

    // Sort by edge count descending, then deterministic by area names for ties
    result.sort_by(|a, b| {
        b.edge_count
            .cmp(&a.edge_count)
            .then_with(|| a.source_area.cmp(&b.source_area))
            .then_with(|| a.target_area.cmp(&b.target_area))
    });
    result
}

/// Format area invocations as text for LLM consumption.
pub fn format_area_invocations(invocations: &[AreaInvocation]) -> String {
    if invocations.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    lines.push("Area connectivity (inter-area invocations):".to_string());
    for inv in invocations {
        let samples = if inv.sample_callees.is_empty() {
            String::new()
        } else {
            format!(" ({})", inv.sample_callees.join(", "))
        };
        lines.push(format!(
            "  {} → {}: {} invocations{}",
            inv.source_area, inv.target_area, inv.edge_count, samples
        ));
    }
    lines.join("\n")
}

/// Extract the L1 (top-level) area from a hierarchy path like "Auth/tokens/validate".
fn l1_area(path: &str) -> &str {
    if path.is_empty() {
        return "";
    }
    path.split('/').next().unwrap_or("")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, Entity, EntityDeps, EntityKind};
    use std::path::PathBuf;

    fn make_entity(name: &str, hierarchy: &str) -> Entity {
        Entity {
            id: String::new(),
            name: name.to_string(),
            kind: EntityKind::Function,
            file: PathBuf::from("src/lib.rs"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: Vec::new(),
            feature_source: None,
            hierarchy_path: hierarchy.to_string(),
            deps: EntityDeps::default(),
            signature: None,
        }
    }

    #[test]
    fn test_area_invocations_cross_area() {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/auth.rs:validate".to_string(),
            make_entity("validate", "Auth/tokens/validate"),
        );
        graph.entities.insert(
            "src/db.rs:query".to_string(),
            make_entity("query", "Database/queries/select"),
        );
        graph.edges.push(DependencyEdge {
            source: "src/auth.rs:validate".to_string(),
            target: "src/db.rs:query".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = compute_area_invocations(&graph);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_area, "Auth");
        assert_eq!(result[0].target_area, "Database");
        assert_eq!(result[0].edge_count, 1);
        assert_eq!(result[0].sample_callees, vec!["query".to_string()]);
    }

    #[test]
    fn test_area_invocations_skip_intra_area() {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/auth.rs:validate".to_string(),
            make_entity("validate", "Auth/tokens/validate"),
        );
        graph.entities.insert(
            "src/auth.rs:refresh".to_string(),
            make_entity("refresh", "Auth/tokens/refresh"),
        );
        graph.edges.push(DependencyEdge {
            source: "src/auth.rs:validate".to_string(),
            target: "src/auth.rs:refresh".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = compute_area_invocations(&graph);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_area_invocations() {
        let invocations = vec![AreaInvocation {
            source_area: "Auth".to_string(),
            target_area: "Database".to_string(),
            edge_count: 5,
            sample_callees: vec!["query".to_string(), "insert".to_string()],
        }];
        let output = format_area_invocations(&invocations);
        assert!(output.contains("Auth → Database: 5 invocations"));
        assert!(output.contains("query, insert"));
    }

    #[test]
    fn test_format_empty() {
        assert_eq!(format_area_invocations(&[]), "");
    }

    #[test]
    fn test_sample_callees_dedup() {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/auth.rs:validate".to_string(),
            make_entity("validate", "Auth/tokens/validate"),
        );
        graph.entities.insert(
            "src/auth.rs:refresh".to_string(),
            make_entity("refresh", "Auth/tokens/refresh"),
        );
        graph.entities.insert(
            "src/db.rs:query".to_string(),
            make_entity("query", "Database/queries/select"),
        );
        // Two different Auth entities invoke the same DB entity
        graph.edges.push(DependencyEdge {
            source: "src/auth.rs:validate".to_string(),
            target: "src/db.rs:query".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.edges.push(DependencyEdge {
            source: "src/auth.rs:refresh".to_string(),
            target: "src/db.rs:query".to_string(),
            kind: EdgeKind::Invokes,
        });

        let result = compute_area_invocations(&graph);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].edge_count, 2);
        // "query" should appear only once in samples, not duplicated
        assert_eq!(result[0].sample_callees.len(), 1);
        assert_eq!(result[0].sample_callees[0], "query");
    }
}
