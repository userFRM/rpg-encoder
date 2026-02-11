//! Reconstruction planning utilities for paper-style topological execution.
//!
//! The paper's reconstruction setting executes repository units in dependency-safe
//! topological order, then groups adjacent units into semantically coherent batches.

use rpg_core::graph::{EdgeKind, EntityKind, RPGraph};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Reconstruction scheduler options.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReconstructionOptions {
    /// Maximum entities per execution batch.
    pub max_batch_size: usize,
    /// Whether to include file-level Module entities in the schedule.
    pub include_modules: bool,
}

impl Default for ReconstructionOptions {
    fn default() -> Self {
        Self {
            max_batch_size: 8,
            include_modules: false,
        }
    }
}

/// A single reconstruction execution batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionBatch {
    /// 1-based batch index in execution order.
    pub batch_index: usize,
    /// Dominant top-level functional area for this batch.
    pub area: String,
    /// Entity IDs in dependency-safe execution order.
    pub entity_ids: Vec<String>,
}

/// End-to-end reconstruction plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionPlan {
    /// Global dependency-safe execution order.
    pub topological_order: Vec<String>,
    /// Execution batches derived from the order.
    pub batches: Vec<ReconstructionBatch>,
}

/// Build a dependency-safe topological execution order.
///
/// The dependency edges are represented as `source -> target` where source depends
/// on target. For reconstruction execution we therefore invert traversal to ensure
/// prerequisites (`target`) come before dependents (`source`).
///
/// When cycles exist, remaining nodes are appended in deterministic lexical order.
pub fn build_topological_execution_order(graph: &RPGraph, include_modules: bool) -> Vec<String> {
    let nodes: BTreeSet<String> = graph
        .entities
        .iter()
        .filter(|(_, entity)| include_modules || entity.kind != EntityKind::Module)
        .map(|(id, _)| id.clone())
        .collect();

    if nodes.is_empty() {
        return Vec::new();
    }

    let mut indegree: HashMap<String, usize> = nodes
        .iter()
        .map(|id| (id.clone(), 0usize))
        .collect::<HashMap<_, _>>();
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen_edges: HashSet<(String, String)> = HashSet::new();

    for edge in &graph.edges {
        if !is_dependency_edge(edge.kind) {
            continue;
        }
        if !nodes.contains(&edge.source) || !nodes.contains(&edge.target) {
            continue;
        }
        if edge.source == edge.target {
            continue;
        }

        // prerequisite -> dependent for topological scheduling
        let prerequisite = edge.target.clone();
        let dependent = edge.source.clone();

        if !seen_edges.insert((prerequisite.clone(), dependent.clone())) {
            continue;
        }

        if let Some(v) = indegree.get_mut(&dependent) {
            *v += 1;
        }
        dependents.entry(prerequisite).or_default().push(dependent);
    }

    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter_map(|(id, deg)| if *deg == 0 { Some(id.clone()) } else { None })
        .collect();
    let mut order = Vec::with_capacity(nodes.len());

    while let Some(node) = ready.pop_first() {
        order.push(node.clone());
        if let Some(nexts) = dependents.get(&node) {
            let mut sorted = nexts.clone();
            sorted.sort();
            sorted.dedup();
            for dependent in sorted {
                if let Some(d) = indegree.get_mut(&dependent)
                    && *d > 0
                {
                    *d -= 1;
                    if *d == 0 {
                        ready.insert(dependent);
                    }
                }
            }
        }
    }

    // Cycles: append remaining nodes deterministically.
    if order.len() < nodes.len() {
        let scheduled: HashSet<&str> = order.iter().map(String::as_str).collect();
        let mut remaining: Vec<String> = nodes
            .iter()
            .filter(|id| !scheduled.contains(id.as_str()))
            .cloned()
            .collect();
        remaining.sort();
        order.extend(remaining);
    }

    order
}

/// Build a paper-style reconstruction plan with topological ordering + semantic batching.
pub fn schedule_reconstruction(
    graph: &RPGraph,
    options: ReconstructionOptions,
) -> ReconstructionPlan {
    let max_batch = options.max_batch_size.max(1);
    let order = build_topological_execution_order(graph, options.include_modules);

    let mut batches = Vec::new();
    let mut current_area = String::new();
    let mut current_ids: Vec<String> = Vec::new();

    for entity_id in &order {
        let area = graph
            .entities
            .get(entity_id)
            .map(|e| top_level_area(&e.hierarchy_path))
            .unwrap_or_else(|| "unscoped".to_string());

        let area_changed = !current_ids.is_empty() && area != current_area;
        let size_hit = current_ids.len() >= max_batch;

        if area_changed || size_hit {
            batches.push(ReconstructionBatch {
                batch_index: batches.len() + 1,
                area: current_area.clone(),
                entity_ids: std::mem::take(&mut current_ids),
            });
        }

        if current_ids.is_empty() {
            current_area = area;
        }
        current_ids.push(entity_id.clone());
    }

    if !current_ids.is_empty() {
        batches.push(ReconstructionBatch {
            batch_index: batches.len() + 1,
            area: current_area,
            entity_ids: current_ids,
        });
    }

    ReconstructionPlan {
        topological_order: order,
        batches,
    }
}

fn is_dependency_edge(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Imports
            | EdgeKind::Invokes
            | EdgeKind::Inherits
            | EdgeKind::Composes
            | EdgeKind::Renders
            | EdgeKind::ReadsState
            | EdgeKind::WritesState
            | EdgeKind::Dispatches
    )
}

fn top_level_area(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "unscoped".to_string();
    }
    trimmed
        .split('/')
        .find(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unscoped".to_string())
}
