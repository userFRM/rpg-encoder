//! Semantic Lifting utilities — parsing and normalization for semantic features.
//!
//! Uses a pipe-delimited line format inspired by TOON (<https://github.com/toon-format/toon>)
//! for LLM output parsing. This is more resilient than JSON — partial corruption only
//! loses individual lines, not the entire response.

use rpg_core::graph::RPGraph;
use std::collections::HashMap;

/// The semantic parsing system prompt (from Appendix A.1.1 of the paper).
/// Uses pipe-delimited line format instead of JSON for LLM-resilient parsing.
pub const SEMANTIC_PARSING_SYSTEM: &str = include_str!("prompts/semantic_parsing.md");

/// File-level feature synthesis prompt (paper §9.1.1).
/// Synthesizes per-entity features into a holistic file-level summary.
pub const FILE_SYNTHESIS_SYSTEM: &str = include_str!("prompts/file_synthesis.md");

/// Domain discovery prompt — guides the LLM to identify functional areas from file features.
pub const DOMAIN_DISCOVERY_PROMPT: &str = include_str!("prompts/domain_discovery.md");

/// Hierarchy construction prompt — guides the LLM to assign files to 3-level hierarchy paths.
pub const HIERARCHY_CONSTRUCTION_PROMPT: &str = include_str!("prompts/hierarchy_construction.md");

/// Semantic routing prompt — guides the LLM to re-route drifted entities in the hierarchy.
pub const SEMANTIC_ROUTING_PROMPT: &str = include_str!("prompts/semantic_routing.md");

/// Strip `<think>...</think>` blocks that some models emit (qwen3, deepseek).
fn strip_think_blocks(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find("<think>") {
        if let Some(end_offset) = result[start..].find("</think>") {
            let end = start + end_offset + "</think>".len();
            result = format!("{}{}", &result[..start], &result[end..]);
        } else {
            // Unclosed think block — truncate from <think> onward
            result.truncate(start);
            break;
        }
    }
    result
}

/// Parse pipe-delimited line format into features map.
/// Format: `entity_name | feature1, feature2, feature3`
///
/// This is far more resilient than JSON — partial corruption only loses
/// individual lines, not the entire response.
pub fn parse_line_features(text: &str) -> HashMap<String, Vec<String>> {
    let text = strip_think_blocks(text);
    let text = text.as_str();

    let mut features: HashMap<String, Vec<String>> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();

        // Skip blank lines, comments, markdown fences
        if line.is_empty() || line.starts_with('#') || line.starts_with("```") {
            continue;
        }

        // Skip lines that look like "Example:" or other non-data
        if !line.contains('|') {
            continue;
        }

        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() != 2 {
            continue;
        }

        let name = parts[0].trim().to_string();
        if name.is_empty() {
            continue;
        }

        let feats: Vec<String> = parts[1]
            .split(',')
            .map(|f| f.trim().to_lowercase())
            .filter(|f| !f.is_empty())
            .collect();

        features.insert(name, feats);
    }

    features
}

/// Normalize semantic features per paper's post-processing:
/// trim whitespace, lowercase, deduplicate.
pub fn normalize_features(features: &mut HashMap<String, Vec<String>>) {
    for feats in features.values_mut() {
        for f in feats.iter_mut() {
            *f = f.trim().to_lowercase();
        }
        feats.sort();
        feats.dedup();
        feats.retain(|f| !f.is_empty());
    }
}

/// Apply extracted features to entities, matching by name.
pub fn apply_features(
    entities: &mut [rpg_core::graph::Entity],
    features: &HashMap<String, Vec<String>>,
) {
    for entity in entities.iter_mut() {
        if let Some(feats) = features.get(&entity.name) {
            entity.semantic_features = feats.clone();
        }
    }
}

/// Aggregate file-level features from child entities (dedup-only, no LLM).
///
/// For each Module entity, collects features from all child entities in the
/// same file and deduplicates them. This is the structural fallback for
/// file-level feature synthesis.
pub fn aggregate_module_features(graph: &mut RPGraph) -> usize {
    let module_data: Vec<(String, Vec<String>)> = graph
        .file_index
        .values()
        .filter_map(|ids| {
            let module_id = ids.iter().find(|id| {
                graph
                    .entities
                    .get(id.as_str())
                    .is_some_and(|e| e.kind == rpg_core::graph::EntityKind::Module)
            })?;

            let mut all_features: Vec<String> = ids
                .iter()
                .filter(|id| *id != module_id)
                .filter_map(|id| graph.entities.get(id))
                .flat_map(|e| e.semantic_features.clone())
                .collect();

            if all_features.is_empty() {
                return None;
            }

            all_features.sort();
            all_features.dedup();
            Some((module_id.clone(), all_features))
        })
        .collect();

    let count = module_data.len();
    for (module_id, features) in module_data {
        if let Some(module) = graph.entities.get_mut(&module_id) {
            module.semantic_features = features;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_features_basic() {
        let input = "parse_args | parse command arguments, validate input flags\nsend_request | send HTTP request, handle connection errors";
        let features = parse_line_features(input);
        assert_eq!(features.len(), 2);
        assert_eq!(
            features.get("parse_args").unwrap(),
            &vec!["parse command arguments", "validate input flags"]
        );
        assert_eq!(
            features.get("send_request").unwrap(),
            &vec!["send http request", "handle connection errors"]
        );
    }

    #[test]
    fn test_parse_line_features_skips_blanks_and_comments() {
        let input = "# This is a comment\n\nfoo | do something\n\n```json\nbaz | do another thing";
        let features = parse_line_features(input);
        assert_eq!(features.len(), 2);
        assert!(features.contains_key("foo"));
        assert!(features.contains_key("baz"));
    }

    #[test]
    fn test_parse_line_features_skips_lines_without_pipe() {
        let input = "Example:\nfoo | feature one\nSome other text\nbar | feature two";
        let features = parse_line_features(input);
        assert_eq!(features.len(), 2);
    }

    #[test]
    fn test_parse_line_features_empty_input() {
        let features = parse_line_features("");
        assert!(features.is_empty());
    }

    #[test]
    fn test_parse_line_features_partial_corruption() {
        let input = "good_func | parse arguments, validate flags\n| broken line no name\nbad_line_no_pipe\nanother_good | serialize json";
        let features = parse_line_features(input);
        assert_eq!(features.len(), 2);
        assert!(features.contains_key("good_func"));
        assert!(features.contains_key("another_good"));
    }

    #[test]
    fn test_parse_line_features_normalizes_case() {
        let input = "MyFunc | Parse JSON Data, Handle HTTP Errors";
        let features = parse_line_features(input);
        assert_eq!(
            features.get("MyFunc").unwrap(),
            &vec!["parse json data", "handle http errors"]
        );
    }

    #[test]
    fn test_parse_line_features_with_think_blocks() {
        let input =
            "<think>Let me analyze this code...</think>\nfoo | parse config\nbar | send request";
        let features = parse_line_features(input);
        assert_eq!(features.len(), 2);
    }

    #[test]
    fn test_parse_line_features_includes_stubs() {
        let input = "real_func | parse config, validate input\nstub_func | ";
        let features = parse_line_features(input);
        assert!(
            features.contains_key("stub_func"),
            "stub entities with empty features should still be included"
        );
        assert!(features.get("stub_func").unwrap().is_empty());
    }

    #[test]
    fn test_normalize_features_non_consecutive_dedup() {
        let mut features = HashMap::new();
        features.insert(
            "foo".to_string(),
            vec![
                "parse config".to_string(),
                "validate input".to_string(),
                "parse config".to_string(),
            ],
        );
        normalize_features(&mut features);
        let feats = features.get("foo").unwrap();
        assert_eq!(
            feats.len(),
            2,
            "non-consecutive duplicates should be removed"
        );
        assert!(feats.contains(&"parse config".to_string()));
        assert!(feats.contains(&"validate input".to_string()));
    }
}
