//! FetchNode: precise entity metadata and source retrieval.

use anyhow::Result;
use rpg_core::graph::{Entity, HierarchyNode, RPGraph};
use std::fs;

/// Detailed entity information returned by FetchNode.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub entity: Entity,
    pub source_code: Option<String>,
    pub hierarchy_context: Vec<String>, // sibling entities in the same hierarchy node
}

/// Detailed hierarchy node information returned by FetchNode for V_H nodes.
#[derive(Debug, Clone)]
pub struct HierarchyFetchResult {
    pub node: HierarchyNode,
    pub child_names: Vec<String>,
    pub entity_count: usize,
}

/// Result of a fetch operation — either a V_L entity or a V_H hierarchy node.
#[derive(Debug, Clone)]
pub enum FetchOutput {
    Entity(FetchResult),
    Hierarchy(HierarchyFetchResult),
}

/// Fetch full details for an entity or hierarchy node by ID.
/// For V_L entities: includes source code from disk.
/// For V_H hierarchy nodes (IDs starting with "h:"): includes children and aggregated features.
pub fn fetch(
    graph: &RPGraph,
    entity_id: &str,
    project_root: &std::path::Path,
) -> Result<FetchOutput> {
    // Try V_L entity first
    if let Some(entity) = graph.get_entity(entity_id) {
        let entity = entity.clone();
        let source_code = read_entity_source(project_root, &entity);
        let hierarchy_context = find_siblings(graph, &entity);
        return Ok(FetchOutput::Entity(FetchResult {
            entity,
            source_code,
            hierarchy_context,
        }));
    }

    // Try V_H hierarchy node
    if entity_id.starts_with("h:")
        && let Some(node) = graph.find_hierarchy_node_by_id(entity_id)
    {
        let child_names: Vec<String> = node.children.keys().cloned().collect();
        let entity_count = node.entity_count();
        return Ok(FetchOutput::Hierarchy(HierarchyFetchResult {
            node: node.clone(),
            child_names,
            entity_count,
        }));
    }

    Err(anyhow::anyhow!("entity not found: {}", entity_id))
}

fn read_entity_source(project_root: &std::path::Path, entity: &Entity) -> Option<String> {
    let file_path = project_root.join(&entity.file);
    let content = fs::read_to_string(&file_path).ok()?;

    let lines: Vec<&str> = content.lines().collect();
    let start = entity.line_start.saturating_sub(1);
    let end = entity.line_end.min(lines.len());

    Some(lines[start..end].join("\n"))
}

fn find_siblings(graph: &RPGraph, entity: &Entity) -> Vec<String> {
    if entity.hierarchy_path.is_empty() {
        return Vec::new();
    }

    graph
        .entities
        .values()
        .filter(|e| e.hierarchy_path == entity.hierarchy_path && e.id != entity.id)
        .map(|e| e.id.clone())
        .collect()
}
