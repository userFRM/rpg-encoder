//! Artifact Grounding — anchor hierarchy to directories and resolve dependency edges.

use rpg_core::graph::{DependencyEdge, EdgeKind, HierarchyNode, RPGraph};
use rpg_core::lca;
use rpg_parser::deps;
use rpg_parser::languages::Language;
use rpg_parser::paradigms::defs::ParadigmDef;
use rpg_parser::paradigms::query_engine::QueryCache;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Paradigm context for TOML-driven dependency extraction.
/// When provided to `populate_entity_deps`, the TOML dep pipeline
/// (query engine + built-in features) runs after base `extract_deps()`.
pub struct ParadigmContext<'a> {
    pub active_defs: Vec<&'a ParadigmDef>,
    pub qcache: &'a QueryCache,
}

/// Ground all hierarchy nodes by computing LCA-based directory paths.
pub fn ground_hierarchy(graph: &mut RPGraph) {
    let entities = &graph.entities;
    for area in graph.hierarchy.values_mut() {
        ground_node(area, entities);
    }
}

fn ground_node(node: &mut HierarchyNode, entities: &BTreeMap<String, rpg_core::graph::Entity>) {
    // First, ground children
    for child in node.children.values_mut() {
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
///
/// When `broadcast_imports` is false, entities without call-site info (no invokes/inherits)
/// get no import edges rather than all file-level imports being broadcast to them.
///
/// When `changed_files` is `Some`, only entities in those files are re-populated (their
/// forward deps are cleared first). When `None`, all files are processed.
pub fn populate_entity_deps(
    graph: &mut RPGraph,
    project_root: &Path,
    broadcast_imports: bool,
    changed_files: Option<&[std::path::PathBuf]>,
    paradigm_ctx: Option<&ParadigmContext<'_>>,
) {
    // Scope to changed files or all files
    let file_list: Vec<_> = match changed_files {
        Some(files) => files.to_vec(),
        None => graph.file_index.keys().cloned().collect(),
    };

    // Clear forward deps only for entities in scoped files
    for rel_path in &file_list {
        if let Some(ids) = graph.file_index.get(rel_path) {
            for id in ids.clone() {
                if let Some(entity) = graph.entities.get_mut(&id) {
                    entity.deps.clear_forward();
                }
            }
        }
    }

    for rel_path in &file_list {
        let file_lang = rel_path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);
        let Some(language) = file_lang else {
            continue;
        };
        let abs_path = project_root.join(rel_path);
        let Ok(source) = std::fs::read_to_string(&abs_path) else {
            continue;
        };

        let mut raw_deps = deps::extract_deps(rel_path, &source, language);

        // TOML-driven paradigm dep pipeline: dep queries + builtin features
        if let Some(ctx) = paradigm_ctx {
            let scopes = deps::build_scopes(&source, language);

            rpg_parser::paradigms::query_engine::execute_dep_queries(
                ctx.qcache,
                &ctx.active_defs,
                rel_path,
                &source,
                language,
                &scopes,
                &mut raw_deps,
            );

            // Collect entity snapshot for builtin dep features
            let raw_entities: Vec<rpg_parser::entities::RawEntity> = graph
                .file_index
                .get(rel_path)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|id| graph.entities.get(id))
                .map(|e| rpg_parser::entities::RawEntity {
                    name: e.name.clone(),
                    kind: e.kind,
                    file: e.file.clone(),
                    line_start: e.line_start,
                    line_end: e.line_end,
                    parent_class: e.parent_class.clone(),
                    source_text: String::new(),
                })
                .collect();

            rpg_parser::paradigms::features::apply_builtin_dep_features(
                &ctx.active_defs,
                rel_path,
                &source,
                language,
                &raw_entities,
                &mut raw_deps,
            );
        }

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

        // Map call-like deps (calls, renders, reads_state, writes_state, dispatches)
        // generically: match caller_entity name to actual entity, push to correct dep vector.
        for (edge_kind, call_deps) in raw_deps.call_dep_vectors() {
            for call in call_deps {
                for id in &entity_ids {
                    if let Some(entity) = graph.entities.get_mut(id) {
                        let matches = entity.name == call.caller_entity
                            || call.caller_entity.ends_with(&format!(".{}", entity.name))
                            || call.caller_entity.ends_with(&format!("::{}", entity.name));
                        if matches {
                            push_forward_dep(&mut entity.deps, edge_kind, &call.callee);
                        }
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

        // Map composes: assign only to Module entities (barrel re-exports are file-level)
        for compose in &raw_deps.composes {
            for id in &entity_ids {
                if let Some(entity) = graph.entities.get_mut(id)
                    && entity.kind == rpg_core::graph::EntityKind::Module
                    && !entity.deps.composes.contains(&compose.target_name)
                {
                    entity.deps.composes.push(compose.target_name.clone());
                }
            }
        }

        // Scoped import assignment: only assign imports that the entity actually references.
        // If the entity invokes or inherits a symbol that matches an import, assign it.
        // Fall back to broadcast if the entity has no call-site info.
        for id in &entity_ids {
            if let Some(entity) = graph.entities.get_mut(id) {
                if entity.deps.has_callsite_info() {
                    for sym in &import_symbols {
                        if entity.deps.references_symbol(sym) && !entity.deps.imports.contains(sym)
                        {
                            entity.deps.imports.push(sym.clone());
                        }
                    }
                } else if broadcast_imports {
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

/// Push a callee to the correct forward dep vector for the given edge kind.
fn push_forward_dep(deps: &mut rpg_core::graph::EntityDeps, kind: EdgeKind, callee: &str) {
    let vec = match kind {
        EdgeKind::Invokes => &mut deps.invokes,
        EdgeKind::Renders => &mut deps.renders,
        EdgeKind::ReadsState => &mut deps.reads_state,
        EdgeKind::WritesState => &mut deps.writes_state,
        EdgeKind::Dispatches => &mut deps.dispatches,
        // These edge kinds are not call-like and are handled separately
        EdgeKind::Imports | EdgeKind::Inherits | EdgeKind::Composes | EdgeKind::Contains => return,
    };
    if !vec.contains(&callee.to_string()) {
        vec.push(callee.to_string());
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
        // Resolve all forward dep kinds generically
        for (edge_kind, dep_names) in deps.forward_deps() {
            for target_name in dep_names {
                resolve_dep(
                    source_id,
                    target_name,
                    source_file,
                    edge_kind,
                    &qualified_index,
                    &name_to_ids,
                    &mut edges,
                );
            }
        }
    }

    // Clear all reverse dep vectors before repopulating (prevents stale refs on re-resolve)
    for entity in graph.entities.values_mut() {
        entity.deps.clear_reverse();
    }

    // Build reverse edges in entity deps
    for edge in &edges {
        if edge.kind == EdgeKind::Contains {
            continue;
        }
        if let Some(target) = graph.entities.get_mut(&edge.target) {
            target.deps.push_reverse(edge.kind, edge.source.clone());
        }
    }

    graph.edges = edges;
}

/// Resolve a single dependency using qualified lookup first, then import-aware fallback.
///
/// The fallback only creates a cross-file edge if the target name is unambiguous
/// (exactly one entity with that name across the entire graph). This avoids false
/// edges for common names like `new`, `parse`, `build`, `run`.
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

    // Fallback: name-based lookup — only if unambiguous (exactly one match outside this file)
    if let Some(target_ids) = name_to_ids.get(target_name) {
        let cross_file_targets: Vec<&String> = target_ids
            .iter()
            .filter(|id| *id != source_id && !id.starts_with(&format!("{}:", source_file)))
            .collect();

        // Only create edge if there's exactly one candidate — refuse to guess among multiples
        if cross_file_targets.len() == 1 {
            edges.push(DependencyEdge {
                source: source_id.to_string(),
                target: cross_file_targets[0].clone(),
                kind,
            });
        }
    }
}
