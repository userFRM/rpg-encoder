//! Structure Reorganization — discover functional areas and build hierarchy.
//!
//! Uses line-delimited formats inspired by TOON (<https://github.com/toon-format/toon>)
//! for LLM output parsing, providing graceful degradation on partial corruption.

use crate::llm::LlmClient;
use anyhow::Result;
use futures_util::stream::{self, StreamExt};
use rpg_core::graph::{Entity, RPGraph};
use std::collections::HashMap;

/// Domain discovery system prompt (from Appendix A.1.2).
/// Uses one-per-line output format for resilient parsing.
const DOMAIN_DISCOVERY_SYSTEM: &str = include_str!("prompts/domain_discovery.md");

/// Hierarchical construction system prompt.
/// Uses pipe-delimited line format for resilient parsing.
const HIERARCHY_SYSTEM: &str = include_str!("prompts/hierarchy_construction.md");

/// Parse one-per-line domain names.
fn parse_line_domains(text: &str) -> Vec<String> {
    let text = LlmClient::strip_think_blocks(text);
    let mut areas = Vec::new();

    for line in text.lines() {
        let line = line.trim();

        // Skip blank lines, comments, markdown fences, numbered prefixes
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("```")
            || line.starts_with("Example")
        {
            continue;
        }

        // Strip leading "- ", "* ", "1. " etc.
        let cleaned = line.trim_start_matches(['-', '*', '•']).trim_start();
        let cleaned = if cleaned.starts_with(|c: char| c.is_ascii_digit()) {
            cleaned
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim_start_matches('.')
                .trim_start()
        } else {
            cleaned
        };

        if cleaned.is_empty() {
            continue;
        }

        // Strip surrounding quotes if present
        let cleaned = cleaned.trim_matches('"').trim_matches('\'').trim();
        if !cleaned.is_empty() {
            areas.push(cleaned.to_string());
        }
    }

    areas
}

/// Parse pipe-delimited hierarchy assignments.
/// Format: `entity_name | Area/category/subcategory`
fn parse_line_hierarchy(text: &str) -> HashMap<String, String> {
    let text = LlmClient::strip_think_blocks(text);
    let mut assignments = HashMap::new();

    for line in text.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') || line.starts_with("```") {
            continue;
        }

        if !line.contains('|') {
            continue;
        }

        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() != 2 {
            continue;
        }

        let name = parts[0].trim().to_string();
        let path = parts[1].trim().to_string();

        if !name.is_empty() && !path.is_empty() && path.contains('/') {
            assignments.insert(name, path);
        }
    }

    assignments
}

/// Discover the high-level functional areas for a repository.
pub async fn discover_domains(
    client: &LlmClient,
    entities: &[Entity],
    repo_name: &str,
) -> Result<Vec<String>> {
    // Build a summary of all semantic features
    let mut features_summary = String::new();
    for entity in entities {
        if !entity.semantic_features.is_empty() {
            features_summary.push_str(&format!(
                "- {} ({}): {}\n",
                entity.name,
                entity.file.display(),
                entity.semantic_features.join(", ")
            ));
        }
    }

    let prompt = format!(
        "### Repository: {}\n\n### Entity Features\n{}\n\nBased on these features, identify the main functional areas.",
        repo_name, features_summary
    );

    let response = client.complete(DOMAIN_DISCOVERY_SYSTEM, &prompt).await?;
    let mut areas = parse_line_domains(&response);

    // Fallback to JSON parsing if line parsing found nothing
    if areas.is_empty()
        && let Ok(json_areas) = LlmClient::parse_json_response::<Vec<String>>(&response)
    {
        areas = json_areas;
    }

    if areas.is_empty() {
        anyhow::bail!("failed to parse domain discovery response");
    }

    Ok(areas)
}

/// Assign entities to hierarchy paths given the discovered functional areas.
pub async fn build_hierarchy(
    client: &LlmClient,
    entities: &[Entity],
    functional_areas: &[String],
    repo_name: &str,
    chunk_size: usize,
    concurrency: usize,
) -> Result<HashMap<String, String>> {
    build_hierarchy_with_chunk_size(
        client,
        entities,
        functional_areas,
        repo_name,
        chunk_size,
        concurrency,
    )
    .await
}

/// Assign entities to hierarchy paths with configurable chunk size and concurrency.
pub async fn build_hierarchy_with_chunk_size(
    client: &LlmClient,
    entities: &[Entity],
    functional_areas: &[String],
    repo_name: &str,
    chunk_size: usize,
    concurrency: usize,
) -> Result<HashMap<String, String>> {
    let areas_list = functional_areas.join("\n");
    let concurrency = concurrency.max(1);

    // Build prompts for each chunk
    let chunk_prompts: Vec<String> = entities
        .chunks(chunk_size)
        .map(|chunk| {
            let mut entity_summary = String::new();
            for entity in chunk {
                entity_summary.push_str(&format!(
                    "- {}: [{}]\n",
                    entity.name,
                    entity.semantic_features.join(", ")
                ));
            }
            format!(
                "### Repository: {}\n\n### Functional Areas\n{}\n\n### Entities to Assign\n{}\n\nAssign each entity to a three-level path.",
                repo_name, areas_list, entity_summary
            )
        })
        .collect();

    // Process chunks concurrently
    let results: Vec<Result<HashMap<String, String>>> = stream::iter(chunk_prompts)
        .map(|prompt| async move {
            let response = client.complete(HIERARCHY_SYSTEM, &prompt).await?;
            let mut assignments = parse_line_hierarchy(&response);

            // Fallback to JSON if line parsing found nothing
            if assignments.is_empty()
                && let Ok(json_assignments) =
                    LlmClient::parse_json_response::<HashMap<String, String>>(&response)
            {
                assignments = json_assignments;
            }

            Ok(assignments)
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    let mut all_assignments: HashMap<String, String> = HashMap::new();
    for result in results {
        match result {
            Ok(assignments) => all_assignments.extend(assignments),
            Err(e) => eprintln!("  Warning: hierarchy chunk failed: {}", e),
        }
    }

    if all_assignments.is_empty() {
        anyhow::bail!("failed to parse hierarchy assignment response");
    }

    Ok(all_assignments)
}

/// FindBestParent system prompt (Algorithm 4 from the paper).
/// Instructs LLM to pick the best-matching hierarchy node from candidates.
const FIND_BEST_PARENT_SYSTEM: &str = include_str!("prompts/find_best_parent.md");

/// Find the best hierarchy parent for a single entity via top-down LLM-guided descent (Algorithm 4).
///
/// Walks the hierarchy tree level by level, asking the LLM at each level to pick
/// the best-matching child node based on the entity's semantic features.
/// Supports early termination: if the LLM returns NONE at any level, the path
/// accumulated so far is returned (the entity stays at the deepest compatible level).
pub async fn find_best_parent(
    client: &LlmClient,
    entity: &Entity,
    hierarchy: &HashMap<String, rpg_core::graph::HierarchyNode>,
    repo_name: &str,
) -> Result<String> {
    let features_str = entity.semantic_features.join(", ");

    // Level 1: Pick best functional area (prefer abstract nodes with children)
    let area_candidates: Vec<(&String, &Vec<String>)> = {
        let abstract_only: Vec<_> = hierarchy
            .iter()
            .filter(|(_, node)| !node.children.is_empty())
            .map(|(name, node)| (name, &node.semantic_features))
            .collect();
        if abstract_only.is_empty() {
            hierarchy
                .iter()
                .map(|(name, node)| (name, &node.semantic_features))
                .collect()
        } else {
            abstract_only
        }
    };

    if area_candidates.is_empty() {
        anyhow::bail!("no hierarchy areas available");
    }

    let best_area = if area_candidates.len() == 1 {
        area_candidates[0].0.clone()
    } else {
        match pick_best_child(client, &features_str, &area_candidates, repo_name).await? {
            Some(area) => area,
            None => {
                // NONE at root level: use first area as fallback
                area_candidates[0].0.clone()
            }
        }
    };

    let area_node = hierarchy
        .get(&best_area)
        .ok_or_else(|| anyhow::anyhow!("LLM picked invalid area: {}", best_area))?;

    // Level 2: Pick best category within area
    if area_node.children.is_empty() {
        return Ok(best_area);
    }

    // Level 2: prefer abstract children (IsAbstract filter from paper Algorithm 4)
    let cat_candidates: Vec<(&String, &Vec<String>)> = {
        let abstract_only: Vec<_> = area_node
            .children
            .iter()
            .filter(|(_, node)| !node.children.is_empty())
            .map(|(name, node)| (name, &node.semantic_features))
            .collect();
        if abstract_only.is_empty() {
            area_node
                .children
                .iter()
                .map(|(name, node)| (name, &node.semantic_features))
                .collect()
        } else {
            abstract_only
        }
    };

    let best_cat = if cat_candidates.len() == 1 {
        cat_candidates[0].0.clone()
    } else {
        match pick_best_child(client, &features_str, &cat_candidates, repo_name).await? {
            Some(cat) => cat,
            None => return Ok(best_area), // Early termination at level 2
        }
    };

    let cat_node = match area_node.children.get(&best_cat) {
        Some(n) => n,
        None => return Ok(format!("{}/{}", best_area, best_cat)),
    };

    // Level 3: Pick best subcategory within category
    if cat_node.children.is_empty() {
        return Ok(format!("{}/{}", best_area, best_cat));
    }

    let sub_candidates: Vec<(&String, &Vec<String>)> = cat_node
        .children
        .iter()
        .map(|(name, node)| (name, &node.semantic_features))
        .collect();

    let best_sub = if sub_candidates.len() == 1 {
        sub_candidates[0].0.clone()
    } else {
        match pick_best_child(client, &features_str, &sub_candidates, repo_name).await? {
            Some(sub) => sub,
            None => return Ok(format!("{}/{}", best_area, best_cat)), // Early termination at level 3
        }
    };

    Ok(format!("{}/{}/{}", best_area, best_cat, best_sub))
}

/// Ask the LLM to pick the best candidate from a list, given entity features.
/// Returns `None` if the LLM determines no candidate is a good fit.
async fn pick_best_child(
    client: &LlmClient,
    entity_features: &str,
    candidates: &[(&String, &Vec<String>)],
    repo_name: &str,
) -> Result<Option<String>> {
    let mut candidates_str = String::new();
    for (name, features) in candidates {
        if features.is_empty() {
            candidates_str.push_str(&format!("- {}\n", name));
        } else {
            candidates_str.push_str(&format!("- {}: [{}]\n", name, features.join(", ")));
        }
    }

    let prompt = format!(
        "### Repository: {}\n\n### Entity Features\n{}\n\n### Candidates\n{}\n\nPick the single best candidate name, or NONE if no candidate fits.",
        repo_name, entity_features, candidates_str
    );

    let response = client
        .complete_with_max_tokens(FIND_BEST_PARENT_SYSTEM, &prompt, 64)
        .await?;
    let picked = response.trim().to_string();

    // Check if LLM returned NONE (no candidate fits)
    if picked.eq_ignore_ascii_case("none") || picked.eq_ignore_ascii_case("\"none\"") {
        return Ok(None);
    }

    // Validate the response matches one of the candidates
    let valid_names: Vec<&str> = candidates.iter().map(|(n, _)| n.as_str()).collect();
    if valid_names.contains(&picked.as_str()) {
        Ok(Some(picked))
    } else {
        // Try fuzzy match: find the candidate whose name is contained in the response
        for name in &valid_names {
            if response.contains(name) {
                return Ok(Some(name.to_string()));
            }
        }
        // Last resort: use first candidate
        Ok(Some(valid_names[0].to_string()))
    }
}

/// System prompt for generating a concise repo-level architectural summary.
const REPO_SUMMARY_SYSTEM: &str = include_str!("prompts/repo_summary.md");

/// Generate a high-level architectural summary for the repository.
///
/// Synthesizes functional areas, entity counts, and top semantic features
/// into a concise 3-5 sentence overview.
pub async fn generate_repo_summary(
    client: &LlmClient,
    graph: &RPGraph,
    repo_name: &str,
) -> Result<String> {
    let mut areas_summary = String::new();
    for (area_name, area_node) in &graph.hierarchy {
        let entity_count = area_node.entity_count();
        let top_features: Vec<&str> = area_node
            .semantic_features
            .iter()
            .take(5)
            .map(|s| s.as_str())
            .collect();
        areas_summary.push_str(&format!(
            "- {} ({} entities): {}\n",
            area_name,
            entity_count,
            top_features.join(", ")
        ));
    }

    let (lifted, total) = graph.lifting_coverage();
    let prompt = format!(
        "### Repository: {}\n### Language: {}\n### Entities: {} ({} semantically lifted)\n\n### Functional Areas\n{}\n\nWrite a concise architectural summary.",
        repo_name, graph.metadata.language, total, lifted, areas_summary
    );

    let response = client
        .complete_with_max_tokens(REPO_SUMMARY_SYSTEM, &prompt, 256)
        .await?;

    // Clean up: strip think blocks, trim whitespace
    let summary = LlmClient::strip_think_blocks(&response).trim().to_string();

    if summary.is_empty() {
        anyhow::bail!("empty repo summary from LLM");
    }

    Ok(summary)
}

/// Apply hierarchy assignments to the RPG graph.
pub fn apply_hierarchy(graph: &mut RPGraph, assignments: &HashMap<String, String>) {
    for (entity_name, path) in assignments {
        // Find the entity by name
        let entity_id = graph
            .entities
            .iter()
            .find(|(_, e)| e.name == *entity_name)
            .map(|(id, _)| id.clone());

        if let Some(id) = entity_id {
            if let Some(entity) = graph.entities.get_mut(&id) {
                entity.hierarchy_path = path.clone();
            }
            graph.insert_into_hierarchy(path, &id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_domains_basic() {
        let input = "CommandLineInterface\nHttpClient\nDataSerialization";
        let areas = parse_line_domains(input);
        assert_eq!(
            areas,
            vec!["CommandLineInterface", "HttpClient", "DataSerialization"]
        );
    }

    #[test]
    fn test_parse_line_domains_with_bullets() {
        let input = "- CommandLineInterface\n- HttpClient\n* DataSerialization";
        let areas = parse_line_domains(input);
        assert_eq!(
            areas,
            vec!["CommandLineInterface", "HttpClient", "DataSerialization"]
        );
    }

    #[test]
    fn test_parse_line_domains_with_numbers() {
        let input = "1. CommandLineInterface\n2. HttpClient\n3. DataSerialization";
        let areas = parse_line_domains(input);
        assert_eq!(
            areas,
            vec!["CommandLineInterface", "HttpClient", "DataSerialization"]
        );
    }

    #[test]
    fn test_parse_line_domains_skips_blanks_and_fences() {
        let input = "```json\nHttpClient\n\nDataSerialization";
        let areas = parse_line_domains(input);
        assert_eq!(areas, vec!["HttpClient", "DataSerialization"]);
    }

    #[test]
    fn test_parse_line_hierarchy_basic() {
        let input = "parse_args | CLI/parse input/read arguments\nsend_request | Http/connections/send data";
        let assignments = parse_line_hierarchy(input);
        assert_eq!(assignments.len(), 2);
        assert_eq!(
            assignments.get("parse_args").unwrap(),
            "CLI/parse input/read arguments"
        );
    }

    #[test]
    fn test_parse_line_hierarchy_skips_invalid() {
        let input = "good | Area/cat/sub\nbad_no_pipe\n| no_name\nalso_bad | no_slash";
        let assignments = parse_line_hierarchy(input);
        assert_eq!(assignments.len(), 1);
        assert!(assignments.contains_key("good"));
    }

    #[test]
    fn test_parse_line_hierarchy_with_think_blocks() {
        let input = "<think>analyzing...</think>\nfoo | Area/cat/sub\nbar | Area/cat2/sub2";
        let assignments = parse_line_hierarchy(input);
        assert_eq!(assignments.len(), 2);
    }
}
