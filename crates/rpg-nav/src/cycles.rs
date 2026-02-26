//! Circular dependency detection for architectural smell analysis.
//!
//! Circular dependencies are architectural smells where:
//! - Module A depends on B, B depends on C, and C depends back on A
//! - This prevents independent compilation, testing, and reuse
//! - Changes ripple through the entire cycle

use rpg_core::graph::{EdgeKind, RPGraph};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0;
        for part in parts {
            if part.is_empty() {
                continue;
            }
            if let Some(i) = path[pos..].find(part) {
                pos += i + part.len();
            } else {
                return false;
            }
        }
        true
    } else {
        path.contains(pattern)
    }
}

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

/// A single circular dependency cycle.
#[derive(Debug, Clone, Serialize)]
pub struct Cycle {
    /// Entity IDs forming the cycle, in order.
    pub cycle: Vec<String>,
    /// Human-readable representation: A → B → C → A
    pub representation: String,
    /// Number of entities in the cycle.
    pub length: usize,
    /// File paths involved in the cycle (deduplicated).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// Configuration for cycle detection.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct CycleConfig {
    /// Maximum number of cycles to return.
    pub max_cycles: usize,
    /// Maximum cycle length to detect.
    pub max_cycle_length: usize,
    /// Minimum cycle length to report.
    pub min_cycle_length: usize,
    /// Filter to specific hierarchy areas (comma-separated, e.g., "Navigation,Parser").
    pub area: Option<String>,
    /// Sort by: "length", "file_count", "entity_count"
    pub sort_by: String,
    /// Include file paths in output.
    pub include_files: bool,
    /// Only show cycles that span multiple files.
    pub cross_file_only: bool,
    /// Only show cycles that span multiple hierarchy areas.
    pub cross_area_only: bool,
    /// Ignore .rpgignore rules.
    pub ignore_rpgignore: bool,
    /// Project root path for .rpgignore lookup.
    pub project_root: Option<std::path::PathBuf>,
}

impl Default for CycleConfig {
    fn default() -> Self {
        Self {
            max_cycles: usize::MAX,
            max_cycle_length: 20,
            min_cycle_length: 2,
            area: None,
            sort_by: "length".to_string(),
            include_files: true,
            cross_file_only: false,
            cross_area_only: false,
            ignore_rpgignore: false,
            project_root: None,
        }
    }
}

/// Breakdown of cycles per hierarchy area.
#[derive(Debug, Clone, Serialize)]
pub struct AreaCycleBreakdown {
    pub area: String,
    pub cycle_count: usize,
    pub length_2: usize,
    pub length_3: usize,
    pub length_4_plus: usize,
    pub entity_count: usize,
    pub file_count: usize,
}

/// Report of all circular dependencies found.
#[derive(Debug, Clone, Serialize)]
pub struct CycleReport {
    /// Total number of cycles detected.
    pub cycle_count: usize,
    /// Total number of entities involved in cycles.
    pub entities_in_cycles: usize,
    /// Total number of files involved in cycles.
    pub files_in_cycles: usize,
    /// Number of unique areas affected by cycles.
    pub areas_in_cycles: usize,
    /// Number of cycles that span multiple files.
    pub cross_file_count: usize,
    /// Number of cycles that span multiple areas.
    pub cross_area_count: usize,
    /// Maximum cycle length found.
    pub max_cycle_length: usize,
    /// Minimum cycle length found.
    pub min_cycle_length: usize,
    /// Average cycle length.
    pub avg_cycle_length: f64,
    /// Breakdown of cycles by length.
    pub length_distribution: LengthDistribution,
    /// Breakdown of cycles per area.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub area_breakdown: Vec<AreaCycleBreakdown>,
    /// All detected cycles.
    pub cycles: Vec<Cycle>,
    /// Summary message.
    pub summary: String,
}

/// Distribution of cycles by length.
#[derive(Debug, Clone, Serialize, Default)]
pub struct LengthDistribution {
    pub length_2: usize,
    pub length_3: usize,
    pub length_4: usize,
    pub length_5_plus: usize,
}

/// Context struct for cycle detection to reduce function arguments.
struct CycleDetectionContext<'a> {
    adj: &'a HashMap<&'a str, Vec<&'a str>>,
    visited: &'a mut HashSet<&'a str>,
    recursion_stack: &'a mut Vec<&'a str>,
    recursion_set: &'a mut HashSet<&'a str>,
    cycles: &'a mut Vec<Cycle>,
    max_length: usize,
    min_length: usize,
}

/// Detect circular dependencies in the graph using DFS with cycle enumeration.
///
/// Algorithm: Modified DFS that tracks the current path and backtracks to find all cycles.
/// Cycles are deduplicated by normalizing their starting point.
pub fn detect_cycles(graph: &RPGraph, config: &CycleConfig) -> CycleReport {
    // Build adjacency list for dependency edges only
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for edge in &graph.edges {
        if DEPENDENCY_EDGE_KINDS.contains(&edge.kind) {
            adj.entry(edge.source.as_str())
                .or_default()
                .push(edge.target.as_str());
        }
    }

    let mut cycles: Vec<Cycle> = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut recursion_stack: Vec<&str> = Vec::new();
    let mut recursion_set: HashSet<&str> = HashSet::new();

    // Create context struct to reduce function arguments
    let mut ctx = CycleDetectionContext {
        adj: &adj,
        visited: &mut visited,
        recursion_stack: &mut recursion_stack,
        recursion_set: &mut recursion_set,
        cycles: &mut cycles,
        max_length: config.max_cycle_length,
        min_length: config.min_cycle_length,
    };

    // For each entity, find cycles starting from it
    for entity_id in graph.entities.keys() {
        if !ctx.visited.contains(entity_id.as_str()) {
            find_cycles_from(entity_id.as_str(), &mut ctx);
        }
    }

    // Deduplicate cycles (same cycle can be found starting from different nodes)
    let mut cycles = deduplicate_cycles(cycles);

    // Filter based on .rpgignore if not ignored
    if !config.ignore_rpgignore {
        if let Some(ref project_root) = config.project_root {
            let rpgignore_path = project_root.join(".rpgignore");
            if rpgignore_path.exists() {
                if let Ok(patterns) = std::fs::read_to_string(&rpgignore_path) {
                    let patterns: Vec<String> = patterns
                        .lines()
                        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                        .map(|l| l.trim().to_string())
                        .collect();

                    if !patterns.is_empty() {
                        cycles.retain(|cycle| {
                            cycle.cycle.iter().any(|entity_id| {
                                if let Some(entity) = graph.entities.get(entity_id) {
                                    let file_path = entity.file.display().to_string();
                                    !patterns
                                        .iter()
                                        .any(|p| glob_match(p, &file_path) || file_path.contains(p))
                                } else {
                                    true
                                }
                            })
                        });
                    }
                }
            }
        }
    }

    // Filter by hierarchy area if specified (supports comma-separated multiple areas)
    if let Some(ref areas_str) = config.area {
        let areas: Vec<&str> = areas_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !areas.is_empty() {
            cycles.retain(|cycle| {
                cycle.cycle.iter().any(|entity_id| {
                    graph
                        .entities
                        .get(entity_id)
                        .map(|e| areas.iter().any(|area| e.hierarchy_path.starts_with(area)))
                        .unwrap_or(false)
                })
            });
        }
    }

    // Filter to only cross-file cycles if configured
    if config.cross_file_only {
        cycles.retain(|cycle| {
            let unique_files: HashSet<String> = cycle
                .cycle
                .iter()
                .filter_map(|id| graph.entities.get(id))
                .map(|e| e.file.display().to_string())
                .collect();
            unique_files.len() > 1
        });
    }

    // Filter to only cross-area cycles if configured
    if config.cross_area_only {
        cycles.retain(|cycle| {
            let unique_areas: HashSet<String> = cycle
                .cycle
                .iter()
                .filter_map(|id| graph.entities.get(id))
                .filter_map(|e| e.hierarchy_path.split('/').next().map(|s| s.to_string()))
                .collect();
            unique_areas.len() > 1
        });
    }

    // Add file information if requested
    if config.include_files {
        for cycle in &mut cycles {
            let mut files: Vec<String> = cycle
                .cycle
                .iter()
                .filter_map(|id| graph.entities.get(id))
                .map(|e| e.file.display().to_string())
                .collect();
            files.sort();
            files.dedup();
            cycle.files = files;
        }
    }

    // Sort cycles based on sort_by parameter
    match config.sort_by.as_str() {
        "file_count" => {
            cycles.sort_by(|a, b| {
                let a_files = a.files.len();
                let b_files = b.files.len();
                b_files
                    .cmp(&a_files)
                    .then_with(|| a.cycle.first().cmp(&b.cycle.first()))
            });
        }
        "entity_count" => {
            cycles.sort_by(|a, b| {
                b.length
                    .cmp(&a.length)
                    .then_with(|| a.cycle.first().cmp(&b.cycle.first()))
            });
        }
        _ => {
            // Default: sort by length, then by first element
            cycles.sort_by(|a, b| {
                a.length
                    .cmp(&b.length)
                    .then_with(|| a.cycle.first().cmp(&b.cycle.first()))
            });
        }
    }

    // Compute statistics
    let cycle_count = cycles.len();
    let entities_in_cycles: HashSet<&str> = cycles
        .iter()
        .flat_map(|c| c.cycle.iter().map(|s| s.as_str()))
        .collect();
    // Compute from entity data, not cycle.files, so this is correct even when include_files=false.
    let files_in_cycles: HashSet<String> = cycles
        .iter()
        .flat_map(|c| {
            c.cycle
                .iter()
                .filter_map(|id| graph.entities.get(id).map(|e| e.file.display().to_string()))
        })
        .collect();

    let max_cycle_length = cycles.iter().map(|c| c.length).max().unwrap_or(0);
    let min_cycle_length = cycles.iter().map(|c| c.length).min().unwrap_or(0);
    let total_length: usize = cycles.iter().map(|c| c.length).sum();
    let avg_cycle_length = if cycle_count > 0 {
        total_length as f64 / cycle_count as f64
    } else {
        0.0
    };

    // Compute length distribution
    let mut length_distribution = LengthDistribution::default();
    for cycle in &cycles {
        match cycle.length {
            2 => length_distribution.length_2 += 1,
            3 => length_distribution.length_3 += 1,
            4 => length_distribution.length_4 += 1,
            _ => length_distribution.length_5_plus += 1,
        }
    }

    // Compute cross-file and cross-area counts
    let cross_file_count = cycles
        .iter()
        .filter(|c| {
            let unique_files: HashSet<_> = c
                .cycle
                .iter()
                .filter_map(|id| graph.entities.get(id))
                .map(|e| e.file.display().to_string())
                .collect();
            unique_files.len() > 1
        })
        .count();

    let cross_area_count = cycles
        .iter()
        .filter(|c| {
            let unique_areas: HashSet<_> = c
                .cycle
                .iter()
                .filter_map(|id| graph.entities.get(id))
                .filter_map(|e| e.hierarchy_path.split('/').next())
                .collect();
            unique_areas.len() > 1
        })
        .count();

    // Compute area breakdown.
    // cycle_count and length_* must be incremented once per cycle per area (not once per entity).
    let mut area_stats: HashMap<String, AreaCycleBreakdown> = HashMap::new();
    for cycle in &cycles {
        // Pass 1: count entities per area and collect unique areas touched by this cycle.
        let mut areas_this_cycle: HashSet<String> = HashSet::new();
        for entity_id in &cycle.cycle {
            if let Some(entity) = graph.entities.get(entity_id) {
                let area = entity
                    .hierarchy_path
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if area.is_empty() {
                    continue;
                }
                area_stats
                    .entry(area.clone())
                    .or_insert(AreaCycleBreakdown {
                        area: area.clone(),
                        cycle_count: 0,
                        length_2: 0,
                        length_3: 0,
                        length_4_plus: 0,
                        entity_count: 0,
                        file_count: 0,
                    })
                    .entity_count += 1;
                areas_this_cycle.insert(area);
            }
        }
        // Pass 2: count this cycle exactly once per area it touches.
        for area in &areas_this_cycle {
            if let Some(entry) = area_stats.get_mut(area) {
                entry.cycle_count += 1;
                match cycle.length {
                    2 => entry.length_2 += 1,
                    3 => entry.length_3 += 1,
                    _ => entry.length_4_plus += 1,
                }
            }
        }
    }

    // Deduplicate files per area and count
    let mut area_file_counts: HashMap<String, HashSet<String>> = HashMap::new();
    for cycle in &cycles {
        for entity_id in &cycle.cycle {
            if let Some(entity) = graph.entities.get(entity_id) {
                let area = entity
                    .hierarchy_path
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !area.is_empty() {
                    area_file_counts
                        .entry(area)
                        .or_default()
                        .insert(entity.file.display().to_string());
                }
            }
        }
    }

    let mut area_breakdown: Vec<AreaCycleBreakdown> = area_stats
        .into_iter()
        .map(|(area, mut stats)| {
            stats.file_count = area_file_counts.get(&area).map(|s| s.len()).unwrap_or(0);
            stats
        })
        .collect();
    area_breakdown.sort_by(|a, b| b.cycle_count.cmp(&a.cycle_count));

    let areas_in_cycles = area_breakdown.len();

    // Generate summary
    let summary = if cycle_count == 0 {
        "No circular dependencies detected. Codebase has acyclic dependency structure.".to_string()
    } else if cycle_count == 1 {
        format!(
            "Found 1 circular dependency involving {} entities in {} file(s). \
             This may indicate tight coupling that should be refactored.",
            entities_in_cycles.len(),
            files_in_cycles.len()
        )
    } else {
        format!(
            "Found {} circular dependencies involving {} entities across {} file(s). \
             Cycles range from {} to {} entities. \
             Consider extracting shared interfaces or applying Dependency Inversion Principle.",
            cycle_count,
            entities_in_cycles.len(),
            files_in_cycles.len(),
            min_cycle_length,
            max_cycle_length
        )
    };

    CycleReport {
        cycle_count,
        entities_in_cycles: entities_in_cycles.len(),
        files_in_cycles: files_in_cycles.len(),
        areas_in_cycles,
        cross_file_count,
        cross_area_count,
        max_cycle_length,
        min_cycle_length,
        avg_cycle_length,
        length_distribution,
        area_breakdown,
        cycles,
        summary,
    }
}

/// DFS-based cycle detection that finds all cycles starting from a node.
fn find_cycles_from<'a>(node: &'a str, ctx: &mut CycleDetectionContext<'a>) {
    ctx.visited.insert(node);
    ctx.recursion_stack.push(node);
    ctx.recursion_set.insert(node);

    if let Some(neighbors) = ctx.adj.get(node) {
        for &neighbor in neighbors {
            // Limit cycle length to prevent exponential blowup
            if ctx.recursion_stack.len() > ctx.max_length {
                continue;
            }

            if ctx.recursion_set.contains(neighbor) {
                // Found a cycle! Extract it from the recursion stack.
                if let Some(cycle_start) = ctx.recursion_stack.iter().position(|&n| n == neighbor) {
                    let cycle_nodes: Vec<String> = ctx.recursion_stack[cycle_start..]
                        .iter()
                        .map(|&s| s.to_string())
                        .collect();

                    // Only add if it meets minimum length requirement
                    if cycle_nodes.len() >= ctx.min_length {
                        let representation = format_cycle(&cycle_nodes);
                        ctx.cycles.push(Cycle {
                            length: cycle_nodes.len(),
                            cycle: cycle_nodes,
                            representation,
                            files: Vec::new(),
                        });
                    }
                }
            } else if !ctx.visited.contains(neighbor) {
                find_cycles_from(neighbor, ctx);
            }
        }
    }

    ctx.recursion_stack.pop();
    ctx.recursion_set.remove(node);
}

/// Format a cycle as "A → B → C → A"
fn format_cycle(cycle: &[String]) -> String {
    if cycle.is_empty() {
        return String::new();
    }
    let mut result = cycle.join(" → ");
    result.push_str(" → ");
    result.push_str(&cycle[0]);
    result
}

/// Deduplicate cycles by normalizing them (rotate to start with smallest ID).
fn deduplicate_cycles(cycles: Vec<Cycle>) -> Vec<Cycle> {
    let mut seen: HashSet<Vec<String>> = HashSet::new();
    let mut result: Vec<Cycle> = Vec::new();

    for mut cycle in cycles {
        // Normalize: rotate to start with the lexicographically smallest element
        if let Some(min_pos) = cycle
            .cycle
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| *v)
            .map(|(i, _)| i)
        {
            cycle.cycle.rotate_left(min_pos);
        }

        if !seen.contains(&cycle.cycle) {
            seen.insert(cycle.cycle.clone());
            cycle.representation = format_cycle(&cycle.cycle);
            result.push(cycle);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{DependencyEdge, Entity, EntityDeps, EntityKind};
    use std::path::PathBuf;

    fn make_entity(id: &str, name: &str, kind: EntityKind, file: &str) -> Entity {
        Entity {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            file: PathBuf::from(file),
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

    fn make_test_graph_no_cycles() -> RPGraph {
        // A -> B -> C (linear chain, no cycle)
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a".to_string(),
            make_entity("a", "fn_a", EntityKind::Function, "src/a.rs"),
        );
        graph.entities.insert(
            "b".to_string(),
            make_entity("b", "fn_b", EntityKind::Function, "src/b.rs"),
        );
        graph.entities.insert(
            "c".to_string(),
            make_entity("c", "fn_c", EntityKind::Function, "src/c.rs"),
        );
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
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

    fn make_test_graph_simple_cycle() -> RPGraph {
        // A -> B -> C -> A (simple 3-node cycle)
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a".to_string(),
            make_entity("a", "fn_a", EntityKind::Function, "src/a.rs"),
        );
        graph.entities.insert(
            "b".to_string(),
            make_entity("b", "fn_b", EntityKind::Function, "src/b.rs"),
        );
        graph.entities.insert(
            "c".to_string(),
            make_entity("c", "fn_c", EntityKind::Function, "src/c.rs"),
        );
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "c".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "c".to_string(),
                target: "a".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();
        graph
    }

    fn make_test_graph_two_node_cycle() -> RPGraph {
        // A -> B -> A (two-node cycle)
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a".to_string(),
            make_entity("a", "fn_a", EntityKind::Function, "src/a.rs"),
        );
        graph.entities.insert(
            "b".to_string(),
            make_entity("b", "fn_b", EntityKind::Function, "src/b.rs"),
        );
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "a".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();
        graph
    }

    fn make_test_graph_multiple_cycles() -> RPGraph {
        // Cycle 1: A -> B -> A
        // Cycle 2: C -> D -> E -> C
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a".to_string(),
            make_entity("a", "fn_a", EntityKind::Function, "src/a.rs"),
        );
        graph.entities.insert(
            "b".to_string(),
            make_entity("b", "fn_b", EntityKind::Function, "src/b.rs"),
        );
        graph.entities.insert(
            "c".to_string(),
            make_entity("c", "fn_c", EntityKind::Function, "src/c.rs"),
        );
        graph.entities.insert(
            "d".to_string(),
            make_entity("d", "fn_d", EntityKind::Function, "src/d.rs"),
        );
        graph.entities.insert(
            "e".to_string(),
            make_entity("e", "fn_e", EntityKind::Function, "src/e.rs"),
        );
        graph.edges = vec![
            // Cycle 1: A <-> B
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "a".to_string(),
                kind: EdgeKind::Invokes,
            },
            // Cycle 2: C -> D -> E -> C
            DependencyEdge {
                source: "c".to_string(),
                target: "d".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "d".to_string(),
                target: "e".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "e".to_string(),
                target: "c".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();
        graph
    }

    #[test]
    fn test_no_cycles_in_linear_chain() {
        let graph = make_test_graph_no_cycles();
        let config = CycleConfig::default();
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 0);
        assert!(report.cycles.is_empty());
        assert!(report.summary.contains("No circular dependencies"));
    }

    #[test]
    fn test_simple_cycle_detection() {
        let graph = make_test_graph_simple_cycle();
        let config = CycleConfig::default();
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 1);
        assert_eq!(report.cycles[0].length, 3);
        assert_eq!(report.entities_in_cycles, 3);
        assert!(report.cycles[0].representation.contains("→"));
        assert!(report.summary.contains("Found 1 circular dependency"));
    }

    #[test]
    fn test_two_node_cycle() {
        let graph = make_test_graph_two_node_cycle();
        let config = CycleConfig::default();
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 1);
        assert_eq!(report.cycles[0].length, 2);
    }

    #[test]
    fn test_multiple_cycles() {
        let graph = make_test_graph_multiple_cycles();
        let config = CycleConfig::default();
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 2);
        assert_eq!(report.entities_in_cycles, 5);
    }

    #[test]
    fn test_max_cycle_length() {
        let mut graph = RPGraph::new("rust");

        // Create a 10-node cycle
        for i in 0..10 {
            let id = format!("n{}", i);
            graph.entities.insert(
                id.clone(),
                make_entity(&id, &id, EntityKind::Function, "src/lib.rs"),
            );
        }

        for i in 0..10 {
            let next = (i + 1) % 10;
            graph.edges.push(DependencyEdge {
                source: format!("n{}", i),
                target: format!("n{}", next),
                kind: EdgeKind::Invokes,
            });
        }
        graph.refresh_metadata();

        // With max_length = 5, should not detect the full cycle
        let config = CycleConfig {
            max_cycle_length: 5,
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);
        assert_eq!(report.cycle_count, 0);

        // With max_length = 15, should detect the cycle
        let config = CycleConfig {
            max_cycle_length: 15,
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);
        assert_eq!(report.cycle_count, 1);
    }

    #[test]
    fn test_cycle_format() {
        let cycle = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let repr = format_cycle(&cycle);
        assert_eq!(repr, "a → b → c → a");
    }

    #[test]
    fn test_cycle_ignores_contains_edges() {
        let mut graph = RPGraph::new("rust");

        // Contains edges should NOT create cycles
        graph.entities.insert(
            "parent".to_string(),
            make_entity("parent", "Parent", EntityKind::Class, "src/parent.rs"),
        );
        graph.entities.insert(
            "child".to_string(),
            make_entity("child", "child", EntityKind::Function, "src/parent.rs"),
        );

        graph.edges = vec![
            // Contains edge (should be ignored for cycle detection)
            DependencyEdge {
                source: "parent".to_string(),
                target: "child".to_string(),
                kind: EdgeKind::Contains,
            },
        ];
        graph.refresh_metadata();

        let config = CycleConfig::default();
        let report = detect_cycles(&graph, &config);
        assert_eq!(report.cycle_count, 0);
    }

    #[test]
    fn test_files_in_cycles() {
        let graph = make_test_graph_simple_cycle();
        let config = CycleConfig {
            include_files: true,
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.files_in_cycles, 3);
        assert_eq!(report.cycles[0].files.len(), 3);
    }

    #[test]
    fn test_empty_graph() {
        let graph = RPGraph::new("rust");
        let config = CycleConfig::default();
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 0);
        assert_eq!(report.entities_in_cycles, 0);
    }

    #[test]
    fn test_deterministic_output() {
        let graph = make_test_graph_multiple_cycles();
        let config = CycleConfig::default();

        let report1 = detect_cycles(&graph, &config);
        let report2 = detect_cycles(&graph, &config);

        // Cycles should be sorted consistently
        assert_eq!(report1.cycle_count, report2.cycle_count);
        for (c1, c2) in report1.cycles.iter().zip(report2.cycles.iter()) {
            assert_eq!(c1.cycle, c2.cycle);
            assert_eq!(c1.representation, c2.representation);
        }
    }

    // ---- helper for area/filter tests ----

    fn make_entity_with_area(id: &str, name: &str, file: &str, area: &str) -> Entity {
        let mut e = make_entity(id, name, EntityKind::Function, file);
        e.hierarchy_path = format!("{}/sub", area);
        e
    }

    /// Two independent 2-node cycles in different hierarchy areas.
    /// Nav: A <-> B (cross-file)
    /// Parser: C <-> D (same file)
    fn make_test_graph_two_areas() -> RPGraph {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a".to_string(),
            make_entity_with_area("a", "fn_a", "src/nav/a.rs", "Navigation"),
        );
        graph.entities.insert(
            "b".to_string(),
            make_entity_with_area("b", "fn_b", "src/nav/b.rs", "Navigation"),
        );
        graph.entities.insert(
            "c".to_string(),
            make_entity_with_area("c", "fn_c", "src/parser/p.rs", "Parser"),
        );
        graph.entities.insert(
            "d".to_string(),
            make_entity_with_area("d", "fn_d", "src/parser/p.rs", "Parser"),
        );
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "a".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "c".to_string(),
                target: "d".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "d".to_string(),
                target: "c".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();
        graph
    }

    #[test]
    fn test_area_filter_isolates_one_area() {
        let graph = make_test_graph_two_areas();
        let config = CycleConfig {
            area: Some("Navigation".to_string()),
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 1);
        let ids: Vec<&str> = report.cycles[0].cycle.iter().map(|s| s.as_str()).collect();
        assert!(
            ids.contains(&"a") && ids.contains(&"b"),
            "expected Navigation cycle a-b"
        );
    }

    #[test]
    fn test_area_breakdown_counts_cycles_not_entities() {
        let graph = make_test_graph_two_areas();
        let config = CycleConfig::default();
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 2);

        let nav = report
            .area_breakdown
            .iter()
            .find(|ab| ab.area == "Navigation")
            .expect("Navigation area missing");
        // Each 2-entity cycle should count as 1 cycle, not 2.
        assert_eq!(
            nav.cycle_count, 1,
            "Navigation: expected 1 cycle, not {}",
            nav.cycle_count
        );
        assert_eq!(nav.length_2, 1);
        assert_eq!(nav.entity_count, 2);

        let parser = report
            .area_breakdown
            .iter()
            .find(|ab| ab.area == "Parser")
            .expect("Parser area missing");
        assert_eq!(
            parser.cycle_count, 1,
            "Parser: expected 1 cycle, not {}",
            parser.cycle_count
        );
        assert_eq!(parser.entity_count, 2);
    }

    #[test]
    fn test_cross_file_only_filter() {
        let graph = make_test_graph_two_areas();
        // Navigation cycle is cross-file (a.rs / b.rs); Parser cycle is same-file (p.rs / p.rs).
        let config = CycleConfig {
            cross_file_only: true,
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 1);
        let ids: Vec<&str> = report.cycles[0].cycle.iter().map(|s| s.as_str()).collect();
        assert!(ids.contains(&"a") && ids.contains(&"b"));
    }

    #[test]
    fn test_cross_area_only_filter() {
        // A cross-area cycle: A (Nav) -> B (Parser) -> A
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a".to_string(),
            make_entity_with_area("a", "fn_a", "src/nav/a.rs", "Navigation"),
        );
        graph.entities.insert(
            "b".to_string(),
            make_entity_with_area("b", "fn_b", "src/parser/b.rs", "Parser"),
        );
        // Also add a same-area cycle: C <-> D (both Nav)
        graph.entities.insert(
            "c".to_string(),
            make_entity_with_area("c", "fn_c", "src/nav/c.rs", "Navigation"),
        );
        graph.entities.insert(
            "d".to_string(),
            make_entity_with_area("d", "fn_d", "src/nav/d.rs", "Navigation"),
        );
        graph.edges = vec![
            DependencyEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "b".to_string(),
                target: "a".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "c".to_string(),
                target: "d".to_string(),
                kind: EdgeKind::Invokes,
            },
            DependencyEdge {
                source: "d".to_string(),
                target: "c".to_string(),
                kind: EdgeKind::Invokes,
            },
        ];
        graph.refresh_metadata();

        let config = CycleConfig {
            cross_area_only: true,
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);

        assert_eq!(report.cycle_count, 1);
        let ids: Vec<&str> = report.cycles[0].cycle.iter().map(|s| s.as_str()).collect();
        assert!(
            ids.contains(&"a") && ids.contains(&"b"),
            "expected cross-area cycle a-b"
        );
    }

    #[test]
    fn test_min_cycle_length_skips_two_node_cycles() {
        let graph = make_test_graph_two_node_cycle();
        let config = CycleConfig {
            min_cycle_length: 3,
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);
        assert_eq!(
            report.cycle_count, 0,
            "2-node cycle should be excluded by min_cycle_length=3"
        );
    }

    #[test]
    fn test_files_in_cycles_correct_when_include_files_false() {
        let graph = make_test_graph_simple_cycle();
        let config = CycleConfig {
            include_files: false,
            ..Default::default()
        };
        let report = detect_cycles(&graph, &config);
        // 3 entities, each in its own file => 3 files in cycles, regardless of include_files flag.
        assert_eq!(report.files_in_cycles, 3);
    }
}
