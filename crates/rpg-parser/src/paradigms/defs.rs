//! Paradigm definition types — deserialized from TOML files.
//!
//! Each TOML file in `paradigms/defs/` defines a paradigm (React, Next.js, Redux, Django, etc.).
//! Adding a new paradigm = drop a TOML file + `cargo build`. No Rust edits needed.

use regex::Regex;
use rpg_core::graph::{EdgeKind, EntityKind};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ParadigmDef {
    pub schema_version: u32,
    pub name: String,
    pub priority: i32,
    pub languages: Vec<String>,
    pub detect: DetectRules,
    #[serde(default)]
    pub classify: Vec<ClassifyRule>,
    #[serde(default)]
    pub entity_queries: Vec<EntityQuery>,
    #[serde(default)]
    pub dep_queries: Vec<DepQuery>,
    #[serde(default)]
    pub features: FeatureFlags,
    #[serde(default)]
    pub prompt_hints: PromptHints,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DetectRules {
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub config_files: Vec<String>,
    #[serde(default)]
    pub dir_with_files: Vec<DirWithFiles>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirWithFiles {
    pub dir: String,
    pub pattern: String,
}

/// Action to apply when a classify rule matches.
///
/// `Reclassify` and `Skip` are **terminal**: they freeze the entity so no
/// lower-priority paradigm can reclassify it. `Tag` is non-terminal.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifyAction {
    /// Reclassify to a different EntityKind (terminal — freezes entity).
    Reclassify(String),
    /// Keep current kind but freeze — no lower-priority paradigm can touch it.
    Skip,
    /// Add a metadata tag without reclassifying (non-terminal).
    Tag(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifyRule {
    pub id: String,
    pub action: ClassifyAction,
    #[serde(rename = "match")]
    pub match_rule: EntityMatch,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct EntityMatch {
    pub kind: Option<String>,
    pub name_regex: Option<String>,
    pub name_starts_uppercase: Option<bool>,
    pub name_min_length: Option<usize>,
    pub source_contains_any: Option<Vec<String>>,
    pub file_name_stem: Option<String>,
    pub file_path_contains: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityQuery {
    pub id: String,
    #[serde(default)]
    pub languages: Vec<String>,
    pub entity_kind: String,
    pub entity_name: String,
    pub parent: Option<String>,
    pub query: String,
    #[serde(default)]
    pub query_by_language: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DepQuery {
    pub id: String,
    #[serde(default)]
    pub languages: Vec<String>,
    pub edge_kind: String,
    pub caller: String,
    pub callee: String,
    pub filter_callee: Option<String>,
    pub query: String,
    #[serde(default)]
    pub query_by_language: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FeatureFlags {
    #[serde(default)]
    pub redux_state_signals: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PromptHints {
    pub lifting: Option<String>,
    pub discovery: Option<String>,
    pub synthesis: Option<String>,
    pub hierarchy: Option<String>,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ValidationError {
    pub paradigm: String,
    pub rule_id: Option<String>,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.rule_id {
            Some(id) => write!(f, "[{}] rule {}: {}", self.paradigm, id, self.message),
            None => write!(f, "[{}]: {}", self.paradigm, self.message),
        }
    }
}

/// Parse a string into an `EntityKind`. Returns `None` for unknown kinds.
pub fn parse_entity_kind(s: &str) -> Option<EntityKind> {
    match s.to_lowercase().as_str() {
        "function" => Some(EntityKind::Function),
        "class" => Some(EntityKind::Class),
        "method" => Some(EntityKind::Method),
        "page" => Some(EntityKind::Page),
        "layout" => Some(EntityKind::Layout),
        "component" => Some(EntityKind::Component),
        "hook" => Some(EntityKind::Hook),
        "store" => Some(EntityKind::Store),
        "module" => Some(EntityKind::Module),
        "controller" => Some(EntityKind::Controller),
        "model" => Some(EntityKind::Model),
        "service" => Some(EntityKind::Service),
        "middleware" => Some(EntityKind::Middleware),
        "route" => Some(EntityKind::Route),
        "test" => Some(EntityKind::Test),
        _ => None,
    }
}

/// Parse a string into an `EdgeKind`. Returns `None` for unknown kinds.
pub fn parse_edge_kind(s: &str) -> Option<EdgeKind> {
    match s.to_lowercase().as_str() {
        "imports" => Some(EdgeKind::Imports),
        "invokes" => Some(EdgeKind::Invokes),
        "inherits" => Some(EdgeKind::Inherits),
        "composes" => Some(EdgeKind::Composes),
        "renders" => Some(EdgeKind::Renders),
        "reads_state" | "readsstate" => Some(EdgeKind::ReadsState),
        "writes_state" | "writesstate" => Some(EdgeKind::WritesState),
        "dispatches" => Some(EdgeKind::Dispatches),
        "contains" => Some(EdgeKind::Contains),
        _ => None,
    }
}

/// Validate all TOML definitions at load time.
///
/// Checks:
/// - `schema_version` is 1
/// - All `entity_kind`/`edge_kind` values are valid variants
/// - All `name_regex` fields compile as `Regex`
/// - No duplicate rule IDs across all loaded definitions
pub fn validate_defs(defs: &[ParadigmDef]) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let mut all_rule_ids = HashSet::new();

    for def in defs {
        // Schema version
        if def.schema_version != 1 {
            errors.push(ValidationError {
                paradigm: def.name.clone(),
                rule_id: None,
                message: format!(
                    "unsupported schema_version {}; expected 1",
                    def.schema_version
                ),
            });
        }

        // Classify rules
        for rule in &def.classify {
            // Unique rule ID
            if !all_rule_ids.insert(rule.id.clone()) {
                errors.push(ValidationError {
                    paradigm: def.name.clone(),
                    rule_id: Some(rule.id.clone()),
                    message: "duplicate rule ID".to_string(),
                });
            }

            // Validate reclassify target
            if let ClassifyAction::Reclassify(ref kind) = rule.action
                && parse_entity_kind(kind).is_none()
            {
                errors.push(ValidationError {
                    paradigm: def.name.clone(),
                    rule_id: Some(rule.id.clone()),
                    message: format!("unknown entity_kind '{}' in reclassify action", kind),
                });
            }

            // Validate regex
            if let Some(ref regex_str) = rule.match_rule.name_regex
                && Regex::new(regex_str).is_err()
            {
                errors.push(ValidationError {
                    paradigm: def.name.clone(),
                    rule_id: Some(rule.id.clone()),
                    message: format!("invalid regex '{}'", regex_str),
                });
            }
        }

        // Entity queries
        for eq in &def.entity_queries {
            if !all_rule_ids.insert(eq.id.clone()) {
                errors.push(ValidationError {
                    paradigm: def.name.clone(),
                    rule_id: Some(eq.id.clone()),
                    message: "duplicate rule ID".to_string(),
                });
            }
            if parse_entity_kind(&eq.entity_kind).is_none() {
                errors.push(ValidationError {
                    paradigm: def.name.clone(),
                    rule_id: Some(eq.id.clone()),
                    message: format!("unknown entity_kind '{}'", eq.entity_kind),
                });
            }
        }

        // Dep queries
        for dq in &def.dep_queries {
            if !all_rule_ids.insert(dq.id.clone()) {
                errors.push(ValidationError {
                    paradigm: def.name.clone(),
                    rule_id: Some(dq.id.clone()),
                    message: "duplicate rule ID".to_string(),
                });
            }
            if parse_edge_kind(&dq.edge_kind).is_none() {
                errors.push(ValidationError {
                    paradigm: def.name.clone(),
                    rule_id: Some(dq.id.clone()),
                    message: format!("unknown edge_kind '{}'", dq.edge_kind),
                });
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load built-in paradigm definitions from TOML files embedded at compile time.
///
/// The `build.rs` script scans `paradigms/defs/*.toml` and generates the include
/// list, so adding a new paradigm = drop a TOML file, no Rust edits needed.
///
/// Sorted by priority (lowest first = highest priority). Validated at load.
pub fn load_builtin_defs() -> Result<Vec<ParadigmDef>, Vec<ValidationError>> {
    let sources: &[&str] = include!(concat!(env!("OUT_DIR"), "/paradigm_includes.rs"));
    let mut defs: Vec<ParadigmDef> = sources
        .iter()
        .map(|s| toml::from_str(s).expect("built-in TOML must parse"))
        .collect();
    defs.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.name.cmp(&b.name))
    });
    validate_defs(&defs)?;
    Ok(defs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtin_defs() {
        let defs = load_builtin_defs().expect("built-in defs should load and validate");
        assert!(defs.len() >= 18, "expected at least 18 paradigm defs");

        // Verify priority ordering (ascending priority, then alphabetical name)
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            &[
                "angular", // 8
                "aspnet",  // 10
                "compose", // 10
                "django",  // 10
                "gin",     // 10
                "laravel", // 10
                "nestjs",  // 10
                "nextjs",  // 10
                "rails",   // 10
                "spring",  // 10
                "swiftui", // 10
                "svelte",  // 12
                "vue",     // 12
                "fastapi", // 15
                "flask",   // 20
                "redux",   // 20
                "express", // 25
                "react",   // 30
            ]
        );
    }

    #[test]
    fn test_parse_entity_kind() {
        assert_eq!(parse_entity_kind("function"), Some(EntityKind::Function));
        assert_eq!(parse_entity_kind("Component"), Some(EntityKind::Component));
        assert_eq!(parse_entity_kind("HOOK"), Some(EntityKind::Hook));
        assert_eq!(parse_entity_kind("store"), Some(EntityKind::Store));
        assert_eq!(parse_entity_kind("page"), Some(EntityKind::Page));
        assert_eq!(parse_entity_kind("layout"), Some(EntityKind::Layout));
        assert_eq!(
            parse_entity_kind("controller"),
            Some(EntityKind::Controller)
        );
        assert_eq!(parse_entity_kind("Model"), Some(EntityKind::Model));
        assert_eq!(parse_entity_kind("SERVICE"), Some(EntityKind::Service));
        assert_eq!(
            parse_entity_kind("middleware"),
            Some(EntityKind::Middleware)
        );
        assert_eq!(parse_entity_kind("route"), Some(EntityKind::Route));
        assert_eq!(parse_entity_kind("test"), Some(EntityKind::Test));
        assert_eq!(parse_entity_kind("bogus"), None);
    }

    #[test]
    fn test_parse_edge_kind() {
        assert_eq!(parse_edge_kind("renders"), Some(EdgeKind::Renders));
        assert_eq!(parse_edge_kind("reads_state"), Some(EdgeKind::ReadsState));
        assert_eq!(parse_edge_kind("dispatches"), Some(EdgeKind::Dispatches));
        assert_eq!(parse_edge_kind("bogus"), None);
    }

    #[test]
    fn test_validate_bad_schema_version() {
        let mut defs = load_builtin_defs().unwrap();
        defs[0].schema_version = 99;
        let result = validate_defs(&defs);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_bad_entity_kind() {
        let def = ParadigmDef {
            schema_version: 1,
            name: "test".to_string(),
            priority: 50,
            languages: vec!["python".to_string()],
            detect: DetectRules::default(),
            classify: vec![ClassifyRule {
                id: "test.bad".to_string(),
                action: ClassifyAction::Reclassify("nonexistent_kind".to_string()),
                match_rule: EntityMatch::default(),
            }],
            entity_queries: Vec::new(),
            dep_queries: Vec::new(),
            features: FeatureFlags::default(),
            prompt_hints: PromptHints::default(),
        };
        let result = validate_defs(&[def]);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_bad_regex() {
        let def = ParadigmDef {
            schema_version: 1,
            name: "test".to_string(),
            priority: 50,
            languages: vec!["python".to_string()],
            detect: DetectRules::default(),
            classify: vec![ClassifyRule {
                id: "test.badregex".to_string(),
                action: ClassifyAction::Skip,
                match_rule: EntityMatch {
                    name_regex: Some("[invalid".to_string()),
                    ..EntityMatch::default()
                },
            }],
            entity_queries: Vec::new(),
            dep_queries: Vec::new(),
            features: FeatureFlags::default(),
            prompt_hints: PromptHints::default(),
        };
        let result = validate_defs(&[def]);
        assert!(result.is_err());
    }

    #[test]
    fn test_react_def_structure() {
        let defs = load_builtin_defs().unwrap();
        let react = defs.iter().find(|d| d.name == "react").unwrap();
        assert_eq!(react.priority, 30);
        assert!(react.detect.deps.contains(&"react".to_string()));
        assert!(!react.classify.is_empty());
        // React should have hook, class_component, fn_component classification rules
        let rule_ids: Vec<&str> = react.classify.iter().map(|r| r.id.as_str()).collect();
        assert!(rule_ids.contains(&"react.hook"));
        assert!(rule_ids.contains(&"react.class_component"));
        assert!(rule_ids.contains(&"react.fn_component"));
    }

    #[test]
    fn test_nextjs_def_structure() {
        let defs = load_builtin_defs().unwrap();
        let nextjs = defs.iter().find(|d| d.name == "nextjs").unwrap();
        assert_eq!(nextjs.priority, 10);
        assert!(
            nextjs
                .detect
                .config_files
                .contains(&"next.config.js".to_string())
        );
    }

    #[test]
    fn test_redux_def_structure() {
        let defs = load_builtin_defs().unwrap();
        let redux = defs.iter().find(|d| d.name == "redux").unwrap();
        assert_eq!(redux.priority, 20);
        assert!(redux.features.redux_state_signals);
        // Should have skip_thunk, store_by_call, store_by_name
        let rule_ids: Vec<&str> = redux.classify.iter().map(|r| r.id.as_str()).collect();
        assert!(rule_ids.contains(&"redux.skip_thunk"));
        assert!(rule_ids.contains(&"redux.store_by_call"));
    }

    #[test]
    fn test_classify_action_deserialization() {
        // Test reclassify
        let toml_str = r#"
            id = "test.reclassify"
            action = { reclassify = "component" }
            [match]
            kind = "function"
        "#;
        let rule: ClassifyRule = toml::from_str(toml_str).unwrap();
        assert!(matches!(rule.action, ClassifyAction::Reclassify(ref k) if k == "component"));

        // Test skip
        let toml_str = r#"
            id = "test.skip"
            action = "skip"
            [match]
            kind = "function"
        "#;
        let rule: ClassifyRule = toml::from_str(toml_str).unwrap();
        assert!(matches!(rule.action, ClassifyAction::Skip));

        // Test tag
        let toml_str = r#"
            id = "test.tag"
            action = { tag = "async" }
            [match]
            kind = "function"
        "#;
        let rule: ClassifyRule = toml::from_str(toml_str).unwrap();
        assert!(matches!(rule.action, ClassifyAction::Tag(ref t) if t == "async"));
    }

    #[test]
    fn test_duplicate_rule_ids_rejected() {
        let def1 = ParadigmDef {
            schema_version: 1,
            name: "a".to_string(),
            priority: 10,
            languages: vec![],
            detect: DetectRules::default(),
            classify: vec![ClassifyRule {
                id: "dup.id".to_string(),
                action: ClassifyAction::Skip,
                match_rule: EntityMatch::default(),
            }],
            entity_queries: Vec::new(),
            dep_queries: Vec::new(),
            features: FeatureFlags::default(),
            prompt_hints: PromptHints::default(),
        };
        let def2 = ParadigmDef {
            schema_version: 1,
            name: "b".to_string(),
            priority: 20,
            languages: vec![],
            detect: DetectRules::default(),
            classify: vec![ClassifyRule {
                id: "dup.id".to_string(),
                action: ClassifyAction::Skip,
                match_rule: EntityMatch::default(),
            }],
            entity_queries: Vec::new(),
            dep_queries: Vec::new(),
            features: FeatureFlags::default(),
            prompt_hints: PromptHints::default(),
        };
        let result = validate_defs(&[def1, def2]);
        assert!(result.is_err());
    }
}
