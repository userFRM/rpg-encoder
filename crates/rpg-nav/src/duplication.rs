//! Duplication detection via Rabin-Karp rolling hash fingerprinting.
//!
//! Implements the CHM (Code Health Meter) duplication analysis from paper.md §3.4:
//! - Tokenization: strip whitespace/comments, normalize identifiers
//! - Rolling hash: Rabin-Karp fingerprinting with configurable window size
//! - Clone detection: HashMap collision-based fingerprint matching
//!
//! This approach is language-agnostic and detects Type-1 (exact) and Type-2 (renamed) clones.

use crate::search::jaccard_similarity;
use rpg_core::graph::{EntityKind, RPGraph};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Base multiplier for rolling hash (per paper: typically 256)
const HASH_BASE: u64 = 256;

/// Large prime modulus to prevent overflow (per paper: 10^9 + 7)
const HASH_MOD: u64 = 1_000_000_007;

/// Default window size in tokens for entity-level fingerprinting.
/// Lowered from 50 (file-level) to 20 to catch function-sized duplicates.
const DEFAULT_WINDOW_SIZE: usize = 20;

/// Minimum duplicate length in tokens to report (filters noise).
/// Lowered from 30 (file-level) to 15 for entity-level snippets.
const MIN_DUPLICATE_TOKENS: usize = 15;

/// A detected clone group with high similarity.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CloneGroup {
    /// Entity IDs participating in this clone group
    pub entities: Vec<String>,
    /// Similarity coefficient (0.0 - 1.0)
    pub similarity: f64,
    /// Estimated duplicated token count
    pub duplicated_tokens: usize,
    /// File paths involved
    pub files: Vec<String>,
}

/// Configuration for duplication detection.
#[derive(Debug, Clone)]
pub struct DuplicationConfig {
    /// Window size in tokens for fingerprinting
    pub window_size: usize,
    /// Minimum tokens to consider as a duplicate
    pub min_tokens: usize,
    /// Minimum similarity threshold to report (0.0 - 1.0)
    pub similarity_threshold: f64,
}

impl Default for DuplicationConfig {
    fn default() -> Self {
        Self {
            window_size: DEFAULT_WINDOW_SIZE,
            min_tokens: MIN_DUPLICATE_TOKENS,
            similarity_threshold: 0.7,
        }
    }
}

/// A detected group of conceptual duplicates identified via feature-set Jaccard similarity.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SemanticCloneGroup {
    /// Entity IDs in this group
    pub entities: Vec<String>,
    /// Jaccard similarity of feature sets: |A ∩ B| / |A ∪ B|
    pub similarity: f64,
    /// Shared features that caused the match
    pub shared_features: Vec<String>,
    /// File paths (parallel to `entities`)
    pub files: Vec<String>,
}

/// Configuration for semantic (feature-based Jaccard) duplication detection.
#[derive(Debug, Clone)]
pub struct SemanticDuplicationConfig {
    /// Jaccard threshold above which pairs are flagged as conceptual duplicates (default: 0.6).
    pub similarity_threshold: f64,
    /// Minimum number of features an entity must have to participate (default: 1).
    pub min_features: usize,
    /// Skip pairs from the same source file — cross-file duplicates are more actionable (default: true).
    pub skip_same_file: bool,
    /// Skip features appearing in more than this many entities; too generic to be discriminative (default: 20).
    pub max_feature_frequency: usize,
    /// Maximum number of groups to return (default: 50).
    pub max_results: usize,
}

impl Default for SemanticDuplicationConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.6,
            min_features: 1,
            skip_same_file: true,
            max_feature_frequency: 20,
            max_results: 50,
        }
    }
}

/// Detect conceptual duplicates by comparing entity semantic feature sets via Jaccard similarity.
///
/// Unlike token-based clone detection, this operates entirely on in-memory `entity.semantic_features`
/// (verb-object phrases from LLM lifting) and requires no disk I/O.
///
/// Uses an inverted index to avoid O(n²) pair generation: only entity pairs sharing at
/// least one feature are considered candidates, reducing work dramatically on large graphs.
pub fn detect_semantic_duplicates(
    graph: &RPGraph,
    config: &SemanticDuplicationConfig,
) -> Vec<SemanticCloneGroup> {
    // Step 1: Collect eligible entities (exclude Modules, require min_features)
    let eligible: Vec<(&String, &str, &[String])> = graph
        .entities
        .iter()
        .filter(|(_, e)| {
            e.kind != EntityKind::Module && e.semantic_features.len() >= config.min_features
        })
        .map(|(id, e)| {
            let file = e.file.to_str().unwrap_or("");
            (id, file, e.semantic_features.as_slice())
        })
        .collect();

    if eligible.len() < 2 {
        return Vec::new();
    }

    // Step 2: Build inverted index: feature → Vec<eligible_index>
    // Skip features that appear in too many entities (too generic to be useful)
    let mut feature_freq: HashMap<&str, usize> = HashMap::new();
    for (_, _, features) in &eligible {
        for f in *features {
            *feature_freq.entry(f.as_str()).or_insert(0) += 1;
        }
    }

    let mut inverted: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, (_, _, features)) in eligible.iter().enumerate() {
        for f in *features {
            if feature_freq.get(f.as_str()).copied().unwrap_or(0) <= config.max_feature_frequency {
                inverted.entry(f.as_str()).or_default().push(idx);
            }
        }
    }

    // Step 3: Collect candidate pairs that share at least one feature
    let mut shared_counts: HashMap<(usize, usize), usize> = HashMap::new();
    for indices in inverted.values() {
        if indices.len() < 2 {
            continue;
        }
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a = indices[i].min(indices[j]);
                let b = indices[i].max(indices[j]);
                *shared_counts.entry((a, b)).or_insert(0) += 1;
            }
        }
    }

    // Step 4: Compute exact Jaccard for candidates and filter by threshold
    let mut groups: Vec<SemanticCloneGroup> = Vec::new();
    for ((a_idx, b_idx), shared_count) in &shared_counts {
        let (a_id, a_file, a_features) = eligible[*a_idx];
        let (b_id, b_file, b_features) = eligible[*b_idx];

        // Early bail: shared / max(|A|, |B|) is an upper bound on Jaccard
        let upper_bound = *shared_count as f64 / a_features.len().max(b_features.len()) as f64;
        if upper_bound < config.similarity_threshold {
            continue;
        }

        if config.skip_same_file && a_file == b_file {
            continue;
        }

        let a_set: HashSet<&str> = a_features.iter().map(|s| s.as_str()).collect();
        let b_set: HashSet<&str> = b_features.iter().map(|s| s.as_str()).collect();
        let sim = jaccard_similarity(&a_set, &b_set);

        if sim < config.similarity_threshold {
            continue;
        }

        let mut shared: Vec<String> = a_set
            .intersection(&b_set)
            .map(|s| (*s).to_string())
            .collect();
        shared.sort();

        groups.push(SemanticCloneGroup {
            entities: vec![a_id.clone(), b_id.clone()],
            similarity: (sim * 1000.0).round() / 1000.0,
            shared_features: shared,
            files: vec![a_file.to_string(), b_file.to_string()],
        });
    }

    // Step 5: Sort by similarity descending, cap results
    groups.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    groups.truncate(config.max_results);
    groups
}

/// Token type for normalized code representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TokenType {
    /// Identifier (variable, function, class name) - normalized
    Identifier,
    /// Keyword (if, else, fn, let, etc.)
    Keyword,
    /// Operator (+, -, *, /, =, etc.)
    Operator,
    /// Literal (number, string - replaced with placeholder)
    Literal,
    /// Punctuation ({, }, (, ), ;, etc.)
    Punctuation,
}

/// A normalized token for fingerprinting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Token {
    kind: TokenType,
    value: u64,
}

/// Tokenize source code into normalized tokens.
///
/// Per paper §3.4: strip whitespace/comments, normalize identifiers,
/// replace literals with placeholders for Type-2 clone detection.
fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            // Skip whitespace
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            // Single-line comment
            '/' if chars.clone().nth(1) == Some('/') => {
                chars.next();
                chars.next();
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            // Multi-line comment (Rust-style)
            '/' if chars.clone().nth(1) == Some('*') => {
                chars.next();
                chars.next();
                while let Some(&c) = chars.peek() {
                    if c == '*' && chars.clone().nth(1) == Some('/') {
                        chars.next();
                        chars.next();
                        break;
                    }
                    chars.next();
                }
            }
            // String literal
            '"' | '\'' => {
                let quote = ch;
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == quote {
                        break;
                    }
                    if c == '\\' {
                        chars.next();
                    }
                }
                tokens.push(Token {
                    kind: TokenType::Literal,
                    value: hash_str("LIT"),
                });
            }
            // Number literal
            '0'..='9' => {
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit()
                        || c == '.'
                        || c == 'x'
                        || c == 'X'
                        || c == 'e'
                        || c == 'E'
                    {
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token {
                    kind: TokenType::Literal,
                    value: hash_str("LIT"),
                });
            }
            // Identifier or keyword
            'a'..='z' | 'A'..='Z' | '_' => {
                let mut ident = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        ident.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let kind = if is_keyword(&ident) {
                    TokenType::Keyword
                } else {
                    TokenType::Identifier
                };
                // Normalize identifiers: hash by kind, not by name (Type-2 detection)
                tokens.push(Token {
                    kind,
                    value: if kind == TokenType::Keyword {
                        hash_str(&ident)
                    } else {
                        hash_str("ID")
                    },
                });
            }
            // Operators (multi-char first)
            '<' | '>' | '=' | '!' | '&' | '|' | '+' | '-' | '*' | '/' | '%' | '^' => {
                let mut op = String::new();
                op.push(chars.next().unwrap());
                // Check for two-char operators
                if let Some(&c) = chars.peek()
                    && matches!(c, '=' | '&' | '|' | '<' | '>' | '+')
                {
                    op.push(c);
                    chars.next();
                }
                tokens.push(Token {
                    kind: TokenType::Operator,
                    value: hash_str(&op),
                });
            }
            // Punctuation
            '{' | '}' | '(' | ')' | '[' | ']' | ';' | ':' | ',' | '.' | '#' | '@' | '~' | '?' => {
                tokens.push(Token {
                    kind: TokenType::Punctuation,
                    value: hash_str(&ch.to_string()),
                });
                chars.next();
            }
            // Unknown - skip
            _ => {
                chars.next();
            }
        }
    }

    tokens
}

/// Check if a string is a programming language keyword.
#[allow(clippy::match_same_arms)]
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        // Rust
        "fn" | "let" | "mut" | "const" | "static" | "pub" | "mod" | "use" | "crate" | "self"
        | "Self" | "super" | "struct" | "enum" | "impl" | "trait" | "type" | "where" | "async"
        | "await" | "move" | "ref" | "match" | "if" | "else" | "loop" | "while" | "for" | "in"
        | "return" | "break" | "continue" | "unsafe" | "extern" | "dyn" | "as"
        // TypeScript/JavaScript
        | "function" | "var" | "class" | "interface" | "extends" | "implements" | "import"
        | "export" | "from" | "default" | "new" | "this" | "typeof" | "instanceof" | "void"
        | "null" | "undefined" | "true" | "false" | "try" | "catch" | "finally" | "throw"
        | "switch" | "case" | "do" | "delete" | "yield" | "constructor" | "readonly"
        // Python
        | "def" | "lambda" | "pass" | "raise" | "except" | "with" | "assert" | "global"
        | "nonlocal" | "print" | "elif"
        // Go
        | "package" | "go" | "chan" | "select" | "defer" | "fallthrough" | "goto" | "range"
        | "map" | "make" | "append" | "copy"
        // Java
        | "public" | "private" | "protected" | "final" | "abstract" | "synchronized"
        | "volatile" | "transient" | "native" | "strictfp" | "throws"
        // C/C++
        | "int" | "char" | "float" | "double" | "long" | "short" | "unsigned" | "signed"
        | "auto" | "register" | "inline" | "restrict" | "sizeof" | "typedef"
    )
}

/// Hash a string to a u64 value.
fn hash_str(s: &str) -> u64 {
    let mut hash: u64 = 0;
    for byte in s.bytes() {
        hash = (hash.wrapping_mul(HASH_BASE).wrapping_add(u64::from(byte))) % HASH_MOD;
    }
    hash
}

/// Compute Rabin-Karp fingerprints for a token stream.
///
/// Per paper Algorithm 4: slide a window of size w over tokens,
/// computing rolling hash for each window position.
fn compute_fingerprints(tokens: &[Token], window_size: usize) -> Vec<u64> {
    if tokens.len() < window_size {
        return Vec::new();
    }

    let mut fingerprints = Vec::with_capacity(tokens.len() - window_size + 1);

    // Pre-compute base^(window_size - 1) mod MOD for rolling hash.
    // Use iterative modular exponentiation to avoid u64 overflow (256^49 >> u64::MAX).
    let base_pow: u64 = (0..window_size - 1).fold(1u64, |acc, _| (acc * HASH_BASE) % HASH_MOD);

    // Compute initial window hash
    let mut hash: u64 = 0;
    for token in tokens.iter().take(window_size) {
        hash = (hash.wrapping_mul(HASH_BASE).wrapping_add(token.value)) % HASH_MOD;
    }
    fingerprints.push(hash);

    // Roll the window
    for i in window_size..tokens.len() {
        // Remove leftmost token's contribution
        let left_val = (tokens[i - window_size].value * base_pow) % HASH_MOD;
        hash = (hash + HASH_MOD - left_val) % HASH_MOD;
        // Add new token
        hash = (hash.wrapping_mul(HASH_BASE).wrapping_add(tokens[i].value)) % HASH_MOD;
        fingerprints.push(hash);
    }

    fingerprints
}

/// Entity with its source code and fingerprints.
#[derive(Debug)]
struct EntityFingerprints {
    entity_id: String,
    file: String,
    fps: Vec<u64>,
    token_count: usize,
}

/// Detect duplication across entities in the graph.
///
/// Per paper §3.4: compute fingerprints for each entity, store in HashMap,
/// find collisions indicating potential clones.
pub fn detect_duplication(
    graph: &RPGraph,
    project_root: &Path,
    config: &DuplicationConfig,
) -> Vec<CloneGroup> {
    use rayon::prelude::*;

    // Collect entities to analyze (skip Module entities)
    let entities: Vec<_> = graph
        .entities
        .iter()
        .filter(|(_, e)| e.kind != EntityKind::Module)
        .collect();

    // Phase 1: Cache file contents (read each file once, shared across entities)
    let file_contents: HashMap<std::path::PathBuf, String> = {
        let unique_files: HashSet<std::path::PathBuf> = entities
            .iter()
            .map(|(_, e)| project_root.join(&e.file))
            .collect();
        unique_files
            .into_iter()
            .filter_map(|p| std::fs::read_to_string(&p).ok().map(|s| (p, s)))
            .collect()
    };

    // Phase 2: Per-entity tokenization using line ranges
    let entity_fps: Vec<EntityFingerprints> = entities
        .par_iter()
        .filter_map(|(id, entity)| {
            let file_path = project_root.join(&entity.file);
            let source = file_contents.get(&file_path)?;

            // Extract only the entity's source lines (1-indexed → 0-indexed)
            let lines: Vec<&str> = source.lines().collect();
            let start = entity.line_start.saturating_sub(1);
            let end = entity.line_end.min(lines.len());
            if start >= end {
                return None;
            }
            let entity_source = lines[start..end].join("\n");

            let tokens = tokenize(&entity_source);
            if tokens.len() < config.min_tokens {
                return None;
            }

            let fingerprints = compute_fingerprints(&tokens, config.window_size);
            if fingerprints.is_empty() {
                return None;
            }

            Some(EntityFingerprints {
                entity_id: (*id).clone(),
                file: entity.file.display().to_string(),
                fps: fingerprints,
                token_count: tokens.len(),
            })
        })
        .collect();

    // Build fingerprint -> entity mapping (find collisions)
    let mut fingerprint_map: HashMap<u64, Vec<usize>> = HashMap::new();
    for (idx, ef) in entity_fps.iter().enumerate() {
        for &fp in &ef.fps {
            fingerprint_map.entry(fp).or_default().push(idx);
        }
    }

    // Find entity pairs with high fingerprint overlap.
    // Deduplicate indices per fingerprint: the same entity can produce many
    // matching windows for a single fingerprint value, so we must count each
    // (entity_a, entity_b) pair at most once per fingerprint to keep
    // similarity ≤ 1.0.
    let mut pair_scores: HashMap<(usize, usize), usize> = HashMap::new();
    for indices in fingerprint_map.values() {
        if indices.len() < 2 {
            continue;
        }
        // Unique entity indices that share this fingerprint
        let unique: Vec<usize> = {
            let mut set: Vec<usize> = indices.clone();
            set.sort_unstable();
            set.dedup();
            set
        };
        if unique.len() < 2 {
            continue;
        }
        for i in 0..unique.len() {
            for j in (i + 1)..unique.len() {
                let a = unique[i]; // already sorted
                let b = unique[j];
                *pair_scores.entry((a, b)).or_insert(0) += 1;
            }
        }
    }

    // Convert to similarity and filter by threshold
    let mut clone_groups: Vec<CloneGroup> = Vec::new();
    for ((a, b), shared) in pair_scores {
        let ef_a = &entity_fps[a];
        let ef_b = &entity_fps[b];

        // Duplication coefficient: shared fingerprints / min(fp_a, fp_b)
        let min_fps = ef_a.fps.len().min(ef_b.fps.len());
        if min_fps == 0 {
            continue;
        }

        let similarity = shared as f64 / min_fps as f64;
        if similarity < config.similarity_threshold {
            continue;
        }

        // Estimate duplicated tokens
        let ratio = (shared as f64 / ef_a.fps.len().max(1) as f64).clamp(0.0, 1.0);
        #[allow(clippy::cast_sign_loss)] // ratio is clamped to [0,1]; result is non-negative
        let duplicated_tokens = (ratio * ef_a.token_count as f64).round() as usize;

        if duplicated_tokens < config.min_tokens {
            continue;
        }

        clone_groups.push(CloneGroup {
            entities: vec![ef_a.entity_id.clone(), ef_b.entity_id.clone()],
            similarity: (similarity * 1000.0).round() / 1000.0,
            duplicated_tokens,
            files: vec![ef_a.file.clone(), ef_b.file.clone()],
        });
    }

    // Sort by similarity descending
    clone_groups.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit to top 50 groups to avoid overwhelming output
    clone_groups.truncate(50);

    clone_groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let source = "fn foo() { let x = 1; }";
        let tokens = tokenize(source);

        // Should have tokens for: fn, foo, (, ), {, let, x, =, 1, ;, }
        assert!(!tokens.is_empty());
        assert!(tokens.len() >= 8);
    }

    #[test]
    fn test_tokenize_normalizes_identifiers() {
        let source1 = "let foo = 1;";
        let source2 = "let bar = 2;";

        let tokens1 = tokenize(source1);
        let tokens2 = tokenize(source2);

        // Both should have same token sequence (identifiers normalized to "ID")
        let values1: Vec<_> = tokens1.iter().map(|t| t.value).collect();
        let values2: Vec<_> = tokens2.iter().map(|t| t.value).collect();

        // Keywords and structure should match
        assert_eq!(tokens1.len(), tokens2.len());
        assert_eq!(values1, values2);
    }

    #[test]
    fn test_tokenize_strips_comments() {
        let source = "fn foo() { /* comment */ let x = 1; }\n// line comment\nlet y = 2;";
        let tokens = tokenize(source);

        // Comments should be stripped
        let token_values: Vec<_> = tokens.iter().map(|t| t.value).collect();
        assert!(!token_values.iter().any(|&v| v == hash_str("comment")));
    }

    #[test]
    fn test_tokenize_normalizes_literals() {
        let source1 = "let x = 42;";
        let source2 = "let x = 999999;";

        let tokens1 = tokenize(source1);
        let tokens2 = tokenize(source2);

        // Literals should both be normalized to same value
        let lit1 = tokens1.iter().find(|t| t.kind == TokenType::Literal);
        let lit2 = tokens2.iter().find(|t| t.kind == TokenType::Literal);

        assert_eq!(lit1.map(|t| t.value), lit2.map(|t| t.value));
    }

    #[test]
    fn test_fingerprints_deterministic() {
        let source = "fn foo() { let x = 1; let y = 2; return x + y; }";
        let tokens = tokenize(source);

        let fp1 = compute_fingerprints(&tokens, 10);
        let fp2 = compute_fingerprints(&tokens, 10);

        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_fingerprints_empty_for_short_input() {
        let source = "fn";
        let tokens = tokenize(source);

        let fp = compute_fingerprints(&tokens, 10);

        assert!(fp.is_empty());
    }

    #[test]
    fn test_identical_code_high_similarity() {
        let source = r"
            fn calculate_total(items: &[Item]) -> f64 {
                let mut total = 0.0;
                for item in items {
                    total += item.price * item.quantity;
                }
                total
            }
        ";

        let tokens = tokenize(source);
        let fps = compute_fingerprints(&tokens, DEFAULT_WINDOW_SIZE);

        // Same code should have matching fingerprints
        let tokens2 = tokenize(source);
        let fps2 = compute_fingerprints(&tokens2, DEFAULT_WINDOW_SIZE);

        assert_eq!(fps, fps2);
    }

    #[test]
    fn test_type2_clone_detection() {
        // Type-2: same structure, renamed identifiers
        let source1 = r"
            fn process_data(input: &str) -> String {
                let result = input.to_uppercase();
                result.trim().to_string()
            }
        ";

        let source2 = r"
            fn handle_text(data: &str) -> String {
                let output = data.to_uppercase();
                output.trim().to_string()
            }
        ";

        let tokens1 = tokenize(source1);
        let tokens2 = tokenize(source2);

        // Structure should be identical after normalization
        let values1: Vec<_> = tokens1.iter().map(|t| t.value).collect();
        let values2: Vec<_> = tokens2.iter().map(|t| t.value).collect();

        assert_eq!(
            values1, values2,
            "Type-2 clones should normalize to same tokens"
        );
    }

    #[test]
    fn test_is_keyword() {
        assert!(is_keyword("fn"));
        assert!(is_keyword("let"));
        assert!(is_keyword("function"));
        assert!(is_keyword("class"));
        assert!(is_keyword("def"));
        assert!(!is_keyword("my_function"));
        assert!(!is_keyword("MyClass"));
        assert!(!is_keyword("variable_name"));
    }

    // --- Semantic duplication tests ---

    use rpg_core::graph::{Entity, EntityDeps};
    use std::path::PathBuf;

    fn make_entity_with_features(id: &str, file: &str, features: Vec<&str>) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: id.to_string(),
            file: PathBuf::from(file),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: features.into_iter().map(|s| s.to_string()).collect(),
            feature_source: Some("llm".to_string()),
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
            signature: None,
        }
    }

    #[test]
    fn test_semantic_duplicates_identical_features() {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/a.rs:process".to_string(),
            make_entity_with_features(
                "src/a.rs:process",
                "src/a.rs",
                vec!["validate input", "handle error"],
            ),
        );
        graph.entities.insert(
            "src/b.rs:handle".to_string(),
            make_entity_with_features(
                "src/b.rs:handle",
                "src/b.rs",
                vec!["validate input", "handle error"],
            ),
        );

        let config = SemanticDuplicationConfig {
            similarity_threshold: 0.6,
            skip_same_file: true,
            ..Default::default()
        };
        let groups = detect_semantic_duplicates(&graph, &config);

        assert_eq!(groups.len(), 1);
        assert!((groups[0].similarity - 1.0).abs() < 0.001);
        assert_eq!(groups[0].shared_features.len(), 2);
    }

    #[test]
    fn test_semantic_duplicates_skips_same_file() {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/a.rs:foo".to_string(),
            make_entity_with_features(
                "src/a.rs:foo",
                "src/a.rs",
                vec!["validate input", "return result"],
            ),
        );
        graph.entities.insert(
            "src/a.rs:bar".to_string(),
            make_entity_with_features(
                "src/a.rs:bar",
                "src/a.rs",
                vec!["validate input", "return result"],
            ),
        );

        // skip_same_file=true should suppress the pair
        let config = SemanticDuplicationConfig {
            similarity_threshold: 0.5,
            skip_same_file: true,
            ..Default::default()
        };
        assert!(detect_semantic_duplicates(&graph, &config).is_empty());

        // skip_same_file=false should surface it
        let config2 = SemanticDuplicationConfig {
            similarity_threshold: 0.5,
            skip_same_file: false,
            ..Default::default()
        };
        assert_eq!(detect_semantic_duplicates(&graph, &config2).len(), 1);
    }

    #[test]
    fn test_semantic_duplicates_skips_unlifted_entities() {
        let mut graph = RPGraph::new("rust");
        // Unlifted entity (no features) — must not participate
        graph.entities.insert(
            "src/a.rs:empty".to_string(),
            make_entity_with_features("src/a.rs:empty", "src/a.rs", vec![]),
        );
        // Two lifted entities from different files with same feature
        graph.entities.insert(
            "src/b.rs:lifted_one".to_string(),
            make_entity_with_features("src/b.rs:lifted_one", "src/b.rs", vec!["handle request"]),
        );
        graph.entities.insert(
            "src/c.rs:lifted_two".to_string(),
            make_entity_with_features("src/c.rs:lifted_two", "src/c.rs", vec!["handle request"]),
        );

        let config = SemanticDuplicationConfig {
            similarity_threshold: 0.9,
            min_features: 1,
            ..Default::default()
        };
        let groups = detect_semantic_duplicates(&graph, &config);

        // Only the two lifted entities should match; the unlifted one must not appear
        assert_eq!(groups.len(), 1);
        assert!(
            !groups[0].entities.contains(&"src/a.rs:empty".to_string()),
            "unlifted entity must not appear in semantic clone groups"
        );
    }

    // --- Per-entity token-based detection tests ---

    /// Helper: create an Entity with specific line range (for detect_duplication tests).
    fn make_entity_at_lines(id: &str, file: &str, line_start: usize, line_end: usize) -> Entity {
        Entity {
            id: id.to_string(),
            kind: EntityKind::Function,
            name: id.to_string(),
            file: PathBuf::from(file),
            line_start,
            line_end,
            parent_class: None,
            semantic_features: Vec::new(),
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
            signature: None,
        }
    }

    #[test]
    fn test_detect_duplication_identical_functions() {
        // Two files each containing the same function at known line ranges.
        // detect_duplication should find them as a clone pair.
        let dir = tempfile::tempdir().unwrap();

        let func_code = r#"fn looks_like_custom_hook(name: &str) -> bool {
    if !name.starts_with("use") || name.len() <= 3 {
        return false;
    }
    name.chars().nth(3).is_some_and(|c| c.is_ascii_uppercase())
}
"#;
        // File A: function at lines 1-6
        let file_a = dir.path().join("a.rs");
        std::fs::write(&file_a, func_code).unwrap();

        // File B: preamble + same function at lines 3-8
        let file_b = dir.path().join("b.rs");
        std::fs::write(&file_b, format!("// preamble\nuse std::io;\n{}", func_code)).unwrap();

        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "a.rs:looks_like_custom_hook".to_string(),
            make_entity_at_lines("a.rs:looks_like_custom_hook", "a.rs", 1, 6),
        );
        graph.entities.insert(
            "b.rs:looks_like_custom_hook".to_string(),
            make_entity_at_lines("b.rs:looks_like_custom_hook", "b.rs", 3, 8),
        );

        let config = DuplicationConfig {
            window_size: 10,
            min_tokens: 10,
            similarity_threshold: 0.5,
        };
        let groups = detect_duplication(&graph, dir.path(), &config);

        assert!(
            !groups.is_empty(),
            "identical functions across files must be detected as clones"
        );
        assert!(
            groups[0].similarity <= 1.0,
            "similarity must not exceed 1.0, got {}",
            groups[0].similarity
        );
        assert!(
            groups[0].similarity > 0.7,
            "identical functions should have high similarity, got {}",
            groups[0].similarity
        );
    }

    #[test]
    fn test_detect_duplication_similarity_bounded() {
        // Ensure the dedup fix keeps similarity ≤ 1.0 even with many fingerprint collisions.
        let dir = tempfile::tempdir().unwrap();

        // Two entities with the EXACT same source → shared fingerprints == min fingerprints
        let source = "fn compute(x: i32, y: i32) -> i32 { let result = x + y; result * result }\n";
        let file_a = dir.path().join("x.rs");
        let file_b = dir.path().join("y.rs");
        std::fs::write(&file_a, source).unwrap();
        std::fs::write(&file_b, source).unwrap();

        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "x.rs:compute".to_string(),
            make_entity_at_lines("x.rs:compute", "x.rs", 1, 1),
        );
        graph.entities.insert(
            "y.rs:compute".to_string(),
            make_entity_at_lines("y.rs:compute", "y.rs", 1, 1),
        );

        let config = DuplicationConfig {
            window_size: 5,
            min_tokens: 5,
            similarity_threshold: 0.1,
        };
        let groups = detect_duplication(&graph, dir.path(), &config);

        for group in &groups {
            assert!(
                group.similarity <= 1.0,
                "similarity must be ≤ 1.0 after dedup fix, got {}",
                group.similarity
            );
        }
    }

    #[test]
    fn test_detect_duplication_no_clones_for_different_code() {
        // Two completely different functions should NOT be reported as clones.
        let dir = tempfile::tempdir().unwrap();

        let file_a = dir.path().join("add.rs");
        std::fs::write(&file_a, "fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();

        let file_b = dir.path().join("greet.rs");
        std::fs::write(
            &file_b,
            "fn greet(name: &str) -> String { format!(\"Hello, {}!\", name) }\n",
        )
        .unwrap();

        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "add.rs:add".to_string(),
            make_entity_at_lines("add.rs:add", "add.rs", 1, 1),
        );
        graph.entities.insert(
            "greet.rs:greet".to_string(),
            make_entity_at_lines("greet.rs:greet", "greet.rs", 1, 1),
        );

        let config = DuplicationConfig {
            window_size: 5,
            min_tokens: 5,
            similarity_threshold: 0.7,
        };
        let groups = detect_duplication(&graph, dir.path(), &config);

        assert!(
            groups.is_empty(),
            "completely different functions should not be reported as clones"
        );
    }

    #[test]
    fn test_detect_duplication_invalid_line_range() {
        // Entity with line_start > line_end or beyond file length → no panic, just skipped.
        let dir = tempfile::tempdir().unwrap();

        let file_a = dir.path().join("short.rs");
        std::fs::write(&file_a, "fn tiny() {}\n").unwrap(); // 1 line

        let mut graph = RPGraph::new("rust");
        // line_start beyond file length
        graph.entities.insert(
            "short.rs:far".to_string(),
            make_entity_at_lines("short.rs:far", "short.rs", 100, 200),
        );
        // line_start = 0 (edge: saturating_sub converts to 0-indexed start of 0)
        graph.entities.insert(
            "short.rs:zero".to_string(),
            make_entity_at_lines("short.rs:zero", "short.rs", 0, 1),
        );

        let config = DuplicationConfig::default();
        // Must not panic
        let groups = detect_duplication(&graph, dir.path(), &config);
        // No meaningful pairs expected from degenerate ranges
        assert!(groups.is_empty() || groups.iter().all(|g| g.similarity <= 1.0));
    }

    #[test]
    fn test_detect_duplication_missing_file() {
        // Entity referencing a non-existent file → gracefully skipped, no panic.
        let dir = tempfile::tempdir().unwrap();

        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "gone.rs:phantom".to_string(),
            make_entity_at_lines("gone.rs:phantom", "gone.rs", 1, 10),
        );

        let config = DuplicationConfig::default();
        let groups = detect_duplication(&graph, dir.path(), &config);
        assert!(
            groups.is_empty(),
            "missing files should be skipped, not cause errors"
        );
    }
}
