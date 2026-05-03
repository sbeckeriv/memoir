use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::config::{LlmProvider, LlmSettings};

#[derive(Debug, Clone, Serialize)]
pub struct AskResponse {
    pub answer: String,
    pub sources: Vec<String>,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

// --- Anthropic messages API ---

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

// --- LM Studio native model list/load API ---

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    key: String,
    #[serde(default)]
    loaded_instances: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct LoadRequest<'a> {
    model: &'a str,
}

// --------------------------------------------

#[derive(Clone)]
pub struct LlmClient {
    client: Client,
    pub provider: LlmProvider,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

impl LlmClient {
    pub fn new(settings: &LlmSettings) -> Self {
        Self {
            client: Client::new(),
            provider: settings.provider,
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            api_key: settings.api_key.clone(),
        }
    }

    /// For LM Studio: checks if the model is loaded and triggers loading if not.
    /// Skips silently for openai/anthropic providers.
    pub async fn ensure_loaded(&self) {
        if self.provider != LlmProvider::LmStudio {
            return;
        }

        let list_url = format!("{}/api/v1/models", self.base_url);
        let resp = match self.client.get(&list_url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "could not reach LM Studio to check model state");
                return;
            }
        };

        let models: ModelsResponse = match resp.json().await {
            Ok(m) => m,
            Err(e) => {
                warn!(error = %e, "failed to parse LM Studio model list");
                return;
            }
        };

        let entry = models.models.iter().find(|m| m.key == self.model);
        match entry {
            Some(m) if !m.loaded_instances.is_empty() => {
                info!(model = %self.model, "model already loaded in LM Studio");
            }
            Some(_) => {
                info!(model = %self.model, "model found but not loaded — requesting load");
                self.load_model().await;
            }
            None => {
                warn!(
                    model = %self.model,
                    available = ?models.models.iter().map(|m| &m.key).collect::<Vec<_>>(),
                    "model not found in LM Studio — check config.toml [llm] model value"
                );
            }
        }
    }

    async fn load_model(&self) {
        let url = format!("{}/api/v1/models/load", self.base_url);
        debug!(url = %url, model = %self.model, "sending model load request");
        match self
            .client
            .post(&url)
            .json(&LoadRequest { model: &self.model })
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                info!(model = %self.model, "model load request accepted by LM Studio");
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                warn!(status = %status, body = %body, "LM Studio load request failed");
            }
            Err(e) => warn!(error = %e, "failed to send model load request"),
        }
    }

    pub async fn generate(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> anyhow::Result<String> {
        match self.provider {
            LlmProvider::Anthropic => self.generate_anthropic(prompt, system_prompt).await,
            _ => self.generate_openai(prompt, system_prompt).await,
        }
    }

    async fn generate_openai(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> anyhow::Result<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        debug!(url = %url, model = %self.model, "sending OpenAI-format LLM request");

        let mut messages = Vec::new();
        if let Some(sp) = system_prompt {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: sp.to_string(),
            });
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        });

        let mut req = self.client.post(&url).json(&ChatRequest {
            model: self.model.clone(),
            messages,
            stream: false,
        });
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let http_resp = req.send().await?;

        let status = http_resp.status();
        let body = http_resp.text().await?;
        debug!(status = %status, body_len = body.len(), "LLM response received");

        if !status.is_success() {
            warn!(status = %status, body = %body, "LLM request failed");
            anyhow::bail!("LLM returned {status}: {body}");
        }

        let parsed: ChatResponse = serde_json::from_str(&body).map_err(|e| {
            warn!(error = %e, body = %body, "failed to parse LLM response");
            e
        })?;

        Ok(parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }

    async fn generate_anthropic(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> anyhow::Result<String> {
        let url = format!("{}/v1/messages", self.base_url);
        debug!(url = %url, model = %self.model, "sending Anthropic messages request");

        let api_key = self.api_key.as_deref().unwrap_or("");
        let http_resp = self
            .client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&AnthropicRequest {
                model: &self.model,
                max_tokens: 1024,
                messages: vec![ChatMessage {
                    role: "user".to_string(),
                    content: prompt.to_string(),
                }],
                system: system_prompt,
            })
            .send()
            .await?;

        let status = http_resp.status();
        let body = http_resp.text().await?;
        debug!(status = %status, body_len = body.len(), "Anthropic response received");

        if !status.is_success() {
            warn!(status = %status, body = %body, "Anthropic request failed");
            anyhow::bail!("Anthropic returned {status}: {body}");
        }

        let parsed: AnthropicResponse = serde_json::from_str(&body).map_err(|e| {
            warn!(error = %e, body = %body, "failed to parse Anthropic response");
            e
        })?;

        Ok(parsed
            .content
            .into_iter()
            .find(|c| c.content_type == "text")
            .and_then(|c| c.text)
            .unwrap_or_default())
    }
}
