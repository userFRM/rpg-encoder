//! Embedding-based semantic search using brute-force cosine similarity.
//!
//! For typical RPG graphs (<10K entities), brute-force is fast enough
//! and avoids heavy native dependencies (usearch, faiss, etc.).

use rpg_core::graph::RPGraph;

/// Result of an embedding-based search.
#[derive(Debug, Clone)]
pub struct EmbeddingSearchResult {
    pub entity_id: String,
    pub entity_name: String,
    pub file: String,
    pub line_start: usize,
    pub score: f32,
}

/// Search entities by embedding similarity.
/// `query_embedding` must be L2-normalized.
/// Returns up to `k` results sorted by descending cosine similarity.
pub fn search_by_embedding(
    graph: &RPGraph,
    query_embedding: &[f32],
    k: usize,
) -> Vec<EmbeddingSearchResult> {
    let mut scored: Vec<EmbeddingSearchResult> = graph
        .entities
        .iter()
        .filter_map(|(id, entity)| {
            let emb = entity.embedding.as_ref()?;
            let sim = cosine_similarity(query_embedding, emb);
            Some(EmbeddingSearchResult {
                entity_id: id.clone(),
                entity_name: entity.name.clone(),
                file: entity.file.display().to_string(),
                line_start: entity.line_start,
                score: sim,
            })
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(k);
    scored
}

/// Check if the graph has any entities with embeddings.
pub fn has_embeddings(graph: &RPGraph) -> bool {
    graph.entities.values().any(|e| e.embedding.is_some())
}

/// Count entities with embeddings.
pub fn embedding_count(graph: &RPGraph) -> usize {
    graph
        .entities
        .values()
        .filter(|e| e.embedding.is_some())
        .count()
}

/// Cosine similarity between two vectors (assumes L2-normalized).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::*;

    fn make_entity_with_embedding(id: &str, name: &str, embedding: Vec<f32>) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: name.to_string(),
            file: std::path::PathBuf::from("test.py"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: Vec::new(),
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
            embedding: Some(embedding),
        }
    }

    #[test]
    fn test_search_by_embedding_basic() {
        let mut graph = RPGraph::new("python");

        // Entity A: embedding [1, 0, 0] (normalized)
        graph.insert_entity(make_entity_with_embedding(
            "a",
            "alpha",
            vec![1.0, 0.0, 0.0],
        ));
        // Entity B: embedding [0, 1, 0]
        graph.insert_entity(make_entity_with_embedding("b", "beta", vec![0.0, 1.0, 0.0]));
        // Entity C: embedding [0.7, 0.7, 0] (roughly normalized)
        let c = vec![
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ];
        graph.insert_entity(make_entity_with_embedding("c", "gamma", c));

        // Query close to [1, 0, 0]
        let query = vec![1.0, 0.0, 0.0];
        let results = search_by_embedding(&graph, &query, 3);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].entity_id, "a"); // exact match
        assert_eq!(results[1].entity_id, "c"); // partial match
        assert_eq!(results[2].entity_id, "b"); // orthogonal
    }

    #[test]
    fn test_search_by_embedding_k_limit() {
        let mut graph = RPGraph::new("python");
        graph.insert_entity(make_entity_with_embedding("a", "alpha", vec![1.0, 0.0]));
        graph.insert_entity(make_entity_with_embedding("b", "beta", vec![0.0, 1.0]));
        graph.insert_entity(make_entity_with_embedding("c", "gamma", vec![0.7, 0.7]));

        let results = search_by_embedding(&graph, &[1.0, 0.0], 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, "a");
    }

    #[test]
    fn test_search_skips_entities_without_embeddings() {
        let mut graph = RPGraph::new("python");
        graph.insert_entity(make_entity_with_embedding("a", "alpha", vec![1.0, 0.0]));

        // Entity without embedding
        let mut e = make_entity_with_embedding("b", "beta", vec![0.0, 1.0]);
        e.embedding = None;
        graph.insert_entity(e);

        let results = search_by_embedding(&graph, &[1.0, 0.0], 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, "a");
    }

    #[test]
    fn test_has_embeddings() {
        let mut graph = RPGraph::new("python");
        assert!(!has_embeddings(&graph));

        graph.insert_entity(make_entity_with_embedding("a", "alpha", vec![1.0, 0.0]));
        assert!(has_embeddings(&graph));
    }

    #[test]
    fn test_embedding_count() {
        let mut graph = RPGraph::new("python");
        assert_eq!(embedding_count(&graph), 0);

        graph.insert_entity(make_entity_with_embedding("a", "alpha", vec![1.0, 0.0]));
        assert_eq!(embedding_count(&graph), 1);

        let mut e = make_entity_with_embedding("b", "beta", vec![0.0, 1.0]);
        e.embedding = None;
        graph.insert_entity(e);
        assert_eq!(embedding_count(&graph), 1);
    }
}
