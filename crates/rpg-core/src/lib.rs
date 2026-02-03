//! Core types and storage for the Repository Planning Graph (RPG).
//!
//! Provides the graph data model ([`graph::RPGraph`]), entity types, dependency edges,
//! hierarchy nodes, JSON persistence, and LCA-based directory grounding.

pub mod config;
pub mod graph;
pub mod lca;
pub mod schema;
pub mod storage;
