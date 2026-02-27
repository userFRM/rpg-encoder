//! LLM provider abstraction for autonomous lifting.
//!
//! Supports Anthropic (Claude Haiku) and OpenAI-compatible (GPT-4o-mini) APIs.
//! Uses blocking HTTP via `ureq` — the CLI has no async runtime.

use serde_json::Value;

/// Errors from LLM provider calls.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("response parse error: {0}")]
    Parse(String),
    #[error("empty response from LLM")]
    EmptyResponse,
}

/// A completed LLM response.
pub struct LlmResponse {
    /// The text content of the response.
    pub text: String,
    /// Input tokens used (from API response, if reported).
    pub input_tokens: Option<u64>,
    /// Output tokens used (from API response, if reported).
    pub output_tokens: Option<u64>,
}

/// Abstraction over LLM API providers.
pub trait LlmProvider: Send {
    /// Send a completion request with system and user messages.
    fn complete(&self, system: &str, user: &str) -> Result<LlmResponse, ProviderError>;

    /// The model name (for display/logging).
    fn model_name(&self) -> &str;

    /// Cost per million input tokens (USD).
    fn cost_per_mtok_input(&self) -> f64;

    /// Cost per million output tokens (USD).
    fn cost_per_mtok_output(&self) -> f64;
}

// ---------------------------------------------------------------------------
// Anthropic Messages API
// ---------------------------------------------------------------------------

/// Anthropic provider using the Messages API.
#[cfg(feature = "anthropic")]
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    agent: ureq::Agent,
}

#[cfg(feature = "anthropic")]
impl AnthropicProvider {
    /// Default model: Claude Haiku 4.5 — fast and cheap.
    pub const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";
    const API_URL: &str = "https://api.anthropic.com/v1/messages";

    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| Self::DEFAULT_MODEL.to_string()),
            agent: ureq::Agent::new_with_config(
                ureq::config::Config::builder()
                    .timeout_global(Some(std::time::Duration::from_secs(120)))
                    .build(),
            ),
        }
    }
}

#[cfg(feature = "anthropic")]
impl LlmProvider for AnthropicProvider {
    fn complete(&self, system: &str, user: &str) -> Result<LlmResponse, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [
                {"role": "user", "content": user}
            ]
        });

        let mut response = self
            .agent
            .post(Self::API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .send_json(&body)
            .map_err(|e| ProviderError::Http(e.to_string()))?;

        let json: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        // Check for API error
        if let Some(err) = json.get("error") {
            return Err(ProviderError::Api {
                status: 400,
                message: err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error")
                    .to_string(),
            });
        }

        // Extract text from content blocks
        let text = json
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find_map(|block| block.get("text").and_then(|t| t.as_str()))
            })
            .ok_or(ProviderError::EmptyResponse)?
            .to_string();

        // Extract usage
        let input_tokens = json
            .get("usage")
            .and_then(|u| u.get("input_tokens"))
            .and_then(|t| t.as_u64());
        let output_tokens = json
            .get("usage")
            .and_then(|u| u.get("output_tokens"))
            .and_then(|t| t.as_u64());

        Ok(LlmResponse {
            text,
            input_tokens,
            output_tokens,
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_mtok_input(&self) -> f64 {
        // Haiku 4.5: $0.80/MTok input
        if self.model.contains("haiku") {
            0.80
        } else if self.model.contains("sonnet") {
            3.00
        } else {
            1.00 // conservative default
        }
    }

    fn cost_per_mtok_output(&self) -> f64 {
        // Haiku 4.5: $4.00/MTok output
        if self.model.contains("haiku") {
            4.00
        } else if self.model.contains("sonnet") {
            15.00
        } else {
            5.00
        }
    }
}

// ---------------------------------------------------------------------------
// OpenAI Chat Completions API
// ---------------------------------------------------------------------------

/// OpenAI-compatible provider (works with OpenAI, Azure, local proxies).
#[cfg(feature = "openai")]
pub struct OpenAiProvider {
    api_key: String,
    model: String,
    base_url: String,
    agent: ureq::Agent,
}

#[cfg(feature = "openai")]
impl OpenAiProvider {
    /// Default model: GPT-4o-mini — fast and cheap.
    pub const DEFAULT_MODEL: &str = "gpt-4o-mini";
    const DEFAULT_BASE_URL: &str = "https://api.openai.com";

    pub fn new(api_key: String, model: Option<String>, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| Self::DEFAULT_MODEL.to_string()),
            base_url: base_url.unwrap_or_else(|| Self::DEFAULT_BASE_URL.to_string()),
            agent: ureq::Agent::new_with_config(
                ureq::config::Config::builder()
                    .timeout_global(Some(std::time::Duration::from_secs(120)))
                    .build(),
            ),
        }
    }
}

#[cfg(feature = "openai")]
impl LlmProvider for OpenAiProvider {
    fn complete(&self, system: &str, user: &str) -> Result<LlmResponse, ProviderError> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        });

        let mut response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .send_json(&body)
            .map_err(|e| ProviderError::Http(e.to_string()))?;

        let json: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        // Check for API error
        if let Some(err) = json.get("error") {
            return Err(ProviderError::Api {
                status: 400,
                message: err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error")
                    .to_string(),
            });
        }

        // Extract text from choices
        let text = json
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|msg| msg.get("content"))
            .and_then(|c| c.as_str())
            .ok_or(ProviderError::EmptyResponse)?
            .to_string();

        // Extract usage
        let input_tokens = json
            .get("usage")
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64());
        let output_tokens = json
            .get("usage")
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|t| t.as_u64());

        Ok(LlmResponse {
            text,
            input_tokens,
            output_tokens,
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_mtok_input(&self) -> f64 {
        // GPT-4o-mini: $0.15/MTok input
        if self.model.contains("4o-mini") {
            0.15
        } else if self.model.contains("4o") {
            2.50
        } else {
            0.50
        }
    }

    fn cost_per_mtok_output(&self) -> f64 {
        // GPT-4o-mini: $0.60/MTok output
        if self.model.contains("4o-mini") {
            0.60
        } else if self.model.contains("4o") {
            10.00
        } else {
            1.50
        }
    }
}

/// Create a provider from CLI arguments.
pub fn create_provider(
    provider_name: &str,
    api_key: &str,
    model: Option<&str>,
    base_url: Option<&str>,
) -> Result<Box<dyn LlmProvider>, ProviderError> {
    match provider_name {
        #[cfg(feature = "anthropic")]
        "anthropic" => Ok(Box::new(AnthropicProvider::new(
            api_key.to_string(),
            model.map(String::from),
        ))),
        #[cfg(feature = "openai")]
        "openai" => Ok(Box::new(OpenAiProvider::new(
            api_key.to_string(),
            model.map(String::from),
            base_url.map(String::from),
        ))),
        other => Err(ProviderError::Http(format!(
            "unknown provider: '{}'. Available: {}",
            other,
            available_providers().join(", ")
        ))),
    }
}

/// List compiled-in provider names.
pub fn available_providers() -> Vec<&'static str> {
    vec![
        #[cfg(feature = "anthropic")]
        "anthropic",
        #[cfg(feature = "openai")]
        "openai",
    ]
}
