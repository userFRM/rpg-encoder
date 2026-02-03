//! Configuration for RPG encoding, navigation, and LLM settings.
//!
//! Load order: `.rpg/config.toml` → environment variables → defaults.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level RPG configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RpgConfig {
    pub llm: LlmConfig,
    pub encoding: EncodingConfig,
    pub navigation: NavigationConfig,
    pub embeddings: EmbeddingConfig,
}

/// LLM-related configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// Maximum tokens per LLM request.
    pub max_tokens: u32,
    /// Number of retry attempts on transient failures.
    pub retry_attempts: u32,
    /// Delay between retries in milliseconds (exponential backoff base values).
    pub retry_delays_ms: Vec<u64>,
    /// Force a specific provider: "anthropic", "openai", "ollama", "local".
    pub provider: Option<String>,
    /// Model name for local/Ollama providers.
    pub local_model: String,
    /// Base URL for local/Ollama providers.
    pub local_url: String,
    /// Whether to auto-pull missing Ollama models.
    pub auto_pull: bool,
}

/// Encoding pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EncodingConfig {
    /// Maximum number of entities per LLM batch (hard cap).
    pub batch_size: usize,
    /// Token budget per batch — batches are filled until this limit.
    /// Aligns with the paper's "controlled token budget" batching strategy.
    pub max_batch_tokens: usize,
    /// Number of entities per hierarchy construction chunk.
    pub hierarchy_chunk_size: usize,
    /// Number of hierarchy chunks to process concurrently.
    pub hierarchy_concurrency: usize,
    /// Jaccard distance threshold to trigger hierarchy re-routing.
    pub drift_threshold: f64,
}

/// Navigation and search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationConfig {
    /// Maximum number of search results returned.
    pub search_result_limit: usize,
}

/// Embedding generation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    /// Embedding provider: "ollama", "openai", or "local".
    pub provider: String,
    /// Embedding model name.
    pub model: String,
    /// Batch size for embedding generation.
    pub batch_size: usize,
    /// Weight for semantic (embedding) score in hybrid search (0.0-1.0).
    pub semantic_weight: f64,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            model: String::new(),
            batch_size: 256,
            semantic_weight: 0.5,
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            max_tokens: 8192,
            retry_attempts: 3,
            retry_delays_ms: vec![1000, 2000, 4000],
            provider: None,
            local_model: "qwen2.5-coder:7b".to_string(),
            local_url: "http://localhost:11434".to_string(),
            auto_pull: true,
        }
    }
}

impl Default for EncodingConfig {
    fn default() -> Self {
        Self {
            batch_size: 16,
            max_batch_tokens: 8000,
            hierarchy_chunk_size: 50,
            hierarchy_concurrency: 4,
            drift_threshold: 0.5,
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
        env_override("RPG_BATCH_SIZE", &mut config.encoding.batch_size);
        env_override(
            "RPG_MAX_BATCH_TOKENS",
            &mut config.encoding.max_batch_tokens,
        );
        env_override("RPG_MAX_TOKENS", &mut config.llm.max_tokens);
        env_override("RPG_RETRY_ATTEMPTS", &mut config.llm.retry_attempts);
        env_override(
            "RPG_HIERARCHY_CHUNK_SIZE",
            &mut config.encoding.hierarchy_chunk_size,
        );
        env_override("RPG_DRIFT_THRESHOLD", &mut config.encoding.drift_threshold);
        env_override(
            "RPG_HIERARCHY_CONCURRENCY",
            &mut config.encoding.hierarchy_concurrency,
        );
        env_override(
            "RPG_SEARCH_LIMIT",
            &mut config.navigation.search_result_limit,
        );

        // LLM provider overrides
        if let Ok(v) = std::env::var("RPG_PROVIDER") {
            config.llm.provider = Some(v);
        }
        if let Ok(v) = std::env::var("RPG_LOCAL_MODEL") {
            config.llm.local_model = v;
        }
        if let Ok(v) = std::env::var("RPG_LOCAL_URL") {
            config.llm.local_url = v;
        }

        // Embedding provider overrides
        if let Ok(v) = std::env::var("RPG_EMBEDDING_PROVIDER") {
            config.embeddings.provider = v;
        }
        if let Ok(v) = std::env::var("RPG_EMBEDDING_MODEL") {
            config.embeddings.model = v;
        }
        env_override(
            "RPG_EMBEDDING_BATCH_SIZE",
            &mut config.embeddings.batch_size,
        );

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RpgConfig::default();
        assert_eq!(config.llm.max_tokens, 8192);
        assert_eq!(config.llm.retry_attempts, 3);
        assert_eq!(config.encoding.batch_size, 16);
        assert_eq!(config.encoding.max_batch_tokens, 8000);
        assert_eq!(config.encoding.hierarchy_chunk_size, 50);
        assert_eq!(config.encoding.hierarchy_concurrency, 4);
        assert_eq!(config.encoding.drift_threshold, 0.5);
        assert_eq!(config.navigation.search_result_limit, 10);
    }

    #[test]
    fn test_config_from_toml() {
        let toml_str = r#"
[llm]
max_tokens = 16384
retry_attempts = 5

[encoding]
batch_size = 64
max_batch_tokens = 24000

[navigation]
search_result_limit = 20
"#;
        let config: RpgConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.max_tokens, 16384);
        assert_eq!(config.llm.retry_attempts, 5);
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
        assert_eq!(config.llm.max_tokens, 8192);
    }
}
