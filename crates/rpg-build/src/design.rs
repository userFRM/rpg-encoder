//! Core design logic — turn a spec into an RPGraph via an LLM.

use crate::prompts::{DESIGN_SYSTEM_PROMPT, format_design_prompt};
use rpg_core::graph::{
    DependencyEdge, EdgeKind, Entity, EntityDeps, EntityKind, GraphMetadata, HierarchyNode, RPGraph,
};
use rpg_lift::{LlmProvider, ProviderError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Configuration for an RPG design call.
pub struct DesignConfig<'a> {
    pub provider: &'a dyn LlmProvider,
    /// Number of retries on parse/API failure (default 2).
    pub max_retries: usize,
}

/// Outcome of a design call.
#[derive(Debug, Clone)]
pub struct DesignReport {
    pub entities: usize,
    pub areas: usize,
    pub edges: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// Errors that can occur during design.
#[derive(Debug, thiserror::Error)]
pub enum DesignError {
    #[error("LLM provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("could not parse design response: {0}")]
    Parse(String),
    #[error("design has no entities — the LLM returned an empty graph")]
    Empty,
}

// ---------------------------------------------------------------------------
// Wire format — what the LLM is asked to produce
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignDoc {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_language")]
    pub primary_language: String,
    pub areas: Vec<DesignArea>,
}

fn default_language() -> String {
    "rust".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignArea {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub categories: Vec<DesignCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignCategory {
    pub name: String,
    pub subcategories: Vec<DesignSubcategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignSubcategory {
    pub name: String,
    pub entities: Vec<DesignEntity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignEntity {
    pub name: String,
    pub kind: String,
    pub file: String,
    #[serde(default)]
    pub parent_class: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub calls: Vec<String>,
    #[serde(default)]
    pub imports: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Design an RPG from a natural-language specification.
///
/// Returns a fully-populated [`RPGraph`] with hierarchy, entities, and
/// dependency edges — but no source code. The connected coding agent walks
/// the graph to generate implementation.
pub fn design_rpg(
    spec: &str,
    config: &DesignConfig<'_>,
) -> Result<(RPGraph, DesignReport), DesignError> {
    let user_prompt = format_design_prompt(spec);

    let mut last_err: Option<DesignError> = None;
    for attempt in 0..=config.max_retries {
        let response = config
            .provider
            .complete(DESIGN_SYSTEM_PROMPT, &user_prompt)
            .map_err(DesignError::Provider)?;

        match parse_design_response(&response.text) {
            Ok(doc) => {
                let graph = rpg_from_design(&doc)?;
                let in_tokens = response.input_tokens.unwrap_or(0);
                let out_tokens = response.output_tokens.unwrap_or(0);
                let cost = (in_tokens as f64 / 1_000_000.0) * config.provider.cost_per_mtok_input()
                    + (out_tokens as f64 / 1_000_000.0) * config.provider.cost_per_mtok_output();
                let report = DesignReport {
                    entities: graph.entities.len(),
                    areas: graph.hierarchy.len(),
                    edges: graph.edges.len(),
                    input_tokens: in_tokens,
                    output_tokens: out_tokens,
                    cost_usd: cost,
                };
                return Ok((graph, report));
            }
            Err(e) => {
                last_err = Some(e);
                if attempt == config.max_retries {
                    break;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| DesignError::Parse("no response".into())))
}

/// Parse an LLM response text into a `DesignDoc`. Tolerant of fenced code
/// blocks and surrounding prose.
pub fn parse_design_response(text: &str) -> Result<DesignDoc, DesignError> {
    let json = extract_json_block(text)
        .ok_or_else(|| DesignError::Parse("no JSON object found in response".into()))?;
    serde_json::from_str::<DesignDoc>(json)
        .map_err(|e| DesignError::Parse(format!("invalid JSON: {}", e)))
}

/// Convert a parsed `DesignDoc` into an `RPGraph`.
///
/// Builds the 3-level hierarchy, populates entities with their semantic
/// features and hierarchy paths, and creates dependency edges from `calls`
/// and `imports` references. Entity IDs that don't resolve to any other
/// entity are silently dropped (the LLM may reference imaginary IDs).
pub fn rpg_from_design(doc: &DesignDoc) -> Result<RPGraph, DesignError> {
    if doc.areas.is_empty() {
        return Err(DesignError::Empty);
    }

    let mut graph = RPGraph::new(&doc.primary_language);
    graph.metadata = GraphMetadata {
        language: doc.primary_language.clone(),
        languages: vec![doc.primary_language.clone()],
        total_files: 0,
        total_entities: 0,
        functional_areas: doc.areas.len(),
        total_edges: 0,
        dependency_edges: 0,
        containment_edges: 0,
        lifted_entities: 0,
        data_flow_edges: 0,
        semantic_hierarchy: true,
        repo_summary: if doc.description.is_empty() {
            None
        } else {
            Some(doc.description.clone())
        },
        paradigms: Vec::new(),
    };

    // Pass 1: build hierarchy + collect entities
    let mut all_entity_ids: Vec<String> = Vec::new();

    for area in &doc.areas {
        let mut area_node = HierarchyNode::new(&area.name);
        if !area.description.is_empty() {
            area_node.description = Some(area.description.clone());
        }

        for cat in &area.categories {
            let mut cat_node = HierarchyNode::new(&cat.name);

            for sub in &cat.subcategories {
                let mut sub_node = HierarchyNode::new(&sub.name);
                let hierarchy_path = format!("{}/{}/{}", area.name, cat.name, sub.name);

                for entity in &sub.entities {
                    let entity_id = canonical_id(entity);
                    let kind = parse_entity_kind(&entity.kind);
                    let mut features: Vec<String> = entity
                        .features
                        .iter()
                        .map(|s| s.trim().to_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect();
                    features.sort();
                    features.dedup();

                    let core_entity = Entity {
                        id: entity_id.clone(),
                        kind,
                        name: entity.name.clone(),
                        file: PathBuf::from(&entity.file),
                        line_start: 0,
                        line_end: 0,
                        parent_class: entity.parent_class.clone(),
                        semantic_features: features,
                        feature_source: Some("design".into()),
                        hierarchy_path: hierarchy_path.clone(),
                        deps: EntityDeps::default(),
                        signature: None,
                    };

                    sub_node.entities.push(entity_id.clone());
                    graph.entities.insert(entity_id.clone(), core_entity);
                    all_entity_ids.push(entity_id);
                }

                cat_node.children.insert(sub.name.clone(), sub_node);
            }

            area_node.children.insert(cat.name.clone(), cat_node);
        }

        graph.hierarchy.insert(area.name.clone(), area_node);
    }

    // Pass 2: resolve dependency references and build edges
    let known_ids: std::collections::HashSet<&String> = all_entity_ids.iter().collect();
    let mut edges: Vec<DependencyEdge> = Vec::new();

    // We need to collect deps per-entity in a separate pass to avoid borrow issues
    let mut entity_deps_updates: BTreeMap<String, EntityDeps> = BTreeMap::new();

    for area in &doc.areas {
        for cat in &area.categories {
            for sub in &cat.subcategories {
                for entity in &sub.entities {
                    let src_id = canonical_id(entity);
                    let mut deps = EntityDeps::default();

                    for callee in &entity.calls {
                        if known_ids.contains(callee) {
                            deps.invokes.push(callee.clone());
                            edges.push(DependencyEdge {
                                source: src_id.clone(),
                                target: callee.clone(),
                                kind: EdgeKind::Invokes,
                            });
                        }
                    }
                    for imp in &entity.imports {
                        if known_ids.contains(imp) {
                            deps.imports.push(imp.clone());
                            edges.push(DependencyEdge {
                                source: src_id.clone(),
                                target: imp.clone(),
                                kind: EdgeKind::Imports,
                            });
                        }
                    }

                    entity_deps_updates.insert(src_id, deps);
                }
            }
        }
    }

    // Apply forward deps
    for (id, deps) in &entity_deps_updates {
        if let Some(entity) = graph.entities.get_mut(id) {
            entity.deps = deps.clone();
        }
    }

    // Build reverse dep indexes (invoked_by, imported_by) by walking edges
    for edge in &edges {
        if let Some(target) = graph.entities.get_mut(&edge.target) {
            match edge.kind {
                EdgeKind::Invokes => target.deps.invoked_by.push(edge.source.clone()),
                EdgeKind::Imports => target.deps.imported_by.push(edge.source.clone()),
                _ => {}
            }
        }
    }

    graph.edges = edges;
    graph.refresh_metadata();
    graph.materialize_containment_edges();
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.rebuild_edge_index();
    graph.rebuild_hierarchy_index();

    // Update total_files based on unique file paths
    let unique_files: std::collections::HashSet<&PathBuf> =
        graph.entities.values().map(|e| &e.file).collect();
    graph.metadata.total_files = unique_files.len();

    Ok(graph)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Canonical entity ID: `file:name` or `file:Class::method`.
fn canonical_id(e: &DesignEntity) -> String {
    match &e.parent_class {
        Some(cls) => format!("{}:{}::{}", e.file, cls, e.name),
        None => format!("{}:{}", e.file, e.name),
    }
}

fn parse_entity_kind(s: &str) -> EntityKind {
    match s.to_lowercase().as_str() {
        "function" | "fn" => EntityKind::Function,
        "class" | "struct" => EntityKind::Class,
        "method" => EntityKind::Method,
        "module" | "file" => EntityKind::Module,
        _ => EntityKind::Function,
    }
}

/// Extract a JSON object from a possibly-fenced response.
///
/// Looks for ` ```json ... ``` ` or `{...}`. Tolerant of LLM prose
/// before/after the block.
fn extract_json_block(text: &str) -> Option<&str> {
    // Try fenced code block first
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim());
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim());
        }
    }
    // Fall back to first balanced {...}
    let start = text.find('{')?;
    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '\\' if in_string => escape = !escape,
            '"' if !escape => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=(start + i)]);
                }
            }
            _ => escape = false,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fenced_block() {
        let text = r#"
Some preamble.

```json
{"name": "X", "areas": []}
```

Trailing prose.
"#;
        let json = extract_json_block(text).unwrap();
        assert_eq!(json.trim(), r#"{"name": "X", "areas": []}"#);
    }

    #[test]
    fn extracts_json_without_fence() {
        let text = "Here's the design: {\"name\":\"X\",\"areas\":[]}";
        let json = extract_json_block(text).unwrap();
        assert_eq!(json, r#"{"name":"X","areas":[]}"#);
    }

    #[test]
    fn rejects_design_with_no_areas() {
        let doc = DesignDoc {
            name: "X".into(),
            description: String::new(),
            primary_language: "rust".into(),
            areas: vec![],
        };
        assert!(matches!(rpg_from_design(&doc), Err(DesignError::Empty)));
    }

    #[test]
    fn builds_graph_from_minimal_design() {
        let doc = DesignDoc {
            name: "Auth".into(),
            description: "test".into(),
            primary_language: "rust".into(),
            areas: vec![DesignArea {
                name: "Security".into(),
                description: String::new(),
                categories: vec![DesignCategory {
                    name: "auth".into(),
                    subcategories: vec![DesignSubcategory {
                        name: "validate".into(),
                        entities: vec![DesignEntity {
                            name: "validate_token".into(),
                            kind: "function".into(),
                            file: "src/auth.rs".into(),
                            parent_class: None,
                            features: vec!["validate JWT token".into()],
                            calls: vec![],
                            imports: vec![],
                        }],
                    }],
                }],
            }],
        };

        let graph = rpg_from_design(&doc).unwrap();
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.hierarchy.len(), 1);
        assert!(graph.metadata.semantic_hierarchy);
        let entity = graph.entities.values().next().unwrap();
        assert_eq!(entity.name, "validate_token");
        assert_eq!(entity.hierarchy_path, "Security/auth/validate");
        assert_eq!(entity.feature_source.as_deref(), Some("design"));
    }

    #[test]
    fn resolves_dependencies_between_entities() {
        let doc = DesignDoc {
            name: "App".into(),
            description: String::new(),
            primary_language: "rust".into(),
            areas: vec![DesignArea {
                name: "Core".into(),
                description: String::new(),
                categories: vec![DesignCategory {
                    name: "biz".into(),
                    subcategories: vec![DesignSubcategory {
                        name: "logic".into(),
                        entities: vec![
                            DesignEntity {
                                name: "caller".into(),
                                kind: "function".into(),
                                file: "src/a.rs".into(),
                                parent_class: None,
                                features: vec![],
                                calls: vec!["src/b.rs:callee".into()],
                                imports: vec![],
                            },
                            DesignEntity {
                                name: "callee".into(),
                                kind: "function".into(),
                                file: "src/b.rs".into(),
                                parent_class: None,
                                features: vec![],
                                calls: vec![],
                                imports: vec![],
                            },
                        ],
                    }],
                }],
            }],
        };

        let graph = rpg_from_design(&doc).unwrap();
        let caller = graph.entities.get("src/a.rs:caller").unwrap();
        assert_eq!(caller.deps.invokes, vec!["src/b.rs:callee"]);
        let callee = graph.entities.get("src/b.rs:callee").unwrap();
        assert_eq!(callee.deps.invoked_by, vec!["src/a.rs:caller"]);
        assert!(
            graph
                .edges
                .iter()
                .any(|e| matches!(e.kind, EdgeKind::Invokes)
                    && e.source == "src/a.rs:caller"
                    && e.target == "src/b.rs:callee")
        );
    }

    #[test]
    fn drops_dangling_dep_references() {
        let doc = DesignDoc {
            name: "App".into(),
            description: String::new(),
            primary_language: "rust".into(),
            areas: vec![DesignArea {
                name: "Core".into(),
                description: String::new(),
                categories: vec![DesignCategory {
                    name: "biz".into(),
                    subcategories: vec![DesignSubcategory {
                        name: "logic".into(),
                        entities: vec![DesignEntity {
                            name: "caller".into(),
                            kind: "function".into(),
                            file: "src/a.rs".into(),
                            parent_class: None,
                            features: vec![],
                            calls: vec!["src/nowhere.rs:imaginary".into()],
                            imports: vec![],
                        }],
                    }],
                }],
            }],
        };

        let graph = rpg_from_design(&doc).unwrap();
        let caller = graph.entities.get("src/a.rs:caller").unwrap();
        // Imaginary dep was silently dropped — better than a broken edge
        assert!(caller.deps.invokes.is_empty());
    }

    #[test]
    fn parses_method_with_parent_class() {
        let doc = DesignDoc {
            name: "App".into(),
            description: String::new(),
            primary_language: "python".into(),
            areas: vec![DesignArea {
                name: "Data".into(),
                description: String::new(),
                categories: vec![DesignCategory {
                    name: "db".into(),
                    subcategories: vec![DesignSubcategory {
                        name: "ops".into(),
                        entities: vec![DesignEntity {
                            name: "execute".into(),
                            kind: "method".into(),
                            file: "src/db.py".into(),
                            parent_class: Some("Connection".into()),
                            features: vec!["execute sql query".into()],
                            calls: vec![],
                            imports: vec![],
                        }],
                    }],
                }],
            }],
        };
        let graph = rpg_from_design(&doc).unwrap();
        assert!(graph.entities.contains_key("src/db.py:Connection::execute"));
    }

    #[test]
    fn parse_response_handles_prose_around_json() {
        let response = r#"Sure! Here's the design for your project:

```json
{
  "name": "TestProject",
  "description": "A test",
  "primary_language": "rust",
  "areas": []
}
```

Hope that helps!"#;
        let doc = parse_design_response(response).unwrap();
        assert_eq!(doc.name, "TestProject");
    }
}
