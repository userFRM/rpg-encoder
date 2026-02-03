//! LLM client for semantic extraction via Anthropic, OpenAI, Moonshot, Ollama, or local APIs.

mod ollama;
mod providers;

pub use providers::LlmProvider;

use anyhow::{Context, Result};
use providers::{
    AnthropicRequest, AnthropicResponse, Message, OpenAIMessage, OpenAIRequest, OpenAIResponse,
};
use rpg_core::config::LlmConfig;
use std::collections::HashMap;

/// A simple LLM client for making completion requests.
pub struct LlmClient {
    provider: LlmProvider,
    http: reqwest::Client,
    max_tokens: u32,
}

impl LlmClient {
    pub fn new(provider: LlmProvider) -> Self {
        Self {
            provider,
            http: reqwest::Client::new(),
            max_tokens: 4096,
        }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(LlmProvider::from_env()?))
    }

    /// Create from environment variables with config-driven max_tokens (sync, cloud-only).
    pub fn from_env_with_config(config: &LlmConfig) -> Result<Self> {
        let mut client = Self::new(LlmProvider::from_env()?);
        client.max_tokens = config.max_tokens;
        Ok(client)
    }

    /// Create with full provider resolution including Ollama auto-detection.
    pub async fn from_env_with_config_async(config: &LlmConfig) -> Result<Self> {
        let provider = LlmProvider::from_env_and_config_async(config).await?;
        Ok(Self {
            provider,
            http: reqwest::Client::new(),
            max_tokens: config.max_tokens,
        })
    }

    /// Human-readable provider name.
    pub fn provider_name(&self) -> &str {
        self.provider.provider_name()
    }

    /// Model name in use.
    pub fn model_name(&self) -> &str {
        self.provider.model_name()
    }

    /// Send a completion request and return the response text.
    pub async fn complete(&self, system: &str, user_prompt: &str) -> Result<String> {
        self.complete_with_max_tokens(system, user_prompt, self.max_tokens)
            .await
    }

    /// Send a completion request with explicit max_tokens.
    pub async fn complete_with_max_tokens(
        &self,
        system: &str,
        user_prompt: &str,
        max_tokens: u32,
    ) -> Result<String> {
        match &self.provider {
            LlmProvider::Anthropic { api_key, model } => {
                let req = AnthropicRequest {
                    model: model.clone(),
                    max_tokens,
                    system: system.to_string(),
                    messages: vec![Message {
                        role: "user".to_string(),
                        content: user_prompt.to_string(),
                    }],
                };

                let resp = self
                    .http
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&req)
                    .send()
                    .await
                    .context("failed to call Anthropic API")?;

                let body = resp
                    .json::<AnthropicResponse>()
                    .await
                    .context("failed to parse Anthropic response")?;

                body.content
                    .first()
                    .map(|c| c.text.clone())
                    .ok_or_else(|| anyhow::anyhow!("empty response from Anthropic"))
            }
            LlmProvider::OpenAI { api_key, model } => {
                let req = OpenAIRequest {
                    model: model.clone(),
                    max_tokens,
                    messages: vec![
                        OpenAIMessage {
                            role: "system".to_string(),
                            content: system.to_string(),
                        },
                        OpenAIMessage {
                            role: "user".to_string(),
                            content: user_prompt.to_string(),
                        },
                    ],
                    temperature: None,
                };

                let resp = self
                    .http
                    .post("https://api.openai.com/v1/chat/completions")
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("content-type", "application/json")
                    .json(&req)
                    .send()
                    .await
                    .context("failed to call OpenAI API")?;

                let body = resp
                    .json::<OpenAIResponse>()
                    .await
                    .context("failed to parse OpenAI response")?;

                body.choices
                    .first()
                    .map(|c| c.message.content.clone())
                    .ok_or_else(|| anyhow::anyhow!("empty response from OpenAI"))
            }
            LlmProvider::OpenAICompatible {
                api_key,
                base_url,
                model,
            } => {
                let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

                let req = OpenAIRequest {
                    model: model.clone(),
                    max_tokens,
                    messages: vec![
                        OpenAIMessage {
                            role: "system".to_string(),
                            content: system.to_string(),
                        },
                        OpenAIMessage {
                            role: "user".to_string(),
                            content: user_prompt.to_string(),
                        },
                    ],
                    temperature: None,
                };

                let resp = self
                    .http
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("content-type", "application/json")
                    .json(&req)
                    .send()
                    .await
                    .with_context(|| format!("failed to call OpenAI-compatible API at {}", url))?;

                let status = resp.status();
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    anyhow::bail!("OpenAI-compatible API returned {}: {}", status, text);
                }

                let body = resp
                    .json::<OpenAIResponse>()
                    .await
                    .context("failed to parse OpenAI-compatible response")?;

                body.choices
                    .first()
                    .map(|c| c.message.content.clone())
                    .ok_or_else(|| anyhow::anyhow!("empty response from OpenAI-compatible API"))
            }
            LlmProvider::Ollama { base_url, model } | LlmProvider::Local { base_url, model } => {
                let provider_label = self.provider.provider_name();
                let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

                // Append /no_think to disable thinking mode on models that support it
                // (e.g. qwen3) — avoids wasting tokens on <think> blocks.
                let user_content = format!("{} /no_think", user_prompt);

                let req = OpenAIRequest {
                    model: model.clone(),
                    max_tokens,
                    messages: vec![
                        OpenAIMessage {
                            role: "system".to_string(),
                            content: system.to_string(),
                        },
                        OpenAIMessage {
                            role: "user".to_string(),
                            content: user_content,
                        },
                    ],
                    // Deterministic output for structured JSON extraction
                    temperature: Some(0.0),
                };

                let resp = self
                    .http
                    .post(&url)
                    .header("content-type", "application/json")
                    .json(&req)
                    .send()
                    .await
                    .with_context(|| format!("failed to call {} API at {}", provider_label, url))?;

                let status = resp.status();
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    anyhow::bail!("{} returned {}: {}", provider_label, status, text);
                }

                let body = resp
                    .json::<OpenAIResponse>()
                    .await
                    .with_context(|| format!("failed to parse {} response", provider_label))?;

                body.choices
                    .first()
                    .map(|c| c.message.content.clone())
                    .ok_or_else(|| anyhow::anyhow!("empty response from {}", provider_label))
            }
        }
    }

    /// Send a completion request with retry logic (exponential backoff).
    pub async fn complete_with_retry(
        &self,
        system: &str,
        user_prompt: &str,
        config: &LlmConfig,
    ) -> Result<String> {
        let mut last_err = None;
        let max_attempts = config.retry_attempts.max(1) as usize;

        for attempt in 0..max_attempts {
            match self.complete(system, user_prompt).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    let delay_idx = attempt.min(config.retry_delays_ms.len().saturating_sub(1));
                    let delay_ms = config
                        .retry_delays_ms
                        .get(delay_idx)
                        .copied()
                        .unwrap_or(4000);

                    if attempt < max_attempts - 1 {
                        eprintln!(
                            "  LLM request failed (attempt {}/{}): {}. Retrying in {}ms...",
                            attempt + 1,
                            max_attempts,
                            e,
                            delay_ms
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM request failed after all retries")))
    }

    /// Complete an LLM request, parse JSON, retry once on parse failure with a stricter prompt.
    pub async fn complete_and_parse_json<T: serde::de::DeserializeOwned>(
        &self,
        system: &str,
        user_prompt: &str,
    ) -> Result<T> {
        // First attempt
        let response = self.complete(system, user_prompt).await?;
        match Self::parse_json_response::<T>(&response) {
            Ok(parsed) => return Ok(parsed),
            Err(first_err) => {
                eprintln!("  JSON parse failed, retrying: {}", first_err);
            }
        }

        // Retry with format correction (paper: "minimal format correction without changing semantic constraints")
        let strict_prompt = format!(
            "{}\n\nPrevious response had invalid JSON. Output the SAME semantic analysis but with correct JSON formatting. No explanation, no markdown.",
            user_prompt
        );
        let response = self.complete(system, &strict_prompt).await?;
        Self::parse_json_response::<T>(&response)
    }

    /// Parse a JSON response from the LLM, extracting from <solution> tags if present.
    pub fn parse_json_response<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
        // Strip <think>...</think> blocks (qwen3, deepseek emit these)
        let text = Self::strip_think_blocks(text);
        let text = text.as_str();

        // Try to extract from <solution>...</solution> tags first
        let json_str = if let Some(start) = text.find("<solution>") {
            let after = &text[start + "<solution>".len()..];
            if let Some(end) = after.find("</solution>") {
                after[..end].trim()
            } else {
                text.trim()
            }
        } else {
            // Try to find JSON directly
            let trimmed = text.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                trimmed
            } else {
                // Try to find JSON block in markdown code fence
                if let Some(start) = text.find("```json") {
                    let after = &text[start + "```json".len()..];
                    if let Some(end) = after.find("```") {
                        after[..end].trim()
                    } else {
                        trimmed
                    }
                } else if let Some(start) = text.find("```") {
                    let after = &text[start + "```".len()..];
                    if let Some(end) = after.find("```") {
                        after[..end].trim()
                    } else {
                        trimmed
                    }
                } else {
                    trimmed
                }
            }
        };

        serde_json::from_str(json_str).context("failed to parse LLM JSON response")
    }

    /// Extract semantic features: function_name -> [features]
    pub fn parse_feature_response(text: &str) -> Result<HashMap<String, Vec<String>>> {
        Self::parse_json_response(text)
    }

    /// Strip `<think>...</think>` blocks that some models (qwen3, deepseek) emit.
    pub fn strip_think_blocks(text: &str) -> String {
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
}
