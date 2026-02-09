//! SearchNode: intent-based code entity discovery.

use rpg_core::graph::{Entity, EntityKind, RPGraph};
use std::collections::{HashMap, HashSet};

/// Search mode (matching the paper's SearchNode tool).
#[derive(Debug, Clone, Copy)]
pub enum SearchMode {
    /// Match against semantic features.
    Features,
    /// Match against entity names, file paths, and code keywords.
    Snippets,
    /// Try features first, fall back to snippets.
    Auto,
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
    /// Filter results to specific entity kinds (function, class, method).
    pub entity_type_filter: Option<Vec<EntityKind>>,
    /// Pre-computed embedding scores (entity_id → cosine score) for hybrid blending.
    /// When provided, features-mode search uses rank-based hybrid scoring.
    pub embedding_scores: Option<&'a std::collections::HashMap<String, f64>>,
}

/// Search the RPG for entities matching a query with a configurable result limit.
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
            entity_type_filter: None,
            embedding_scores: None,
        },
    )
}

/// Search with full parameters (paper-complete SearchNode).
pub fn search_with_params(graph: &RPGraph, params: &SearchParams) -> Vec<SearchResult> {
    let query_lower = params.query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
    if query_terms.is_empty() {
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

    // Collect IDs of entities that passed all user filters (scope/file/line/type).
    // This ensures semantic-only results from embeddings respect the same filters.
    let candidate_ids: HashSet<&String> = entities.iter().map(|(id, _)| *id).collect();

    match params.mode {
        SearchMode::Features => {
            let lexical = search_features(&entities, &query_terms, params.limit);
            maybe_hybrid_rerank(
                graph,
                &candidate_ids,
                lexical,
                params.embedding_scores,
                params.limit,
            )
        }
        SearchMode::Snippets => search_snippets(&entities, &query_terms, params.limit),
        SearchMode::Auto => {
            // Merge features + snippets.
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

            // Apply hybrid reranking if embeddings available
            maybe_hybrid_rerank(
                graph,
                &candidate_ids,
                merged,
                params.embedding_scores,
                params.limit,
            )
        }
    }
}

/// Compute Jaccard similarity between two token sets.
/// Kept as utility — IDF-weighted overlap is used for search scoring, but Jaccard
/// is still the fallback for drift detection in evolution.rs.
#[allow(dead_code)]
fn jaccard_similarity(a: &HashSet<&str>, b: &HashSet<&str>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        return 0.0;
    }
    intersection / union
}

/// Build an IDF (inverse document frequency) map from entity features.
/// IDF(term) = ln(N / (1 + df(term))), where N = total entities, df = entities containing term.
/// Rare terms get higher IDF values — a query matching a rare term is more discriminating.
fn compute_idf(entities: &[(&String, &Entity)]) -> std::collections::HashMap<String, f64> {
    let n = entities.len() as f64;
    let mut df: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (_, entity) in entities {
        // Collect unique tokens across all features for this entity
        let mut entity_tokens: HashSet<String> = HashSet::new();
        for feature in &entity.semantic_features {
            for token in feature.to_lowercase().split_whitespace() {
                entity_tokens.insert(token.to_string());
            }
        }
        for token in entity_tokens {
            *df.entry(token).or_insert(0) += 1;
        }
    }

    df.into_iter()
        .map(|(term, count)| {
            let idf = (n / (1.0 + count as f64)).ln().max(0.0);
            (term, idf)
        })
        .collect()
}

/// IDF-weighted token overlap: sum of IDF values for matching tokens, normalized.
/// More discriminating than raw Jaccard because rare terms contribute more.
fn idf_weighted_overlap(
    query_tokens: &HashSet<&str>,
    text_tokens: &HashSet<&str>,
    idf: &std::collections::HashMap<String, f64>,
) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }

    let mut match_weight = 0.0;
    let mut total_weight = 0.0;

    for qt in query_tokens {
        let w = idf.get(*qt).copied().unwrap_or(1.0);
        total_weight += w;
        if text_tokens.contains(qt) {
            match_weight += w;
        }
    }

    if total_weight == 0.0 {
        return 0.0;
    }
    match_weight / total_weight
}

/// Score a text against query terms using multiple signals:
/// 1. IDF-weighted token overlap (primary) — rare terms matter more
/// 2. Exact phrase bonus
/// 3. Edit distance for near-misses
fn multi_signal_score(
    text: &str,
    query: &str,
    query_terms: &[&str],
    idf: &std::collections::HashMap<String, f64>,
) -> f64 {
    let text_lower = text.to_lowercase();
    let text_tokens: HashSet<&str> = text_lower.split_whitespace().collect();
    let query_tokens: HashSet<&str> = query_terms.iter().copied().collect::<HashSet<_>>();

    // Signal 1: IDF-weighted token overlap (rare terms weighted higher)
    let overlap = idf_weighted_overlap(&query_tokens, &text_tokens, idf);

    // Signal 2: Exact phrase bonus (full query as contiguous substring)
    let phrase_bonus = if text_lower.contains(&query.to_lowercase()) {
        0.5
    } else {
        0.0
    };

    // Signal 3: Edit distance for near-misses (best match per query term)
    // Only count fuzzy matches above 0.6 similarity to avoid noise
    let mut edit_score = 0.0;
    for term in query_terms {
        let mut best = 0.0_f64;
        for token in &text_tokens {
            let sim = strsim::normalized_levenshtein(term, token);
            if sim > 0.6 {
                best = best.max(sim);
            }
        }
        edit_score += best;
    }
    if !query_terms.is_empty() {
        edit_score /= query_terms.len() as f64;
    }

    // Weighted combination: IDF overlap (40%) + phrase (20%) + edit distance (40%)
    overlap * 0.4 + phrase_bonus * 0.2 + edit_score * 0.4
}

fn search_features(
    entities: &[(&String, &Entity)],
    query_terms: &[&str],
    result_limit: usize,
) -> Vec<SearchResult> {
    let idf = compute_idf(entities);
    let mut results: Vec<SearchResult> = Vec::new();
    let query_joined = query_terms.join(" ");

    for (id, entity) in entities {
        let mut score = 0.0;
        let mut matched = Vec::new();

        for feature in &entity.semantic_features {
            let feature_score = multi_signal_score(feature, &query_joined, query_terms, &idf);

            if feature_score > 0.05 {
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
    // Snippets match against names/paths, not features — use empty IDF (equal weights)
    let empty_idf = std::collections::HashMap::new();
    let mut results: Vec<SearchResult> = Vec::new();
    let query_joined = query_terms.join(" ");

    for (id, entity) in entities {
        // Name: high weight (2x)
        let name_score =
            multi_signal_score(&entity.name, &query_joined, query_terms, &empty_idf) * 2.0;

        // File path: medium weight (1x)
        let file_str = entity.file.display().to_string();
        let file_score = multi_signal_score(&file_str, &query_joined, query_terms, &empty_idf);

        // Hierarchy path: low weight (0.5x)
        let path_score = multi_signal_score(
            &entity.hierarchy_path,
            &query_joined,
            query_terms,
            &empty_idf,
        ) * 0.5;

        let score = name_score + file_score + path_score;

        if score > 0.05 {
            results.push(SearchResult {
                entity_id: (*id).clone(),
                entity_name: entity.name.clone(),
                file: file_str,
                line_start: entity.line_start,
                score,
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

/// Rank-based normalization: assign scores based on rank position.
/// Returns entity_id → normalized score (1.0 = best, 0.0 = worst).
fn rank_normalize(scores: &HashMap<String, f64>) -> HashMap<String, f64> {
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

/// If embedding scores are available, rerank results using rank-based hybrid blending
/// (0.6 semantic + 0.4 lexical). Otherwise, pass through unchanged.
///
/// Entities that only appear in semantic scores (not in lexical results) are included
/// as stub results — enabling true semantic discovery of entities the keyword search missed.
/// Only entities in `candidate_ids` are considered (preserves user filters like scope/file/line).
fn maybe_hybrid_rerank(
    graph: &RPGraph,
    candidate_ids: &HashSet<&String>,
    lexical_results: Vec<SearchResult>,
    embedding_scores: Option<&HashMap<String, f64>>,
    limit: usize,
) -> Vec<SearchResult> {
    let Some(sem_scores) = embedding_scores else {
        return lexical_results;
    };
    if sem_scores.is_empty() {
        return lexical_results;
    }

    // Filter semantic scores to only include entities that passed user filters
    let filtered_sem: HashMap<String, f64> = sem_scores
        .iter()
        .filter(|(id, _)| candidate_ids.contains(id))
        .map(|(id, &score)| (id.clone(), score))
        .collect();

    if filtered_sem.is_empty() {
        return lexical_results;
    }

    // Build lexical score map from results
    let lex_scores: HashMap<String, f64> = lexical_results
        .iter()
        .map(|r| (r.entity_id.clone(), r.score))
        .collect();

    // Rank-normalize both score sets
    let sem_ranks = rank_normalize(&filtered_sem);
    let lex_ranks = rank_normalize(&lex_scores);

    // Blend: 0.6 * semantic_rank + 0.4 * lexical_rank
    let semantic_weight = 0.6;
    let lexical_weight = 1.0 - semantic_weight;

    let mut all_ids: HashSet<&String> = HashSet::new();
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

    // Rebuild SearchResult vec in blended order, preserving matched_features etc.
    let result_map: HashMap<String, SearchResult> = lexical_results
        .into_iter()
        .map(|r| (r.entity_id.clone(), r))
        .collect();

    blended
        .into_iter()
        .map(|(id, score)| {
            if let Some(r) = result_map.get(&id) {
                // Lexical hit — preserve matched_features
                SearchResult { score, ..r.clone() }
            } else {
                // Semantic-only discovery — create stub from graph entity
                let (name, file, line_start, lifted) = graph
                    .entities
                    .get(&id)
                    .map(|e| {
                        (
                            e.name.clone(),
                            e.file.display().to_string(),
                            e.line_start,
                            !e.semantic_features.is_empty(),
                        )
                    })
                    .unwrap_or_else(|| (id.clone(), String::new(), 0, false));
                SearchResult {
                    entity_id: id,
                    entity_name: name,
                    file,
                    line_start,
                    score,
                    matched_features: Vec::new(),
                    lifted,
                }
            }
        })
        .collect()
}

/// Collect entities from one or more hierarchy scopes.
/// Supports comma-separated scopes per paper's `search_scopes` (list of paths).
fn collect_scoped_entities(graph: &RPGraph, scope: &str) -> Vec<String> {
    let scopes: Vec<&str> = scope.split(',').map(|s| s.trim()).collect();
    let mut all_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for single_scope in scopes {
        for id in collect_single_scope(graph, single_scope) {
            if seen.insert(id.clone()) {
                all_ids.push(id);
            }
        }
    }
    all_ids
}

fn collect_single_scope(graph: &RPGraph, scope: &str) -> Vec<String> {
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
