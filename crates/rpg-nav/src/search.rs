//! SearchNode: intent-based code entity discovery.

use rpg_core::graph::{Entity, EntityKind, RPGraph};
use std::collections::HashSet;

/// Search mode (matching the paper's SearchNode tool).
#[derive(Debug, Clone, Copy)]
pub enum SearchMode {
    /// Match against semantic features.
    Features,
    /// Match against entity names, file paths, and code keywords.
    Snippets,
    /// Try features first, fall back to snippets.
    Auto,
    /// Pure embedding-based semantic search (requires embeddings in graph).
    Semantic,
    /// Weighted combination of keyword + embedding search.
    Hybrid,
}

/// A search result with relevance score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entity_id: String,
    pub entity_name: String,
    pub file: String,
    pub line_start: usize,
    pub score: f64,
    pub matched_features: Vec<String>,
    pub lifted: bool,
}

/// Full search parameters matching the paper's SearchNode spec.
pub struct SearchParams<'a> {
    pub query: &'a str,
    pub mode: SearchMode,
    pub scope: Option<&'a str>,
    pub limit: usize,
    /// Filter to entities within this line range (start, end) inclusive.
    pub line_nums: Option<(usize, usize)>,
    /// Glob pattern to filter entities by file path.
    pub file_pattern: Option<&'a str>,
    pub query_embedding: Option<&'a [f32]>,
    pub semantic_weight: f64,
    /// Filter results to specific entity kinds (function, class, method).
    pub entity_type_filter: Option<Vec<EntityKind>>,
}

/// Search the RPG for entities matching a query with a configurable result limit.
/// Pass `query_embedding` for Semantic/Hybrid modes to use actual embedding search.
pub fn search(
    graph: &RPGraph,
    query: &str,
    mode: SearchMode,
    scope: Option<&str>,
    limit: usize,
) -> Vec<SearchResult> {
    search_with_params(
        graph,
        &SearchParams {
            query,
            mode,
            scope,
            limit,
            line_nums: None,
            file_pattern: None,
            query_embedding: None,
            semantic_weight: 0.5,
            entity_type_filter: None,
        },
    )
}

/// Search with an optional query embedding for Semantic/Hybrid modes.
pub fn search_with_embedding(
    graph: &RPGraph,
    query: &str,
    mode: SearchMode,
    scope: Option<&str>,
    limit: usize,
    query_embedding: Option<&[f32]>,
    semantic_weight: f64,
) -> Vec<SearchResult> {
    search_with_params(
        graph,
        &SearchParams {
            query,
            mode,
            scope,
            limit,
            line_nums: None,
            file_pattern: None,
            query_embedding,
            semantic_weight,
            entity_type_filter: None,
        },
    )
}

/// Search with full parameters (paper-complete SearchNode).
pub fn search_with_params(graph: &RPGraph, params: &SearchParams) -> Vec<SearchResult> {
    let query_lower = params.query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
    if query_terms.is_empty() && params.query_embedding.is_none() {
        return Vec::new();
    }

    // Build file pattern matcher if specified
    let file_matcher = params
        .file_pattern
        .and_then(|pat| globset::Glob::new(pat).ok().map(|g| g.compile_matcher()));

    let entities: Box<dyn Iterator<Item = (&String, &Entity)>> = if let Some(scope) = params.scope {
        let scoped_ids: HashSet<String> =
            collect_scoped_entities(graph, scope).into_iter().collect();
        Box::new(
            graph
                .entities
                .iter()
                .filter(move |(id, _)| scoped_ids.contains(id.as_str())),
        )
    } else {
        Box::new(graph.entities.iter())
    };

    // Apply file_pattern, line_nums, and entity_type filters
    let entities: Vec<(&String, &Entity)> = entities
        .filter(|(_, entity)| {
            // File pattern filter
            if let Some(ref matcher) = file_matcher
                && !matcher.is_match(entity.file.as_path())
            {
                return false;
            }
            // Line range filter
            if let Some((start, end)) = params.line_nums
                && (entity.line_end < start || entity.line_start > end)
            {
                return false;
            }
            // Entity type filter
            if let Some(ref kinds) = params.entity_type_filter
                && !kinds.contains(&entity.kind)
            {
                return false;
            }
            true
        })
        .collect();

    match params.mode {
        SearchMode::Features => search_features(&entities, &query_terms, params.limit),
        SearchMode::Snippets => search_snippets(&entities, &query_terms, params.limit),
        SearchMode::Auto => {
            // Merge features + snippets (like Hybrid but without embeddings).
            // Previously this was fallback-only, which meant any partial feature
            // match would suppress correct snippet matches entirely.
            let feat_results = search_features(&entities, &query_terms, params.limit * 2);
            let snip_results = search_snippets(&entities, &query_terms, params.limit * 2);

            let mut score_map: std::collections::HashMap<String, SearchResult> =
                std::collections::HashMap::new();

            for r in feat_results {
                score_map
                    .entry(r.entity_id.clone())
                    .and_modify(|existing| {
                        existing.score += r.score;
                        existing.matched_features.extend(r.matched_features.clone());
                    })
                    .or_insert(r);
            }
            for r in snip_results {
                score_map
                    .entry(r.entity_id.clone())
                    .and_modify(|existing| existing.score += r.score * 0.5)
                    .or_insert_with(|| SearchResult {
                        score: r.score * 0.5,
                        ..r
                    });
            }

            let mut merged: Vec<SearchResult> = score_map.into_values().collect();
            merged.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            merged.truncate(params.limit);
            merged
        }
        SearchMode::Semantic => {
            if let Some(emb) = params.query_embedding {
                let emb_results =
                    crate::embedding_search::search_by_embedding(graph, emb, params.limit);
                emb_results
                    .into_iter()
                    .map(|r| SearchResult {
                        entity_id: r.entity_id,
                        entity_name: r.entity_name,
                        file: r.file,
                        line_start: r.line_start,
                        score: r.score as f64,
                        matched_features: Vec::new(),
                        lifted: true, // embedding search implies entity has features
                    })
                    .collect()
            } else {
                eprintln!(
                    "Warning: Semantic search requested but no query embedding provided. Falling back to keyword search."
                );
                let mut results = search_features(&entities, &query_terms, params.limit);
                if results.is_empty() {
                    results = search_snippets(&entities, &query_terms, params.limit);
                }
                results
            }
        }
        SearchMode::Hybrid => {
            let semantic_weight = params.semantic_weight;
            let result_limit = params.limit;

            // Keyword component: features + snippets
            let feat_results = search_features(&entities, &query_terms, result_limit * 2);
            let snip_results = search_snippets(&entities, &query_terms, result_limit * 2);

            let mut score_map: std::collections::HashMap<String, SearchResult> =
                std::collections::HashMap::new();

            let keyword_weight = 1.0 - semantic_weight;

            for r in feat_results {
                score_map
                    .entry(r.entity_id.clone())
                    .and_modify(|existing| existing.score += r.score * keyword_weight)
                    .or_insert_with(|| SearchResult {
                        score: r.score * keyword_weight,
                        ..r.clone()
                    });
            }
            for r in snip_results {
                score_map
                    .entry(r.entity_id.clone())
                    .and_modify(|existing| existing.score += r.score * 0.5 * keyword_weight)
                    .or_insert_with(|| SearchResult {
                        score: r.score * 0.5 * keyword_weight,
                        ..r.clone()
                    });
            }

            // Embedding component (if available)
            if let Some(emb) = params.query_embedding {
                let emb_results =
                    crate::embedding_search::search_by_embedding(graph, emb, result_limit * 2);
                for r in emb_results {
                    let emb_score = r.score as f64 * semantic_weight;
                    score_map
                        .entry(r.entity_id.clone())
                        .and_modify(|existing| existing.score += emb_score)
                        .or_insert(SearchResult {
                            entity_id: r.entity_id,
                            entity_name: r.entity_name,
                            file: r.file,
                            line_start: r.line_start,
                            score: emb_score,
                            matched_features: Vec::new(),
                            lifted: true,
                        });
                }
            }

            let mut merged: Vec<SearchResult> = score_map.into_values().collect();
            merged.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            merged.truncate(result_limit);
            merged
        }
    }
}

fn search_features(
    entities: &[(&String, &Entity)],
    query_terms: &[&str],
    result_limit: usize,
) -> Vec<SearchResult> {
    let mut results: Vec<SearchResult> = Vec::new();

    for (id, entity) in entities {
        let mut score = 0.0;
        let mut matched = Vec::new();

        for feature in &entity.semantic_features {
            let feature_lower = feature.to_lowercase();
            let mut feature_score = 0.0;

            for term in query_terms {
                if feature_lower.contains(term) {
                    feature_score += 1.0;
                }
            }

            if feature_score > 0.0 {
                // Normalize by number of query terms
                feature_score /= query_terms.len() as f64;
                score += feature_score;
                matched.push(feature.clone());
            }
        }

        if score > 0.0 {
            results.push(SearchResult {
                entity_id: (*id).clone(),
                entity_name: entity.name.clone(),
                file: entity.file.display().to_string(),
                line_start: entity.line_start,
                score,
                matched_features: matched,
                lifted: !entity.semantic_features.is_empty(),
            });
        }
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(result_limit);
    results
}

fn search_snippets(
    entities: &[(&String, &Entity)],
    query_terms: &[&str],
    result_limit: usize,
) -> Vec<SearchResult> {
    let mut results: Vec<SearchResult> = Vec::new();

    for (id, entity) in entities {
        let mut score = 0.0;
        let name_lower = entity.name.to_lowercase();
        let file_lower = entity.file.display().to_string().to_lowercase();
        let path_lower = entity.hierarchy_path.to_lowercase();

        for term in query_terms {
            if name_lower.contains(term) {
                score += 2.0; // name matches are high value
            }
            if file_lower.contains(term) {
                score += 1.0;
            }
            if path_lower.contains(term) {
                score += 0.5;
            }
        }

        if score > 0.0 {
            results.push(SearchResult {
                entity_id: (*id).clone(),
                entity_name: entity.name.clone(),
                file: entity.file.display().to_string(),
                line_start: entity.line_start,
                score: score / query_terms.len() as f64,
                matched_features: Vec::new(),
                lifted: !entity.semantic_features.is_empty(),
            });
        }
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(result_limit);
    results
}

fn collect_scoped_entities(graph: &RPGraph, scope: &str) -> Vec<String> {
    let parts: Vec<&str> = scope.split('/').collect();
    if parts.is_empty() {
        return Vec::new();
    }

    if let Some(area) = graph.hierarchy.get(parts[0]) {
        let mut node = area;
        for &part in &parts[1..] {
            if let Some(child) = node.children.get(part) {
                node = child;
            } else {
                return Vec::new();
            }
        }
        node.all_entity_ids()
    } else {
        Vec::new()
    }
}
