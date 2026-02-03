//! LLM provider configuration and API request/response types.

use super::ollama::{detect_ollama, model_is_available, pull_ollama_model};
use anyhow::{Context, Result};
use rpg_core::config::LlmConfig;
use serde::{Deserialize, Serialize};

/// LLM provider configuration.
#[derive(Debug, Clone)]
pub enum LlmProvider {
    Anthropic {
        api_key: String,
        model: String,
    },
    OpenAI {
        api_key: String,
        model: String,
    },
    /// Any OpenAI-compatible API with Bearer token auth (Moonshot, Together, OpenRouter, etc.)
    OpenAICompatible {
        api_key: String,
        base_url: String,
        model: String,
    },
    Ollama {
        base_url: String,
        model: String,
    },
    Local {
        base_url: String,
        model: String,
    },
}

impl LlmProvider {
    /// Create from environment variables. Prefers ANTHROPIC_API_KEY, then OPENAI_API_KEY, then MOONSHOT_API_KEY.
    /// Does NOT detect Ollama (use `from_env_and_config_async` for that).
    pub fn from_env() -> Result<Self> {
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            Ok(Self::Anthropic {
                api_key: key,
                model: std::env::var("RPG_MODEL")
                    .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
            })
        } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            Ok(Self::OpenAI {
                api_key: key,
                model: std::env::var("RPG_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
            })
        } else if let Ok(key) = std::env::var("MOONSHOT_API_KEY") {
            Ok(Self::OpenAICompatible {
                api_key: key,
                base_url: "https://api.moonshot.ai/v1".to_string(),
                model: std::env::var("RPG_MODEL").unwrap_or_else(|_| "kimi-k2.5".to_string()),
            })
        } else {
            anyhow::bail!(
                "No LLM API key found. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, or MOONSHOT_API_KEY."
            )
        }
    }

    /// Full async provider resolution with Ollama auto-detection.
    ///
    /// Priority chain:
    /// 1. `config.provider` forced -> use that provider
    /// 2. `ANTHROPIC_API_KEY` -> Anthropic
    /// 3. `OPENAI_API_KEY` -> OpenAI
    /// 4. `MOONSHOT_API_KEY` -> Moonshot (Kimi)
    /// 5. Ollama on `config.local_url` -> auto-detected
    /// 6. `RPG_LOCAL_URL` env var -> any OpenAI-compatible server
    /// 7. Error with helpful message
    pub async fn from_env_and_config_async(config: &LlmConfig) -> Result<Self> {
        // If provider is forced, use it directly
        if let Some(ref forced) = config.provider {
            return Self::from_forced_provider(forced, config);
        }

        // 1. Anthropic
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            return Ok(Self::Anthropic {
                api_key: key,
                model: std::env::var("RPG_MODEL")
                    .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
            });
        }

        // 2. OpenAI
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            return Ok(Self::OpenAI {
                api_key: key,
                model: std::env::var("RPG_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
            });
        }

        // 3. Moonshot (Kimi) — OpenAI-compatible
        if let Ok(key) = std::env::var("MOONSHOT_API_KEY") {
            return Ok(Self::OpenAICompatible {
                api_key: key,
                base_url: "https://api.moonshot.ai/v1".to_string(),
                model: std::env::var("RPG_MODEL").unwrap_or_else(|_| "kimi-k2.5".to_string()),
            });
        }

        // 4. Ollama auto-detect
        let base_url = &config.local_url;
        if let Some(installed) = detect_ollama(base_url).await {
            let model = &config.local_model;
            if model_is_available(&installed, model) {
                eprintln!("  Ollama detected with model: {}", model);
                return Ok(Self::Ollama {
                    base_url: base_url.clone(),
                    model: model.clone(),
                });
            }
            // Model not installed — try auto-pull
            if config.auto_pull {
                eprintln!(
                    "  Ollama detected but model '{}' not found. Pulling...",
                    model
                );
                pull_ollama_model(base_url, model).await?;
                return Ok(Self::Ollama {
                    base_url: base_url.clone(),
                    model: model.clone(),
                });
            }
            anyhow::bail!(
                "Ollama is running but model '{}' is not installed.\n\
                 Run: ollama pull {}\n\
                 Or set auto_pull = true in .rpg/config.toml",
                model,
                model
            );
        }

        // 5. RPG_LOCAL_URL env var -> any OpenAI-compatible server
        if let Ok(url) = std::env::var("RPG_LOCAL_URL") {
            let model = config.local_model.clone();
            eprintln!("  Using local LLM server at: {} (model: {})", url, model);
            return Ok(Self::Local {
                base_url: url,
                model,
            });
        }

        // 6. Nothing found
        anyhow::bail!(
            "No LLM provider available. Options:\n\
             - Set ANTHROPIC_API_KEY for best quality\n\
             - Set OPENAI_API_KEY for OpenAI\n\
             - Set MOONSHOT_API_KEY for Moonshot (Kimi)\n\
             - Install Ollama (https://ollama.com) for free local inference\n\
             - Set RPG_LOCAL_URL for any OpenAI-compatible server (LM Studio, vLLM, etc.)"
        )
    }

    /// Resolve a forced provider name to a provider instance.
    fn from_forced_provider(provider: &str, config: &LlmConfig) -> Result<Self> {
        match provider {
            "anthropic" => {
                let key = std::env::var("ANTHROPIC_API_KEY")
                    .context("RPG_PROVIDER=anthropic but ANTHROPIC_API_KEY not set")?;
                Ok(Self::Anthropic {
                    api_key: key,
                    model: std::env::var("RPG_MODEL")
                        .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
                })
            }
            "openai" => {
                let key = std::env::var("OPENAI_API_KEY")
                    .context("RPG_PROVIDER=openai but OPENAI_API_KEY not set")?;
                Ok(Self::OpenAI {
                    api_key: key,
                    model: std::env::var("RPG_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
                })
            }
            "ollama" => Ok(Self::Ollama {
                base_url: config.local_url.clone(),
                model: config.local_model.clone(),
            }),
            "moonshot" => {
                let key = std::env::var("MOONSHOT_API_KEY")
                    .context("RPG_PROVIDER=moonshot but MOONSHOT_API_KEY not set")?;
                Ok(Self::OpenAICompatible {
                    api_key: key,
                    base_url: "https://api.moonshot.ai/v1".to_string(),
                    model: std::env::var("RPG_MODEL").unwrap_or_else(|_| "kimi-k2.5".to_string()),
                })
            }
            "openai-compatible" => {
                let key = std::env::var("RPG_API_KEY")
                    .context("RPG_PROVIDER=openai-compatible but RPG_API_KEY not set")?;
                let url =
                    std::env::var("RPG_BASE_URL").unwrap_or_else(|_| config.local_url.clone());
                let model =
                    std::env::var("RPG_MODEL").unwrap_or_else(|_| config.local_model.clone());
                Ok(Self::OpenAICompatible {
                    api_key: key,
                    base_url: url,
                    model,
                })
            }
            "local" => {
                let url =
                    std::env::var("RPG_LOCAL_URL").unwrap_or_else(|_| config.local_url.clone());
                Ok(Self::Local {
                    base_url: url,
                    model: config.local_model.clone(),
                })
            }
            other => anyhow::bail!(
                "Unknown provider '{}'. Valid: anthropic, openai, moonshot, openai-compatible, ollama, local",
                other
            ),
        }
    }

    /// Human-readable provider name.
    pub fn provider_name(&self) -> &str {
        match self {
            Self::Anthropic { .. } => "Anthropic",
            Self::OpenAI { .. } => "OpenAI",
            Self::OpenAICompatible { .. } => "OpenAI-Compatible",
            Self::Ollama { .. } => "Ollama (local)",
            Self::Local { .. } => "Local",
        }
    }

    /// Model name in use.
    pub fn model_name(&self) -> &str {
        match self {
            Self::Anthropic { model, .. }
            | Self::OpenAI { model, .. }
            | Self::OpenAICompatible { model, .. }
            | Self::Ollama { model, .. }
            | Self::Local { model, .. } => model,
        }
    }
}

// ---------------------------------------------------------------------------
// API Request / Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub(crate) struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<Message>,
    pub system: String,
}

#[derive(Serialize)]
pub(crate) struct OpenAIRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
pub(crate) struct OpenAIMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
pub(crate) struct AnthropicResponse {
    pub content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
pub(crate) struct AnthropicContent {
    pub text: String,
}

#[derive(Deserialize)]
pub(crate) struct OpenAIResponse {
    pub choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAIChoice {
    pub message: OpenAIChoiceMessage,
}

#[derive(Deserialize)]
pub(crate) struct OpenAIChoiceMessage {
    pub content: String,
}
