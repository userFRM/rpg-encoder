//! Semantic Snapshot: whole-repo understanding compressed for context injection.
//!
//! Produces a token-efficient summary of the entire RPG — hierarchy, entities,
//! features, and dependency skeleton — designed to fit in an LLM context window
//! at session start (~25-30K tokens for a 1000-entity codebase).

use rpg_core::graph::{HierarchyNode, RPGraph};
use std::collections::BTreeMap;

/// Request parameters for building a semantic snapshot.
pub struct SnapshotRequest {
    /// Target token budget (default: 30_000).
    pub token_budget: usize,
    /// Include dependency skeleton (default: true).
    pub include_deps: bool,
    /// Include per-entity features (default: true).
    pub include_features: bool,
    /// Max features to include per entity (default: 3).
    pub max_features_per_entity: usize,
    /// Max deps to include per entity per direction (default: 5).
    pub max_deps_per_entity: usize,
}

impl Default for SnapshotRequest {
    fn default() -> Self {
        Self {
            token_budget: 30_000,
            include_deps: true,
            include_features: true,
            max_features_per_entity: 3,
            max_deps_per_entity: 5,
        }
    }
}

/// Top-level snapshot result.
pub struct SnapshotResult {
    pub stats: SnapshotStats,
    pub hierarchy_tree: Vec<SnapshotArea>,
    pub entity_groups: Vec<AreaEntityGroup>,
    pub dep_skeleton: Vec<DepEntry>,
    /// Top entities by connectivity — the architectural backbone.
    pub hot_spots: Vec<HotSpot>,
    pub token_estimate: usize,
}

/// Graph-level statistics.
pub struct SnapshotStats {
    pub total_entities: usize,
    pub lifted_entities: usize,
    pub total_files: usize,
    pub total_edges: usize,
    pub languages: String,
    pub hierarchy_type: String,
    pub coverage_pct: f64,
}

/// A top-level hierarchy area with aggregated features.
pub struct SnapshotArea {
    pub name: String,
    pub entity_count: usize,
    pub aggregate_features: Vec<String>,
    pub categories: Vec<SnapshotCategory>,
}

/// A category within an area.
pub struct SnapshotCategory {
    pub name: String,
    pub entity_count: usize,
    pub subcategories: Vec<SnapshotSubcategory>,
}

/// A subcategory leaf.
pub struct SnapshotSubcategory {
    pub name: String,
    pub entity_count: usize,
}

/// Entities grouped by hierarchy area path.
pub struct AreaEntityGroup {
    pub area_path: String,
    pub entities: Vec<SnapshotEntity>,
}

/// A single entity in the snapshot.
pub struct SnapshotEntity {
    pub name: String,
    pub kind: String,
    pub features: Vec<String>,
    pub file: String,
    pub lifted: bool,
}

/// A high-connectivity entity — architectural backbone.
pub struct HotSpot {
    pub name: String,
    pub file: String,
    pub kind: String,
    pub total_connections: usize,
    pub features: Vec<String>,
}

/// Condensed dependency entry for one entity.
pub struct DepEntry {
    pub name: String,
    pub calls: Vec<String>,
    pub called_by: Vec<String>,
    pub inherits: Vec<String>,
}

/// Build a complete semantic snapshot of the repository.
pub fn build_semantic_snapshot(graph: &RPGraph, request: &SnapshotRequest) -> SnapshotResult {
    let (lifted, total) = graph.lifting_coverage();
    let coverage_pct = if total > 0 {
        lifted as f64 / total as f64 * 100.0
    } else {
        0.0
    };

    let stats = SnapshotStats {
        total_entities: graph.metadata.total_entities,
        lifted_entities: lifted,
        total_files: graph.metadata.total_files,
        total_edges: graph.metadata.total_edges,
        languages: if graph.metadata.languages.is_empty() {
            graph.metadata.language.clone()
        } else {
            graph.metadata.languages.join(", ")
        },
        hierarchy_type: if graph.metadata.semantic_hierarchy {
            format!("semantic ({} areas)", graph.metadata.functional_areas)
        } else {
            "structural".into()
        },
        coverage_pct,
    };

    // Build hierarchy tree
    let hierarchy_tree = build_hierarchy_tree(&graph.hierarchy);

    // Group entities by hierarchy path
    let mut groups: BTreeMap<String, Vec<SnapshotEntity>> = BTreeMap::new();
    for entity in graph.entities.values() {
        let path = if entity.hierarchy_path.is_empty() {
            "Unplaced".to_string()
        } else {
            entity.hierarchy_path.clone()
        };
        let features = if request.include_features {
            entity
                .semantic_features
                .iter()
                .take(request.max_features_per_entity)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        groups.entry(path).or_default().push(SnapshotEntity {
            name: entity.name.clone(),
            kind: format!("{:?}", entity.kind).to_lowercase(),
            features,
            file: entity.file.display().to_string(),
            lifted: !entity.semantic_features.is_empty(),
        });
    }

    let entity_groups: Vec<AreaEntityGroup> = groups
        .into_iter()
        .map(|(path, mut entities)| {
            entities.sort_by(|a, b| a.file.cmp(&b.file).then(a.name.cmp(&b.name)));
            AreaEntityGroup {
                area_path: path,
                entities,
            }
        })
        .collect();

    // Build dependency skeleton
    let dep_skeleton = if request.include_deps {
        build_dep_skeleton(graph, request.max_deps_per_entity)
    } else {
        Vec::new()
    };

    // Compute hot spots — top 10 entities by total connectivity
    let hot_spots = build_hot_spots(graph, 10);

    // Estimate tokens
    let mut result = SnapshotResult {
        stats,
        hierarchy_tree,
        entity_groups,
        dep_skeleton,
        hot_spots,
        token_estimate: 0,
    };
    result.token_estimate = estimate_tokens(&result);

    // Progressive trimming if over budget
    if result.token_estimate > request.token_budget {
        trim_to_budget(&mut result, request.token_budget);
    }

    result
}

fn build_hierarchy_tree(hierarchy: &BTreeMap<String, HierarchyNode>) -> Vec<SnapshotArea> {
    hierarchy
        .values()
        .map(|area| {
            let categories = area
                .children
                .values()
                .map(|cat| {
                    let subcategories = cat
                        .children
                        .values()
                        .map(|sub| SnapshotSubcategory {
                            name: sub.name.clone(),
                            entity_count: sub.entity_count(),
                        })
                        .collect();
                    SnapshotCategory {
                        name: cat.name.clone(),
                        entity_count: cat.entity_count(),
                        subcategories,
                    }
                })
                .collect();
            SnapshotArea {
                name: area.name.clone(),
                entity_count: area.entity_count(),
                aggregate_features: area.semantic_features.iter().take(8).cloned().collect(),
                categories,
            }
        })
        .collect()
}

/// Produce a short qualified name: "Class::method" if parent exists, else bare name.
fn qualified_name(e: &rpg_core::graph::Entity) -> String {
    match &e.parent_class {
        Some(cls) => format!("{}::{}", cls, e.name),
        None => e.name.clone(),
    }
}

fn build_dep_skeleton(graph: &RPGraph, max_per_dir: usize) -> Vec<DepEntry> {
    let mut entries = Vec::new();

    for entity in graph.entities.values() {
        let calls: Vec<String> = entity
            .deps
            .invokes
            .iter()
            .take(max_per_dir)
            .filter_map(|id| graph.entities.get(id.as_str()).map(qualified_name))
            .collect();
        let called_by: Vec<String> = entity
            .deps
            .invoked_by
            .iter()
            .take(max_per_dir)
            .filter_map(|id| graph.entities.get(id.as_str()).map(qualified_name))
            .collect();
        let inherits: Vec<String> = entity
            .deps
            .inherits
            .iter()
            .take(max_per_dir)
            .filter_map(|id| graph.entities.get(id.as_str()).map(qualified_name))
            .collect();

        if calls.is_empty() && called_by.is_empty() && inherits.is_empty() {
            continue;
        }
        entries.push(DepEntry {
            name: qualified_name(entity),
            calls,
            called_by,
            inherits,
        });
    }
    entries
}

fn build_hot_spots(graph: &RPGraph, top_n: usize) -> Vec<HotSpot> {
    let mut scored: Vec<(&str, usize)> = graph
        .entities
        .iter()
        .map(|(id, e)| {
            let connections = e.deps.invokes.len()
                + e.deps.invoked_by.len()
                + e.deps.imports.len()
                + e.deps.imported_by.len()
                + e.deps.inherits.len()
                + e.deps.inherited_by.len();
            (id.as_str(), connections)
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));

    scored
        .into_iter()
        .take(top_n)
        .filter(|(_, c)| *c > 0)
        .filter_map(|(id, connections)| {
            graph.entities.get(id).map(|e| HotSpot {
                name: e.name.clone(),
                file: e.file.display().to_string(),
                kind: format!("{:?}", e.kind).to_lowercase(),
                total_connections: connections,
                features: e.semantic_features.iter().take(2).cloned().collect(),
            })
        })
        .collect()
}

fn estimate_tokens(result: &SnapshotResult) -> usize {
    let mut chars = 0usize;

    // Stats header
    chars += 200;

    // Hierarchy tree
    for area in &result.hierarchy_tree {
        chars += area.name.len() + 30;
        for feat in &area.aggregate_features {
            chars += feat.len() + 4;
        }
        for cat in &area.categories {
            chars += cat.name.len() + 20;
            for sub in &cat.subcategories {
                chars += sub.name.len() + 15;
            }
        }
    }

    // Entity groups
    for group in &result.entity_groups {
        chars += group.area_path.len() + 10;
        for e in &group.entities {
            chars += e.name.len() + e.kind.len() + e.file.len() + 20;
            for f in &e.features {
                chars += f.len() + 4;
            }
        }
    }

    // Dep skeleton
    for d in &result.dep_skeleton {
        chars += d.name.len() + 10;
        for c in &d.calls {
            chars += c.len() + 4;
        }
        for c in &d.called_by {
            chars += c.len() + 4;
        }
        for i in &d.inherits {
            chars += i.len() + 4;
        }
    }

    // Hot spots
    for h in &result.hot_spots {
        chars += h.name.len() + h.file.len() + h.kind.len() + 30;
        for f in &h.features {
            chars += f.len() + 4;
        }
    }

    // ~4 chars per token + TOON formatting overhead
    chars / 4 + 500
}

/// Progressively trim the snapshot to fit within the token budget.
fn trim_to_budget(result: &mut SnapshotResult, budget: usize) {
    // Step 1: Reduce features per entity to 2, then 1
    for max_feat in [2, 1] {
        for group in &mut result.entity_groups {
            for e in &mut group.entities {
                e.features.truncate(max_feat);
            }
        }
        result.token_estimate = estimate_tokens(result);
        if result.token_estimate <= budget {
            return;
        }
    }

    // Step 2: Remove all per-entity features
    for group in &mut result.entity_groups {
        for e in &mut group.entities {
            e.features.clear();
        }
    }
    result.token_estimate = estimate_tokens(result);
    if result.token_estimate <= budget {
        return;
    }

    // Step 3: Drop dependency skeleton and hot spots
    result.dep_skeleton.clear();
    result.hot_spots.clear();
    result.token_estimate = estimate_tokens(result);
    // Note: if still over budget after all trimming, the token_estimate field
    // will exceed the budget. Callers can check this to detect the condition.
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{Entity, EntityDeps, EntityKind, GraphMetadata, HierarchyNode, RPGraph};
    use std::path::PathBuf;

    fn make_test_graph() -> RPGraph {
        let mut graph = RPGraph::new("rust");
        graph.metadata = GraphMetadata {
            language: "rust".into(),
            languages: vec!["rust".into()],
            total_files: 3,
            total_entities: 4,
            functional_areas: 2,
            total_edges: 2,
            dependency_edges: 2,
            containment_edges: 0,
            lifted_entities: 3,
            data_flow_edges: 0,
            semantic_hierarchy: true,
            repo_summary: None,
            paradigms: Vec::new(),
        };

        let entities = vec![
            Entity {
                id: "src/auth.rs:validate_token".into(),
                kind: EntityKind::Function,
                name: "validate_token".into(),
                file: PathBuf::from("src/auth.rs"),
                line_start: 10,
                line_end: 30,
                parent_class: None,
                semantic_features: vec![
                    "validate JWT tokens".into(),
                    "reject expired sessions".into(),
                ],
                feature_source: Some("llm".into()),
                hierarchy_path: "Security/auth/validate".into(),
                deps: EntityDeps {
                    invoked_by: vec!["src/api.rs:handle_login".into()],
                    ..Default::default()
                },
                signature: None,
            },
            Entity {
                id: "src/api.rs:handle_login".into(),
                kind: EntityKind::Function,
                name: "handle_login".into(),
                file: PathBuf::from("src/api.rs"),
                line_start: 5,
                line_end: 25,
                parent_class: None,
                semantic_features: vec![
                    "authenticate user credentials".into(),
                    "create session token".into(),
                    "return auth response".into(),
                ],
                feature_source: Some("llm".into()),
                hierarchy_path: "API/endpoints/auth".into(),
                deps: EntityDeps {
                    invokes: vec!["src/auth.rs:validate_token".into()],
                    ..Default::default()
                },
                signature: None,
            },
            Entity {
                id: "src/db.rs:query".into(),
                kind: EntityKind::Function,
                name: "query".into(),
                file: PathBuf::from("src/db.rs"),
                line_start: 1,
                line_end: 20,
                parent_class: None,
                semantic_features: vec!["execute database queries".into()],
                feature_source: Some("llm".into()),
                hierarchy_path: "Data/storage/query".into(),
                deps: EntityDeps::default(),
                signature: None,
            },
            Entity {
                id: "src/new.rs:unlifted".into(),
                kind: EntityKind::Function,
                name: "unlifted".into(),
                file: PathBuf::from("src/new.rs"),
                line_start: 1,
                line_end: 10,
                parent_class: None,
                semantic_features: Vec::new(),
                feature_source: None,
                hierarchy_path: "Unplaced".into(),
                deps: EntityDeps::default(),
                signature: None,
            },
        ];

        for e in entities {
            graph.entities.insert(e.id.clone(), e);
        }

        // Build hierarchy
        let mut security = HierarchyNode::new("Security");
        let mut auth_cat = HierarchyNode::new("auth");
        let mut validate_sub = HierarchyNode::new("validate");
        validate_sub
            .entities
            .push("src/auth.rs:validate_token".into());
        validate_sub.semantic_features = vec!["validate JWT tokens".into()];
        auth_cat.children.insert("validate".into(), validate_sub);
        security.children.insert("auth".into(), auth_cat);
        security.semantic_features = vec!["handle authentication and authorization".into()];

        let mut api = HierarchyNode::new("API");
        let mut endpoints_cat = HierarchyNode::new("endpoints");
        let mut auth_sub = HierarchyNode::new("auth");
        auth_sub.entities.push("src/api.rs:handle_login".into());
        endpoints_cat.children.insert("auth".into(), auth_sub);
        api.children.insert("endpoints".into(), endpoints_cat);
        api.semantic_features = vec!["serve HTTP API endpoints".into()];

        graph.hierarchy.insert("Security".into(), security);
        graph.hierarchy.insert("API".into(), api);

        graph
    }

    #[test]
    fn test_snapshot_produces_all_sections() {
        let graph = make_test_graph();
        let result = build_semantic_snapshot(&graph, &SnapshotRequest::default());

        assert_eq!(result.stats.total_entities, 4);
        assert_eq!(result.stats.lifted_entities, 3);
        assert_eq!(result.hierarchy_tree.len(), 2);
        assert!(!result.entity_groups.is_empty());
        assert!(result.token_estimate > 0);
    }

    #[test]
    fn test_snapshot_groups_by_hierarchy() {
        let graph = make_test_graph();
        let result = build_semantic_snapshot(&graph, &SnapshotRequest::default());

        let paths: Vec<&str> = result
            .entity_groups
            .iter()
            .map(|g| g.area_path.as_str())
            .collect();
        assert!(paths.contains(&"Security/auth/validate"));
        assert!(paths.contains(&"API/endpoints/auth"));
    }

    #[test]
    fn test_snapshot_respects_feature_limit() {
        let graph = make_test_graph();
        let result = build_semantic_snapshot(
            &graph,
            &SnapshotRequest {
                max_features_per_entity: 1,
                ..Default::default()
            },
        );

        for group in &result.entity_groups {
            for e in &group.entities {
                assert!(e.features.len() <= 1, "features should be capped at 1");
            }
        }
    }

    #[test]
    fn test_snapshot_dep_skeleton() {
        let graph = make_test_graph();
        let result = build_semantic_snapshot(&graph, &SnapshotRequest::default());

        // handle_login calls validate_token
        let login_dep = result
            .dep_skeleton
            .iter()
            .find(|d| d.name == "handle_login");
        assert!(login_dep.is_some(), "handle_login should have dep entry");
        assert!(login_dep.unwrap().calls.contains(&"validate_token".into()));

        // validate_token is called by handle_login
        let token_dep = result
            .dep_skeleton
            .iter()
            .find(|d| d.name == "validate_token");
        assert!(token_dep.is_some());
        assert!(
            token_dep
                .unwrap()
                .called_by
                .contains(&"handle_login".into())
        );
    }

    #[test]
    fn test_snapshot_trimming() {
        let graph = make_test_graph();
        let result = build_semantic_snapshot(
            &graph,
            &SnapshotRequest {
                token_budget: 1, // impossibly small budget
                ..Default::default()
            },
        );

        // Should have trimmed features and deps
        for group in &result.entity_groups {
            for e in &group.entities {
                assert!(e.features.is_empty(), "features should be trimmed");
            }
        }
        assert!(result.dep_skeleton.is_empty(), "deps should be trimmed");
    }

    #[test]
    fn test_snapshot_coverage_stats() {
        let graph = make_test_graph();
        let result = build_semantic_snapshot(&graph, &SnapshotRequest::default());

        assert!((result.stats.coverage_pct - 75.0).abs() < 0.1); // 3/4 lifted
    }
}
