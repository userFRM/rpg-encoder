//! # rpg-build
//!
//! Design Repository Planning Graphs from natural-language specifications.
//!
//! `rpg-build` is the inverse of `rpg-encoder`: where `rpg-encoder` extracts a
//! semantic graph from existing source code, `rpg-build` produces a graph from
//! a natural-language project specification. The output is a complete
//! [`RPGraph`](rpg_core::graph::RPGraph) with hierarchy, entities, dependencies,
//! and verb-object features, but no source code. The connected coding agent
//! walks the graph in dependency-safe order and generates the implementation.
//!
//! Inspired by the ZeroRepo paper (Luo et al., 2026, arXiv:2509.16198), but
//! limited to the design phases (feature planning + architecture). Code
//! generation is left to the connected coding agent.
//!
//! ## Example
//!
//! ```no_run
//! use rpg_build::{design_rpg, DesignConfig};
//! use rpg_lift::create_provider;
//!
//! let provider = create_provider("anthropic", "sk-ant-...", None, None).unwrap();
//! let config = DesignConfig {
//!     provider: provider.as_ref(),
//!     max_retries: 2,
//! };
//!
//! let (graph, report) = design_rpg("A multiplayer snake game with AI opponents", &config)
//!     .expect("design failed");
//!
//! println!("Designed {} entities across {} areas (cost: ${:.4})",
//!     graph.entities.len(), graph.hierarchy.len(), report.cost_usd);
//! ```

pub mod design;
pub mod prompts;

pub use design::{
    DesignConfig, DesignError, DesignReport, design_rpg, parse_design_response, rpg_from_design,
};
