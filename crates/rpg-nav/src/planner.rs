//! Change planning: find relevant entities, compute modification order, assess impact.
//!
//! Orchestrates search + impact_radius + dependency ordering to answer
//! "what existing code needs to change for goal X, and in what order?"

use crate::explore::{Direction, get_neighbors};
use crate::impact::compute_impact_radius;
use crate::search::{SearchMode, SearchParams, search_with_params};
use rpg_core::graph::RPGraph;
use std::collections::{HashMap, HashSet};

/// Request parameters for change planning.
pub struct PlanChangeRequest<'a> {
    pub goal: &'a str,
    pub scope: Option<&'a str>,
    pub max_entities: usize,
}

/// A relevant entity identified for the change plan.
#[derive(Debug, Clone)]
pub struct RelevantEntity {
    pub entity_id: String,
    pub name: String,
    pub file: String,
    pub features: Vec<String>,
    pub relevance: f64,
}

/// Impact summary for a target entity.
#[derive(Debug, Clone)]
pub struct ImpactSummary {
    pub entity_id: String,
    pub upstream_count: usize,
    pub upstream_files: Vec<String>,
}

/// The complete change plan.
#[derive(Debug, Clone)]
pub struct ChangePlan {
    pub goal: String,
    pub relevant_entities: Vec<RelevantEntity>,
    /// Entity IDs in dependency-safe modification order (leaf dependencies first).
    pub modification_order: Vec<String>,
    pub impact_summary: Vec<ImpactSummary>,
    /// Entity IDs of test entities that reference target entities.
    pub test_coverage: Vec<String>,
    /// Lifting coverage as a percentage (for partial-lift warning).
    pub coverage_pct: f64,
}

/// Plan code changes: find relevant entities, compute modification order, assess impact.
pub fn plan_change(
    graph: &RPGraph,
    request: &PlanChangeRequest,
    embedding_scores: Option<&HashMap<String, f64>>,
) -> ChangePlan {
    let (lifted, total) = graph.lifting_coverage();
    let coverage_pct = if total > 0 {
        lifted as f64 / total as f64 * 100.0
    } else {
        0.0
    };

    // Step 1: Search for relevant entities
    let results = search_with_params(
        graph,
        &SearchParams {
            query: request.goal,
            mode: SearchMode::Auto,
            scope: request.scope,
            limit: request.max_entities,
            line_nums: None,
            file_pattern: None,
            entity_type_filter: None,
            embedding_scores,
        },
    );

    let relevant_entities: Vec<RelevantEntity> = results
        .iter()
        .filter_map(|r| {
            let entity = graph.get_entity(&r.entity_id)?;
            Some(RelevantEntity {
                entity_id: r.entity_id.clone(),
                name: r.entity_name.clone(),
                file: r.file.clone(),
                features: entity.semantic_features.clone(),
                relevance: r.score,
            })
        })
        .collect();

    let target_ids: HashSet<String> = relevant_entities
        .iter()
        .map(|e| e.entity_id.clone())
        .collect();

    // Step 2: Compute impact radius for each target
    let impact_summary: Vec<ImpactSummary> = relevant_entities
        .iter()
        .filter_map(|re| {
            let impact = compute_impact_radius(
                graph,
                &re.entity_id,
                Direction::Upstream,
                2,
                None,
                Some(50),
            )?;
            let upstream_files: Vec<String> = impact
                .reachable
                .iter()
                .map(|e| e.file.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            Some(ImpactSummary {
                entity_id: re.entity_id.clone(),
                upstream_count: impact.total,
                upstream_files,
            })
        })
        .collect();

    // Step 3: Topological sort of target entities for modification order
    let modification_order = topological_sort(graph, &target_ids);

    // Step 4: Find test entities that reference targets
    let test_coverage = find_test_coverage(graph, &target_ids);

    ChangePlan {
        goal: request.goal.to_string(),
        relevant_entities,
        modification_order,
        impact_summary,
        test_coverage,
        coverage_pct,
    }
}

/// Topological sort of target entities based on their internal dependency edges.
/// Leaf dependencies (entities that don't depend on other targets) come first.
fn topological_sort(graph: &RPGraph, target_ids: &HashSet<String>) -> Vec<String> {
    // Build adjacency: for each target, which other targets does it depend on?
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    for id in target_ids {
        deps.insert(id.clone(), Vec::new());
    }

    for id in target_ids {
        let neighbors = get_neighbors(graph, id, Direction::Downstream, None);
        for (neighbor_id, _kind, _label) in &neighbors {
            if target_ids.contains(neighbor_id) && neighbor_id != id {
                deps.entry(id.clone())
                    .or_default()
                    .push(neighbor_id.clone());
            }
        }
    }

    // Kahn's algorithm — leaf dependencies first.
    // deps[A] = [B] means "A depends on B", so B should come before A.
    // Build reverse map: for each B, which nodes depend on B?
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    for id in target_ids {
        dependents.entry(id.clone()).or_default();
    }
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    for id in target_ids {
        let dep_count = deps.get(id).map_or(0, |d| d.len());
        in_degree.insert(id.clone(), dep_count);
    }
    for (id, dep_list) in &deps {
        for dep in dep_list {
            dependents.entry(dep.clone()).or_default().push(id.clone());
        }
    }

    let mut queue: Vec<String> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(id, _)| id.clone())
        .collect();
    queue.sort(); // deterministic ordering

    let mut order = Vec::new();
    while let Some(id) = queue.pop() {
        order.push(id.clone());
        if let Some(dep_ids) = dependents.get(&id) {
            for dep_id in dep_ids {
                if let Some(deg) = in_degree.get_mut(dep_id) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push(dep_id.clone());
                        queue.sort(); // keep deterministic
                    }
                }
            }
        }
    }

    // Any remaining (cycles) — append in sorted order
    for id in target_ids {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }

    order
}

/// Find test entities that have edges to any target entity.
fn find_test_coverage(graph: &RPGraph, target_ids: &HashSet<String>) -> Vec<String> {
    let mut test_ids = Vec::new();

    // Look for entities whose name or file path suggests they're tests
    for (id, entity) in &graph.entities {
        if target_ids.contains(id) {
            continue;
        }
        let is_test = entity.kind == rpg_core::graph::EntityKind::Test
            || entity.name.starts_with("test_")
            || entity.name.starts_with("Test")
            || entity.file.to_string_lossy().contains("test");

        if !is_test {
            continue;
        }

        // Check if this test entity has edges to any target
        let neighbors = get_neighbors(graph, id, Direction::Downstream, None);
        let touches_target = neighbors.iter().any(|(nid, _, _)| target_ids.contains(nid));
        if touches_target {
            test_ids.push(id.clone());
        }
    }

    test_ids.sort();
    test_ids
}

/// Format a change plan as a human-readable markdown string.
pub fn format_change_plan(plan: &ChangePlan) -> String {
    let mut out = format!("## Change Plan for: \"{}\"\n\n", plan.goal);

    // Relevant entities
    out.push_str("### Relevant Entities (by relevance)\n\n");
    for (i, re) in plan.relevant_entities.iter().enumerate() {
        let features_str = if re.features.is_empty() {
            "(no features)".to_string()
        } else {
            format!("\"{}\"", re.features.join(", "))
        };
        out.push_str(&format!(
            "{}. `{}` ({}) — {} ({:.2})\n",
            i + 1,
            re.entity_id,
            re.file,
            features_str,
            re.relevance,
        ));
    }

    // Modification order
    if !plan.modification_order.is_empty() {
        out.push_str("\n### Modification Order (dependency-safe)\n\n");
        for (i, eid) in plan.modification_order.iter().enumerate() {
            out.push_str(&format!("{}. `{}`\n", i + 1, eid));
        }
    }

    // Impact radius
    if !plan.impact_summary.is_empty() {
        out.push_str("\n### Impact Radius\n\n");
        for impact in &plan.impact_summary {
            out.push_str(&format!(
                "- `{}`: {} upstream dependent{} across {} file{}\n",
                impact.entity_id,
                impact.upstream_count,
                if impact.upstream_count == 1 { "" } else { "s" },
                impact.upstream_files.len(),
                if impact.upstream_files.len() == 1 {
                    ""
                } else {
                    "s"
                },
            ));
        }
    }

    // Test coverage
    if !plan.test_coverage.is_empty() {
        out.push_str("\n### Related Tests\n\n");
        for tid in &plan.test_coverage {
            out.push_str(&format!("- `{}`\n", tid));
        }
    }

    // Coverage warning
    if plan.coverage_pct < 100.0 {
        out.push_str(&format!(
            "\nNote: {:.0}% lifting coverage. Results improve with full lifting.\n",
            plan.coverage_pct,
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, EdgeKind, Entity, EntityDeps, EntityKind, RPGraph};
    use std::path::PathBuf;

    fn make_entity(
        id: &str,
        name: &str,
        file: &str,
        kind: EntityKind,
        features: Vec<&str>,
    ) -> Entity {
        Entity {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            file: PathBuf::from(file),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: features.into_iter().map(|s| s.to_string()).collect(),
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
        }
    }

    fn build_test_graph() -> RPGraph {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/config.rs:Config".to_string(),
            make_entity(
                "src/config.rs:Config",
                "Config",
                "src/config.rs",
                EntityKind::Class,
                vec!["hold configuration values"],
            ),
        );
        graph.entities.insert(
            "src/server.rs:Server".to_string(),
            make_entity(
                "src/server.rs:Server",
                "Server",
                "src/server.rs",
                EntityKind::Class,
                vec!["serve api requests"],
            ),
        );
        graph.entities.insert(
            "src/server.rs:Server::start".to_string(),
            make_entity(
                "src/server.rs:Server::start",
                "start",
                "src/server.rs",
                EntityKind::Method,
                vec!["start http server"],
            ),
        );
        // Add edge: Server::start invokes Config
        graph.edges.push(DependencyEdge {
            source: "src/server.rs:Server::start".to_string(),
            target: "src/config.rs:Config".to_string(),
            kind: EdgeKind::Invokes,
        });
        // Add a test entity
        graph.entities.insert(
            "tests/server_test.rs:test_start".to_string(),
            make_entity(
                "tests/server_test.rs:test_start",
                "test_start",
                "tests/server_test.rs",
                EntityKind::Test,
                vec!["test server startup"],
            ),
        );
        graph.edges.push(DependencyEdge {
            source: "tests/server_test.rs:test_start".to_string(),
            target: "src/server.rs:Server::start".to_string(),
            kind: EdgeKind::Invokes,
        });
        graph.refresh_metadata();
        graph
    }

    #[test]
    fn test_plan_finds_relevant_entities() {
        let graph = build_test_graph();
        let request = PlanChangeRequest {
            goal: "server",
            scope: None,
            max_entities: 10,
        };
        let plan = plan_change(&graph, &request, None);
        assert!(
            !plan.relevant_entities.is_empty(),
            "should find entities matching 'server'"
        );
    }

    #[test]
    fn test_plan_dependency_ordering() {
        let graph = build_test_graph();
        let mut targets = HashSet::new();
        targets.insert("src/server.rs:Server::start".to_string());
        targets.insert("src/config.rs:Config".to_string());
        let order = topological_sort(&graph, &targets);
        // Config should come before Server::start (Config is a leaf dependency)
        let config_pos = order.iter().position(|id| id.contains("Config")).unwrap();
        let start_pos = order.iter().position(|id| id.contains("start")).unwrap();
        assert!(
            config_pos < start_pos,
            "Config (leaf) should come before Server::start in modification order"
        );
    }

    #[test]
    fn test_plan_test_coverage() {
        let graph = build_test_graph();
        let mut targets = HashSet::new();
        targets.insert("src/server.rs:Server::start".to_string());
        let tests = find_test_coverage(&graph, &targets);
        assert!(
            tests.contains(&"tests/server_test.rs:test_start".to_string()),
            "should find test_start as related test"
        );
    }

    #[test]
    fn test_plan_impact_radius() {
        let graph = build_test_graph();
        let request = PlanChangeRequest {
            goal: "server start",
            scope: None,
            max_entities: 10,
        };
        let plan = plan_change(&graph, &request, None);
        // Server::start should have upstream dependents (test_start invokes it)
        let start_impact = plan
            .impact_summary
            .iter()
            .find(|s| s.entity_id.contains("Server::start"));
        assert!(
            start_impact.is_some(),
            "Server::start should appear in impact_summary"
        );
        let impact = start_impact.unwrap();
        assert!(
            impact.upstream_count > 0,
            "Server::start should have upstream dependents"
        );
    }

    #[test]
    fn test_plan_partial_lift_note() {
        let graph = build_test_graph();
        let request = PlanChangeRequest {
            goal: "anything",
            scope: None,
            max_entities: 5,
        };
        let plan = plan_change(&graph, &request, None);
        let output = format_change_plan(&plan);
        assert!(output.contains("Change Plan for:"));
    }

    #[test]
    fn test_plan_partial_lift_fallback() {
        // plan_change works without semantic features (lexical search only).
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/config.rs:Config".to_string(),
            make_entity(
                "src/config.rs:Config",
                "Config",
                "src/config.rs",
                EntityKind::Class,
                vec![], // NO semantic features
            ),
        );
        graph.entities.insert(
            "src/server.rs:Server".to_string(),
            make_entity(
                "src/server.rs:Server",
                "Server",
                "src/server.rs",
                EntityKind::Class,
                vec![], // NO semantic features
            ),
        );
        graph.refresh_metadata();

        let request = PlanChangeRequest {
            goal: "Server",
            scope: None,
            max_entities: 10,
        };
        let plan = plan_change(&graph, &request, None);

        // Lexical name match should find Server even without semantic features
        assert!(
            !plan.relevant_entities.is_empty(),
            "lexical search should find Server even without semantic features"
        );
        assert!(
            plan.relevant_entities.iter().any(|e| e.name == "Server"),
            "Server should be in relevant entities"
        );
        // No features on any entity → 0% coverage
        assert!(
            plan.coverage_pct < 1.0,
            "coverage should be ~0% with no features"
        );
        let output = format_change_plan(&plan);
        assert!(
            output.contains("coverage"),
            "output should note partial lifting coverage"
        );
    }
}
