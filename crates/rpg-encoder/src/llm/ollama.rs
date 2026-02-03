//! Ollama auto-detection, model pulling, and availability checks.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;

/// Response from Ollama's /api/tags endpoint.
#[derive(Deserialize)]
pub(crate) struct OllamaTagsResponse {
    pub models: Vec<OllamaModelInfo>,
}

#[derive(Deserialize)]
pub(crate) struct OllamaModelInfo {
    pub name: String,
}

/// Probe Ollama at `base_url`. Returns installed model names on success, None if unreachable.
pub(crate) async fn detect_ollama(base_url: &str) -> Option<Vec<String>> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;

    let resp = client.get(&url).send().await.ok()?;
    let tags: OllamaTagsResponse = resp.json().await.ok()?;
    Some(tags.models.into_iter().map(|m| m.name).collect())
}

/// Pull a model from Ollama, streaming progress to stderr.
pub(crate) async fn pull_ollama_model(base_url: &str, model: &str) -> Result<()> {
    let url = format!("{}/api/pull", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "name": model, "stream": true }))
        .send()
        .await
        .context("failed to start Ollama model pull")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Ollama pull failed with status {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let mut stream = resp.bytes_stream();
    let mut last_status = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("error reading pull stream")?;
        // Each line is a JSON object with status and optional progress
        for line in bytes.split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) {
                let status = v["status"].as_str().unwrap_or("").to_string();
                if status != last_status {
                    eprintln!("  pull: {}", status);
                    last_status = status;
                }
            }
        }
    }

    eprintln!("  Model '{}' pulled successfully.", model);
    Ok(())
}

/// Check if a requested model is in the installed list.
/// Handles Ollama's naming: "qwen2.5-coder:7b" matches "qwen2.5-coder:7b",
/// and "qwen2.5-coder:7b" matches "qwen2.5-coder:7b-instruct-..." etc.
pub(crate) fn model_is_available(installed: &[String], requested: &str) -> bool {
    let requested_lower = requested.to_lowercase();
    installed.iter().any(|m| {
        m.to_lowercase() == requested_lower
            || m.to_lowercase()
                .starts_with(&format!("{}:", requested_lower))
            || requested_lower.starts_with(&format!("{}:", m.to_lowercase()))
    })
}
