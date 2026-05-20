use std::future::Future;

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
    pub max_tokens: u32,
    extra_params: Option<serde_json::Value>,
}

impl LlmClient {
    pub fn new(settings: &LlmSettings) -> Self {
        let extra_params =
            settings
                .extra_params
                .as_deref()
                .and_then(|s| match serde_json::from_str(s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        warn!("llm.extra_params is not valid JSON, ignoring: {e}");
                        None
                    }
                });
        Self {
            client: Client::new(),
            provider: settings.provider,
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            api_key: settings.api_key.clone(),
            max_tokens: settings.max_tokens,
            extra_params,
        }
    }

    fn apply_extra(&self, body: &mut serde_json::Value) {
        if let (Some(extra), Some(obj)) = (&self.extra_params, body.as_object_mut())
            && let Some(extra_obj) = extra.as_object()
        {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
    }

    /// For LM Studio: checks if the model is loaded and triggers loading if not.
    /// Skips silently for openai/anthropic/disabled providers.
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
            LlmProvider::Disabled => anyhow::bail!("LLM is disabled (provider = \"none\")"),
            LlmProvider::Anthropic => self.generate_anthropic(prompt, system_prompt).await,
            _ => self.generate_openai(prompt, system_prompt).await,
        }
    }

    pub async fn generate_conversation(
        &self,
        history: &[(String, String)],
        system_prompt: Option<&str>,
    ) -> anyhow::Result<String> {
        match self.provider {
            LlmProvider::Disabled => anyhow::bail!("LLM is disabled (provider = \"none\")"),
            LlmProvider::Anthropic => self.conversation_anthropic(history, system_prompt).await,
            _ => self.conversation_openai(history, system_prompt).await,
        }
    }

    /// Multi-turn chat with tool calling. The LLM may call `search_history` or
    /// `get_page`; `tool_fn(name, args)` dispatches each call and returns
    /// (xml_content, source_urls).
    pub async fn generate_with_tools<F, Fut>(
        &self,
        history: &[(String, String)],
        system: &str,
        tool_fn: F,
    ) -> anyhow::Result<(String, Vec<String>)>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: Future<Output = anyhow::Result<(String, Vec<String>)>>,
    {
        match self.provider {
            LlmProvider::Disabled => anyhow::bail!("LLM is disabled (provider = \"none\")"),
            LlmProvider::Anthropic => self.chat_anthropic_tools(history, system, tool_fn).await,
            _ => self.chat_openai_tools(history, system, tool_fn).await,
        }
    }

    async fn chat_openai_tools<F, Fut>(
        &self,
        history: &[(String, String)],
        system: &str,
        tool_fn: F,
    ) -> anyhow::Result<(String, Vec<String>)>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: Future<Output = anyhow::Result<(String, Vec<String>)>>,
    {
        use serde_json::json;
        let url = format!("{}/v1/chat/completions", self.base_url);

        let tools = json!([
            {
                "type": "function",
                "function": {
                    "name": "search_history",
                    "description": "Search the user's personal browsing history index for pages they have previously visited. Use this when the user asks about something they may have read or researched.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Search terms" }
                        },
                        "required": ["query"]
                    }
                }
            },
            {
                "type": "function",
                "function": {
                    "name": "get_page",
                    "description": "Retrieve the full indexed body of a specific page by URL. Use this after search_history identifies a relevant page and you need its complete content to answer the question in depth.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "url": { "type": "string", "description": "The exact URL of the page to retrieve" }
                        },
                        "required": ["url"]
                    }
                }
            }
        ]);

        let mut messages: Vec<serde_json::Value> = Vec::new();
        messages.push(json!({ "role": "system", "content": system }));
        for (role, content) in history {
            messages.push(json!({ "role": role, "content": content }));
        }

        let mut all_sources: Vec<String> = Vec::new();

        for _ in 0..8 {
            let mut body = json!({
                "model": self.model,
                "max_tokens": self.max_tokens,
                "messages": messages,
                "tools": tools,
                "stream": false,
            });
            self.apply_extra(&mut body);
            let mut req = self.client.post(&url).json(&body);
            if let Some(key) = &self.api_key {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
            let resp = req.send().await?;
            let status = resp.status();
            let body = resp.text().await?;
            if !status.is_success() {
                anyhow::bail!("LLM returned {status}: {body}");
            }
            let parsed: serde_json::Value = serde_json::from_str(&body)?;
            let choice = &parsed["choices"][0];
            let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop");
            let message = choice["message"].clone();

            if finish_reason == "tool_calls" {
                messages.push(message.clone());
                if let Some(calls) = message["tool_calls"].as_array() {
                    for tc in calls {
                        let call_id = tc["id"].as_str().unwrap_or("").to_string();
                        let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                        let args: serde_json::Value = serde_json::from_str(
                            tc["function"]["arguments"].as_str().unwrap_or("{}"),
                        )
                        .unwrap_or_default();
                        let (xml, urls) = tool_fn(name, args).await?;
                        all_sources.extend(urls);
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": xml,
                        }));
                    }
                }
            } else {
                let text = message["content"].as_str().unwrap_or("").to_string();
                return Ok((text, all_sources));
            }
        }
        anyhow::bail!("tool call loop exceeded maximum iterations")
    }

    async fn chat_anthropic_tools<F, Fut>(
        &self,
        history: &[(String, String)],
        system: &str,
        tool_fn: F,
    ) -> anyhow::Result<(String, Vec<String>)>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: Future<Output = anyhow::Result<(String, Vec<String>)>>,
    {
        use serde_json::json;
        let url = format!("{}/v1/messages", self.base_url);
        let api_key = self.api_key.as_deref().unwrap_or("");

        let tools = json!([
            {
                "name": "search_history",
                "description": "Search the user's personal browsing history index for pages they have previously visited.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search terms" }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "get_page",
                "description": "Retrieve the full indexed body of a specific page by URL. Use this after search_history identifies a relevant page and you need its complete content to answer the question in depth.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "The exact URL of the page to retrieve" }
                    },
                    "required": ["url"]
                }
            }
        ]);

        let mut messages: Vec<serde_json::Value> = history
            .iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect();

        let mut all_sources: Vec<String> = Vec::new();

        for _ in 0..8 {
            let mut body = json!({
                "model": self.model,
                "max_tokens": self.max_tokens,
                "system": system,
                "messages": messages,
                "tools": tools,
            });
            self.apply_extra(&mut body);
            let resp = self
                .client
                .post(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await?;
            let status = resp.status();
            let body = resp.text().await?;
            if !status.is_success() {
                anyhow::bail!("Anthropic returned {status}: {body}");
            }
            let parsed: serde_json::Value = serde_json::from_str(&body)?;
            let stop_reason = parsed["stop_reason"].as_str().unwrap_or("end_turn");
            let content = parsed["content"].as_array().cloned().unwrap_or_default();

            if stop_reason == "tool_use" {
                messages.push(json!({ "role": "assistant", "content": content }));
                let mut results: Vec<serde_json::Value> = Vec::new();
                for block in &content {
                    if block["type"].as_str() == Some("tool_use") {
                        let id = block["id"].as_str().unwrap_or("").to_string();
                        let name = block["name"].as_str().unwrap_or("").to_string();
                        let args = block["input"].clone();
                        let (xml, urls) = tool_fn(name, args).await?;
                        all_sources.extend(urls);
                        results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": xml,
                        }));
                    }
                }
                messages.push(json!({ "role": "user", "content": results }));
            } else {
                let text = content
                    .iter()
                    .find(|b| b["type"].as_str() == Some("text"))
                    .and_then(|b| b["text"].as_str())
                    .unwrap_or("")
                    .to_string();
                return Ok((text, all_sources));
            }
        }
        anyhow::bail!("tool call loop exceeded maximum iterations")
    }

    async fn conversation_openai(
        &self,
        history: &[(String, String)],
        system_prompt: Option<&str>,
    ) -> anyhow::Result<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut messages = Vec::new();
        if let Some(sp) = system_prompt {
            messages.push(ChatMessage {
                role: "system".into(),
                content: sp.into(),
            });
        }
        for (role, content) in history {
            messages.push(ChatMessage {
                role: role.clone(),
                content: content.clone(),
            });
        }
        let mut req = self.client.post(&url).json(&ChatRequest {
            model: self.model.clone(),
            messages,
            stream: false,
        });
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let resp = req.send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("LLM returned {status}: {body}");
        }
        let parsed: ChatResponse = serde_json::from_str(&body)?;
        Ok(parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }

    async fn conversation_anthropic(
        &self,
        history: &[(String, String)],
        system_prompt: Option<&str>,
    ) -> anyhow::Result<String> {
        let url = format!("{}/v1/messages", self.base_url);
        let messages: Vec<ChatMessage> = history
            .iter()
            .map(|(r, c)| ChatMessage {
                role: r.clone(),
                content: c.clone(),
            })
            .collect();
        let api_key = self.api_key.as_deref().unwrap_or("");
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&AnthropicRequest {
                model: &self.model,
                max_tokens: self.max_tokens,
                messages,
                system: system_prompt,
            })
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("Anthropic returned {status}: {body}");
        }
        let parsed: AnthropicResponse = serde_json::from_str(&body)?;
        Ok(parsed
            .content
            .into_iter()
            .find(|c| c.content_type == "text")
            .and_then(|c| c.text)
            .unwrap_or_default())
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
                max_tokens: self.max_tokens,
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
