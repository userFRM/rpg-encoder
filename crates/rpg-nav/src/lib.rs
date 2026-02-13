//! Navigation tools for querying the Repository Planning Graph.
//!
//! Provides SearchNode (intent-based discovery), FetchNode (entity details),
//! ExploreRPG (dependency traversal), and TOON serialization for LLM-optimized output.

pub mod context;
pub mod dataflow;
pub mod diff;
#[cfg(feature = "embeddings")]
pub mod embeddings;
pub mod explore;
pub mod export;
pub mod fetch;
pub mod impact;
pub mod paths;
pub mod planner;
pub mod search;
pub mod slice;
pub mod toon;
