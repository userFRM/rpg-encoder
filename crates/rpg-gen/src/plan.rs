//! Generation plan data structures.
//!
//! This module defines the top-level orchestration types for code generation,
//! including the state machine that tracks progress through generation phases.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::interfaces::InterfaceDesign;
use crate::skeleton::FileSkeletonSet;
use crate::spec::FeatureTree;
use crate::tasks::{IterationTelemetry, OutcomeKind, PlannedTask, TaskGraph};
use crate::validation::ValidationResult;

/// Aggregated metrics for a specific model/backbone family.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackboneStats {
    #[serde(default)]
    pub iterations: usize,
    #[serde(default)]
    pub tasks_passed: usize,
    #[serde(default)]
    pub tasks_failed: usize,
    #[serde(default)]
    pub prompt_tokens: usize,
    #[serde(default)]
    pub completion_tokens: usize,
    #[serde(default)]
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub latency_ms: u64,
}

/// The complete generation plan.
///
/// This is the top-level structure that tracks all state for a code generation
/// session. It uses a state machine pattern to prevent invalid states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationPlan {
    /// Schema version for forward compatibility
    pub version: String,

    /// Revision for staleness checks (similar to routing pattern)
    pub revision: String,

    /// When the plan was created
    pub created_at: DateTime<Utc>,

    /// When the plan was last updated
    pub updated_at: DateTime<Utc>,

    /// Project root directory
    pub project_root: PathBuf,

    /// Original specification text
    pub spec: String,

    /// Target programming language
    pub language: String,

    /// Current state (state machine pattern)
    pub state: GenerationState,

    /// Aggregate statistics
    #[serde(default)]
    pub stats: GenerationStats,
}

impl GenerationPlan {
    /// Create a new generation plan.
    #[must_use]
    pub fn new(
        project_root: impl Into<PathBuf>,
        spec: impl Into<String>,
        language: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            version: "1.0.0".to_string(),
            revision: Self::compute_revision(&now),
            created_at: now,
            updated_at: now,
            project_root: project_root.into(),
            spec: spec.into(),
            language: language.into(),
            state: GenerationState::Initializing,
            stats: GenerationStats::default(),
        }
    }

    /// Compute a revision hash from the current time.
    fn compute_revision(time: &DateTime<Utc>) -> String {
        let mut hasher = Sha256::new();
        hasher.update(time.to_rfc3339().as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)[..8].to_string()
    }

    /// Update the revision and timestamp.
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
        self.revision = Self::compute_revision(&self.updated_at);
    }

    /// Get the current phase.
    #[must_use]
    pub fn phase(&self) -> GenerationPhase {
        match &self.state {
            GenerationState::Initializing => GenerationPhase::Initializing,
            GenerationState::Planning { .. } => GenerationPhase::Planning,
            GenerationState::DesigningInterfaces { .. } => GenerationPhase::DesigningInterfaces,
            GenerationState::GeneratingSkeletons { .. } => GenerationPhase::GeneratingSkeletons,
            GenerationState::Executing { .. } => GenerationPhase::Executing,
            GenerationState::Validating { .. } => GenerationPhase::Validating,
            GenerationState::Complete { .. } => GenerationPhase::Complete,
            GenerationState::Failed { .. } => GenerationPhase::Failed,
        }
    }

    /// Check if the plan is in a terminal state.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            GenerationState::Complete { .. } | GenerationState::Failed { .. }
        )
    }

    /// Transition to planning state with tasks.
    pub fn start_planning(&mut self, tasks: Vec<PlannedTask>) {
        self.state = GenerationState::Planning { tasks };
        self.touch();
    }

    /// Transition to interface design state.
    pub fn start_interface_design(&mut self, feature_tree: FeatureTree) {
        self.state = GenerationState::DesigningInterfaces {
            feature_tree,
            interface_design: None,
        };
        self.touch();
    }

    /// Transition to skeleton generation state.
    pub fn start_skeleton_generation(
        &mut self,
        feature_tree: FeatureTree,
        interface_design: InterfaceDesign,
    ) {
        self.state = GenerationState::GeneratingSkeletons {
            feature_tree,
            interface_design,
            skeletons: None,
        };
        self.touch();
    }

    /// Transition to execution state.
    pub fn start_execution(
        &mut self,
        feature_tree: FeatureTree,
        interface_design: InterfaceDesign,
        skeletons: FileSkeletonSet,
        task_graph: TaskGraph,
    ) {
        self.stats.total_tasks = task_graph.tasks.len();
        self.state = GenerationState::Executing {
            feature_tree,
            interface_design,
            skeletons,
            task_graph,
            current_batch: 0,
        };
        self.touch();
    }

    /// Transition to validation state.
    pub fn start_validation(
        &mut self,
        feature_tree: FeatureTree,
        interface_design: InterfaceDesign,
        skeletons: FileSkeletonSet,
        task_graph: TaskGraph,
    ) {
        self.state = GenerationState::Validating {
            feature_tree,
            interface_design,
            skeletons,
            task_graph,
            results: Vec::new(),
        };
        self.touch();
    }

    /// Mark as complete.
    pub fn mark_complete(&mut self, final_graph_path: PathBuf) {
        self.state = GenerationState::Complete { final_graph_path };
        self.touch();
    }

    /// Mark as failed.
    pub fn mark_failed(&mut self, error: String, recoverable: bool) {
        self.state = GenerationState::Failed { error, recoverable };
        self.touch();
    }
}

/// State machine for generation progress.
///
/// Each variant contains the accumulated state for that phase,
/// preventing invalid state combinations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
pub enum GenerationState {
    /// Initial state, plan just created
    Initializing,

    /// Planning phase: decomposing spec into tasks
    Planning { tasks: Vec<PlannedTask> },

    /// Designing interfaces between modules
    DesigningInterfaces {
        feature_tree: FeatureTree,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interface_design: Option<InterfaceDesign>,
    },

    /// Generating file skeletons
    GeneratingSkeletons {
        feature_tree: FeatureTree,
        interface_design: InterfaceDesign,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skeletons: Option<FileSkeletonSet>,
    },

    /// Executing generation tasks
    Executing {
        feature_tree: FeatureTree,
        interface_design: InterfaceDesign,
        skeletons: FileSkeletonSet,
        task_graph: TaskGraph,
        current_batch: usize,
    },

    /// Validating generated code
    Validating {
        feature_tree: FeatureTree,
        interface_design: InterfaceDesign,
        skeletons: FileSkeletonSet,
        task_graph: TaskGraph,
        results: Vec<ValidationResult>,
    },

    /// Successfully completed
    Complete { final_graph_path: PathBuf },

    /// Failed (potentially recoverable)
    Failed { error: String, recoverable: bool },
}

/// Phase identifier (for display and status).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationPhase {
    /// Initial state
    Initializing,
    /// Planning tasks
    Planning,
    /// Designing interfaces
    DesigningInterfaces,
    /// Generating skeletons
    GeneratingSkeletons,
    /// Executing tasks
    Executing,
    /// Validating results
    Validating,
    /// Complete
    Complete,
    /// Failed
    Failed,
}

impl std::fmt::Display for GenerationPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initializing => write!(f, "Initializing"),
            Self::Planning => write!(f, "Planning"),
            Self::DesigningInterfaces => write!(f, "Designing Interfaces"),
            Self::GeneratingSkeletons => write!(f, "Generating Skeletons"),
            Self::Executing => write!(f, "Executing"),
            Self::Validating => write!(f, "Validating"),
            Self::Complete => write!(f, "Complete"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

/// Aggregate statistics for the generation plan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerationStats {
    /// Total number of features in the spec
    #[serde(default)]
    pub total_features: usize,

    /// Total number of interfaces designed
    #[serde(default)]
    pub total_interfaces: usize,

    /// Total number of files to generate
    #[serde(default)]
    pub total_files: usize,

    /// Total number of tasks
    #[serde(default)]
    pub total_tasks: usize,

    /// Number of tasks completed
    #[serde(default)]
    pub tasks_completed: usize,

    /// Number of tasks validated
    #[serde(default)]
    pub tasks_validated: usize,

    /// Number of tasks failed
    #[serde(default)]
    pub tasks_failed: usize,

    /// Validation pass rate (0.0 - 1.0)
    #[serde(default)]
    pub validation_pass_rate: f64,
    /// Total number of TDD iterations recorded.
    #[serde(default)]
    pub total_iterations: usize,
    /// Failure type histogram (test_failure, code_error, test_error, env_error).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub failure_type_counts: BTreeMap<String, usize>,
    /// Total prompt tokens recorded by telemetry.
    #[serde(default)]
    pub total_prompt_tokens: usize,
    /// Total completion tokens recorded by telemetry.
    #[serde(default)]
    pub total_completion_tokens: usize,
    /// Total estimated model cost in USD.
    #[serde(default)]
    pub total_estimated_cost_usd: f64,
    /// Sum of recorded runtime latencies in milliseconds.
    #[serde(default)]
    pub total_runtime_ms: u64,
    /// Mean quality confidence for generated representations [0, 1], when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_confidence_score: Option<f64>,
    /// Per-backbone efficiency counters.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_backbone: BTreeMap<String, BackboneStats>,
}

impl GenerationStats {
    /// Update completion percentage.
    pub fn update_completion(&mut self) {
        if self.total_tasks > 0 {
            self.validation_pass_rate = self.tasks_validated as f64 / self.total_tasks as f64;
        }
    }

    /// Record failure histogram from an outcome kind.
    pub fn record_failure_kind(&mut self, outcome: &OutcomeKind) {
        let key = match outcome {
            OutcomeKind::Pass => return,
            OutcomeKind::TestFailure { .. } => "test_failure",
            OutcomeKind::CodeError { .. } => "code_error",
            OutcomeKind::TestError { .. } => "test_error",
            OutcomeKind::EnvError { .. } => "env_error",
        };
        *self.failure_type_counts.entry(key.to_string()).or_insert(0) += 1;
    }

    /// Record iteration-level telemetry for cost/efficiency reporting.
    pub fn record_iteration_telemetry(
        &mut self,
        telemetry: Option<&IterationTelemetry>,
        outcome: &OutcomeKind,
    ) {
        self.total_iterations += 1;

        let Some(t) = telemetry else {
            return;
        };

        if let Some(tokens) = t.prompt_tokens {
            self.total_prompt_tokens += tokens;
        }
        if let Some(tokens) = t.completion_tokens {
            self.total_completion_tokens += tokens;
        }
        if let Some(cost) = t.estimated_cost_usd {
            self.total_estimated_cost_usd += cost;
        }
        if let Some(latency) = t.duration_ms {
            self.total_runtime_ms = self.total_runtime_ms.saturating_add(latency);
        }

        if let Some(backbone) = t.backbone.as_ref().or(t.model.as_ref()) {
            let entry = self.per_backbone.entry(backbone.clone()).or_default();
            entry.iterations += 1;
            if let Some(tokens) = t.prompt_tokens {
                entry.prompt_tokens += tokens;
            }
            if let Some(tokens) = t.completion_tokens {
                entry.completion_tokens += tokens;
            }
            if let Some(cost) = t.estimated_cost_usd {
                entry.estimated_cost_usd += cost;
            }
            if let Some(latency) = t.duration_ms {
                entry.latency_ms = entry.latency_ms.saturating_add(latency);
            }
            match outcome {
                OutcomeKind::Pass => entry.tasks_passed += 1,
                OutcomeKind::EnvError { .. } => {}
                _ => entry.tasks_failed += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generation_plan_creation() {
        let plan = GenerationPlan::new("/tmp/test", "Create a hello world app", "rust");

        assert_eq!(plan.version, "1.0.0");
        assert_eq!(plan.language, "rust");
        assert!(matches!(plan.state, GenerationState::Initializing));
        assert_eq!(plan.phase(), GenerationPhase::Initializing);
    }

    #[test]
    fn test_phase_transitions() {
        let mut plan = GenerationPlan::new("/tmp/test", "Test spec", "rust");

        plan.start_planning(vec![]);
        assert_eq!(plan.phase(), GenerationPhase::Planning);

        plan.start_interface_design(FeatureTree::default());
        assert_eq!(plan.phase(), GenerationPhase::DesigningInterfaces);

        plan.mark_failed("Test error".to_string(), true);
        assert_eq!(plan.phase(), GenerationPhase::Failed);
        assert!(plan.is_terminal());
    }

    #[test]
    fn test_revision_updates() {
        let mut plan = GenerationPlan::new("/tmp/test", "Test spec", "rust");
        let old_revision = plan.revision.clone();

        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));

        plan.touch();
        assert_ne!(plan.revision, old_revision);
    }

    #[test]
    fn test_stats_update() {
        let mut stats = GenerationStats {
            total_tasks: 10,
            tasks_validated: 8,
            ..Default::default()
        };
        stats.update_completion();

        assert!((stats.validation_pass_rate - 0.8).abs() < f64::EPSILON);
    }
}
