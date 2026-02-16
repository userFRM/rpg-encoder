//! # rpg-gen
//!
//! Code generation planning and orchestration for RPG.
//!
//! This crate provides the infrastructure for generating code from specifications,
//! using the connected coding agent as the code writer. It includes:
//!
//! - **Spec decomposition**: Break down natural language specs into feature trees
//! - **Interface design**: Design module interfaces and data types before implementation
//! - **Task scheduling**: Generate dependency-ordered tasks for code generation
//! - **Validation**: Compare generated code against the plan using rpg-encoder
//!
//! ## Architecture
//!
//! Unlike traditional code generators, rpg-gen orchestrates the connected coding agent
//! (Claude Code, Cursor, etc.) as the code writer. We provide:
//!
//! 1. Planning tools (spec → structured plan)
//! 2. Task distribution (serve dependency-ordered tasks)
//! 3. Validation (use rpg-encoder to verify generated code matches plan)

pub mod interfaces;
pub mod plan;
pub mod skeleton;
pub mod spec;
pub mod tasks;
pub mod validation;

// Re-export main types for convenience
pub use interfaces::{
    DataTypeKind, DataTypeSpec, FieldSpec, FunctionSignature, InterfaceDepKind, InterfaceDesign,
    ModuleInterface, Parameter, Visibility,
};
pub use plan::{BackboneStats, GenerationPhase, GenerationPlan, GenerationState, GenerationStats};
pub use skeleton::{
    BodyHint, EntitySkeleton, EntitySkeletonKind, FileKind, FileSkeleton, FileSkeletonSet,
};
pub use spec::{
    Complexity, Constraint, ConstraintKind, Feature, FeatureArea, FeatureTree, QualityRequirement,
};
pub use tasks::{
    BatchStatus, GenerationTask, IterationTelemetry, OutcomeKind, PlannedTask, SandboxMode,
    TaskBatch, TaskGraph, TaskIterationHistory, TaskKind, TaskOutcome, TaskStatus, TestResults,
};
pub use validation::{
    IssueCategory, IssueSeverity, ValidationCoverage, ValidationIssue, ValidationResult,
    ValidationStatus,
};

/// Prompt for spec decomposition (LLM guidance)
pub const SPEC_DECOMPOSITION_PROMPT: &str = include_str!("prompts/spec_decomposition.md");

/// Prompt for interface design (LLM guidance)
pub const INTERFACE_DESIGN_PROMPT: &str = include_str!("prompts/interface_design.md");

/// Prompt for task instructions (LLM guidance)
pub const TASK_INSTRUCTIONS_PROMPT: &str = include_str!("prompts/task_instructions.md");

/// Prompt for TDD-enhanced task instructions (LLM guidance)
pub const TDD_TASK_INSTRUCTIONS_PROMPT: &str = include_str!("prompts/tdd_task_instructions.md");

/// Prompt for validation critique (LLM guidance)
pub const VALIDATION_CRITIQUE_PROMPT: &str = include_str!("prompts/validation_critique.md");
