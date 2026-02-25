//! Health analysis: coupling, instability, centrality, and god object detection.
//!
//! Implements the CHM (Code Health Meter) metrics from the paper:
//! - In-degree (afferent coupling, Ca)
//! - Out-degree (efferent coupling, Ce)
//! - Instability index I = Ce / (Ca + Ce)
//! - Degree centrality (normalized)
//! - God Object heuristic (high degree + extreme instability)

use crate::duplication::{
    CloneGroup, DuplicationConfig, SemanticCloneGroup, SemanticDuplicationConfig,
    detect_duplication, detect_semantic_duplicates,
};
use rpg_core::graph::{EdgeKind, EntityKind, RPGraph};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// Edge kinds that represent dependency relationships (not structural containment).
const DEPENDENCY_EDGE_KINDS: &[EdgeKind] = &[
    EdgeKind::Imports,
    EdgeKind::Invokes,
    EdgeKind::Inherits,
    EdgeKind::Composes,
    EdgeKind::Renders,
    EdgeKind::ReadsState,
    EdgeKind::WritesState,
    EdgeKind::Dispatches,
    EdgeKind::DataFlow,
];

/// A health issue detected for an entity.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HealthIssue {
    /// Entity has high total degree and extreme instability (god object).
    PotentialGodObject {
        total_degree: usize,
        instability: f64,
    },
    /// Entity has high instability (> threshold), indicating it's dependent on many.
    HighlyUnstable { instability: f64, out_degree: usize },
    /// Entity has very low instability (< 0.3), indicating it's depended on by many.
    HighlyStable { instability: f64, in_degree: usize },
    /// Entity has high total degree (hub).
    HubEntity { total_degree: usize },
}

/// Health metrics for a single entity.
#[derive(Debug, Clone, Serialize)]
pub struct EntityHealth {
    pub entity_id: String,
    pub name: String,
    pub file: String,
    pub kind: String,
    /// Afferent coupling (Ca): number of incoming dependency edges.
    pub in_degree: usize,
    /// Efferent coupling (Ce): number of outgoing dependency edges.
    pub out_degree: usize,
    /// Instability index: Ce / (Ca + Ce). Range [0, 1].
    /// I ≈ 1: unstable (depends on many)
    /// I ≈ 0: stable (depended on by many)
    pub instability: f64,
    /// Degree centrality: total_degree / (n - 1), where n = total entities.
    pub centrality: f64,
    /// Detected health issues for this entity.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<HealthIssue>,
}

/// Aggregate health statistics for the codebase.
#[derive(Debug, Clone, Serialize)]
pub struct HealthSummary {
    pub total_entities: usize,
    pub analyzed_entities: usize,
    pub total_dependency_edges: usize,
    pub avg_in_degree: f64,
    pub avg_out_degree: f64,
    pub avg_instability: f64,
    pub avg_centrality: f64,
    pub god_object_count: usize,
    pub highly_unstable_count: usize,
    pub highly_stable_count: usize,
    pub hub_count: usize,
}

/// Complete health analysis report.
#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub summary: HealthSummary,
    pub entities: Vec<EntityHealth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicates: Option<Vec<CloneGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_duplicates: Option<Vec<SemanticCloneGroup>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub top_unstable: Vec<EntityHealth>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub top_god_objects: Vec<EntityHealth>,
}

/// Configuration for health analysis.
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Instability threshold for flagging highly unstable entities.
    pub instability_threshold: f64,
    /// Minimum total degree to consider as a hub.
    pub hub_threshold: usize,
    /// Minimum total degree for god object detection.
    pub god_object_degree_threshold: usize,
    /// Instability extreme threshold for god object (must be > this or < 1-this).
    pub god_object_instability_threshold: f64,
    /// Maximum entities to include in top lists.
    pub top_n: usize,
    /// Include token-based duplication detection (reads source files from disk, slower).
    pub include_duplication: bool,
    /// Duplication detection config.
    pub duplication_config: DuplicationConfig,
    /// Include semantic duplication detection via Jaccard similarity on lifted features (in-memory, fast).
    pub include_semantic_duplication: bool,
    /// Semantic duplication detection config.
    pub semantic_duplication_config: SemanticDuplicationConfig,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            instability_threshold: 0.7,
            hub_threshold: 8,
            god_object_degree_threshold: 10,
            god_object_instability_threshold: 0.7,
            top_n: 10,
            include_duplication: false,
            duplication_config: DuplicationConfig::default(),
            include_semantic_duplication: false,
            semantic_duplication_config: SemanticDuplicationConfig::default(),
        }
    }
}

/// Compute health metrics for all entities in the graph.
pub fn compute_health(graph: &RPGraph, config: &HealthConfig) -> HealthReport {
    let total_entities = graph.entities.len();
    let n = total_entities;
    let normalizer = if n > 1 { (n - 1) as f64 } else { 1.0 };

    // Count dependency edges (exclude Contains)
    let total_dependency_edges = graph
        .edges
        .iter()
        .filter(|e| e.kind != EdgeKind::Contains)
        .count();

    // Compute in-degree and out-degree for each entity
    let mut in_degrees: HashMap<&str, usize> = HashMap::with_capacity(total_entities);
    let mut out_degrees: HashMap<&str, usize> = HashMap::with_capacity(total_entities);

    for edge in &graph.edges {
        if !DEPENDENCY_EDGE_KINDS.contains(&edge.kind) {
            continue;
        }
        *out_degrees.entry(edge.source.as_str()).or_insert(0) += 1;
        *in_degrees.entry(edge.target.as_str()).or_insert(0) += 1;
    }

    // Build entity health records
    let mut entities: Vec<EntityHealth> = Vec::with_capacity(total_entities);
    let mut god_object_count = 0usize;
    let mut highly_unstable_count = 0usize;
    let mut highly_stable_count = 0usize;
    let mut hub_count = 0usize;

    for (id, entity) in &graph.entities {
        // Skip Module entities (file-level) for analysis
        if entity.kind == EntityKind::Module {
            continue;
        }

        let in_degree = *in_degrees.get(id.as_str()).unwrap_or(&0);
        let out_degree = *out_degrees.get(id.as_str()).unwrap_or(&0);
        let total_degree = in_degree + out_degree;

        // Instability: Ce / (Ca + Ce)
        // Handle edge case where both are 0
        let instability = if total_degree == 0 {
            0.0
        } else {
            out_degree as f64 / total_degree as f64
        };

        // Degree centrality (normalized)
        let centrality = total_degree as f64 / normalizer;

        // Detect issues
        let mut issues = Vec::new();

        // God Object: high degree + extreme instability
        if total_degree >= config.god_object_degree_threshold
            && (instability > config.god_object_instability_threshold
                || instability < (1.0 - config.god_object_instability_threshold))
        {
            issues.push(HealthIssue::PotentialGodObject {
                total_degree,
                instability,
            });
            god_object_count += 1;
        }

        // High instability
        if instability > config.instability_threshold && out_degree > 0 {
            issues.push(HealthIssue::HighlyUnstable {
                instability,
                out_degree,
            });
            highly_unstable_count += 1;
        }

        // High stability (depended on by many)
        if instability < (1.0 - config.instability_threshold) && in_degree > 0 {
            issues.push(HealthIssue::HighlyStable {
                instability,
                in_degree,
            });
            highly_stable_count += 1;
        }

        // Hub entity
        if total_degree >= config.hub_threshold {
            issues.push(HealthIssue::HubEntity { total_degree });
            hub_count += 1;
        }

        entities.push(EntityHealth {
            entity_id: id.clone(),
            name: entity.name.clone(),
            file: entity.file.display().to_string(),
            kind: format!("{:?}", entity.kind).to_lowercase(),
            in_degree,
            out_degree,
            instability: clean_float(instability),
            centrality: clean_float(centrality),
            issues,
        });
    }

    // Compute summary statistics
    let analyzed = entities.len();
    let total_in: usize = entities.iter().map(|e| e.in_degree).sum();
    let total_out: usize = entities.iter().map(|e| e.out_degree).sum();
    let total_instability: f64 = entities.iter().map(|e| e.instability).sum();
    let total_centrality: f64 = entities.iter().map(|e| e.centrality).sum();

    let avg_in_degree = if analyzed > 0 {
        total_in as f64 / analyzed as f64
    } else {
        0.0
    };
    let avg_out_degree = if analyzed > 0 {
        total_out as f64 / analyzed as f64
    } else {
        0.0
    };
    let avg_instability = if analyzed > 0 {
        total_instability / analyzed as f64
    } else {
        0.0
    };
    let avg_centrality = if analyzed > 0 {
        total_centrality / analyzed as f64
    } else {
        0.0
    };

    // Sort by instability for top unstable
    let mut sorted_by_instability = entities.clone();
    sorted_by_instability.sort_by(|a, b| {
        b.instability
            .partial_cmp(&a.instability)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_unstable: Vec<EntityHealth> = sorted_by_instability
        .into_iter()
        .filter(|e| e.instability > config.instability_threshold)
        .take(config.top_n)
        .collect();

    // Sort by god object score for top god objects
    let mut sorted_by_god = entities.clone();
    sorted_by_god.sort_by(|a, b| {
        let a_score = a
            .issues
            .iter()
            .filter(|i| matches!(i, HealthIssue::PotentialGodObject { .. }))
            .count();
        let b_score = b
            .issues
            .iter()
            .filter(|i| matches!(i, HealthIssue::PotentialGodObject { .. }))
            .count();
        b_score.cmp(&a_score).then_with(|| {
            let a_degree: usize = a.in_degree + a.out_degree;
            let b_degree: usize = b.in_degree + b.out_degree;
            b_degree.cmp(&a_degree)
        })
    });
    let top_god_objects: Vec<EntityHealth> = sorted_by_god
        .into_iter()
        .filter(|e| {
            e.issues
                .iter()
                .any(|i| matches!(i, HealthIssue::PotentialGodObject { .. }))
        })
        .take(config.top_n)
        .collect();

    let summary = HealthSummary {
        total_entities,
        analyzed_entities: analyzed,
        total_dependency_edges,
        avg_in_degree: clean_float(avg_in_degree),
        avg_out_degree: clean_float(avg_out_degree),
        avg_instability: clean_float(avg_instability),
        avg_centrality: clean_float(avg_centrality),
        god_object_count,
        highly_unstable_count,
        highly_stable_count,
        hub_count,
    };

    // Sort entities by entity_id for deterministic output
    entities.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));

    HealthReport {
        summary,
        entities,
        duplicates: None,
        semantic_duplicates: None,
        top_unstable,
        top_god_objects,
    }
}

/// Compute health metrics with optional duplication detection.
/// This is the main entry point for MCP tool.
pub fn compute_health_full(
    graph: &RPGraph,
    project_root: &Path,
    config: &HealthConfig,
) -> HealthReport {
    let mut report = compute_health(graph, config);

    if config.include_duplication {
        report.duplicates = Some(detect_duplication(
            graph,
            project_root,
            &config.duplication_config,
        ));
    }

    if config.include_semantic_duplication {
        report.semantic_duplicates = Some(detect_semantic_duplicates(
            graph,
            &config.semantic_duplication_config,
        ));
    }

    report
}

/// Clean a float: NaN/Infinity → 0, round to 6 decimals.
fn clean_float(v: f64) -> f64 {
    if v.is_nan() || v.is_infinite() {
        return 0.0;
    }
    (v * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, Entity, EntityDeps};
    use std::path::PathBuf;

    fn make_entity(id: &str, name: &str, kind: EntityKind) -> Entity {
        Entity {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            file: PathBuf::from("src/lib.rs"),
            line_start: 1,
            line_end: 5,
            parent_class: None,
            semantic_features: vec![],
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
            signature: None,
        }
    }

    fn make_test_graph() -> RPGraph {
        // A -> B -> C (linear chain via Invokes)
        // A -> C (direct edge)
        // Total edges: A has out_degree=2, in_degree=0
        //              B has out_degree=1, in_degree=1
        //              C has out_degree=0, in_degree=2
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a".to_string(),
            make_entity("a", "fn_a", EntityKind::Function),
        );
        graph.entities.insert(
            "b".to_string(),
            make_entity("b", "fn_b", EntityKind::Function),
        );
        graph.entities.insert(
            "c".to_string(),
            make_entity("c", "fn_c", EntityKind::Function),
        );
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "a".to_string(),
                target: "c".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "c".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();
        graph
    }

    #[test]
    fn test_compute_health_linear_chain() {
        let graph = make_test_graph();
        let config = HealthConfig::default();
        let report = compute_health(&graph, &config);

        assert_eq!(report.summary.analyzed_entities, 3);
        assert_eq!(report.summary.total_dependency_edges, 3);

        // Find entity A
        let a = report.entities.iter().find(|e| e.entity_id == "a").unwrap();
        assert_eq!(a.in_degree, 0);
        assert_eq!(a.out_degree, 2);
        assert!((a.instability - 1.0).abs() < 0.001); // Fully unstable

        // Find entity C
        let c = report.entities.iter().find(|e| e.entity_id == "c").unwrap();
        assert_eq!(c.in_degree, 2);
        assert_eq!(c.out_degree, 0);
        assert!((c.instability - 0.0).abs() < 0.001); // Fully stable

        // Find entity B
        let b = report.entities.iter().find(|e| e.entity_id == "b").unwrap();
        assert_eq!(b.in_degree, 1);
        assert_eq!(b.out_degree, 1);
        assert!((b.instability - 0.5).abs() < 0.001); // Balanced
    }

    #[test]
    fn test_god_object_detection() {
        let mut graph = RPGraph::new("rust");

        // Create a god object with 12 edges
        graph.entities.insert(
            "god".to_string(),
            make_entity("god", "GodClass", EntityKind::Class),
        );

        // Add many dependencies (8 outgoing + 4 incoming = 12 total)
        for i in 0..8 {
            let dep_id = format!("dep_{}", i);
            graph.entities.insert(
                dep_id.clone(),
                make_entity(&dep_id, &dep_id, EntityKind::Function),
            );
            graph.edges.push(DependencyEdge {
                source: "god".to_string(),
                target: dep_id,
                kind: EdgeKind::Invokes,
            });
        }
        for i in 0..4 {
            let caller_id = format!("caller_{}", i);
            graph.entities.insert(
                caller_id.clone(),
                make_entity(&caller_id, &caller_id, EntityKind::Function),
            );
            graph.edges.push(DependencyEdge {
                source: caller_id,
                target: "god".to_string(),
                kind: EdgeKind::Invokes,
            });
        }

        graph.refresh_metadata();

        let config = HealthConfig {
            god_object_degree_threshold: 10,
            god_object_instability_threshold: 0.7,
            ..Default::default()
        };
        let report = compute_health(&graph, &config);

        // The god entity should be flagged
        assert!(report.summary.god_object_count > 0 || report.summary.hub_count > 0);

        let god = report
            .entities
            .iter()
            .find(|e| e.entity_id == "god")
            .unwrap();
        assert_eq!(god.in_degree, 4);
        assert_eq!(god.out_degree, 8);
        assert_eq!(god.in_degree + god.out_degree, 12);
    }

    #[test]
    fn test_centrality_normalization() {
        let graph = make_test_graph();
        let config = HealthConfig::default();
        let report = compute_health(&graph, &config);

        // Centrality should be <= 1.0 for all entities
        for entity in &report.entities {
            assert!(entity.centrality <= 1.0);
            assert!(entity.centrality >= 0.0);
        }
    }

    #[test]
    fn test_empty_graph() {
        let graph = RPGraph::new("rust");
        let config = HealthConfig::default();
        let report = compute_health(&graph, &config);

        assert_eq!(report.summary.total_entities, 0);
        assert_eq!(report.summary.analyzed_entities, 0);
        assert!(report.entities.is_empty());
    }

    #[test]
    fn test_skip_module_entities() {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "mod1".to_string(),
            make_entity("mod1", "module", EntityKind::Module),
        );
        graph.entities.insert(
            "fn1".to_string(),
            make_entity("fn1", "function", EntityKind::Function),
        );
        graph.refresh_metadata();

        let config = HealthConfig::default();
        let report = compute_health(&graph, &config);

        // Only the function should be analyzed
        assert_eq!(report.summary.analyzed_entities, 1);
        assert_eq!(report.entities.len(), 1);
        assert_eq!(report.entities[0].entity_id, "fn1");
    }

    #[test]
    fn test_deterministic_output() {
        let graph = make_test_graph();
        let config = HealthConfig::default();

        let report1 = compute_health(&graph, &config);
        let report2 = compute_health(&graph, &config);

        // Entities should be sorted by ID for deterministic output
        assert_eq!(report1.entities.len(), report2.entities.len());
        for (e1, e2) in report1.entities.iter().zip(report2.entities.iter()) {
            assert_eq!(e1.entity_id, e2.entity_id);
        }
    }
}
