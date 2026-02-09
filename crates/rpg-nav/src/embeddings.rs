//! Embedding-based semantic search using fastembed.
//!
//! Stores per-feature embeddings for each entity, enabling max-cosine similarity
//! search that preserves multi-role entity semantics. Uses BGE-small-en-v1.5
//! (384 dimensions) via the fastembed crate.

use anyhow::{Context, Result, ensure};
use fastembed::{EmbeddingModel, TextEmbedding};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Magic bytes for the binary embedding file format.
const MAGIC: u32 = 0x5250_4745; // "RPGE"
const FORMAT_VERSION: u32 = 1;
const DIMENSION: usize = 384;

/// Metadata sidecar for the embedding index.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingMeta {
    pub model: String,
    pub dimension: u32,
    pub version: u32,
    pub graph_updated_at: String,
}

/// Per-entity embedding data: one vector per semantic feature.
#[derive(Debug, Clone)]
struct EntityEmbeddings {
    /// One embedding vector per feature.
    vectors: Vec<Vec<f32>>,
}

/// In-memory embedding index for semantic search.
pub struct EmbeddingIndex {
    model: TextEmbedding,
    /// Map from entity_id → feature-level embeddings.
    entities: HashMap<String, EntityEmbeddings>,
    /// Path to the .rpg directory for persistence.
    rpg_dir: PathBuf,
    /// The graph's updated_at timestamp this index corresponds to.
    graph_updated_at: String,
}

impl EmbeddingIndex {
    /// Load existing index from disk, or create a new empty one.
    /// The model is initialized lazily on first use.
    pub fn load_or_init(project_root: &Path, graph_updated_at: &str) -> Result<Self> {
        let rpg_dir = project_root.join(".rpg");
        let model = init_model(&rpg_dir)?;

        let embeddings_path = rpg_dir.join("embeddings.bin");
        let meta_path = rpg_dir.join("embeddings.meta.json");

        // Try loading existing index (resilient to corruption)
        if embeddings_path.exists() && meta_path.exists() {
            match Self::try_load_existing(&meta_path, &embeddings_path, graph_updated_at) {
                Ok(Some(entities)) => {
                    return Ok(Self {
                        model,
                        entities,
                        rpg_dir,
                        graph_updated_at: graph_updated_at.to_string(),
                    });
                }
                Ok(None) => {
                    // Index stale or mismatched — start fresh
                }
                Err(e) => {
                    // Corrupt on-disk data — delete and start fresh
                    eprintln!("rpg: corrupt embedding index, rebuilding: {e}");
                    let _ = std::fs::remove_file(&embeddings_path);
                    let _ = std::fs::remove_file(&meta_path);
                }
            }
        }

        // No valid index — start fresh
        Ok(Self {
            model,
            entities: HashMap::new(),
            rpg_dir,
            graph_updated_at: graph_updated_at.to_string(),
        })
    }

    /// Try to load existing embedding data. Returns Ok(Some(entities)) if valid,
    /// Ok(None) if stale/mismatched, Err if corrupt.
    fn try_load_existing(
        meta_path: &Path,
        embeddings_path: &Path,
        graph_updated_at: &str,
    ) -> Result<Option<HashMap<String, EntityEmbeddings>>> {
        let meta_json =
            std::fs::read_to_string(meta_path).context("failed to read embeddings meta")?;
        let meta: EmbeddingMeta =
            serde_json::from_str(&meta_json).context("failed to parse embeddings meta")?;

        if meta.graph_updated_at != graph_updated_at
            || meta.model != "BAAI/bge-small-en-v1.5"
            || meta.dimension != DIMENSION as u32
        {
            return Ok(None);
        }

        let entities = load_binary(embeddings_path)?;
        Ok(Some(entities))
    }

    /// Embed features for a set of entities and add/update them in the index.
    /// `entity_features` maps entity_id → list of semantic feature strings.
    pub fn embed_entities(
        &mut self,
        entity_features: &HashMap<String, Vec<String>>,
    ) -> Result<usize> {
        if entity_features.is_empty() {
            return Ok(0);
        }

        // Collect all features into one batch for efficient embedding
        let mut all_features: Vec<String> = Vec::new();
        let mut feature_map: Vec<(String, usize, usize)> = Vec::new(); // (entity_id, start, count)

        for (entity_id, features) in entity_features {
            if features.is_empty() {
                continue;
            }
            let start = all_features.len();
            all_features.extend(features.iter().cloned());
            feature_map.push((entity_id.clone(), start, features.len()));
        }

        if all_features.is_empty() {
            return Ok(0);
        }

        // Embed all features in one batch
        let embeddings = self
            .model
            .embed(all_features, None)
            .context("failed to embed features")?;

        // Distribute embeddings back to entities
        let mut count = 0;
        for (entity_id, start, feat_count) in &feature_map {
            let vectors: Vec<Vec<f32>> = embeddings[*start..*start + *feat_count].to_vec();
            self.entities
                .insert(entity_id.clone(), EntityEmbeddings { vectors });
            count += 1;
        }

        Ok(count)
    }

    /// Remove entities that no longer exist in the graph.
    pub fn prune(&mut self, valid_entity_ids: &std::collections::HashSet<String>) {
        self.entities.retain(|id, _| valid_entity_ids.contains(id));
    }

    /// Score all entities against a query string using max-cosine similarity.
    /// Returns entity_id → score (0.0..1.0).
    pub fn score_all(&mut self, query: &str) -> Result<HashMap<String, f64>> {
        let query_embeddings = self
            .model
            .embed(vec![query.to_string()], None)
            .context("failed to embed query")?;

        let query_vec = &query_embeddings[0];
        let mut scores = HashMap::new();

        for (entity_id, entity_emb) in &self.entities {
            let max_sim = entity_emb
                .vectors
                .iter()
                .map(|fv| cosine_similarity(query_vec, fv))
                .fold(f64::NEG_INFINITY, f64::max);

            if max_sim > 0.0 {
                scores.insert(entity_id.clone(), max_sim);
            }
        }

        Ok(scores)
    }

    /// Number of entities in the index.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Save the index to disk (binary + meta sidecar).
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.rpg_dir)?;
        save_binary(&self.rpg_dir.join("embeddings.bin"), &self.entities)?;

        let meta = EmbeddingMeta {
            model: "BAAI/bge-small-en-v1.5".to_string(),
            dimension: DIMENSION as u32,
            version: FORMAT_VERSION,
            graph_updated_at: self.graph_updated_at.clone(),
        };
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(self.rpg_dir.join("embeddings.meta.json"), meta_json)?;

        Ok(())
    }

    /// Update the tracked graph timestamp (call before save after graph changes).
    pub fn set_graph_updated_at(&mut self, ts: &str) {
        self.graph_updated_at = ts.to_string();
    }
}

/// Initialize the fastembed model with cache in .rpg/models/.
fn init_model(rpg_dir: &Path) -> Result<TextEmbedding> {
    let cache_dir = rpg_dir.join("models");
    std::fs::create_dir_all(&cache_dir)?;

    let options = fastembed::TextInitOptions::new(EmbeddingModel::BGESmallENV15)
        .with_show_download_progress(true)
        .with_cache_dir(cache_dir);

    let model = TextEmbedding::try_new(options)
        .context("failed to initialize embedding model (BGE-small-en-v1.5)")?;

    Ok(model)
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    debug_assert_eq!(a.len(), b.len());
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;

    for (ai, bi) in a.iter().zip(b.iter()) {
        let ai = f64::from(*ai);
        let bi = f64::from(*bi);
        dot += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    dot / denom
}

/// Save entity embeddings to binary format.
fn save_binary(path: &Path, entities: &HashMap<String, EntityEmbeddings>) -> Result<()> {
    let mut buf: Vec<u8> = Vec::new();

    // Header (16 bytes)
    buf.write_all(&MAGIC.to_le_bytes())?;
    buf.write_all(&FORMAT_VERSION.to_le_bytes())?;
    buf.write_all(&(DIMENSION as u32).to_le_bytes())?;
    buf.write_all(&(entities.len() as u32).to_le_bytes())?;

    // Per entity
    for (id, emb) in entities {
        let id_bytes = id.as_bytes();
        ensure!(
            u16::try_from(id_bytes.len()).is_ok(),
            "entity id too long: {} bytes",
            id_bytes.len()
        );
        ensure!(
            u16::try_from(emb.vectors.len()).is_ok(),
            "too many feature vectors for {}: {}",
            id,
            emb.vectors.len()
        );
        buf.write_all(&(id_bytes.len() as u16).to_le_bytes())?;
        buf.write_all(id_bytes)?;
        buf.write_all(&(emb.vectors.len() as u16).to_le_bytes())?;
        for vec in &emb.vectors {
            for &val in vec {
                buf.write_all(&val.to_le_bytes())?;
            }
        }
    }

    std::fs::write(path, buf)?;
    Ok(())
}

/// Load entity embeddings from binary format.
fn load_binary(path: &Path) -> Result<HashMap<String, EntityEmbeddings>> {
    let data = std::fs::read(path).context("failed to read embeddings.bin")?;
    let mut cursor = &data[..];

    // Header
    let magic = read_u32(&mut cursor)?;
    anyhow::ensure!(magic == MAGIC, "invalid embeddings magic bytes");
    let version = read_u32(&mut cursor)?;
    anyhow::ensure!(version == FORMAT_VERSION, "unsupported embeddings version");
    let dimension = read_u32(&mut cursor)? as usize;
    anyhow::ensure!(dimension == DIMENSION, "dimension mismatch");
    let entity_count = read_u32(&mut cursor)? as usize;

    let mut entities = HashMap::with_capacity(entity_count);

    for _ in 0..entity_count {
        let id_len = read_u16(&mut cursor)? as usize;
        anyhow::ensure!(
            cursor.len() >= id_len,
            "unexpected end of embeddings file (need {id_len} bytes for entity id)"
        );
        let id = std::str::from_utf8(&cursor[..id_len])
            .context("invalid entity id")?
            .to_string();
        cursor = &cursor[id_len..];

        let feature_count = read_u16(&mut cursor)? as usize;
        let mut vectors = Vec::with_capacity(feature_count);

        for _ in 0..feature_count {
            let mut vec = Vec::with_capacity(dimension);
            for _ in 0..dimension {
                vec.push(read_f32(&mut cursor)?);
            }
            vectors.push(vec);
        }

        entities.insert(id, EntityEmbeddings { vectors });
    }

    Ok(entities)
}

fn read_u32(cursor: &mut &[u8]) -> Result<u32> {
    anyhow::ensure!(
        cursor.len() >= 4,
        "unexpected end of embeddings file (need 4 bytes for u32)"
    );
    let bytes: [u8; 4] = cursor[..4].try_into().unwrap();
    *cursor = &cursor[4..];
    Ok(u32::from_le_bytes(bytes))
}

fn read_u16(cursor: &mut &[u8]) -> Result<u16> {
    anyhow::ensure!(
        cursor.len() >= 2,
        "unexpected end of embeddings file (need 2 bytes for u16)"
    );
    let bytes: [u8; 2] = cursor[..2].try_into().unwrap();
    *cursor = &cursor[2..];
    Ok(u16::from_le_bytes(bytes))
}

fn read_f32(cursor: &mut &[u8]) -> Result<f32> {
    anyhow::ensure!(
        cursor.len() >= 4,
        "unexpected end of embeddings file (need 4 bytes for f32)"
    );
    let bytes: [u8; 4] = cursor[..4].try_into().unwrap();
    *cursor = &cursor[4..];
    Ok(f32::from_le_bytes(bytes))
}

/// Rank-based normalization: assign scores based on rank position.
/// Returns entity_id → normalized score (1.0 = best, 0.0 = worst).
pub fn rank_normalize(scores: &HashMap<String, f64>) -> HashMap<String, f64> {
    if scores.is_empty() {
        return HashMap::new();
    }

    let mut sorted: Vec<(&String, &f64)> = scores.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));

    let total = sorted.len() as f64;
    sorted
        .iter()
        .enumerate()
        .map(|(rank, (id, _))| ((*id).clone(), 1.0 - (rank as f64 / total)))
        .collect()
}

/// Blend semantic and lexical scores using rank-based normalization.
/// Returns entity_id → blended score, sorted descending.
pub fn hybrid_blend(
    semantic_scores: &HashMap<String, f64>,
    lexical_scores: &HashMap<String, f64>,
    semantic_weight: f64,
    limit: usize,
) -> Vec<(String, f64)> {
    let sem_ranks = rank_normalize(semantic_scores);
    let lex_ranks = rank_normalize(lexical_scores);
    let lexical_weight = 1.0 - semantic_weight;

    // Union of all entity IDs
    let mut all_ids: std::collections::HashSet<&String> = std::collections::HashSet::new();
    all_ids.extend(sem_ranks.keys());
    all_ids.extend(lex_ranks.keys());

    let mut blended: Vec<(String, f64)> = all_ids
        .into_iter()
        .map(|id| {
            let sem = sem_ranks.get(id).copied().unwrap_or(0.0);
            let lex = lex_ranks.get(id).copied().unwrap_or(0.0);
            (id.clone(), semantic_weight * sem + lexical_weight * lex)
        })
        .collect();

    blended.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    blended.truncate(limit);
    blended
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_rank_normalize() {
        let mut scores = HashMap::new();
        scores.insert("a".to_string(), 0.9);
        scores.insert("b".to_string(), 0.5);
        scores.insert("c".to_string(), 0.1);

        let ranks = rank_normalize(&scores);
        // "a" is rank 0 → 1.0 - 0/3 = 1.0
        // "b" is rank 1 → 1.0 - 1/3 ≈ 0.667
        // "c" is rank 2 → 1.0 - 2/3 ≈ 0.333
        assert!((ranks["a"] - 1.0).abs() < 1e-6);
        assert!((ranks["b"] - 2.0 / 3.0).abs() < 1e-6);
        assert!((ranks["c"] - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_rank_normalize_empty() {
        let scores = HashMap::new();
        let ranks = rank_normalize(&scores);
        assert!(ranks.is_empty());
    }

    #[test]
    fn test_hybrid_blend() {
        let mut sem = HashMap::new();
        sem.insert("a".to_string(), 0.9);
        sem.insert("b".to_string(), 0.1);

        let mut lex = HashMap::new();
        lex.insert("a".to_string(), 0.1);
        lex.insert("b".to_string(), 0.9);

        let blended = hybrid_blend(&sem, &lex, 0.6, 10);
        assert_eq!(blended.len(), 2);
        // "a" has high semantic rank, low lexical rank
        // "b" has low semantic rank, high lexical rank
        // With 0.6 semantic weight, "a" should score higher
        assert_eq!(blended[0].0, "a");
    }

    #[test]
    fn test_binary_roundtrip() {
        let mut entities = HashMap::new();
        entities.insert(
            "test:func".to_string(),
            EntityEmbeddings {
                vectors: vec![vec![0.1; DIMENSION], vec![0.2; DIMENSION]],
            },
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");

        save_binary(&path, &entities).unwrap();
        let loaded = load_binary(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("test:func"));
        assert_eq!(loaded["test:func"].vectors.len(), 2);
        assert!((loaded["test:func"].vectors[0][0] - 0.1).abs() < 1e-6);
    }
}
