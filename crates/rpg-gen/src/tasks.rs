//! Task scheduling and execution data structures.
//!
//! This module defines types for organizing code generation into
//! dependency-ordered tasks that can be executed by the connected agent.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::skeleton::EntitySkeleton;

// ─── TDD Iteration Tracking ─────────────────────────────────────────────────

/// Default maximum retry count for TDD iterations.
const DEFAULT_MAX_RETRIES: usize = 3;

/// Execution sandbox mode used for test/runtime commands.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Run directly on the host environment.
    #[default]
    Local,
    /// Run inside a Docker container.
    Docker,
    /// Do not execute commands (dry-run / classification-only).
    None,
}

/// Runtime telemetry for a single iteration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IterationTelemetry {
    /// Sandbox mode used for execution.
    #[serde(default)]
    pub sandbox_mode: SandboxMode,
    /// Command that was executed (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Runner family (cargo, pytest, jest, go test, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// Runtime latency in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Captured stdout bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_bytes: Option<usize>,
    /// Captured stderr bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_bytes: Option<usize>,
    /// Prompt tokens consumed for this iteration (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<usize>,
    /// Completion tokens consumed for this iteration (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<usize>,
    /// Model identifier used (if applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Backbone family used (e.g., gpt-4.1, claude-4.5, local).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backbone: Option<String>,
    /// Estimated cost in USD for this iteration (if computed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    /// Number of automatic command adaptations attempted.
    #[serde(default)]
    pub auto_adaptations: usize,
}

/// Outcome of a single TDD iteration for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutcome {
    /// Which task this outcome belongs to
    pub task_id: String,
    /// What happened
    pub outcome: OutcomeKind,
    /// Optional test results if tests were run
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_results: Option<TestResults>,
    /// Runtime telemetry (sandbox, latency, token/cost, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<IterationTelemetry>,
    /// Which iteration (0-based)
    pub iteration: usize,
    /// When this outcome was recorded
    pub reported_at: DateTime<Utc>,
}

/// The kind of outcome from a TDD iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OutcomeKind {
    /// Tests pass, code is correct
    Pass,
    /// Tests ran but some failed — code is buggy, tests are correct
    TestFailure {
        failing_count: usize,
        summary: String,
    },
    /// Code has compilation/syntax errors
    CodeError { error_message: String },
    /// Test code itself is broken (won't compile or has test setup errors)
    TestError { error_message: String },
    /// Environment error (missing tool, permission denied, etc.) — does NOT count toward retries
    EnvError { error_message: String },
}

/// Results from running a test suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    /// Total number of tests
    pub total: usize,
    /// Number that passed
    pub passed: usize,
    /// Number that failed
    pub failed: usize,
    /// Path to the test file, if applicable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_file: Option<PathBuf>,
}

/// History of TDD iterations for a single task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskIterationHistory {
    /// All recorded outcomes (ordered by iteration)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<TaskOutcome>,
    /// Maximum retries before marking as Failed (default: 3)
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
}

fn default_max_retries() -> usize {
    DEFAULT_MAX_RETRIES
}

impl Default for TaskIterationHistory {
    fn default() -> Self {
        Self {
            outcomes: Vec::new(),
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }
}

impl TaskIterationHistory {
    /// Count retries that count toward the limit (excludes EnvError).
    #[must_use]
    pub fn counted_retries(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| !matches!(o.outcome, OutcomeKind::Pass | OutcomeKind::EnvError { .. }))
            .count()
    }

    /// Check if the task has exceeded max retries.
    #[must_use]
    pub fn exceeded_max_retries(&self) -> bool {
        self.counted_retries() >= self.max_retries
    }

    /// Record a new outcome.
    pub fn record(&mut self, outcome: TaskOutcome) {
        self.outcomes.push(outcome);
    }

    /// Get the latest outcome.
    #[must_use]
    pub fn latest(&self) -> Option<&TaskOutcome> {
        self.outcomes.last()
    }
}

/// The complete task graph for code generation.
///
/// Similar to `ReconstructionPlan` in rpg-encoder but for generation,
/// not reconstruction of existing code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraph {
    /// Schema version for forward compatibility
    pub version: String,

    /// When the task graph was created
    pub created_at: DateTime<Utc>,

    /// All tasks keyed by ID
    pub tasks: BTreeMap<String, GenerationTask>,

    /// Dependency-safe execution order (task IDs)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topological_order: Vec<String>,

    /// Tasks grouped into batches for execution
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub batches: Vec<TaskBatch>,
}

impl Default for TaskGraph {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
            created_at: Utc::now(),
            tasks: BTreeMap::new(),
            topological_order: Vec::new(),
            batches: Vec::new(),
        }
    }
}

impl TaskGraph {
    /// Create a new empty task graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all pending tasks.
    #[must_use]
    pub fn pending_tasks(&self) -> Vec<&GenerationTask> {
        self.tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Pending))
            .collect()
    }

    /// Get all completed tasks.
    #[must_use]
    pub fn completed_tasks(&self) -> Vec<&GenerationTask> {
        self.tasks
            .values()
            .filter(|t| {
                matches!(
                    t.status,
                    TaskStatus::Completed { .. } | TaskStatus::Validated
                )
            })
            .collect()
    }

    /// Get all failed tasks.
    #[must_use]
    pub fn failed_tasks(&self) -> Vec<&GenerationTask> {
        self.tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .collect()
    }
}

/// A planned task (before execution details are filled in).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTask {
    /// Unique task ID
    pub id: String,

    /// Target entity ID (planned, not yet resolved)
    pub planned_entity_id: String,

    /// File path where the entity will be created
    pub file_path: PathBuf,

    /// Kind of task
    pub kind: TaskKind,

    /// Dependencies on other tasks (by ID)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,

    /// Semantic features describing what this task should accomplish
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_features: Vec<String>,
}

impl PlannedTask {
    /// Create a new planned task.
    #[must_use]
    pub fn new(id: impl Into<String>, file_path: impl Into<PathBuf>, kind: TaskKind) -> Self {
        Self {
            id: id.into(),
            planned_entity_id: String::new(),
            file_path: file_path.into(),
            kind,
            dependencies: Vec::new(),
            semantic_features: Vec::new(),
        }
    }

    /// Convert to a full `GenerationTask`.
    #[must_use]
    pub fn into_generation_task(self, skeleton: EntitySkeleton) -> GenerationTask {
        GenerationTask {
            planned_id: self.id,
            resolved_entity_id: None,
            file_path: self.file_path,
            kind: self.kind,
            dependencies: self.dependencies,
            signature: None,
            semantic_features: self.semantic_features,
            skeleton,
            context_entities: Vec::new(),
            status: TaskStatus::Pending,
            acceptance_criteria: Vec::new(),
            iteration_history: TaskIterationHistory::default(),
        }
    }
}

/// A single generation task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationTask {
    /// Stable ID for the planned entity (before code exists)
    pub planned_id: String,

    /// Resolved entity ID (set after code is generated and parsed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_entity_id: Option<String>,

    /// File path where the entity will be created
    pub file_path: PathBuf,

    /// Kind of task
    pub kind: TaskKind,

    /// Dependencies on other tasks (by planned_id)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,

    /// Planned signature (reuse existing type from rpg-core)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<rpg_core::graph::Signature>,

    /// Semantic features describing behavior
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_features: Vec<String>,

    /// Entity skeleton with generation hints
    pub skeleton: EntitySkeleton,

    /// Entity IDs for context (dependencies, similar code)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_entities: Vec<String>,

    /// Current status
    #[serde(default)]
    pub status: TaskStatus,

    /// Acceptance criteria (testable conditions)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,

    /// TDD iteration history (empty by default for backward compatibility)
    #[serde(default)]
    pub iteration_history: TaskIterationHistory,
}

impl GenerationTask {
    /// Check if this task is ready to execute (all dependencies completed).
    pub fn is_ready(&self, completed: &[String]) -> bool {
        self.dependencies.iter().all(|dep| completed.contains(dep))
    }

    /// Mark the task as completed.
    pub fn mark_completed(&mut self, file_path: PathBuf, content_hash: String) {
        self.status = TaskStatus::Completed {
            file_path,
            content_hash,
        };
    }

    /// Mark the task as validated.
    pub fn mark_validated(&mut self) {
        if matches!(self.status, TaskStatus::Completed { .. }) {
            self.status = TaskStatus::Validated;
        }
    }

    /// Mark the task as failed.
    pub fn mark_failed(&mut self, error: String) {
        let retry_count = if let TaskStatus::Failed { retry_count, .. } = &self.status {
            retry_count + 1
        } else {
            1
        };
        self.status = TaskStatus::Failed { error, retry_count };
    }
}

/// Kind of generation task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Create a new file
    CreateFile,
    /// Generate a struct/class
    GenerateStruct,
    /// Generate a function
    GenerateFunction,
    /// Generate an implementation block
    GenerateImpl,
    /// Generate a test
    GenerateTest,
    /// Wire up modules (imports, exports)
    WireModule,
}

/// Status of a generation task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Not yet started
    #[default]
    Pending,
    /// Currently being generated
    InProgress,
    /// Generated but not yet validated
    Completed {
        /// Path to the generated file
        file_path: PathBuf,
        /// SHA256 hash of the content (not the content itself!)
        content_hash: String,
    },
    /// Validated against the plan
    Validated,
    /// Failed to generate or validate
    Failed {
        /// Error message
        error: String,
        /// Number of retry attempts
        retry_count: usize,
    },
}

impl TaskStatus {
    /// Check if the task is in a terminal state.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Validated | Self::Failed { .. })
    }

    /// Check if the task needs attention (failed or pending).
    #[must_use]
    pub const fn needs_attention(&self) -> bool {
        matches!(self, Self::Pending | Self::Failed { .. })
    }
}

/// A batch of tasks that can be executed together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBatch {
    /// Batch index (0-based)
    pub batch_index: usize,

    /// Functional area for coherence
    pub area: String,

    /// Task IDs in this batch
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_ids: Vec<String>,

    /// Token budget for this batch
    #[serde(default)]
    pub token_budget: usize,

    /// Batch status
    #[serde(default)]
    pub status: BatchStatus,
}

impl TaskBatch {
    /// Create a new task batch.
    #[must_use]
    pub fn new(batch_index: usize, area: impl Into<String>) -> Self {
        Self {
            batch_index,
            area: area.into(),
            task_ids: Vec::new(),
            token_budget: 0,
            status: BatchStatus::default(),
        }
    }
}

/// Status of a task batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    /// Not yet started
    #[default]
    Pending,
    /// Currently executing
    InProgress,
    /// All tasks completed
    Completed,
    /// Some tasks failed
    PartiallyFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::EntitySkeletonKind;

    #[test]
    fn test_task_graph_queries() {
        let mut graph = TaskGraph::new();

        let skeleton = EntitySkeleton::new("task1", "func1", EntitySkeletonKind::Function);
        let task1 = PlannedTask::new("task1", "src/lib.rs", TaskKind::GenerateFunction)
            .into_generation_task(skeleton);

        let skeleton2 = EntitySkeleton::new("task2", "func2", EntitySkeletonKind::Function);
        let mut task2 = PlannedTask::new("task2", "src/lib.rs", TaskKind::GenerateFunction)
            .into_generation_task(skeleton2);
        task2.mark_completed(PathBuf::from("src/lib.rs"), "abc123".to_string());

        graph.tasks.insert("task1".to_string(), task1);
        graph.tasks.insert("task2".to_string(), task2);

        assert_eq!(graph.pending_tasks().len(), 1);
        assert_eq!(graph.completed_tasks().len(), 1);
        assert_eq!(graph.failed_tasks().len(), 0);
    }

    #[test]
    fn test_task_status_transitions() {
        let skeleton = EntitySkeleton::new("test", "test", EntitySkeletonKind::Function);
        let mut task = PlannedTask::new("test", "src/test.rs", TaskKind::GenerateFunction)
            .into_generation_task(skeleton);

        assert!(matches!(task.status, TaskStatus::Pending));

        task.mark_completed(PathBuf::from("src/test.rs"), "hash".to_string());
        assert!(matches!(task.status, TaskStatus::Completed { .. }));

        task.mark_validated();
        assert!(matches!(task.status, TaskStatus::Validated));
    }

    #[test]
    fn test_task_ready_check() {
        let skeleton = EntitySkeleton::new("task", "task", EntitySkeletonKind::Function);
        let mut task = PlannedTask::new("task", "src/lib.rs", TaskKind::GenerateFunction)
            .into_generation_task(skeleton);
        task.dependencies.push("dep1".to_string());
        task.dependencies.push("dep2".to_string());

        assert!(!task.is_ready(&["dep1".to_string()]));
        assert!(task.is_ready(&["dep1".to_string(), "dep2".to_string()]));
    }

    #[test]
    fn test_task_outcome_serialization_roundtrip() {
        let outcome = TaskOutcome {
            task_id: "auth:login".to_string(),
            outcome: OutcomeKind::TestFailure {
                failing_count: 2,
                summary: "assertion failed".to_string(),
            },
            test_results: Some(TestResults {
                total: 5,
                passed: 3,
                failed: 2,
                test_file: Some(PathBuf::from("tests/test_auth.rs")),
            }),
            telemetry: Some(IterationTelemetry {
                sandbox_mode: SandboxMode::Local,
                command: Some("cargo test -p app".into()),
                runner: Some("cargo".into()),
                duration_ms: Some(250),
                stdout_bytes: Some(1200),
                stderr_bytes: Some(40),
                prompt_tokens: Some(200),
                completion_tokens: Some(80),
                model: Some("gpt-4.1".into()),
                backbone: Some("gpt".into()),
                estimated_cost_usd: Some(0.003),
                auto_adaptations: 1,
            }),
            iteration: 1,
            reported_at: Utc::now(),
        };

        let json = serde_json::to_string(&outcome).unwrap();
        let deserialized: TaskOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.task_id, "auth:login");
        assert_eq!(deserialized.iteration, 1);
        assert!(matches!(
            deserialized.outcome,
            OutcomeKind::TestFailure {
                failing_count: 2,
                ..
            }
        ));
        assert_eq!(deserialized.test_results.unwrap().failed, 2);
        assert_eq!(
            deserialized.telemetry.unwrap().sandbox_mode,
            SandboxMode::Local
        );
    }

    #[test]
    fn test_outcome_kind_variants_roundtrip() {
        let variants: Vec<OutcomeKind> = vec![
            OutcomeKind::Pass,
            OutcomeKind::TestFailure {
                failing_count: 1,
                summary: "fail".into(),
            },
            OutcomeKind::CodeError {
                error_message: "syntax error".into(),
            },
            OutcomeKind::TestError {
                error_message: "test setup".into(),
            },
            OutcomeKind::EnvError {
                error_message: "missing cargo".into(),
            },
        ];

        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let _: OutcomeKind = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn test_iteration_history_retry_counting() {
        let mut history = TaskIterationHistory::default();
        assert_eq!(history.counted_retries(), 0);
        assert!(!history.exceeded_max_retries());

        // Record a test failure (counts)
        history.record(TaskOutcome {
            task_id: "t1".into(),
            outcome: OutcomeKind::TestFailure {
                failing_count: 1,
                summary: "fail".into(),
            },
            test_results: None,
            telemetry: None,
            iteration: 0,
            reported_at: Utc::now(),
        });
        assert_eq!(history.counted_retries(), 1);

        // Record an env error (does NOT count)
        history.record(TaskOutcome {
            task_id: "t1".into(),
            outcome: OutcomeKind::EnvError {
                error_message: "missing tool".into(),
            },
            test_results: None,
            telemetry: None,
            iteration: 1,
            reported_at: Utc::now(),
        });
        assert_eq!(history.counted_retries(), 1);

        // Record two more code errors (counts)
        for i in 2..4 {
            history.record(TaskOutcome {
                task_id: "t1".into(),
                outcome: OutcomeKind::CodeError {
                    error_message: "err".into(),
                },
                test_results: None,
                telemetry: None,
                iteration: i,
                reported_at: Utc::now(),
            });
        }
        assert_eq!(history.counted_retries(), 3);
        assert!(history.exceeded_max_retries());
    }

    #[test]
    fn test_iteration_history_default_serde() {
        // Ensure backward compatibility: deserializing a task without iteration_history works
        let json = r#"{
            "planned_id": "test",
            "file_path": "src/test.rs",
            "kind": "generate_function",
            "skeleton": {"id": "test:func", "name": "func", "kind": "function"},
            "status": "pending"
        }"#;
        let task: GenerationTask = serde_json::from_str(json).unwrap();
        assert!(task.iteration_history.outcomes.is_empty());
        assert_eq!(task.iteration_history.max_retries, 3);
    }
}
