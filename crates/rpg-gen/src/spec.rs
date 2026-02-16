//! Specification decomposition data structures.
//!
//! This module defines types for decomposing natural language specifications
//! into structured feature trees that can guide code generation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A hierarchical decomposition of a specification into semantic features.
///
/// Similar to the hierarchy in rpg-core but for planned (not yet existing) code.
/// The feature tree captures functional areas, features, constraints, and quality
/// requirements extracted from a natural language specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureTree {
    /// Schema version for forward compatibility
    pub version: String,

    /// Brief summary of what the system does
    pub spec_summary: String,

    /// Functional areas (major subsystems)
    pub functional_areas: BTreeMap<String, FeatureArea>,

    /// Constraints on the implementation
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<Constraint>,

    /// Quality requirements (testing, documentation, etc.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality_requirements: Vec<QualityRequirement>,
}

impl Default for FeatureTree {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
            spec_summary: String::new(),
            functional_areas: BTreeMap::new(),
            constraints: Vec::new(),
            quality_requirements: Vec::new(),
        }
    }
}

/// A functional area representing a major subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureArea {
    /// Name of the area (e.g., "Authentication", "DataStorage")
    pub name: String,

    /// Description of what this area is responsible for
    pub description: String,

    /// Features within this area
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<Feature>,

    /// Dependencies on other areas (by name)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

impl FeatureArea {
    /// Create a new feature area with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            features: Vec::new(),
            dependencies: Vec::new(),
        }
    }
}

/// A single feature within a functional area.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feature {
    /// Unique identifier (e.g., "auth.login", "storage.query")
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Semantic features using verb-object phrases (same style as rpg-encoder)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_features: Vec<String>,

    /// Acceptance criteria (testable conditions)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,

    /// Estimated complexity for implementation
    #[serde(default)]
    pub estimated_complexity: Complexity,
}

impl Feature {
    /// Create a new feature with the given ID and name.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            semantic_features: Vec::new(),
            acceptance_criteria: Vec::new(),
            estimated_complexity: Complexity::default(),
        }
    }
}

/// Complexity estimate for a feature or task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    /// Getter/setter, delegation, trivial logic
    Trivial,
    /// Single responsibility, no branching
    #[default]
    Simple,
    /// Branches, multiple calls, error handling
    Moderate,
    /// Multiple responsibilities, complex logic
    Complex,
}

impl Complexity {
    /// Convert complexity to an estimated effort multiplier.
    #[must_use]
    pub const fn effort_multiplier(self) -> f64 {
        match self {
            Self::Trivial => 0.25,
            Self::Simple => 1.0,
            Self::Moderate => 2.0,
            Self::Complex => 4.0,
        }
    }
}

impl std::fmt::Display for Complexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trivial => write!(f, "trivial"),
            Self::Simple => write!(f, "simple"),
            Self::Moderate => write!(f, "moderate"),
            Self::Complex => write!(f, "complex"),
        }
    }
}

/// A constraint on the implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    /// Kind of constraint
    pub kind: ConstraintKind,

    /// Description of the constraint
    pub description: String,
}

impl Constraint {
    /// Create a new constraint.
    #[must_use]
    pub fn new(kind: ConstraintKind, description: impl Into<String>) -> Self {
        Self {
            kind,
            description: description.into(),
        }
    }
}

/// Kind of constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    /// Programming language constraint
    Language,
    /// Framework or library constraint
    Framework,
    /// Performance requirement
    Performance,
    /// Security requirement
    Security,
    /// Compatibility requirement
    Compatibility,
}

/// A quality requirement (testing, documentation, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRequirement {
    /// Category (e.g., "testing", "documentation", "error_handling")
    pub category: String,

    /// Specific requirements within this category
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<String>,
}

impl QualityRequirement {
    /// Create a new quality requirement.
    #[must_use]
    pub fn new(category: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            requirements: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_tree_serialization() {
        let area = FeatureArea {
            name: "Auth".to_string(),
            description: "Authentication system".to_string(),
            features: vec![Feature::new("auth.login", "Login")],
            dependencies: Vec::new(),
        };

        let tree = FeatureTree {
            spec_summary: "Test spec".to_string(),
            functional_areas: [("Auth".to_string(), area)].into_iter().collect(),
            constraints: vec![Constraint::new(
                ConstraintKind::Language,
                "Must be written in Rust",
            )],
            ..Default::default()
        };

        let json = serde_json::to_string(&tree).unwrap();
        let parsed: FeatureTree = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(parsed.spec_summary, "Test spec");
        assert!(parsed.functional_areas.contains_key("Auth"));
    }

    #[test]
    fn test_complexity_effort() {
        assert!(Complexity::Trivial.effort_multiplier() < Complexity::Simple.effort_multiplier());
        assert!(Complexity::Simple.effort_multiplier() < Complexity::Moderate.effort_multiplier());
        assert!(Complexity::Moderate.effort_multiplier() < Complexity::Complex.effort_multiplier());
    }
}
