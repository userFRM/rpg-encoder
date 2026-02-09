//! Configuration for RPG encoding and navigation settings.
//!
//! Load order: `.rpg/config.toml` → environment variables → defaults.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level RPG configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RpgConfig {
    pub encoding: EncodingConfig,
    pub navigation: NavigationConfig,
    pub storage: StorageConfig,
}

/// Storage configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Compress graph.json with zstd before writing.
    /// Decompression on load is automatic (detected by magic bytes).
    pub compress: bool,
}

/// Encoding pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EncodingConfig {
    /// Maximum number of entities per batch (hard cap).
    pub batch_size: usize,
    /// Token budget per batch — batches are filled until this limit.
    /// Aligns with the paper's "controlled token budget" batching strategy.
    pub max_batch_tokens: usize,
    /// Number of entities per hierarchy construction chunk.
    pub hierarchy_chunk_size: usize,
    /// Jaccard distance threshold to trigger hierarchy re-routing (legacy, midpoint reference).
    pub drift_threshold: f64,
    /// Drift below this threshold is ignored (minor edit). Default: 0.3.
    pub drift_ignore_threshold: f64,
    /// Drift above this threshold triggers automatic routing. Default: 0.7.
    /// Drift between ignore and auto is "borderline" — agent is asked to judge.
    pub drift_auto_threshold: f64,
    /// Whether to broadcast file-level imports to entities without call-site info.
    /// When false (default), entities without invokes/inherits get no import edges.
    /// The paper says E_dep via "AST analysis" — broadcasting contradicts this.
    pub broadcast_imports: bool,
    /// Maximum depth for the structural file-path fallback hierarchy.
    /// The semantic hierarchy is always 3-level per paper spec.
    pub max_hierarchy_depth: usize,
}

/// Navigation and search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationConfig {
    /// Maximum number of search results returned.
    pub search_result_limit: usize,
}

impl Default for EncodingConfig {
    fn default() -> Self {
        Self {
            batch_size: 50,
            max_batch_tokens: 8000,
            hierarchy_chunk_size: 50,
            drift_threshold: 0.5,
            drift_ignore_threshold: 0.3,
            drift_auto_threshold: 0.7,
            broadcast_imports: false,
            max_hierarchy_depth: 3,
        }
    }
}

impl Default for NavigationConfig {
    fn default() -> Self {
        Self {
            search_result_limit: 10,
        }
    }
}

/// Helper to parse an env var and apply it to a config field.
fn env_override<T: std::str::FromStr>(var: &str, target: &mut T) {
    if let Ok(v) = std::env::var(var)
        && let Ok(n) = v.parse()
    {
        *target = n;
    }
}

impl RpgConfig {
    /// Load config from `.rpg/config.toml` in the project root, with env var overrides.
    /// Falls back to defaults if no config file exists.
    pub fn load(project_root: &Path) -> Result<Self> {
        let config_path = project_root.join(".rpg").join("config.toml");

        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            Self::default()
        };

        // Environment variable overrides
        env_override(
            "RPG_DRIFT_IGNORE_THRESHOLD",
            &mut config.encoding.drift_ignore_threshold,
        );
        env_override(
            "RPG_DRIFT_AUTO_THRESHOLD",
            &mut config.encoding.drift_auto_threshold,
        );
        env_override("RPG_BATCH_SIZE", &mut config.encoding.batch_size);
        env_override(
            "RPG_MAX_BATCH_TOKENS",
            &mut config.encoding.max_batch_tokens,
        );
        env_override(
            "RPG_HIERARCHY_CHUNK_SIZE",
            &mut config.encoding.hierarchy_chunk_size,
        );
        env_override("RPG_DRIFT_THRESHOLD", &mut config.encoding.drift_threshold);
        env_override(
            "RPG_SEARCH_LIMIT",
            &mut config.navigation.search_result_limit,
        );

        // Validate drift thresholds
        if config.encoding.drift_ignore_threshold >= config.encoding.drift_auto_threshold {
            anyhow::bail!(
                "drift_ignore_threshold ({}) must be less than drift_auto_threshold ({})",
                config.encoding.drift_ignore_threshold,
                config.encoding.drift_auto_threshold,
            );
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RpgConfig::default();
        assert_eq!(config.encoding.batch_size, 50);
        assert_eq!(config.encoding.max_batch_tokens, 8000);
        assert_eq!(config.encoding.hierarchy_chunk_size, 50);
        assert_eq!(config.encoding.drift_threshold, 0.5);
        assert_eq!(config.encoding.drift_ignore_threshold, 0.3);
        assert_eq!(config.encoding.drift_auto_threshold, 0.7);
        assert_eq!(config.navigation.search_result_limit, 10);
    }

    #[test]
    fn test_config_from_toml() {
        let toml_str = r"
[encoding]
batch_size = 64
max_batch_tokens = 24000

[navigation]
search_result_limit = 20
";
        let config: RpgConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.encoding.batch_size, 64);
        assert_eq!(config.encoding.max_batch_tokens, 24000);
        assert_eq!(config.navigation.search_result_limit, 20);
        // Defaults for unspecified fields
        assert_eq!(config.encoding.hierarchy_chunk_size, 50);
        assert_eq!(config.encoding.drift_threshold, 0.5);
    }

    #[test]
    fn test_config_load_nonexistent() {
        let config = RpgConfig::load(Path::new("/nonexistent/path")).unwrap();
        assert_eq!(config.encoding.batch_size, 50);
    }
}
