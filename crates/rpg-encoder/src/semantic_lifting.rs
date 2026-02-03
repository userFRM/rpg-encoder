//! Semantic Lifting — extract semantic features for code entities via LLM.
//!
//! Uses a pipe-delimited line format inspired by TOON (<https://github.com/toon-format/toon>)
//! for LLM output parsing. This is more resilient than JSON — partial corruption only
//! loses individual lines, not the entire response.

use crate::llm::LlmClient;
use anyhow::Result;
use rpg_core::config::LlmConfig;
use rpg_parser::entities::RawEntity;
use std::collections::HashMap;

/// The semantic parsing system prompt (from Appendix A.1.1 of the paper).
/// Uses pipe-delimited line format instead of JSON for LLM-resilient parsing.
pub const SEMANTIC_PARSING_SYSTEM: &str = include_str!("prompts/semantic_parsing.md");

/// Parse pipe-delimited line format into features map.
/// Format: `entity_name | feature1, feature2, feature3`
///
/// This is far more resilient than JSON — partial corruption only loses
/// individual lines, not the entire response.
pub fn parse_line_features(text: &str) -> HashMap<String, Vec<String>> {
    // Strip <think>...</think> blocks (qwen3, deepseek emit these)
    let text = LlmClient::strip_think_blocks(text);
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

/// Process a batch of raw entities through the LLM to extract semantic features.
///
/// When `config` is provided, uses `complete_with_retry` for transient failure resilience.
/// Also performs schema validation: logs warnings for missing entities and retries
/// with a sub-batch if >50% are missing.
pub async fn lift_batch(
    client: &LlmClient,
    entities: &[RawEntity],
    repo_name: &str,
    repo_info: &str,
    config: Option<&LlmConfig>,
) -> Result<HashMap<String, Vec<String>>> {
    if entities.is_empty() {
        return Ok(HashMap::new());
    }

    let prompt = build_lift_prompt(entities, repo_name, repo_info);

    // Use retry when config is available
    let response = if let Some(cfg) = config {
        client
            .complete_with_retry(SEMANTIC_PARSING_SYSTEM, &prompt, cfg)
            .await?
    } else {
        client.complete(SEMANTIC_PARSING_SYSTEM, &prompt).await?
    };

    let mut features = parse_line_features(&response);

    // If line parsing found nothing, try JSON fallback for backward compat
    if features.is_empty()
        && let Ok(json_features) =
            LlmClient::parse_json_response::<HashMap<String, Vec<String>>>(&response)
    {
        features = json_features;
    }

    // Schema validation: check for missing entities
    let missing: Vec<&RawEntity> = entities
        .iter()
        .filter(|e| !features.contains_key(&e.name))
        .collect();

    if !missing.is_empty() {
        eprintln!(
            "  Warning: {}/{} entities missing from LLM response",
            missing.len(),
            entities.len()
        );

        // Retry with missing-only sub-batch if >50% missing and config available
        if missing.len() * 2 > entities.len() && config.is_some() {
            let retry_entities: Vec<RawEntity> = missing.iter().map(|e| (*e).clone()).collect();
            let retry_prompt = build_lift_prompt(&retry_entities, repo_name, repo_info);

            let retry_response = if let Some(cfg) = config {
                client
                    .complete_with_retry(SEMANTIC_PARSING_SYSTEM, &retry_prompt, cfg)
                    .await
            } else {
                client
                    .complete(SEMANTIC_PARSING_SYSTEM, &retry_prompt)
                    .await
            };

            if let Ok(resp) = retry_response {
                let retry_features = parse_line_features(&resp);
                for (name, feats) in retry_features {
                    features.entry(name).or_insert(feats);
                }
            }
        }
    }

    normalize_features(&mut features);
    Ok(features)
}

/// Build the user prompt for a batch of entities.
fn build_lift_prompt(entities: &[RawEntity], repo_name: &str, repo_info: &str) -> String {
    let mut prompt = format!(
        "### Repository Name\n{}\n\n### Repository Overview\n{}\n\n### Code to Analyze\n\n",
        repo_name, repo_info
    );

    for entity in entities {
        prompt.push_str(&format!(
            "#### {} ({:?}) in {}\n```\n{}\n```\n\n",
            entity.name,
            entity.kind,
            entity.file.display(),
            entity.source_text
        ));
    }

    prompt
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
        // Only the corrupted line is lost, not the whole batch
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
        // Paper: "If a function does not implement meaningful features, still include it with an empty list"
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
                "parse config".to_string(), // non-consecutive duplicate
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
