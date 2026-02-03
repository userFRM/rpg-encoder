//! Navigation tools for querying the Repository Planning Graph.
//!
//! Provides SearchNode (intent-based discovery), FetchNode (entity details),
//! ExploreRPG (dependency traversal), and TOON serialization for LLM-optimized output.

pub mod embedding_search;
pub mod explore;
pub mod fetch;
pub mod search;
pub mod toon;
