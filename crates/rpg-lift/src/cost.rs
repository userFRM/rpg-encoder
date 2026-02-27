//! Cost estimation for autonomous lifting.

use crate::provider::LlmProvider;
use rpg_core::graph::RPGraph;

/// Pre-computed cost estimate for a lifting run.
#[derive(Debug, Clone)]
pub struct CostEstimate {
    /// Total entities that need LLM lifting (after auto-lift).
    pub entities_to_lift: usize,
    /// Entities handled by auto-lift (no LLM cost).
    pub entities_auto_lifted: usize,
    /// Estimated number of LLM batches.
    pub estimated_batches: usize,
    /// Estimated input tokens across all phases.
    pub estimated_input_tokens: u64,
    /// Estimated output tokens across all phases.
    pub estimated_output_tokens: u64,
    /// Estimated total cost in USD.
    pub estimated_cost_usd: f64,
    /// Model name.
    pub model: String,
}

impl std::fmt::Display for CostEstimate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Cost Estimate:")?;
        writeln!(
            f,
            "  Auto-lifted (free): {} entities",
            self.entities_auto_lifted
        )?;
        writeln!(
            f,
            "  LLM lifting needed: {} entities ({} batches)",
            self.entities_to_lift, self.estimated_batches
        )?;
        writeln!(
            f,
            "  Estimated tokens: ~{} input, ~{} output",
            self.estimated_input_tokens, self.estimated_output_tokens
        )?;
        writeln!(f, "  Model: {}", self.model)?;
        write!(f, "  Estimated cost: ${:.4}", self.estimated_cost_usd)
    }
}

/// Running cost tracker during lifting.
#[derive(Debug, Default)]
pub struct CostTracker {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    input_rate: f64,
    output_rate: f64,
}

impl CostTracker {
    pub fn new(provider: &dyn LlmProvider) -> Self {
        Self {
            total_input_tokens: 0,
            total_output_tokens: 0,
            input_rate: provider.cost_per_mtok_input(),
            output_rate: provider.cost_per_mtok_output(),
        }
    }

    /// Record token usage from a response.
    pub fn record(&mut self, input_tokens: Option<u64>, output_tokens: Option<u64>) {
        if let Some(t) = input_tokens {
            self.total_input_tokens += t;
        }
        if let Some(t) = output_tokens {
            self.total_output_tokens += t;
        }
    }

    /// Current total cost in USD.
    pub fn total_cost_usd(&self) -> f64 {
        (self.total_input_tokens as f64 / 1_000_000.0) * self.input_rate
            + (self.total_output_tokens as f64 / 1_000_000.0) * self.output_rate
    }
}

/// Estimate lifting cost without making API calls.
///
/// Scans the graph to count entities needing LLM lifting (excluding auto-liftable),
/// estimates token counts using the 4-chars-per-token heuristic, and computes cost.
pub fn estimate_cost(
    graph: &RPGraph,
    provider: &dyn LlmProvider,
    project_root: &std::path::Path,
) -> CostEstimate {
    let scope = rpg_encoder::lift::resolve_scope(graph, "*");

    // Try auto-lift on raw entities to estimate how many need LLM
    let raw_entities =
        rpg_encoder::lift::collect_raw_entities(graph, &scope, project_root).unwrap_or_default();

    let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap_or_default();
    let active_paradigms: Vec<String> = graph.metadata.paradigms.clone();
    let engine = rpg_encoder::lift::AutoLiftEngine::new(&paradigm_defs, &active_paradigms);

    let mut auto_lifted = 0usize;
    let mut llm_needed = Vec::new();

    for raw in &raw_entities {
        match engine.try_lift_with_confidence(raw) {
            Some((_, rpg_encoder::lift::LiftConfidence::Accept)) => {
                auto_lifted += 1;
            }
            _ => {
                llm_needed.push(raw);
            }
        }
    }

    // Estimate tokens for entity lifting batches
    let batches = rpg_encoder::lift::build_token_aware_batches(&raw_entities, 25, 8000);
    let llm_batches = if raw_entities.is_empty() {
        0
    } else {
        // Scale batches by ratio of LLM-needed entities
        let ratio = llm_needed.len() as f64 / raw_entities.len() as f64;
        #[allow(clippy::cast_sign_loss)]
        {
            (batches.len() as f64 * ratio).ceil() as usize
        }
    };

    // Estimate input tokens: system prompt (~500 tokens) + entity source per batch
    let system_tokens = 500u64;
    let avg_source_tokens: u64 = if llm_needed.is_empty() {
        0
    } else {
        llm_needed
            .iter()
            .map(|r| (r.source_text.len() as u64) / 4)
            .sum::<u64>()
            / llm_needed.len() as u64
    };
    let tokens_per_batch = system_tokens + avg_source_tokens * 25;
    let lift_input_tokens = tokens_per_batch * llm_batches as u64;

    // Output: ~30 tokens per entity (name + features)
    let lift_output_tokens = llm_needed.len() as u64 * 30;

    // Synthesis + hierarchy phases: ~20% overhead on top of lifting
    let synthesis_tokens = lift_input_tokens / 5;
    let synthesis_output = lift_output_tokens / 5;

    let total_input = lift_input_tokens + synthesis_tokens;
    let total_output = lift_output_tokens + synthesis_output;

    let cost = (total_input as f64 / 1_000_000.0) * provider.cost_per_mtok_input()
        + (total_output as f64 / 1_000_000.0) * provider.cost_per_mtok_output();

    CostEstimate {
        entities_to_lift: llm_needed.len(),
        entities_auto_lifted: auto_lifted,
        estimated_batches: llm_batches,
        estimated_input_tokens: total_input,
        estimated_output_tokens: total_output,
        estimated_cost_usd: cost,
        model: provider.model_name().to_string(),
    }
}
