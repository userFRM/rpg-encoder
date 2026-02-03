//! RPG encoding pipeline: semantic lifting, hierarchy construction, and incremental evolution.
//!
//! Implements the three-phase encoding from the RPG paper:
//! semantic lifting (LLM feature extraction), hierarchy construction (structure reorganization),
//! grounding (artifact anchoring), plus incremental update algorithms.

pub mod embeddings;
pub mod evolution;
pub mod grounding;
pub mod hierarchy;
pub mod lift;
pub mod llm;
pub mod semantic_lifting;
