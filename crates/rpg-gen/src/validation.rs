//! Validation data structures.
//!
//! This module defines types for validating generated code against the plan,
//! using rpg-encoder to parse and compare features.

use serde::{Deserialize, Serialize};

/// Result of validating a single generated entity against the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Entity ID (planned or resolved)
    pub entity_id: String,

    /// Overall validation status
    pub status: ValidationStatus,

    /// Specific issues found
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ValidationIssue>,

    /// Feature coverage analysis
    pub coverage: ValidationCoverage,
}

impl ValidationResult {
    /// Create a passing validation result.
    #[must_use]
    pub fn pass(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            status: ValidationStatus::Pass,
            issues: Vec::new(),
            coverage: ValidationCoverage::default(),
        }
    }

    /// Create a failing validation result.
    #[must_use]
    pub fn fail(entity_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            status: ValidationStatus::Fail,
            issues: vec![ValidationIssue {
                severity: IssueSeverity::Error,
                category: IssueCategory::MissingFeature,
                message: error.into(),
                suggestion: None,
            }],
            coverage: ValidationCoverage::default(),
        }
    }

    /// Check if validation passed (with or without warnings).
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(
            self.status,
            ValidationStatus::Pass | ValidationStatus::PassWithWarnings
        )
    }

    /// Add an issue to the result.
    pub fn add_issue(&mut self, issue: ValidationIssue) {
        self.issues.push(issue);
        // Update status based on issues
        if self
            .issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
        {
            self.status = ValidationStatus::Fail;
        } else if self
            .issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Warning)
        {
            self.status = ValidationStatus::PassWithWarnings;
        }
    }
}

/// Overall validation status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    /// All checks passed
    #[default]
    Pass,
    /// Passed with warnings
    PassWithWarnings,
    /// Failed validation
    Fail,
}

impl std::fmt::Display for ValidationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pass => write!(f, "PASS"),
            Self::PassWithWarnings => write!(f, "PASS (with warnings)"),
            Self::Fail => write!(f, "FAIL"),
        }
    }
}

/// A specific validation issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Severity of the issue
    pub severity: IssueSeverity,

    /// Category of the issue
    pub category: IssueCategory,

    /// Human-readable message
    pub message: String,

    /// Suggested fix (if available)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl ValidationIssue {
    /// Create a new validation issue.
    #[must_use]
    pub fn new(
        severity: IssueSeverity,
        category: IssueCategory,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            category,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Add a suggestion to the issue.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Create an error-level issue.
    #[must_use]
    pub fn error(category: IssueCategory, message: impl Into<String>) -> Self {
        Self::new(IssueSeverity::Error, category, message)
    }

    /// Create a warning-level issue.
    #[must_use]
    pub fn warning(category: IssueCategory, message: impl Into<String>) -> Self {
        Self::new(IssueSeverity::Warning, category, message)
    }

    /// Create an info-level issue.
    #[must_use]
    pub fn info(category: IssueCategory, message: impl Into<String>) -> Self {
        Self::new(IssueSeverity::Info, category, message)
    }
}

/// Severity of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    /// Blocking error
    Error,
    /// Non-blocking warning
    Warning,
    /// Informational
    Info,
}

/// Category of validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCategory {
    /// A planned feature is missing
    MissingFeature,
    /// Function signature doesn't match plan
    WrongSignature,
    /// Missing dependency/import
    MissingDependency,
    /// Code style violation
    StyleViolation,
    /// Missing documentation
    DocumentationMissing,
    /// Insufficient test coverage
    TestCoverage,
    /// Extra unplanned feature
    ExtraFeature,
    /// Type mismatch
    TypeMismatch,
}

impl std::fmt::Display for IssueCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFeature => write!(f, "Missing Feature"),
            Self::WrongSignature => write!(f, "Wrong Signature"),
            Self::MissingDependency => write!(f, "Missing Dependency"),
            Self::StyleViolation => write!(f, "Style Violation"),
            Self::DocumentationMissing => write!(f, "Documentation Missing"),
            Self::TestCoverage => write!(f, "Test Coverage"),
            Self::ExtraFeature => write!(f, "Extra Feature"),
            Self::TypeMismatch => write!(f, "Type Mismatch"),
        }
    }
}

/// Feature coverage analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationCoverage {
    /// Features that were planned
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planned_features: Vec<String>,

    /// Features that were implemented
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implemented_features: Vec<String>,

    /// Features that are missing
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_features: Vec<String>,

    /// Extra features not in the plan
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_features: Vec<String>,

    /// Coverage percentage (0.0 - 100.0)
    #[serde(default)]
    pub coverage_pct: f64,
}

impl ValidationCoverage {
    /// Compute coverage from planned and implemented features.
    #[must_use]
    pub fn compute(planned: Vec<String>, implemented: Vec<String>) -> Self {
        let missing: Vec<String> = planned
            .iter()
            .filter(|p| !implemented.contains(p))
            .cloned()
            .collect();

        let extra: Vec<String> = implemented
            .iter()
            .filter(|i| !planned.contains(i))
            .cloned()
            .collect();

        let coverage_pct = if planned.is_empty() {
            100.0
        } else {
            ((planned.len() - missing.len()) as f64 / planned.len() as f64) * 100.0
        };

        Self {
            planned_features: planned,
            implemented_features: implemented,
            missing_features: missing,
            extra_features: extra,
            coverage_pct,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_result_pass() {
        let result = ValidationResult::pass("test::entity");
        assert!(result.passed());
        assert!(result.issues.is_empty());
    }

    #[test]
    fn test_validation_result_fail() {
        let result = ValidationResult::fail("test::entity", "Missing implementation");
        assert!(!result.passed());
        assert_eq!(result.issues.len(), 1);
    }

    #[test]
    fn test_validation_result_add_issue() {
        let mut result = ValidationResult::pass("test::entity");

        result.add_issue(ValidationIssue::warning(
            IssueCategory::DocumentationMissing,
            "Missing doc comment",
        ));
        assert_eq!(result.status, ValidationStatus::PassWithWarnings);

        result.add_issue(ValidationIssue::error(
            IssueCategory::MissingFeature,
            "Missing required feature",
        ));
        assert_eq!(result.status, ValidationStatus::Fail);
    }

    #[test]
    fn test_coverage_computation() {
        let planned = vec![
            "feature1".to_string(),
            "feature2".to_string(),
            "feature3".to_string(),
        ];
        let implemented = vec![
            "feature1".to_string(),
            "feature2".to_string(),
            "extra".to_string(),
        ];

        let coverage = ValidationCoverage::compute(planned, implemented);

        assert_eq!(coverage.missing_features, vec!["feature3"]);
        assert_eq!(coverage.extra_features, vec!["extra"]);
        assert!((coverage.coverage_pct - 66.666_666_666_666_66).abs() < 0.01);
    }

    #[test]
    fn test_issue_with_suggestion() {
        let issue =
            ValidationIssue::error(IssueCategory::WrongSignature, "Parameter type mismatch")
                .with_suggestion("Change `i32` to `u32`");

        assert_eq!(issue.severity, IssueSeverity::Error);
        assert!(issue.suggestion.is_some());
    }
}
