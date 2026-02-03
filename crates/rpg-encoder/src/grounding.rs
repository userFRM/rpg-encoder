//! Artifact Grounding — anchor hierarchy to directories and resolve dependency edges.

use rpg_core::graph::{DependencyEdge, EdgeKind, HierarchyNode, RPGraph};
use rpg_core::lca;
use rpg_parser::deps;
use rpg_parser::languages::Language;
use std::collections::HashMap;
use std::path::Path;

/// Ground all hierarchy nodes by computing LCA-based directory paths.
pub fn ground_hierarchy(graph: &mut RPGraph) {
    let entities = &graph.entities;
    for (_, area) in graph.hierarchy.iter_mut() {
        ground_node(area, entities);
    }
}

fn ground_node(node: &mut HierarchyNode, entities: &HashMap<String, rpg_core::graph::Entity>) {
    // First, ground children
    for (_, child) in node.children.iter_mut() {
        ground_node(child, entities);
    }

    // Collect all file paths in this subtree
    let paths = node.collect_file_paths(entities);
    if !paths.is_empty() {
        let lca_dirs = lca::compute_lca(&paths);
        // Store ALL LCA results (multi-LCA per paper Algorithm 1)
        node.grounded_paths = lca_dirs;
    }
}

/// Populate entity deps from AST-extracted raw dependencies.
/// This must be called before `resolve_dependencies` so that entity deps contain
/// the callee/import/inherit names that resolve_dependencies will match to entity IDs.
pub fn populate_entity_deps(graph: &mut RPGraph, project_root: &Path, language: Language) {
    // For each file in the index, extract raw deps and map them to entities
    let file_list: Vec<_> = graph.file_index.keys().cloned().collect();

    for rel_path in &file_list {
        let abs_path = project_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let raw_deps = deps::extract_deps(rel_path, &source, language);

        // Get entity IDs for this file
        let entity_ids = match graph.file_index.get(rel_path) {
            Some(ids) => ids.clone(),
            None => continue,
        };

        // Extract import symbols
        let import_symbols: Vec<String> = raw_deps
            .imports
            .iter()
            .flat_map(|imp| {
                if imp.symbols.is_empty() {
                    // import module → use the last segment as symbol
                    vec![
                        imp.module
                            .rsplit("::")
                            .next()
                            .or_else(|| imp.module.rsplit('.').next())
                            .unwrap_or(&imp.module)
                            .to_string(),
                    ]
                } else {
                    imp.symbols.clone()
                }
            })
            .collect();

        // Map calls: match caller_entity name to actual entity
        for call in &raw_deps.calls {
            // Find the entity whose name matches the caller
            for id in &entity_ids {
                if let Some(entity) = graph.entities.get_mut(id) {
                    let matches = entity.name == call.caller_entity
                        || call.caller_entity.ends_with(&format!(".{}", entity.name))
                        || call.caller_entity.ends_with(&format!("::{}", entity.name));
                    if matches && !entity.deps.invokes.contains(&call.callee) {
                        entity.deps.invokes.push(call.callee.clone());
                    }
                }
            }
        }

        // Map inherits: match child_class to entity
        for inherit in &raw_deps.inherits {
            for id in &entity_ids {
                if let Some(entity) = graph.entities.get_mut(id)
                    && entity.name == inherit.child_class
                    && !entity.deps.inherits.contains(&inherit.parent_class)
                {
                    entity.deps.inherits.push(inherit.parent_class.clone());
                }
            }
        }

        // Scoped import assignment: only assign imports that the entity actually references.
        // If the entity invokes or inherits a symbol that matches an import, assign it.
        // Fall back to broadcast if the entity has no call-site info.
        for id in &entity_ids {
            if let Some(entity) = graph.entities.get_mut(id) {
                let has_callsite_info =
                    !entity.deps.invokes.is_empty() || !entity.deps.inherits.is_empty();

                if has_callsite_info {
                    // Only assign imports that the entity actually references
                    for sym in &import_symbols {
                        let referenced =
                            entity.deps.invokes.contains(sym) || entity.deps.inherits.contains(sym);
                        if referenced && !entity.deps.imports.contains(sym) {
                            entity.deps.imports.push(sym.clone());
                        }
                    }
                } else {
                    // Fallback: broadcast all imports when no call-site info available
                    for sym in &import_symbols {
                        if !entity.deps.imports.contains(sym) {
                            entity.deps.imports.push(sym.clone());
                        }
                    }
                }
            }
        }
    }
}

/// Resolve raw dependency references into proper entity-to-entity edges.
pub fn resolve_dependencies(graph: &mut RPGraph) {
    // Build a qualified name index: "file_display:name" → id
    let qualified_index: HashMap<String, String> = graph
        .entities
        .iter()
        .map(|(id, entity)| {
            let key = format!("{}:{}", entity.file.display(), entity.name);
            (key, id.clone())
        })
        .collect();

    // Build a simple name-to-id index for fallback matching
    let name_to_ids: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (id, entity) in &graph.entities {
            map.entry(entity.name.clone()).or_default().push(id.clone());
        }
        map
    };

    let mut edges = Vec::new();

    // Collect edges from entity deps
    let entity_pairs: Vec<(String, rpg_core::graph::EntityDeps, String)> = graph
        .entities
        .iter()
        .map(|(id, e)| (id.clone(), e.deps.clone(), e.file.display().to_string()))
        .collect();

    for (source_id, deps, source_file) in &entity_pairs {
        // Resolve invokes
        for callee_name in &deps.invokes {
            resolve_dep(
                source_id,
                callee_name,
                source_file,
                EdgeKind::Invokes,
                &qualified_index,
                &name_to_ids,
                &mut edges,
            );
        }

        // Resolve inherits
        for parent_name in &deps.inherits {
            resolve_dep(
                source_id,
                parent_name,
                source_file,
                EdgeKind::Inherits,
                &qualified_index,
                &name_to_ids,
                &mut edges,
            );
        }

        // Resolve imports (match by symbol name within project)
        for import in &deps.imports {
            resolve_dep(
                source_id,
                import,
                source_file,
                EdgeKind::Imports,
                &qualified_index,
                &name_to_ids,
                &mut edges,
            );
        }
    }

    // Build reverse edges in entity deps
    for edge in &edges {
        match edge.kind {
            EdgeKind::Invokes => {
                if let Some(target) = graph.entities.get_mut(&edge.target)
                    && !target.deps.invoked_by.contains(&edge.source)
                {
                    target.deps.invoked_by.push(edge.source.clone());
                }
            }
            EdgeKind::Inherits => {
                if let Some(target) = graph.entities.get_mut(&edge.target)
                    && !target.deps.inherited_by.contains(&edge.source)
                {
                    target.deps.inherited_by.push(edge.source.clone());
                }
            }
            EdgeKind::Imports => {
                if let Some(target) = graph.entities.get_mut(&edge.target)
                    && !target.deps.imported_by.contains(&edge.source)
                {
                    target.deps.imported_by.push(edge.source.clone());
                }
            }
            EdgeKind::Contains => {} // Containment edges don't have reverse dep entries
        }
    }

    graph.edges = edges;
}

/// Resolve a single dependency using qualified lookup first, then name-based fallback.
fn resolve_dep(
    source_id: &str,
    target_name: &str,
    source_file: &str,
    kind: EdgeKind,
    qualified_index: &HashMap<String, String>,
    name_to_ids: &HashMap<String, Vec<String>>,
    edges: &mut Vec<DependencyEdge>,
) {
    // Try qualified lookup first: same file
    let qualified_key = format!("{}:{}", source_file, target_name);
    if let Some(target_id) = qualified_index.get(&qualified_key)
        && target_id != source_id
    {
        edges.push(DependencyEdge {
            source: source_id.to_string(),
            target: target_id.clone(),
            kind,
        });
        return;
    }

    // Fallback: name-based lookup
    if let Some(target_ids) = name_to_ids.get(target_name) {
        for target_id in target_ids {
            if target_id != source_id {
                edges.push(DependencyEdge {
                    source: source_id.to_string(),
                    target: target_id.clone(),
                    kind,
                });
            }
        }
    }
}
