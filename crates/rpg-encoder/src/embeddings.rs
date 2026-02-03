//! Embedding generation for semantic similarity search.
//!
//! Supports multiple providers for generating dense vector embeddings
//! of code entities based on their semantic features and metadata:
//! - **Ollama** — local embeddings via Ollama's API (default, zero config if Ollama running)
//! - **OpenAI** — cloud embeddings via OpenAI's text-embedding API
//! - **Local** — offline ONNX-based embeddings via fastembed (requires `local-embeddings` feature)

use anyhow::{Context, Result};
use rpg_core::config::EmbeddingConfig;
use serde::{Deserialize, Serialize};

/// Embedding provider configuration.
#[derive(Debug, Clone)]
pub enum EmbeddingProvider {
    /// OpenAI text-embedding API.
    OpenAI { api_key: String, model: String },
    /// Ollama local embedding API.
    Ollama { base_url: String, model: String },
    /// Local ONNX-based embeddings via fastembed (no network required).
    #[cfg(feature = "local-embeddings")]
    Local,
}

/// Embedding generator that produces dense vectors for code entities.
pub struct EmbeddingGenerator {
    provider: EmbeddingProvider,
    http: reqwest::Client,
}

#[derive(Serialize)]
struct OpenAIEmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
}

#[derive(Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl EmbeddingGenerator {
    /// Create a new embedding generator from an `EmbeddingConfig`.
    ///
    /// This is the preferred constructor — it respects config.toml + env var overrides.
    /// Detection order (for provider = "auto"):
    /// 1. If `OPENAI_API_KEY` is set → OpenAI
    /// 2. If `local-embeddings` feature enabled → Local fastembed
    /// 3. Fallback → Ollama at localhost:11434
    pub fn from_config(config: &EmbeddingConfig) -> Result<Self> {
        let provider = config.provider.as_str();
        let model = if config.model.is_empty() {
            None
        } else {
            Some(config.model.clone())
        };

        match provider {
            "openai" => {
                let api_key = std::env::var("OPENAI_API_KEY")
                    .context("embeddings.provider=openai but OPENAI_API_KEY not set")?;
                let model = model.unwrap_or_else(|| "text-embedding-3-small".to_string());
                Ok(Self::new(EmbeddingProvider::OpenAI { api_key, model }))
            }
            "ollama" => {
                let base_url = std::env::var("RPG_LOCAL_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());
                let model = model.unwrap_or_else(|| "nomic-embed-text".to_string());
                Ok(Self::new(EmbeddingProvider::Ollama { base_url, model }))
            }
            #[cfg(feature = "local-embeddings")]
            "local" => Ok(Self::new(EmbeddingProvider::Local)),
            #[cfg(not(feature = "local-embeddings"))]
            "local" => {
                anyhow::bail!(
                    "local embeddings require the 'local-embeddings' feature. \
                     Rebuild with: cargo build --features local-embeddings"
                )
            }
            "auto" | "" => {
                // Auto-detect: OpenAI if API key available
                if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
                    let model = model.unwrap_or_else(|| "text-embedding-3-small".to_string());
                    return Ok(Self::new(EmbeddingProvider::OpenAI { api_key, model }));
                }
                // Local fastembed if feature enabled
                #[cfg(feature = "local-embeddings")]
                {
                    return Ok(Self::new(EmbeddingProvider::Local));
                }
                // Fallback to Ollama
                #[allow(unreachable_code)]
                {
                    let base_url = std::env::var("RPG_LOCAL_URL")
                        .unwrap_or_else(|_| "http://localhost:11434".to_string());
                    let model = model.unwrap_or_else(|| "nomic-embed-text".to_string());
                    Ok(Self::new(EmbeddingProvider::Ollama { base_url, model }))
                }
            }
            other => anyhow::bail!("unknown embedding provider: {}", other),
        }
    }

    /// Create a new embedding generator from environment variables only.
    ///
    /// Convenience wrapper that builds a default `EmbeddingConfig` with env overrides.
    /// Prefer `from_config()` when a loaded config is available.
    pub fn from_env() -> Result<Self> {
        let mut config = EmbeddingConfig::default();
        if let Ok(v) = std::env::var("RPG_EMBEDDING_PROVIDER") {
            config.provider = v;
        }
        if let Ok(v) = std::env::var("RPG_EMBEDDING_MODEL") {
            config.model = v;
        }
        Self::from_config(&config)
    }

    /// Create a generator with an explicit provider.
    pub fn new(provider: EmbeddingProvider) -> Self {
        Self {
            provider,
            http: reqwest::Client::new(),
        }
    }

    /// Build the text representation of an entity for embedding.
    /// Combines kind, name, semantic features, and hierarchy path.
    pub fn entity_text(entity: &rpg_core::graph::Entity) -> String {
        let kind = format!("{:?}", entity.kind).to_lowercase();
        let mut text = if entity.semantic_features.is_empty() {
            format!("{} {}", kind, entity.name)
        } else {
            format!(
                "{} {} — {}",
                kind,
                entity.name,
                entity.semantic_features.join(", ")
            )
        };
        if !entity.hierarchy_path.is_empty() {
            text.push_str(&format!(" [{}]", entity.hierarchy_path));
        }
        text
    }

    /// Generate embeddings for a batch of text inputs.
    pub async fn generate_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        match &self.provider {
            EmbeddingProvider::OpenAI { api_key, model } => {
                self.generate_batch_openai(api_key, model, texts).await
            }
            EmbeddingProvider::Ollama { base_url, model } => {
                self.generate_batch_ollama(base_url, model, texts).await
            }
            #[cfg(feature = "local-embeddings")]
            EmbeddingProvider::Local => generate_local_embeddings(texts),
        }
    }

    /// Generate embeddings via OpenAI API.
    async fn generate_batch_openai(
        &self,
        api_key: &str,
        model: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>> {
        let req = OpenAIEmbeddingRequest {
            model: model.to_string(),
            input: texts.to_vec(),
        };

        let resp = self
            .http
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&req)
            .send()
            .await
            .context("failed to call OpenAI embeddings API")?;

        let body = resp
            .json::<OpenAIEmbeddingResponse>()
            .await
            .context("failed to parse OpenAI embeddings response")?;

        let mut embeddings: Vec<Vec<f32>> = body.data.into_iter().map(|d| d.embedding).collect();

        if embeddings.len() != texts.len() {
            anyhow::bail!(
                "embedding count mismatch: expected {}, got {}",
                texts.len(),
                embeddings.len()
            );
        }

        for emb in &mut embeddings {
            normalize_l2(emb);
        }

        Ok(embeddings)
    }

    /// Generate embeddings via Ollama's /api/embed endpoint.
    async fn generate_batch_ollama(
        &self,
        base_url: &str,
        model: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", base_url);
        let req = OllamaEmbedRequest {
            model: model.to_string(),
            input: texts.to_vec(),
        };

        let resp = self
            .http
            .post(&url)
            .header("content-type", "application/json")
            .json(&req)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to call Ollama embeddings API at {}. Is Ollama running?",
                    url
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama embed API returned {}: {}", status, body);
        }

        let body = resp
            .json::<OllamaEmbedResponse>()
            .await
            .context("failed to parse Ollama embeddings response")?;

        let mut embeddings = body.embeddings;

        if embeddings.len() != texts.len() {
            anyhow::bail!(
                "embedding count mismatch: expected {}, got {}",
                texts.len(),
                embeddings.len()
            );
        }

        for emb in &mut embeddings {
            normalize_l2(emb);
        }

        Ok(embeddings)
    }

    /// Generate an embedding for a single text query.
    pub async fn generate_single(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.generate_batch(&[text.to_string()]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))
    }

    /// Generate embeddings for a set of entities and apply them to the graph.
    pub async fn embed_entities(
        &self,
        graph: &mut rpg_core::graph::RPGraph,
        batch_size: usize,
    ) -> Result<usize> {
        let entity_ids: Vec<String> = graph.entities.keys().cloned().collect();
        let texts: Vec<String> = entity_ids
            .iter()
            .filter_map(|id| graph.entities.get(id))
            .map(Self::entity_text)
            .collect();
        let mut total = 0;

        for (batch_idx, chunk) in texts.chunks(batch_size).enumerate() {
            let embeddings = self.generate_batch(chunk).await?;
            let start = batch_idx * batch_size;
            for (i, embedding) in embeddings.into_iter().enumerate() {
                if let Some(entity) = graph.entities.get_mut(&entity_ids[start + i]) {
                    entity.embedding = Some(embedding);
                    total += 1;
                }
            }
        }

        Ok(total)
    }

    /// Return a human-readable description of the active provider.
    pub fn provider_name(&self) -> &str {
        match &self.provider {
            EmbeddingProvider::OpenAI { .. } => "OpenAI",
            EmbeddingProvider::Ollama { .. } => "Ollama",
            #[cfg(feature = "local-embeddings")]
            EmbeddingProvider::Local => "Local (fastembed)",
        }
    }
}

/// L2-normalize a vector in place.
fn normalize_l2(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Compute cosine similarity between two vectors.
/// Assumes both are L2-normalized (dot product = cosine similarity).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Generate embeddings using fastembed local ONNX models.
#[cfg(feature = "local-embeddings")]
pub fn generate_local_embeddings(texts: &[String]) -> Result<Vec<Vec<f32>>> {
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

    let model = TextEmbedding::try_new(InitOptions {
        model_name: EmbeddingModel::AllMiniLML6V2,
        ..Default::default()
    })
    .context("failed to initialize fastembed model")?;

    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let mut embeddings = model
        .embed(text_refs, None)
        .context("fastembed embedding generation failed")?;

    for emb in &mut embeddings {
        normalize_l2(emb);
    }

    Ok(embeddings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_l2() {
        let mut v = vec![3.0, 4.0];
        normalize_l2(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_l2_zero() {
        let mut v = vec![0.0, 0.0];
        normalize_l2(&mut v);
        assert_eq!(v, vec![0.0, 0.0]);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![0.6, 0.8];
        let b = vec![0.6, 0.8];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn test_entity_text_with_features() {
        let entity = rpg_core::graph::Entity {
            id: "test".to_string(),
            kind: rpg_core::graph::EntityKind::Function,
            name: "process_data".to_string(),
            file: std::path::PathBuf::from("test.py"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: vec!["data processing".to_string(), "validation".to_string()],
            hierarchy_path: String::new(),
            deps: rpg_core::graph::EntityDeps::default(),
            embedding: None,
        };
        let text = EmbeddingGenerator::entity_text(&entity);
        assert!(text.contains("function"));
        assert!(text.contains("process_data"));
        assert!(text.contains("data processing"));
    }

    #[test]
    fn test_entity_text_no_features() {
        let entity = rpg_core::graph::Entity {
            id: "test".to_string(),
            kind: rpg_core::graph::EntityKind::Class,
            name: "MyClass".to_string(),
            file: std::path::PathBuf::from("test.py"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: Vec::new(),
            hierarchy_path: String::new(),
            deps: rpg_core::graph::EntityDeps::default(),
            embedding: None,
        };
        let text = EmbeddingGenerator::entity_text(&entity);
        assert_eq!(text, "class MyClass");
    }

    #[test]
    fn test_entity_text_with_hierarchy() {
        let entity = rpg_core::graph::Entity {
            id: "test".to_string(),
            kind: rpg_core::graph::EntityKind::Function,
            name: "validate".to_string(),
            file: std::path::PathBuf::from("test.py"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: vec!["validate input".to_string()],
            hierarchy_path: "DataProcessing/validation/input checks".to_string(),
            deps: rpg_core::graph::EntityDeps::default(),
            embedding: None,
        };
        let text = EmbeddingGenerator::entity_text(&entity);
        assert!(text.contains("[DataProcessing/validation/input checks]"));
    }

    #[test]
    fn test_provider_name() {
        let generator = EmbeddingGenerator::new(EmbeddingProvider::Ollama {
            base_url: "http://localhost:11434".to_string(),
            model: "nomic-embed-text".to_string(),
        });
        assert_eq!(generator.provider_name(), "Ollama");

        let generator = EmbeddingGenerator::new(EmbeddingProvider::OpenAI {
            api_key: "test".to_string(),
            model: "text-embedding-3-small".to_string(),
        });
        assert_eq!(generator.provider_name(), "OpenAI");
    }

    #[test]
    fn test_from_config_ollama() {
        // from_config with explicit ollama should work without env vars
        let config = EmbeddingConfig {
            provider: "ollama".to_string(),
            model: "nomic-embed-text".to_string(),
            batch_size: 256,
            semantic_weight: 0.5,
        };
        let generator = EmbeddingGenerator::from_config(&config).unwrap();
        assert_eq!(generator.provider_name(), "Ollama");
    }
}
