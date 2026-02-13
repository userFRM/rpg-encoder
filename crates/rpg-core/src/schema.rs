//! JSON schema validation and version handling for RPG files.
//!
//! Uses semver-compatible version checking: graphs are accepted if their
//! major version matches the current schema. Minor/patch differences are
//! handled by `migrate()`.

use crate::graph::RPGraph;
use anyhow::{Context, Result};
use semver::Version;

const CURRENT_VERSION: &str = "2.1.0";

/// Validate an RPGraph's schema version using semver compatibility.
///
/// Accepts any version with the same major version as CURRENT_VERSION.
/// For example, if current is 2.0.0, accepts 2.0.0, 2.1.0, 2.0.3, etc.
/// Rejects 1.x.x or 3.x.x.
pub fn validate_version(graph: &RPGraph) -> Result<()> {
    let current = Version::parse(CURRENT_VERSION).context("invalid CURRENT_VERSION constant")?;
    let found = Version::parse(&graph.version)
        .with_context(|| format!("invalid RPG version string: {}", graph.version))?;

    if found.major != current.major {
        anyhow::bail!(
            "RPG major version mismatch: schema requires {}.x.x, found {}",
            current.major,
            graph.version
        );
    }

    Ok(())
}

/// Apply any necessary migrations to bring the graph up to the current version.
///
/// Called after deserialization when the version is compatible but not identical.
/// Currently a no-op — add transformation logic here when schema changes are made.
pub fn migrate(graph: &mut RPGraph) -> Result<()> {
    let current = Version::parse(CURRENT_VERSION)?;
    let found = Version::parse(&graph.version)?;

    if found < current {
        // Future migrations go here, e.g.:
        // if found < Version::new(2, 1, 0) { ... }
        graph.version = CURRENT_VERSION.to_string();
    }

    Ok(())
}

/// Serialize an RPGraph to a pretty-printed JSON string.
///
/// Edges are sorted by (source, target, kind) for deterministic output,
/// ensuring minimal git diffs when the graph is re-saved.
pub fn to_json(graph: &RPGraph) -> Result<String> {
    let mut graph = graph.clone();
    graph.edges.sort();
    serde_json::to_string_pretty(&graph).context("failed to serialize RPG to JSON")
}

/// Deserialize an RPGraph from a JSON string.
pub fn from_json(json: &str) -> Result<RPGraph> {
    let mut graph: RPGraph =
        serde_json::from_str(json).context("failed to deserialize RPG from JSON")?;
    validate_version(&graph)?;
    migrate(&mut graph)?;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_with_version(version: &str) -> RPGraph {
        let json = format!(
            r#"{{
                "version": "{}",
                "created_at": "2025-01-01T00:00:00Z",
                "updated_at": "2025-01-01T00:00:00Z",
                "metadata": {{
                    "language": "rust",
                    "total_files": 0,
                    "total_entities": 0,
                    "functional_areas": 0,
                    "total_edges": 0,
                    "dependency_edges": 0,
                    "containment_edges": 0
                }},
                "entities": {{}},
                "edges": [],
                "hierarchy": {{}},
                "file_index": {{}}
            }}"#,
            version
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn test_exact_version_match() {
        let graph = graph_with_version("2.0.0");
        assert!(validate_version(&graph).is_ok());
    }

    #[test]
    fn test_compatible_minor_bump() {
        let graph = graph_with_version("2.1.0");
        assert!(validate_version(&graph).is_ok());
    }

    #[test]
    fn test_compatible_patch_bump() {
        let graph = graph_with_version("2.0.3");
        assert!(validate_version(&graph).is_ok());
    }

    #[test]
    fn test_incompatible_major_bump() {
        let graph = graph_with_version("3.0.0");
        assert!(validate_version(&graph).is_err());
    }

    #[test]
    fn test_incompatible_old_major() {
        let graph = graph_with_version("1.0.0");
        assert!(validate_version(&graph).is_err());
    }

    #[test]
    fn test_migrate_updates_version() {
        let mut graph = graph_with_version("2.0.0");
        graph.version = "2.0.0".to_string();
        assert!(migrate(&mut graph).is_ok());
        assert_eq!(graph.version, CURRENT_VERSION);
    }

    #[test]
    fn test_invalid_version_string() {
        let graph = graph_with_version("not-a-version");
        assert!(validate_version(&graph).is_err());
    }
}
