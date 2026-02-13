//! RPG encoding pipeline: semantic lifting, hierarchy construction, and incremental evolution.
//!
//! Implements the three-phase encoding from the RPG paper:
//! semantic lifting (feature extraction), hierarchy construction (structure reorganization),
//! grounding (artifact anchoring), plus incremental update algorithms.
//!
//! Semantic lifting is performed by the connected coding agent via the MCP interactive
//! protocol (get_entities_for_lifting → submit_lift_results), not by external LLM API calls.

pub mod critic;
pub mod evolution;
pub mod grounding;
pub mod hierarchy;
pub mod lift;
pub mod reconstruction;
pub mod semantic_lifting;
