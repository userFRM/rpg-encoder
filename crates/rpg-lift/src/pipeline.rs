//! Autonomous lifting pipeline — fire-and-forget semantic analysis.
//!
//! Orchestrates the full lifting flow: auto-lift → LLM entity lifting → finalize →
//! file synthesis → domain discovery → hierarchy construction. Each phase reuses
//! existing rpg-encoder utilities with LLM calls handled via the provider trait.

use crate::cost::CostTracker;
use crate::progress::LiftProgress;
use crate::provider::{LlmProvider, ProviderError};
use rpg_core::graph::RPGraph;
use rpg_encoder::lift::{
    AutoLiftEngine, LiftConfidence, build_token_aware_batches, collect_raw_entities, resolve_scope,
};
use rpg_encoder::semantic_lifting::{
    DOMAIN_DISCOVERY_PROMPT, FILE_SYNTHESIS_SYSTEM, HIERARCHY_CONSTRUCTION_PROMPT,
    SEMANTIC_PARSING_SYSTEM, aggregate_module_features, parse_line_features,
};
use rpg_parser::entities::RawEntity;
use std::collections::HashMap;
use std::path::Path;

/// Configuration for an autonomous lifting run.
pub struct LiftConfig<'a> {
    pub provider: &'a dyn LlmProvider,
    pub project_root: &'a Path,
    pub scope: &'a str,
    pub max_retries: usize,
    pub batch_size: usize,
    pub batch_tokens: usize,
}

/// Result of a completed lifting run.
#[derive(Debug)]
pub struct LiftReport {
    pub entities_auto_lifted: usize,
    pub entities_llm_lifted: usize,
    pub entities_failed: usize,
    pub batches_processed: usize,
    pub files_synthesized: usize,
    pub hierarchy_assigned: bool,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
    pub errors: Vec<String>,
}

/// Run the full autonomous lifting pipeline.
pub fn run_pipeline(
    graph: &mut RPGraph,
    config: &LiftConfig<'_>,
) -> Result<LiftReport, PipelineError> {
    let progress = LiftProgress::new();
    let mut tracker = CostTracker::new(config.provider);
    let mut errors: Vec<String> = Vec::new();

    // Phase 1: Resolve scope and collect raw entities
    let scope = resolve_scope(graph, config.scope);
    if scope.entity_ids.is_empty() {
        progress.finish();
        return Ok(LiftReport {
            entities_auto_lifted: 0,
            entities_llm_lifted: 0,
            entities_failed: 0,
            batches_processed: 0,
            files_synthesized: 0,
            hierarchy_assigned: false,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: 0.0,
            errors: vec!["No entities to lift (scope resolved to empty set)".to_string()],
        });
    }

    let raw_entities = collect_raw_entities(graph, &scope, config.project_root)
        .map_err(|e| PipelineError::Setup(e.to_string()))?;

    if raw_entities.is_empty() {
        progress.finish();
        return Ok(LiftReport {
            entities_auto_lifted: 0,
            entities_llm_lifted: 0,
            entities_failed: 0,
            batches_processed: 0,
            files_synthesized: 0,
            hierarchy_assigned: false,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: 0.0,
            errors: vec!["No source files could be read for scoped entities".to_string()],
        });
    }

    eprintln!(
        "  Found {} entities to process ({} in scope)",
        raw_entities.len(),
        scope.entity_ids.len()
    );

    // Phase 2: Auto-lift trivial entities
    let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
    let active_paradigms: Vec<String> = graph.metadata.paradigms.clone();
    let engine = AutoLiftEngine::new(&paradigm_defs, &active_paradigms);

    progress.start_phase("Auto-lift", raw_entities.len() as u64);

    let mut auto_lifted = 0usize;
    let mut needs_llm: Vec<&RawEntity> = Vec::new();

    for raw in &raw_entities {
        match engine.try_lift_with_confidence(raw) {
            Some((features, LiftConfidence::Accept)) => {
                let entity_id = raw.id();
                if let Some(entity) = graph.entities.get_mut(&entity_id) {
                    entity.semantic_features = features;
                    entity.feature_source = Some("auto".to_string());
                }
                auto_lifted += 1;
            }
            Some((features, LiftConfidence::Review)) => {
                // Apply auto-lift features but still queue for LLM review
                let entity_id = raw.id();
                if let Some(entity) = graph.entities.get_mut(&entity_id) {
                    entity.semantic_features = features;
                    entity.feature_source = Some("auto-review".to_string());
                }
                auto_lifted += 1;
                // Don't add to needs_llm — accept the auto-lift for autonomous mode
            }
            _ => {
                needs_llm.push(raw);
            }
        }
        progress.tick_phase();
    }

    progress.suspend(|| {
        eprintln!(
            "  Auto-lifted {} entities, {} need LLM",
            auto_lifted,
            needs_llm.len()
        );
    });

    // Phase 3: LLM entity lifting
    let mut llm_lifted = 0usize;
    let mut llm_failed = 0usize;
    let mut batches_done = 0usize;

    if !needs_llm.is_empty() {
        // Build owned copies for batching
        let llm_raws: Vec<RawEntity> = needs_llm.iter().map(|r| (*r).clone()).collect();
        let batches = build_token_aware_batches(&llm_raws, config.batch_size, config.batch_tokens);

        progress.start_phase("LLM Lift", batches.len() as u64);

        let repo_info =
            rpg_encoder::lift::generate_repo_info(graph, &project_name(config.project_root));

        for (batch_idx, &(start, end)) in batches.iter().enumerate() {
            let batch = &llm_raws[start..end];
            let user_prompt = format_entity_batch(batch, batch_idx == 0, &repo_info);

            match call_with_retry(
                config.provider,
                SEMANTIC_PARSING_SYSTEM,
                &user_prompt,
                config.max_retries,
            ) {
                Ok(response) => {
                    tracker.record(response.input_tokens, response.output_tokens);

                    let features = parse_line_features(&response.text);

                    // Apply features to graph entities
                    let mut batch_applied = 0;
                    for raw in batch {
                        let entity_id = raw.id();
                        // Match by entity name (parse_line_features returns name → features)
                        let name_key = &raw.name;
                        if let Some(feats) = features.get(name_key)
                            && let Some(entity) = graph.entities.get_mut(&entity_id)
                        {
                            entity.semantic_features = feats.clone();
                            entity.feature_source = Some("llm".to_string());
                            batch_applied += 1;
                        }
                    }
                    llm_lifted += batch_applied;
                    llm_failed += batch.len() - batch_applied;
                }
                Err(e) => {
                    let msg = format!("Batch {} failed: {}", batch_idx, e);
                    errors.push(msg.clone());
                    progress.suspend(|| eprintln!("  WARNING: {}", msg));
                    llm_failed += batch.len();
                }
            }

            batches_done += 1;
            progress.tick_phase();
            progress.update_cost(tracker.total_cost_usd(), tracker.total_cost_usd() * 1.2);

            // Save after each batch for crash recovery
            let config_storage = rpg_core::config::RpgConfig::load(config.project_root)
                .unwrap_or_default()
                .storage;
            graph.refresh_metadata();
            if let Err(e) =
                rpg_core::storage::save_with_config(config.project_root, graph, &config_storage)
            {
                errors.push(format!("Save after batch {} failed: {}", batch_idx, e));
            }
        }
    }

    // Phase 4: Finalize — aggregate module features
    progress.start_phase("Finalize", 1);
    let modules_aggregated = aggregate_module_features(graph);
    progress.tick_phase();

    progress.suspend(|| {
        eprintln!(
            "  Aggregated features for {} file modules",
            modules_aggregated
        );
    });

    // Phase 5: File synthesis
    let files_synthesized = run_file_synthesis(graph, config, &mut tracker, &mut errors, &progress);

    // Phase 6: Domain discovery + hierarchy construction
    let hierarchy_assigned =
        run_hierarchy_construction(graph, config, &mut tracker, &mut errors, &progress);

    // Final save
    graph.refresh_metadata();
    let config_storage = rpg_core::config::RpgConfig::load(config.project_root)
        .unwrap_or_default()
        .storage;
    let _ = rpg_core::storage::save_with_config(config.project_root, graph, &config_storage);

    progress.finish();

    Ok(LiftReport {
        entities_auto_lifted: auto_lifted,
        entities_llm_lifted: llm_lifted,
        entities_failed: llm_failed,
        batches_processed: batches_done,
        files_synthesized,
        hierarchy_assigned,
        total_input_tokens: tracker.total_input_tokens,
        total_output_tokens: tracker.total_output_tokens,
        total_cost_usd: tracker.total_cost_usd(),
        errors,
    })
}

// ---------------------------------------------------------------------------
// Phase 5: File synthesis
// ---------------------------------------------------------------------------

fn run_file_synthesis(
    graph: &mut RPGraph,
    config: &LiftConfig<'_>,
    tracker: &mut CostTracker,
    errors: &mut Vec<String>,
    progress: &LiftProgress,
) -> usize {
    // Collect Module entities with aggregated features
    let modules: Vec<(String, Vec<String>)> = graph
        .entities
        .iter()
        .filter(|(_, e)| {
            e.kind == rpg_core::graph::EntityKind::Module && !e.semantic_features.is_empty()
        })
        .map(|(id, e)| (id.clone(), e.semantic_features.clone()))
        .collect();

    if modules.is_empty() {
        return 0;
    }

    // Batch modules for synthesis (70 per batch)
    let batch_size = 70;
    let total_batches = modules.len().div_ceil(batch_size);

    progress.start_phase("Synthesis", total_batches as u64);

    let mut synthesized = 0usize;

    for (batch_idx, chunk) in modules.chunks(batch_size).enumerate() {
        let user_prompt = format_synthesis_batch(chunk);

        match call_with_retry(
            config.provider,
            FILE_SYNTHESIS_SYSTEM,
            &user_prompt,
            config.max_retries,
        ) {
            Ok(response) => {
                tracker.record(response.input_tokens, response.output_tokens);

                // Parse synthesis response — one line per file
                for line in response.text.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }

                    // Format: file_path | features  OR  just comma-separated features (for single-file batches)
                    if let Some((file_key, features_str)) = line.split_once('|') {
                        let file_key = file_key.trim();
                        let features: Vec<String> = features_str
                            .split(',')
                            .map(|f| f.trim().to_lowercase())
                            .filter(|f| !f.is_empty())
                            .collect();

                        // Find matching module by file path prefix
                        if let Some((module_id, _)) =
                            chunk.iter().find(|(id, _)| id.contains(file_key))
                            && let Some(entity) = graph.entities.get_mut(module_id)
                            && !features.is_empty()
                        {
                            entity.semantic_features = features;
                            synthesized += 1;
                        }
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Synthesis batch {} failed: {}", batch_idx, e));
            }
        }

        progress.tick_phase();
        progress.update_cost(tracker.total_cost_usd(), tracker.total_cost_usd() * 1.1);
    }

    progress.suspend(|| {
        eprintln!("  Synthesized features for {} file modules", synthesized);
    });

    synthesized
}

// ---------------------------------------------------------------------------
// Phase 6: Hierarchy construction
// ---------------------------------------------------------------------------

fn run_hierarchy_construction(
    graph: &mut RPGraph,
    config: &LiftConfig<'_>,
    tracker: &mut CostTracker,
    errors: &mut Vec<String>,
    progress: &LiftProgress,
) -> bool {
    let clusters = rpg_encoder::hierarchy::cluster_files_for_hierarchy(graph, 70);

    if clusters.is_empty() {
        return false;
    }

    // Step 1: Domain discovery — identify functional areas
    progress.start_phase("Discovery", 1);

    let file_features = collect_file_features(graph);
    let discovery_prompt = format_discovery_prompt(&file_features);

    let areas = match call_with_retry(
        config.provider,
        DOMAIN_DISCOVERY_PROMPT,
        &discovery_prompt,
        config.max_retries,
    ) {
        Ok(response) => {
            tracker.record(response.input_tokens, response.output_tokens);

            // Parse areas: one per line, PascalCase
            let areas: Vec<String> = response
                .text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("```"))
                .collect();

            if areas.is_empty() {
                errors.push("Domain discovery returned no areas".to_string());
                return false;
            }

            progress.suspend(|| {
                eprintln!(
                    "  Discovered {} functional areas: {}",
                    areas.len(),
                    areas.join(", ")
                );
            });

            areas
        }
        Err(e) => {
            errors.push(format!("Domain discovery failed: {}", e));
            progress.tick_phase();
            return false;
        }
    };

    progress.tick_phase();

    // Step 2: Hierarchy assignment — assign files to 3-level paths
    progress.start_phase("Hierarchy", clusters.len() as u64);

    let mut all_assignments: HashMap<String, String> = HashMap::new();

    for (cluster_idx, cluster) in clusters.iter().enumerate() {
        let user_prompt = format_hierarchy_prompt(&cluster.files, &areas, &file_features);

        match call_with_retry(
            config.provider,
            HIERARCHY_CONSTRUCTION_PROMPT,
            &user_prompt,
            config.max_retries,
        ) {
            Ok(response) => {
                tracker.record(response.input_tokens, response.output_tokens);

                // Parse assignments: file_path | FunctionalArea/category/subcategory
                for line in response.text.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') || line.starts_with("```") {
                        continue;
                    }
                    if let Some((file_path, hierarchy_path)) = line.split_once('|') {
                        let file_path = file_path.trim().to_string();
                        let hierarchy_path = hierarchy_path.trim().to_string();
                        if !hierarchy_path.is_empty()
                            && hierarchy_path.contains('/')
                            && !file_path.is_empty()
                        {
                            all_assignments.insert(file_path, hierarchy_path);
                        }
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Hierarchy batch {} failed: {}", cluster_idx, e));
            }
        }

        progress.tick_phase();
        progress.update_cost(tracker.total_cost_usd(), tracker.total_cost_usd());
    }

    if all_assignments.is_empty() {
        errors.push("No hierarchy assignments could be parsed".to_string());
        return false;
    }

    // Apply hierarchy
    rpg_encoder::hierarchy::apply_hierarchy(graph, &all_assignments);

    // Rebuild graph hierarchy metadata
    graph.metadata.semantic_hierarchy = true;
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();

    progress.suspend(|| {
        eprintln!(
            "  Applied {} hierarchy assignments across {} areas",
            all_assignments.len(),
            areas.len()
        );
    });

    true
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a batch of raw entities for the LLM entity-lifting prompt.
fn format_entity_batch(batch: &[RawEntity], is_first: bool, repo_info: &str) -> String {
    let mut prompt = String::new();

    if is_first {
        prompt.push_str(repo_info);
        prompt.push_str("\n\n");
    }

    for raw in batch {
        let kind = format!("{:?}", raw.kind);
        prompt.push_str(&format!("### {} ({})\n", raw.id(), kind));
        if let Some(parent) = &raw.parent_class {
            prompt.push_str(&format!("Parent: {}\n", parent));
        }

        // Truncate source to 40 lines
        let lines: Vec<&str> = raw.source_text.lines().collect();
        let truncated = lines.len() > 40;
        let source = if truncated {
            let mut s: String = lines[..40].join("\n");
            s.push_str("\n// ... truncated ...");
            s
        } else {
            raw.source_text.clone()
        };

        prompt.push_str("```\n");
        prompt.push_str(&source);
        prompt.push_str("\n```\n\n");
    }

    prompt
}

/// Format file modules for synthesis.
fn format_synthesis_batch(modules: &[(String, Vec<String>)]) -> String {
    let mut prompt = String::new();
    for (module_id, features) in modules {
        // Extract file path from module ID (format: "path/to/file.rs:module")
        let file_path = module_id.split(':').next().unwrap_or(module_id);
        prompt.push_str(&format!(
            "### {}\nEntity features: {}\n\n",
            file_path,
            features.join(", ")
        ));
    }
    prompt
}

/// Collect file-level features for domain discovery.
fn collect_file_features(graph: &RPGraph) -> HashMap<String, Vec<String>> {
    graph
        .entities
        .iter()
        .filter(|(_, e)| {
            e.kind == rpg_core::graph::EntityKind::Module && !e.semantic_features.is_empty()
        })
        .map(|(_, e)| {
            let path = rpg_core::graph::normalize_path(&e.file);
            (path, e.semantic_features.clone())
        })
        .collect()
}

/// Format the domain discovery prompt with file features.
fn format_discovery_prompt(file_features: &HashMap<String, Vec<String>>) -> String {
    let mut prompt = String::from(
        "Analyze this repository and identify its main functional areas.\n\nFile features:\n",
    );

    // Sort for deterministic output
    let mut files: Vec<(&String, &Vec<String>)> = file_features.iter().collect();
    files.sort_by_key(|(path, _)| *path);

    for (path, features) in &files {
        prompt.push_str(&format!("  {} — {}\n", path, features.join(", ")));
    }

    prompt
}

/// Format hierarchy assignment prompt for a file cluster.
fn format_hierarchy_prompt(
    files: &[String],
    areas: &[String],
    file_features: &HashMap<String, Vec<String>>,
) -> String {
    let mut prompt = String::from("Assign each file to a 3-level hierarchy path.\n\n");

    prompt.push_str("Functional areas:\n");
    for area in areas {
        prompt.push_str(&format!("  {}\n", area));
    }

    prompt.push_str("\nFiles to assign:\n");
    for file in files {
        let features = file_features
            .get(file)
            .map(|f| f.join(", "))
            .unwrap_or_default();
        prompt.push_str(&format!("  {} — {}\n", file, features));
    }

    prompt
}

// ---------------------------------------------------------------------------
// Retry wrapper
// ---------------------------------------------------------------------------

/// Call the LLM with retry logic.
fn call_with_retry(
    provider: &dyn LlmProvider,
    system: &str,
    user: &str,
    max_retries: usize,
) -> Result<crate::provider::LlmResponse, ProviderError> {
    let mut last_err = None;

    for attempt in 0..=max_retries {
        match provider.complete(system, user) {
            Ok(response) => return Ok(response),
            Err(e) => {
                tracing::warn!("LLM call attempt {} failed: {}", attempt + 1, e);
                last_err = Some(e);
                if attempt < max_retries {
                    // Brief pause before retry
                    std::thread::sleep(std::time::Duration::from_secs(2u64.pow(attempt as u32)));
                }
            }
        }
    }

    Err(last_err.unwrap())
}

/// Extract a project name from the root path.
fn project_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

// ---------------------------------------------------------------------------
// Pipeline errors
// ---------------------------------------------------------------------------

/// Errors from the autonomous lifting pipeline.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("setup error: {0}")]
    Setup(String),
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
}
