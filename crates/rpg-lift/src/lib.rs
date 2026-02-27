//! Autonomous LLM-driven semantic lifting for RPG.
//!
//! This crate provides a fire-and-forget CLI pipeline that calls cheap LLM APIs
//! (Anthropic Haiku, OpenAI GPT-4o-mini) to perform the full semantic lifting
//! workflow without a connected coding agent.
//!
//! # Architecture
//!
//! - **provider**: `LlmProvider` trait with Anthropic and OpenAI implementations
//! - **pipeline**: Orchestrates auto-lift → LLM lifting → synthesis → hierarchy
//! - **cost**: Pre-scan cost estimation and runtime tracking
//! - **progress**: Terminal progress bars via `indicatif`

pub mod cost;
pub mod pipeline;
pub mod progress;
pub mod provider;

pub use cost::{CostEstimate, estimate_cost};
pub use pipeline::{LiftConfig, LiftReport, PipelineError, run_pipeline};
pub use provider::{LlmProvider, ProviderError, available_providers, create_provider};
