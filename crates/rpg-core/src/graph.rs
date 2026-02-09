//! Graph data model for the Repository Planning Graph (RPG).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// The complete Repository Planning Graph: G = (V, E) where V = V_H ∪ V_L.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RPGraph {
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub base_commit: Option<String>,
    pub metadata: GraphMetadata,
    /// V_H: high-level semantic hierarchy nodes.
    pub hierarchy: BTreeMap<String, HierarchyNode>,
    /// V_L: low-level code entities (functions, classes, methods).
    pub entities: BTreeMap<String, Entity>,
    /// E = E_dep ∪ E_feature: all edges (dependency + containment).
    pub edges: Vec<DependencyEdge>,
    /// Reverse index: file path → entity IDs in that file.
    pub file_index: BTreeMap<PathBuf, Vec<String>>,
    /// Performance index: entity ID → edge indices in `edges` vec.
    /// Rebuilt on load and after edge mutations via `rebuild_edge_index()`.
    #[serde(skip)]
    pub edge_index: HashMap<String, Vec<usize>>,
    /// Performance index: hierarchy node ID → key path to reach the node.
    /// Rebuilt during `assign_hierarchy_ids()` or via `rebuild_hierarchy_index()`.
    #[serde(skip)]
    pub hierarchy_node_index: HashMap<String, Vec<String>>,
}

/// Aggregate statistics and metadata for the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMetadata {
    pub language: String,
    /// All languages indexed in this graph (ordered by file count).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    pub total_files: usize,
    pub total_entities: usize,
    pub functional_areas: usize,
    pub total_edges: usize,
    pub dependency_edges: usize,
    pub containment_edges: usize,
    /// Number of entities that have been semantically lifted.
    #[serde(default)]
    pub lifted_entities: usize,
    /// Whether the hierarchy is LLM-generated (true) or file-path structural (false).
    #[serde(default)]
    pub semantic_hierarchy: bool,
    /// High-level architectural summary of the repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_summary: Option<String>,
    /// Detected paradigms/frameworks (e.g., "react", "nextjs", "redux").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paradigms: Vec<String>,
}

/// A code entity (V_L node): function, class, or method.
/// Each node v = (f, m) with semantic features f and structural metadata m.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub kind: EntityKind,
    pub name: String,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub parent_class: Option<String>,
    /// Semantic features f: atomic verb-object phrases.
    pub semantic_features: Vec<String>,
    pub hierarchy_path: String,
    pub deps: EntityDeps,
}

/// Resolved dependency relationships for an entity (forward and reverse).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityDeps {
    pub imports: Vec<String>,
    pub invokes: Vec<String>,
    pub inherits: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub composes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renders: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_state: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes_state: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatches: Vec<String>,
    pub imported_by: Vec<String>,
    pub invoked_by: Vec<String>,
    pub inherited_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub composed_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rendered_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state_read_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state_written_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatched_by: Vec<String>,
}

impl EntityDeps {
    /// Clear all forward dependency vectors.
    pub fn clear_forward(&mut self) {
        self.imports.clear();
        self.invokes.clear();
        self.inherits.clear();
        self.composes.clear();
        self.renders.clear();
        self.reads_state.clear();
        self.writes_state.clear();
        self.dispatches.clear();
    }

    /// Clear all reverse dependency vectors.
    pub fn clear_reverse(&mut self) {
        self.imported_by.clear();
        self.invoked_by.clear();
        self.inherited_by.clear();
        self.composed_by.clear();
        self.rendered_by.clear();
        self.state_read_by.clear();
        self.state_written_by.clear();
        self.dispatched_by.clear();
    }

    /// Iterate all forward dep vectors with their edge kinds.
    pub fn forward_deps(&self) -> [(EdgeKind, &Vec<String>); 8] {
        [
            (EdgeKind::Imports, &self.imports),
            (EdgeKind::Invokes, &self.invokes),
            (EdgeKind::Inherits, &self.inherits),
            (EdgeKind::Composes, &self.composes),
            (EdgeKind::Renders, &self.renders),
            (EdgeKind::ReadsState, &self.reads_state),
            (EdgeKind::WritesState, &self.writes_state),
            (EdgeKind::Dispatches, &self.dispatches),
        ]
    }

    /// Push a source ID to the correct reverse dep vector for the given edge kind.
    pub fn push_reverse(&mut self, kind: EdgeKind, source_id: String) {
        let vec = match kind {
            EdgeKind::Imports => &mut self.imported_by,
            EdgeKind::Invokes => &mut self.invoked_by,
            EdgeKind::Inherits => &mut self.inherited_by,
            EdgeKind::Composes => &mut self.composed_by,
            EdgeKind::Renders => &mut self.rendered_by,
            EdgeKind::ReadsState => &mut self.state_read_by,
            EdgeKind::WritesState => &mut self.state_written_by,
            EdgeKind::Dispatches => &mut self.dispatched_by,
            EdgeKind::Contains => return,
        };
        if !vec.contains(&source_id) {
            vec.push(source_id);
        }
    }

    /// Whether this entity has any call-site dependency info (invokes, inherits, or
    /// paradigm-specific edges like renders/reads_state/writes_state/dispatches).
    pub fn has_callsite_info(&self) -> bool {
        !self.invokes.is_empty()
            || !self.inherits.is_empty()
            || !self.renders.is_empty()
            || !self.reads_state.is_empty()
            || !self.writes_state.is_empty()
            || !self.dispatches.is_empty()
    }

    /// Whether any forward dep vector (excluding imports) contains the given symbol.
    pub fn references_symbol(&self, sym: &str) -> bool {
        self.invokes.iter().any(|s| s == sym)
            || self.inherits.iter().any(|s| s == sym)
            || self.renders.iter().any(|s| s == sym)
            || self.reads_state.iter().any(|s| s == sym)
            || self.writes_state.iter().any(|s| s == sym)
            || self.dispatches.iter().any(|s| s == sym)
    }
}

/// A node in the semantic hierarchy tree (V_H node).
/// Unified with Entity as a proper graph node: has id, semantic_features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyNode {
    /// Unique ID: "h:Area/Category/Subcategory"
    pub id: String,
    pub name: String,
    /// LCA-grounded directory paths for this subtree.
    pub grounded_paths: Vec<PathBuf>,
    pub children: BTreeMap<String, HierarchyNode>,
    pub entities: Vec<String>,
    /// Aggregated semantic features from all entities in this subtree.
    pub semantic_features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl HierarchyNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            name: name.into(),
            grounded_paths: Vec::new(),
            children: BTreeMap::new(),
            entities: Vec::new(),
            semantic_features: Vec::new(),
            description: None,
        }
    }

    /// Check if this node and all children are empty of entities.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.children.values().all(|c| c.is_empty())
    }

    /// Recursively count all entities in this subtree.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
            + self
                .children
                .values()
                .map(|c| c.entity_count())
                .sum::<usize>()
    }

    /// Collect all entity IDs in this subtree.
    pub fn all_entity_ids(&self) -> Vec<String> {
        let mut ids = self.entities.clone();
        for child in self.children.values() {
            ids.extend(child.all_entity_ids());
        }
        ids
    }

    /// Collect all file paths from entities in this subtree.
    pub fn collect_file_paths(&self, entities: &BTreeMap<String, Entity>) -> Vec<PathBuf> {
        let ids = self.all_entity_ids();
        ids.iter()
            .filter_map(|id| entities.get(id).map(|e| e.file.clone()))
            .collect()
    }

    /// Prune empty children recursively. Returns true if this node itself is now empty.
    pub fn prune_empty(&mut self) -> bool {
        let empty_keys: Vec<String> = self
            .children
            .iter_mut()
            .filter_map(|(k, v)| {
                if v.prune_empty() {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect();
        for key in empty_keys {
            self.children.remove(&key);
        }
        self.is_empty()
    }

    /// Assign IDs to this node and all descendants based on hierarchy path.
    pub fn assign_ids(&mut self, path_prefix: &str) {
        let my_path = if path_prefix.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", path_prefix, self.name)
        };
        self.id = format!("h:{}", my_path);
        for child in self.children.values_mut() {
            child.assign_ids(&my_path);
        }
    }

    /// Bottom-up aggregation: collect deduplicated semantic features from all children.
    pub fn aggregate_features(&mut self, entities: &BTreeMap<String, Entity>) {
        for child in self.children.values_mut() {
            child.aggregate_features(entities);
        }
        let mut all: Vec<String> = Vec::new();
        for eid in &self.entities {
            if let Some(entity) = entities.get(eid) {
                all.extend(entity.semantic_features.iter().cloned());
            }
        }
        for child in self.children.values() {
            all.extend(child.semantic_features.iter().cloned());
        }
        all.sort();
        all.dedup();
        self.semantic_features = all;
    }
}

/// An edge in the unified edge set E = E_dep ∪ E_feature.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
}

/// The kind of relationship between two nodes in the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// E_dep: import/use dependency.
    Imports,
    /// E_dep: function call or method invocation.
    Invokes,
    /// E_dep: class inheritance or trait implementation.
    Inherits,
    /// E_dep: composition/aggregation (e.g., module re-exports, class composition).
    Composes,
    /// E_dep: React/JSX render dependency (component/page renders component).
    Renders,
    /// E_dep: state read dependency (selector/store reads).
    ReadsState,
    /// E_dep: state write dependency (setters/store updates).
    WritesState,
    /// E_dep: state/event dispatch dependency.
    Dispatches,
    /// E_feature: hierarchy containment (parent → child).
    Contains,
}

/// The kind of code entity extracted from source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Function,
    Class,
    Method,
    Page,
    Layout,
    Component,
    Hook,
    Store,
    Module,
    Controller,
    Model,
    Service,
    Middleware,
    Route,
    Test,
}

impl RPGraph {
    /// Create a new empty graph for the given language.
    pub fn new(language: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            version: "2.0.0".to_string(),
            created_at: now,
            updated_at: now,
            base_commit: None,
            metadata: GraphMetadata {
                language: language.into(),
                languages: Vec::new(),
                total_files: 0,
                total_entities: 0,
                functional_areas: 0,
                total_edges: 0,
                dependency_edges: 0,
                containment_edges: 0,
                lifted_entities: 0,
                semantic_hierarchy: false,
                repo_summary: None,
                paradigms: Vec::new(),
            },
            hierarchy: BTreeMap::new(),
            entities: BTreeMap::new(),
            edges: Vec::new(),
            file_index: BTreeMap::new(),
            edge_index: HashMap::new(),
            hierarchy_node_index: HashMap::new(),
        }
    }

    /// Recompute metadata from current state and rebuild performance indexes.
    pub fn refresh_metadata(&mut self) {
        self.metadata.total_entities = self.entities.len();
        self.metadata.total_files = self.file_index.len();
        self.metadata.functional_areas = self.hierarchy.len();
        self.metadata.total_edges = self.edges.len();
        self.metadata.dependency_edges = self
            .edges
            .iter()
            .filter(|e| e.kind != EdgeKind::Contains)
            .count();
        self.metadata.containment_edges = self
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Contains)
            .count();
        self.metadata.lifted_entities = self
            .entities
            .values()
            .filter(|e| !e.semantic_features.is_empty())
            .count();
        self.updated_at = Utc::now();
        self.rebuild_edge_index();
    }

    /// Rebuild the edge index from the current edge list.
    /// Call after bulk edge mutations (grounding, containment materialization).
    pub fn rebuild_edge_index(&mut self) {
        self.edge_index.clear();
        for (i, edge) in self.edges.iter().enumerate() {
            self.edge_index
                .entry(edge.source.clone())
                .or_default()
                .push(i);
            self.edge_index
                .entry(edge.target.clone())
                .or_default()
                .push(i);
        }
    }

    /// Rebuild the hierarchy node index for O(1) lookups by node ID.
    /// Called automatically by `assign_hierarchy_ids()`.
    pub fn rebuild_hierarchy_index(&mut self) {
        self.hierarchy_node_index.clear();
        for (area_name, area) in &self.hierarchy {
            Self::index_hierarchy_node(
                area,
                std::slice::from_ref(area_name),
                &mut self.hierarchy_node_index,
            );
        }
    }

    fn index_hierarchy_node(
        node: &HierarchyNode,
        path: &[String],
        index: &mut HashMap<String, Vec<String>>,
    ) {
        if !node.id.is_empty() {
            index.insert(node.id.clone(), path.to_vec());
        }
        for (child_name, child) in &node.children {
            let mut child_path = path.to_vec();
            child_path.push(child_name.clone());
            Self::index_hierarchy_node(child, &child_path, index);
        }
    }

    /// Return (lifted, total) entity counts.
    pub fn lifting_coverage(&self) -> (usize, usize) {
        let non_module = self
            .entities
            .values()
            .filter(|e| e.kind != EntityKind::Module);
        let total = non_module.clone().count();
        let lifted = non_module
            .filter(|e| !e.semantic_features.is_empty())
            .count();
        (lifted, total)
    }

    /// Return unlifted non-module entities grouped by file path.
    /// Each entry is (file_display_string, Vec<entity_id>), sorted by count descending.
    pub fn unlifted_by_file(&self) -> Vec<(String, Vec<String>)> {
        let mut by_file: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (id, entity) in &self.entities {
            if entity.kind != EntityKind::Module && entity.semantic_features.is_empty() {
                by_file
                    .entry(entity.file.to_string_lossy().to_string())
                    .or_default()
                    .push(id.clone());
            }
        }
        let mut result: Vec<(String, Vec<String>)> = by_file.into_iter().collect();
        result.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        result
    }

    /// Return per-area lifting coverage: Vec of (area_name, lifted, total).
    pub fn area_coverage(&self) -> Vec<(String, usize, usize)> {
        let mut result = Vec::new();
        for (area_name, node) in &self.hierarchy {
            let ids = node.all_entity_ids();
            let total = ids.len();
            let lifted = ids
                .iter()
                .filter(|id| {
                    self.entities
                        .get(*id)
                        .is_some_and(|e| !e.semantic_features.is_empty())
                })
                .count();
            result.push((area_name.clone(), lifted, total));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    /// Build a hierarchy from file paths (structural fallback when no LLM is available).
    /// Groups entities by directory structure: top-dir / sub-dir / file-stem.
    pub fn build_file_path_hierarchy(&mut self) {
        self.hierarchy.clear();
        self.metadata.semantic_hierarchy = false;

        let entity_ids: Vec<String> = self.entities.keys().cloned().collect();
        for id in &entity_ids {
            let entity = &self.entities[id];
            let components: Vec<&str> = entity
                .file
                .components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => s.to_str(),
                    _ => None,
                })
                .collect();

            let path = match components.len() {
                0 => continue,
                1 => {
                    // Single file at root: use file stem
                    let stem = components[0]
                        .rsplit_once('.')
                        .map_or(components[0], |(s, _)| s);
                    stem.to_string()
                }
                2 => {
                    // dir/file.ext → dir/file_stem
                    let stem = components[1]
                        .rsplit_once('.')
                        .map_or(components[1], |(s, _)| s);
                    format!("{}/{}", components[0], stem)
                }
                _ => {
                    // dir/subdir/.../file.ext → dir/subdir/file_stem
                    let last = components.last().unwrap();
                    let stem = last.rsplit_once('.').map_or(*last, |(s, _)| s);
                    format!("{}/{}/{}", components[0], components[1], stem)
                }
            };

            // Update entity's hierarchy_path
            if let Some(e) = self.entities.get_mut(id) {
                e.hierarchy_path = path.clone();
            }
            self.insert_into_hierarchy(&path, id);
        }
    }

    pub fn insert_entity(&mut self, entity: Entity) {
        let file = entity.file.clone();
        let id = entity.id.clone();
        self.entities.insert(id.clone(), entity);
        self.file_index.entry(file).or_default().push(id);
    }

    pub fn remove_entity(&mut self, id: &str) -> Option<Entity> {
        if let Some(entity) = self.entities.remove(id) {
            if let Some(ids) = self.file_index.get_mut(&entity.file) {
                ids.retain(|i| i != id);
                if ids.is_empty() {
                    self.file_index.remove(&entity.file);
                }
            }
            self.edges.retain(|e| e.source != id && e.target != id);
            self.remove_entity_from_hierarchy(id);
            Some(entity)
        } else {
            None
        }
    }

    /// Remove an entity from whichever hierarchy node contains it.
    pub fn remove_entity_from_hierarchy(&mut self, entity_id: &str) {
        for area in self.hierarchy.values_mut() {
            Self::remove_from_subtree(area, entity_id);
        }
        let empty_keys: Vec<String> = self
            .hierarchy
            .iter_mut()
            .filter_map(|(k, v)| {
                if v.prune_empty() {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect();
        for key in empty_keys {
            self.hierarchy.remove(&key);
        }
    }

    fn remove_from_subtree(node: &mut HierarchyNode, entity_id: &str) {
        node.entities.retain(|id| id != entity_id);
        for child in node.children.values_mut() {
            Self::remove_from_subtree(child, entity_id);
        }
    }

    pub fn insert_into_hierarchy(&mut self, hierarchy_path: &str, entity_id: &str) {
        let parts: Vec<&str> = hierarchy_path.split('/').collect();
        if parts.is_empty() {
            return;
        }

        let area = self
            .hierarchy
            .entry(parts[0].to_string())
            .or_insert_with(|| HierarchyNode::new(parts[0]));

        let mut current = area;
        for &part in &parts[1..] {
            current = current
                .children
                .entry(part.to_string())
                .or_insert_with(|| HierarchyNode::new(part));
        }
        if !current.entities.contains(&entity_id.to_string()) {
            current.entities.push(entity_id.to_string());
        }
    }

    pub fn get_entity(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    /// Find a hierarchy node (V_H) by its ID (e.g., "h:Auth/login/validation").
    pub fn find_hierarchy_node_by_id(&self, id: &str) -> Option<&HierarchyNode> {
        // Use the index for O(1) lookup when available
        if let Some(path) = self.hierarchy_node_index.get(id) {
            if path.is_empty() {
                return None;
            }
            let mut current = self.hierarchy.get(&path[0])?;
            for key in &path[1..] {
                current = current.children.get(key)?;
            }
            if current.id == id {
                return Some(current);
            }
        }
        // Fallback to recursive traversal if index not built
        for area in self.hierarchy.values() {
            if let Some(node) = Self::find_node_by_id_recursive(area, id) {
                return Some(node);
            }
        }
        None
    }

    fn find_node_by_id_recursive<'a>(
        node: &'a HierarchyNode,
        id: &str,
    ) -> Option<&'a HierarchyNode> {
        if node.id == id {
            return Some(node);
        }
        for child in node.children.values() {
            if let Some(found) = Self::find_node_by_id_recursive(child, id) {
                return Some(found);
            }
        }
        None
    }

    /// Get display info for any node (V_L entity or V_H hierarchy node).
    /// Returns (name, description) for unified display.
    pub fn get_node_display_info(&self, id: &str) -> Option<(String, String)> {
        if let Some(entity) = self.entities.get(id) {
            return Some((
                entity.name.clone(),
                format!(
                    "{} in {}",
                    format!("{:?}", entity.kind).to_lowercase(),
                    entity.file.display()
                ),
            ));
        }
        if let Some(node) = self.find_hierarchy_node_by_id(id) {
            let desc = if node.grounded_paths.is_empty() {
                format!("hierarchy node ({} entities)", node.entity_count())
            } else {
                let paths: Vec<String> = node
                    .grounded_paths
                    .iter()
                    .take(3)
                    .map(|p| p.display().to_string())
                    .collect();
                format!(
                    "hierarchy node ({} entities, grounded: {})",
                    node.entity_count(),
                    paths.join(", ")
                )
            };
            return Some((node.name.clone(), desc));
        }
        None
    }

    pub fn edges_for(&self, entity_id: &str) -> Vec<&DependencyEdge> {
        if let Some(indices) = self.edge_index.get(entity_id) {
            indices.iter().filter_map(|&i| self.edges.get(i)).collect()
        } else {
            // Fallback to linear scan if index not built
            self.edges
                .iter()
                .filter(|e| e.source == entity_id || e.target == entity_id)
                .collect()
        }
    }

    /// Assign IDs to all hierarchy nodes and rebuild the hierarchy index.
    pub fn assign_hierarchy_ids(&mut self) {
        for area in self.hierarchy.values_mut() {
            area.assign_ids("");
        }
        self.rebuild_hierarchy_index();
    }

    /// Aggregate semantic features bottom-up through the hierarchy.
    /// Uses split borrows to avoid cloning the entire entity map.
    pub fn aggregate_hierarchy_features(&mut self) {
        let Self {
            entities,
            hierarchy,
            ..
        } = self;
        for area in hierarchy.values_mut() {
            area.aggregate_features(entities);
        }
    }

    /// Create Module entities for each file in the graph (paper §3.1: "files, classes, and functions").
    /// Must be called after all entities have been inserted.
    pub fn create_module_entities(&mut self) {
        let files: Vec<PathBuf> = self.file_index.keys().cloned().collect();
        for file in files {
            let module_name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("module")
                .to_string();
            let module_id = format!("{}:{}", file.display(), module_name);

            // Skip if a Module entity already exists for this file
            if self.entities.contains_key(&module_id) {
                continue;
            }

            // Get the line range from existing entities in this file
            let line_end = self.file_index.get(&file).map_or(1, |ids| {
                ids.iter()
                    .filter_map(|id| self.entities.get(id))
                    .map(|e| e.line_end)
                    .max()
                    .unwrap_or(1)
            });

            let entity = Entity {
                id: module_id.clone(),
                kind: EntityKind::Module,
                name: module_name,
                file: file.clone(),
                line_start: 1,
                line_end,
                parent_class: None,
                semantic_features: Vec::new(),
                hierarchy_path: String::new(),
                deps: EntityDeps::default(),
            };
            self.entities.insert(module_id.clone(), entity);
            self.file_index.entry(file).or_default().push(module_id);
        }
    }

    /// Aggregate child entity features onto Module entities for each file.
    /// Creates the E_feature edge between file-level and function-level nodes (paper §3.1).
    pub fn aggregate_module_features(&mut self) {
        for (file, ids) in &self.file_index {
            // Find the Module entity for this file
            let module_id = ids
                .iter()
                .find(|id| {
                    self.entities
                        .get(id.as_str())
                        .is_some_and(|e| e.kind == EntityKind::Module)
                })
                .cloned();

            if let Some(module_id) = module_id {
                // Collect features from all non-module entities in this file
                let mut all_features: Vec<String> = Vec::new();
                for id in ids {
                    if *id == module_id {
                        continue;
                    }
                    if let Some(entity) = self.entities.get(id) {
                        all_features.extend(entity.semantic_features.clone());
                    }
                }

                // Deduplicate
                all_features.sort();
                all_features.dedup();

                if let Some(module) = self.entities.get_mut(&module_id) {
                    module.semantic_features = all_features;
                }
            }
            let _ = file; // suppress unused warning
        }
    }

    /// Materialize E_feature (containment) edges from the hierarchy tree into `self.edges`.
    pub fn materialize_containment_edges(&mut self) {
        self.edges.retain(|e| e.kind != EdgeKind::Contains);
        let mut contains = Vec::new();
        for area in self.hierarchy.values() {
            Self::collect_containment_edges(area, &mut contains);
        }
        self.edges.extend(contains);
    }

    fn collect_containment_edges(node: &HierarchyNode, edges: &mut Vec<DependencyEdge>) {
        for child in node.children.values() {
            if !node.id.is_empty() && !child.id.is_empty() {
                edges.push(DependencyEdge {
                    source: node.id.clone(),
                    target: child.id.clone(),
                    kind: EdgeKind::Contains,
                });
            }
            Self::collect_containment_edges(child, edges);
        }
        for eid in &node.entities {
            if !node.id.is_empty() {
                edges.push(DependencyEdge {
                    source: node.id.clone(),
                    target: eid.clone(),
                    kind: EdgeKind::Contains,
                });
            }
        }
    }
}
