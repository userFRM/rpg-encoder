//! JSON schema validation and version handling for RPG files.

use crate::graph::RPGraph;
use anyhow::{Context, Result};

const CURRENT_VERSION: &str = "2.0.0";

/// Validate an RPGraph's schema version.
pub fn validate_version(graph: &RPGraph) -> Result<()> {
    if graph.version != CURRENT_VERSION {
        anyhow::bail!(
            "RPG version mismatch: expected {}, found {}",
            CURRENT_VERSION,
            graph.version
        );
    }
    Ok(())
}

/// Serialize an RPGraph to a pretty-printed JSON string.
pub fn to_json(graph: &RPGraph) -> Result<String> {
    serde_json::to_string_pretty(graph).context("failed to serialize RPG to JSON")
}

/// Deserialize an RPGraph from a JSON string.
pub fn from_json(json: &str) -> Result<RPGraph> {
    let graph: RPGraph =
        serde_json::from_str(json).context("failed to deserialize RPG from JSON")?;
    validate_version(&graph)?;
    Ok(graph)
}
