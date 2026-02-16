//! Representation quality controls, ablations, and external validation utilities.

use chrono::Utc;
use rpg_core::graph::{Entity, RPGraph};
use rpg_core::storage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QualityBaseline {
    created_at: String,
    revision: String,
    entity_features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
struct AblationMetrics {
    queries: usize,
    acc_at_k: f64,
    file_acc_at_k: f64,
    mrr: f64,
}

#[derive(Debug, Clone, Serialize)]
struct AblationReport {
    created_at: String,
    k: usize,
    max_queries: usize,
    full_rpg: AblationMetrics,
    snippets_only: AblationMetrics,
    no_semantic_features: AblationMetrics,
}

fn ontology_catalog() -> [(&'static str, &'static [&'static str]); 12] {
    [
        ("auth", &["validate credential", "manage session"]),
        ("token", &["validate token", "issue token"]),
        ("login", &["authenticate user", "create session"]),
        ("config", &["load config", "validate config"]),
        ("cache", &["read cache", "write cache"]),
        ("db", &["query data", "persist data"]),
        ("store", &["read state", "write state"]),
        ("route", &["route request", "validate route params"]),
        ("api", &["serve endpoint", "validate request"]),
        ("parse", &["parse input", "normalize input"]),
        ("render", &["render view", "compose component"]),
        ("test", &["verify behavior", "assert outcome"]),
    ]
}

fn is_vague_verb(verb: &str) -> bool {
    matches!(
        verb,
        "handle" | "process" | "deal" | "manage" | "do" | "make" | "run" | "work"
    )
}

fn phrase_quality_score(phrase: &str) -> f64 {
    let terms: Vec<&str> = phrase
        .split_whitespace()
        .filter(|s| !s.trim().is_empty())
        .collect();
    if terms.is_empty() {
        return 0.0;
    }
    if terms.len() == 1 {
        return 0.2;
    }

    let verb = terms[0].to_lowercase();
    let mut score = if is_vague_verb(&verb) { 0.35 } else { 0.85 };

    if terms.len() > 7 {
        score *= 0.75;
    }
    if phrase.contains('_') || phrase.contains("::") {
        score *= 0.8;
    }
    score
}

fn source_reliability(entity: &Entity) -> f64 {
    match entity.feature_source.as_deref() {
        Some("auto") => 0.9,
        Some("llm") => 0.78,
        Some("synthesized") => 0.72,
        Some("planned") => 0.66,
        Some("ontology_seeded") => 0.58,
        Some(_) => 0.65,
        None => 0.5,
    }
}

fn entity_confidence(entity: &Entity) -> f64 {
    if entity.semantic_features.is_empty() {
        return 0.0;
    }
    let mean_phrase = entity
        .semantic_features
        .iter()
        .map(|f| phrase_quality_score(f))
        .sum::<f64>()
        / entity.semantic_features.len() as f64;

    0.4 * source_reliability(entity) + 0.6 * mean_phrase
}

pub fn seed_ontology_features(
    project_root: &Path,
    graph: &mut RPGraph,
    max_per_entity: usize,
) -> Result<String, String> {
    let mut entities_seeded = 0usize;
    let mut features_added = 0usize;
    let per_entity_cap = max_per_entity.max(1);

    for entity in graph.entities.values_mut() {
        let conf = entity_confidence(entity);
        if !entity.semantic_features.is_empty() && conf >= 0.55 {
            continue;
        }

        let haystack = format!(
            "{} {} {}",
            entity.name.to_lowercase(),
            entity.file.to_string_lossy().to_lowercase(),
            entity.hierarchy_path.to_lowercase()
        );

        let mut added_here = 0usize;
        for (keyword, seeds) in ontology_catalog() {
            if !haystack.contains(keyword) {
                continue;
            }
            for seed in seeds {
                if added_here >= per_entity_cap {
                    break;
                }
                if !entity.semantic_features.iter().any(|f| f == seed) {
                    entity.semantic_features.push((*seed).to_string());
                    added_here += 1;
                    features_added += 1;
                }
            }
            if added_here >= per_entity_cap {
                break;
            }
        }

        if added_here > 0 {
            entities_seeded += 1;
            if entity.feature_source.is_none() || entity.feature_source.as_deref() == Some("llm") {
                entity.feature_source = Some("ontology_seeded".to_string());
            }
        }
    }

    graph.refresh_metadata();
    storage::save(project_root, graph).map_err(|e| format!("failed to save graph: {}", e))?;

    Ok(format!(
        "## ONTOLOGY SEEDING COMPLETE\n\n\
        entities_seeded: {}\n\
        features_added: {}\n\
        max_per_entity: {}\n",
        entities_seeded, features_added, per_entity_cap
    ))
}

pub fn assess_representation_quality(
    project_root: &Path,
    graph: &RPGraph,
    drift_threshold: f64,
    write_baseline: bool,
    max_examples: usize,
) -> Result<String, String> {
    let baseline_path = storage::quality_baseline_file(project_root);
    let baseline: Option<QualityBaseline> = if baseline_path.exists() {
        std::fs::read_to_string(&baseline_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
    } else {
        None
    };

    let mut confidence_values = Vec::new();
    let mut low_conf: Vec<(String, f64)> = Vec::new();
    let mut drifted: Vec<(String, f64)> = Vec::new();
    let mut source_counts: BTreeMap<String, usize> = BTreeMap::new();

    for (entity_id, entity) in &graph.entities {
        if entity.kind == rpg_core::graph::EntityKind::Module {
            continue;
        }
        let conf = entity_confidence(entity);
        confidence_values.push(conf);
        if conf < 0.5 {
            low_conf.push((entity_id.clone(), conf));
        }
        let source = entity
            .feature_source
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        *source_counts.entry(source).or_insert(0) += 1;

        if let Some(base) = baseline.as_ref()
            && let Some(old) = base.entity_features.get(entity_id)
        {
            let drift = rpg_encoder::evolution::compute_drift(old, &entity.semantic_features);
            if drift >= drift_threshold {
                drifted.push((entity_id.clone(), drift));
            }
        }
    }

    let mean_conf = if confidence_values.is_empty() {
        0.0
    } else {
        confidence_values.iter().sum::<f64>() / confidence_values.len() as f64
    };

    low_conf.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    drifted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    if write_baseline {
        let baseline_data = QualityBaseline {
            created_at: Utc::now().to_rfc3339(),
            revision: graph.updated_at.to_rfc3339(),
            entity_features: graph
                .entities
                .iter()
                .map(|(id, e)| (id.clone(), e.semantic_features.clone()))
                .collect(),
        };
        let raw = serde_json::to_string_pretty(&baseline_data)
            .map_err(|e| format!("failed to serialize baseline: {}", e))?;
        if let Some(parent) = baseline_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }
        std::fs::write(&baseline_path, raw)
            .map_err(|e| format!("failed to save {}: {}", baseline_path.display(), e))?;
    }

    let mut out = format!(
        "## REPRESENTATION QUALITY\n\n\
        mean_confidence: {:.3}\n\
        low_confidence_entities: {}\n\
        drift_threshold: {:.2}\n\
        drift_alerts: {}\n\
        baseline_file: {}\n",
        mean_conf,
        low_conf.len(),
        drift_threshold,
        drifted.len(),
        baseline_path.display()
    );

    out.push_str("\n## FEATURE SOURCES\n");
    for (source, count) in &source_counts {
        out.push_str(&format!("- {}: {}\n", source, count));
    }

    if !low_conf.is_empty() {
        out.push_str("\n## LOW CONFIDENCE EXAMPLES\n");
        for (id, conf) in low_conf.iter().take(max_examples.max(1)) {
            out.push_str(&format!("- {} ({:.3})\n", id, conf));
        }
    }

    if !drifted.is_empty() {
        out.push_str("\n## HIGH DRIFT EXAMPLES\n");
        for (id, drift) in drifted.iter().take(max_examples.max(1)) {
            out.push_str(&format!("- {} (drift {:.2})\n", id, drift));
        }
    }

    Ok(out)
}

fn eval_queries(
    graph: &RPGraph,
    variant_graph: &RPGraph,
    queries: &[(String, String, String)],
    k: usize,
    mode: rpg_nav::search::SearchMode,
) -> AblationMetrics {
    let mut hits = 0usize;
    let mut file_hits = 0usize;
    let mut reciprocal_sum = 0.0f64;

    for (query, expected_id, expected_file) in queries {
        let params = rpg_nav::search::SearchParams {
            query,
            mode,
            scope: None,
            limit: k,
            line_nums: None,
            file_pattern: None,
            entity_type_filter: None,
            embedding_scores: None,
            diff_context: None,
        };
        let results = rpg_nav::search::search_with_params(variant_graph, &params);
        for (rank, result) in results.iter().enumerate() {
            if result.entity_id == *expected_id {
                hits += 1;
                reciprocal_sum += 1.0 / (rank as f64 + 1.0);
                break;
            }
        }
        if results.iter().any(|r| r.file == *expected_file) {
            file_hits += 1;
        }
    }

    let n = queries.len().max(1) as f64;
    let _ = graph; // Keep signature aligned for future topology-aware variants.
    AblationMetrics {
        queries: queries.len(),
        acc_at_k: hits as f64 / n,
        file_acc_at_k: file_hits as f64 / n,
        mrr: reciprocal_sum / n,
    }
}

pub fn run_representation_ablation(
    project_root: &Path,
    graph: &RPGraph,
    max_queries: usize,
    k: usize,
) -> Result<String, String> {
    let mut queries: Vec<(String, String, String)> = Vec::new();
    for (id, entity) in &graph.entities {
        if entity.semantic_features.is_empty() || entity.kind == rpg_core::graph::EntityKind::Module
        {
            continue;
        }
        if let Some(feature) = entity.semantic_features.first() {
            queries.push((
                feature.clone(),
                id.clone(),
                entity.file.to_string_lossy().to_string(),
            ));
        }
        if queries.len() >= max_queries.max(1) {
            break;
        }
    }

    if queries.is_empty() {
        return Err("no semantic features available for ablation".into());
    }

    let full = eval_queries(
        graph,
        graph,
        &queries,
        k.max(1),
        rpg_nav::search::SearchMode::Features,
    );
    let snippets = eval_queries(
        graph,
        graph,
        &queries,
        k.max(1),
        rpg_nav::search::SearchMode::Snippets,
    );

    let mut no_sem_graph = graph.clone();
    for entity in no_sem_graph.entities.values_mut() {
        entity.semantic_features.clear();
    }
    let no_semantics = eval_queries(
        graph,
        &no_sem_graph,
        &queries,
        k.max(1),
        rpg_nav::search::SearchMode::Auto,
    );

    let report = AblationReport {
        created_at: Utc::now().to_rfc3339(),
        k: k.max(1),
        max_queries: max_queries.max(1),
        full_rpg: full.clone(),
        snippets_only: snippets.clone(),
        no_semantic_features: no_semantics.clone(),
    };
    let report_path = storage::ablation_report_file(project_root);
    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }
    let raw = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("failed to serialize ablation report: {}", e))?;
    std::fs::write(&report_path, raw)
        .map_err(|e| format!("failed to save {}: {}", report_path.display(), e))?;

    Ok(format!(
        "## REPRESENTATION ABLATION\n\n\
        report_file: {}\n\
        queries: {}\n\
        k: {}\n\n\
        full_rpg: acc@{}={:.3}, file_acc@{}={:.3}, mrr={:.3}\n\
        snippets_only: acc@{}={:.3}, file_acc@{}={:.3}, mrr={:.3}\n\
        no_semantic_features: acc@{}={:.3}, file_acc@{}={:.3}, mrr={:.3}\n",
        report_path.display(),
        queries.len(),
        k.max(1),
        k.max(1),
        full.acc_at_k,
        k.max(1),
        full.file_acc_at_k,
        full.mrr,
        k.max(1),
        snippets.acc_at_k,
        k.max(1),
        snippets.file_acc_at_k,
        snippets.mrr,
        k.max(1),
        no_semantics.acc_at_k,
        k.max(1),
        no_semantics.file_acc_at_k,
        no_semantics.mrr
    ))
}

#[derive(Debug, Serialize)]
struct BlindedTask {
    task_id: String,
    query: String,
    scope: String,
}

pub fn export_external_validation_bundle(
    project_root: &Path,
    graph: &RPGraph,
    sample_size: usize,
    k: usize,
) -> Result<String, String> {
    let root = storage::external_validation_dir(project_root);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("failed to create {}: {}", root.display(), e))?;
    let run_id = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let bundle_dir = root.join(&run_id);
    let public_dir = bundle_dir.join("public");
    let private_dir = bundle_dir.join("private");
    std::fs::create_dir_all(&public_dir)
        .map_err(|e| format!("failed to create {}: {}", public_dir.display(), e))?;
    std::fs::create_dir_all(&private_dir)
        .map_err(|e| format!("failed to create {}: {}", private_dir.display(), e))?;

    let mut tasks = Vec::new();
    let mut answer_key: BTreeMap<String, String> = BTreeMap::new();
    for (idx, (id, entity)) in graph
        .entities
        .iter()
        .filter(|(_, e)| !e.semantic_features.is_empty())
        .take(sample_size.max(1))
        .enumerate()
    {
        let task_id = format!("task-{:04}", idx + 1);
        let query = entity
            .semantic_features
            .first()
            .cloned()
            .unwrap_or_default();
        let scope = entity
            .hierarchy_path
            .split('/')
            .next()
            .unwrap_or("global")
            .to_string();
        tasks.push(BlindedTask {
            task_id: task_id.clone(),
            query,
            scope,
        });
        answer_key.insert(task_id, id.clone());
    }

    let tasks_path = public_dir.join("tasks.blinded.json");
    let answers_path = private_dir.join("answer_key.private.json");
    let template_path = public_dir.join("result_template.json");
    let readme_path = public_dir.join("README.md");

    std::fs::write(
        &tasks_path,
        serde_json::to_string_pretty(&tasks)
            .map_err(|e| format!("failed to serialize tasks: {}", e))?,
    )
    .map_err(|e| format!("failed to write {}: {}", tasks_path.display(), e))?;
    std::fs::write(
        &answers_path,
        serde_json::to_string_pretty(&answer_key)
            .map_err(|e| format!("failed to serialize answer key: {}", e))?,
    )
    .map_err(|e| format!("failed to write {}: {}", answers_path.display(), e))?;
    std::fs::write(
        &template_path,
        format!(
            "{{\n  \"run_id\": \"{}\",\n  \"model\": \"\",\n  \"backbone\": \"\",\n  \"k\": {},\n  \"predictions\": [\n    {{\"task_id\": \"task-0001\", \"entity_ids\": [\"...\"]}}\n  ]\n}}\n",
            run_id,
            k.max(1)
        ),
    )
    .map_err(|e| format!("failed to write {}: {}", template_path.display(), e))?;

    let readme = format!(
        "# External Validation Bundle ({run_id})\n\n\
        This pack enables third-party, blinded reproduction.\n\n\
        Public files are under `public/`; private scoring files are under `private/`.\n\n\
        1. Share `public/tasks.blinded.json` and your repository snapshot with evaluators.\n\
        2. Keep `private/answer_key.private.json` hidden from evaluators.\n\
        3. Evaluators submit predictions using `public/result_template.json`.\n\
        4. Score predictions offline against the private answer key (Acc@k and MRR).\n"
    );
    std::fs::write(&readme_path, readme)
        .map_err(|e| format!("failed to write {}: {}", readme_path.display(), e))?;

    Ok(format!(
        "## EXTERNAL VALIDATION BUNDLE EXPORTED\n\n\
        bundle: {}\n\
        public_dir: {}\n\
        private_dir: {}\n\
        blinded_tasks: {}\n\
        k_template: {}\n\
        files:\n\
        - {}\n\
        - {}\n\
        - {}\n\
        - {}\n",
        bundle_dir.display(),
        public_dir.display(),
        private_dir.display(),
        tasks.len(),
        k.max(1),
        tasks_path.display(),
        answers_path.display(),
        template_path.display(),
        readme_path.display(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{Entity, EntityDeps, EntityKind};
    use tempfile::TempDir;

    fn tiny_graph() -> RPGraph {
        let mut graph = RPGraph::new("rust");
        graph.entities.insert(
            "src/auth.rs:login".to_string(),
            Entity {
                id: "src/auth.rs:login".to_string(),
                kind: EntityKind::Function,
                name: "login".to_string(),
                file: "src/auth.rs".into(),
                line_start: 1,
                line_end: 10,
                parent_class: None,
                semantic_features: vec!["authenticate user".to_string()],
                feature_source: Some("llm".to_string()),
                hierarchy_path: "Auth/session/login".to_string(),
                deps: EntityDeps::default(),
                signature: None,
            },
        );
        graph.entities.insert(
            "src/config.rs:load".to_string(),
            Entity {
                id: "src/config.rs:load".to_string(),
                kind: EntityKind::Function,
                name: "load_config".to_string(),
                file: "src/config.rs".into(),
                line_start: 1,
                line_end: 10,
                parent_class: None,
                semantic_features: Vec::new(),
                feature_source: None,
                hierarchy_path: "Core/config/load".to_string(),
                deps: EntityDeps::default(),
                signature: None,
            },
        );
        graph.refresh_metadata();
        graph
    }

    #[test]
    fn test_seed_ontology_features_adds_features() {
        let tmp = TempDir::new().unwrap();
        let mut graph = tiny_graph();
        let out = seed_ontology_features(tmp.path(), &mut graph, 2).unwrap();
        assert!(out.contains("entities_seeded"));
        let config_entity = graph.entities.get("src/config.rs:load").unwrap();
        assert!(!config_entity.semantic_features.is_empty());
    }

    #[test]
    fn test_assess_representation_quality_creates_baseline() {
        let tmp = TempDir::new().unwrap();
        let graph = tiny_graph();
        let out = assess_representation_quality(tmp.path(), &graph, 0.5, true, 5).unwrap();
        assert!(out.contains("mean_confidence"));
        assert!(storage::quality_baseline_file(tmp.path()).exists());
    }
}
