/*!
 * LLM Client Module
 *
 * Supports Ollama by default and prefers OpenAI when configured in
 * `$HOME/.localcoder/settings.json`.
 */

use crate::types::{AgentResponse, OllamaChatResponse, ToolUseCall};
use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen3.5:4b";

#[derive(Debug, Clone, Deserialize)]
struct LLMSettings {
    #[serde(default)]
    ollama: Option<OllamaSettings>,
    #[serde(default)]
    openai: Option<OpenAISettings>,
}

#[derive(Debug, Clone, Deserialize)]
struct OllamaSettings {
    url: String,
    model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAISettings {
    base_url: String,
    api_key: String,
    model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Ollama,
    OpenAI,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagModel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAIToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolCall {
    id: String,
    function: OpenAIFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedSettings {
    #[serde(default)]
    ollama: Option<PersistedOllamaSettings>,
    #[serde(default)]
    openai: Option<OpenAISettings>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedOllamaSettings {
    url: String,
    model: String,
}

/// LLM client used by the REPL and agent loop.
pub struct LLMClient {
    client: Client,
    provider: Provider,
    base_url: String,
    api_key: Option<String>,
    model: String,
    max_tokens: u32,
}

impl LLMClient {
    /// Create a client from `$HOME/.localcoder/settings.json`.
    pub fn new() -> Result<Self> {
        let settings = Self::load_settings()?;
        Ok(Self::from_settings(settings))
    }

    /// Ensure the settings file exists before command handling starts.
    pub fn ensure_settings_file() -> Result<PathBuf> {
        let home = env::var_os("HOME").map(PathBuf::from);
        Self::ensure_settings_file_with(home.as_deref())
    }

    pub fn home_settings_path() -> Result<PathBuf> {
        let home = env::var_os("HOME").context("$HOME is not set")?;
        Ok(PathBuf::from(home).join(".localcoder/settings.json"))
    }

    fn from_settings(settings: LLMSettings) -> Self {
        if let Some(openai) = settings.openai {
            return Self {
                client: Client::new(),
                provider: Provider::OpenAI,
                base_url: openai.base_url.trim_end_matches('/').to_string(),
                api_key: Some(openai.api_key),
                model: openai.model,
                max_tokens: 4096,
            };
        }

        let ollama = settings
            .ollama
            .expect("validated settings must include ollama when openai is absent");
        Self {
            client: Client::new(),
            provider: Provider::Ollama,
            base_url: ollama.url.trim_end_matches('/').to_string(),
            api_key: None,
            model: ollama.model,
            max_tokens: 4096,
        }
    }

    fn load_settings() -> Result<LLMSettings> {
        let path = Self::resolve_settings_path()?;
        Self::load_settings_from_path(&path)
    }

    fn resolve_settings_path() -> Result<PathBuf> {
        let home = env::var_os("HOME").map(PathBuf::from);
        Self::resolve_settings_path_with(home.as_deref())
    }

    fn resolve_settings_path_with(home: Option<&Path>) -> Result<PathBuf> {
        if let Some(home) = home {
            let home_path = home.join(".localcoder/settings.json");
            if home_path.exists() {
                return Ok(home_path);
            }
        }

        Err(anyhow!("missing $HOME/.localcoder/settings.json"))
    }

    fn ensure_settings_file_with(home: Option<&Path>) -> Result<PathBuf> {
        if let Ok(path) = Self::resolve_settings_path_with(home) {
            return Ok(path);
        }

        let home = home.context("$HOME is not set")?;
        let path = home.join(".localcoder/settings.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create settings directory: {}", parent.display())
            })?;
        }

        fs::write(&path, Self::default_settings_json())
            .with_context(|| format!("failed to write settings file: {}", path.display()))?;

        Ok(path)
    }

    fn load_settings_from_path(path: &Path) -> Result<LLMSettings> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read settings file: {}", path.display()))?;
        let settings: LLMSettings = serde_json::from_str(&raw)
            .with_context(|| format!("invalid settings JSON: {}", path.display()))?;

        if let Some(openai) = settings.openai.as_ref() {
            validate_non_empty("settings.openai.base_url", &openai.base_url)?;
            validate_non_empty("settings.openai.api_key", &openai.api_key)?;
            validate_non_empty("settings.openai.model", &openai.model)?;
            return Ok(settings);
        }

        let ollama = settings
            .ollama
            .as_ref()
            .ok_or_else(|| anyhow!("settings must include either openai or ollama"))?;
        validate_non_empty("settings.ollama.url", &ollama.url)?;
        validate_non_empty("settings.ollama.model", &ollama.model)?;

        Ok(settings)
    }

    fn default_settings_json() -> String {
        serde_json::to_string_pretty(&json!({
            "ollama": {
                "url": DEFAULT_OLLAMA_URL,
                "model": DEFAULT_OLLAMA_MODEL
            }
        }))
        .expect("default settings json must serialize")
    }

    pub fn provider_name(&self) -> &'static str {
        match self.provider {
            Provider::Ollama => "Ollama",
            Provider::OpenAI => "OpenAI",
        }
    }

    /// Send a tool-aware chat request to the active provider.
    pub async fn call_with_tools(
        &self,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<AgentResponse> {
        match self.provider {
            Provider::Ollama => self.call_with_tools_ollama(messages, tools).await,
            Provider::OpenAI => self.call_with_tools_openai(messages, tools).await,
        }
    }

    async fn call_with_tools_ollama(
        &self,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<AgentResponse> {
        let body = if tools.is_empty() {
            json!({
                "model": self.model,
                "messages": messages,
                "stream": false,
                "options": {
                    "num_predict": self.max_tokens
                }
            })
        } else {
            json!({
                "model": self.model,
                "messages": messages,
                "stream": false,
                "tools": tools,
                "options": {
                    "num_predict": self.max_tokens
                }
            })
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Ollama request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned error {}: {}", status, error_text);
        }

        let response: OllamaChatResponse = response
            .json()
            .await
            .context("failed to parse Ollama response")?;

        let text = response.message.content.unwrap_or_default();
        if !text.is_empty() {
            print!("{}", text);
        }

        let tool_uses = response
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tool_call| ToolUseCall {
                id: None,
                name: tool_call.function.name,
                arguments: tool_call.function.arguments,
            })
            .collect::<Vec<_>>();

        Ok(build_agent_response(text, tool_uses))
    }

    async fn call_with_tools_openai(
        &self,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<AgentResponse> {
        let openai_messages = messages
            .iter()
            .enumerate()
            .map(|(index, message)| map_message_for_openai(index, message))
            .collect::<Vec<_>>();

        let body = if tools.is_empty() {
            json!({
                "model": self.model,
                "messages": openai_messages,
                "stream": false,
                "max_tokens": self.max_tokens
            })
        } else {
            json!({
                "model": self.model,
                "messages": openai_messages,
                "stream": false,
                "tools": tools,
                "tool_choice": "auto",
                "max_tokens": self.max_tokens
            })
        };

        let response = self
            .authorized_request(
                self.client
                    .post(format!("{}/chat/completions", self.base_url))
                    .header("content-type", "application/json"),
            )?
            .json(&body)
            .send()
            .await
            .context("OpenAI request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI returned error {}: {}", status, error_text);
        }

        let response: OpenAIChatResponse = response
            .json()
            .await
            .context("failed to parse OpenAI response")?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("OpenAI returned no choices"))?;

        let text = choice.message.content.unwrap_or_default();
        if !text.is_empty() {
            print!("{}", text);
        }

        let tool_uses = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tool_call| {
                let arguments = serde_json::from_str(&tool_call.function.arguments)
                    .unwrap_or_else(|_| json!({ "_raw": tool_call.function.arguments }));
                ToolUseCall {
                    id: Some(tool_call.id),
                    name: tool_call.function.name,
                    arguments,
                }
            })
            .collect::<Vec<_>>();

        Ok(build_agent_response(text, tool_uses))
    }

    pub async fn summarize_messages(&self, messages: &[Value]) -> Result<String> {
        let prompt = format!(
            "以下是一段对话历史，请生成简洁摘要，保留：\n1. 已完成的任务和结果\n2. 重要文件修改\n3. 用户的关键偏好和决定\n4. 未完成的任务\n\n对话历史：\n{}",
            crate::compact::summarize_for_prompt(messages)
        );
        self.complete_prompt(&prompt, 1024).await
    }

    pub async fn complete_prompt(&self, prompt: &str, max_tokens: u32) -> Result<String> {
        match self.provider {
            Provider::Ollama => self.complete_prompt_ollama(prompt, max_tokens).await,
            Provider::OpenAI => self.complete_prompt_openai(prompt, max_tokens).await,
        }
    }

    async fn complete_prompt_ollama(&self, prompt: &str, max_tokens: u32) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "stream": false,
            "options": {
                "num_predict": max_tokens
            }
        });

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Ollama prompt request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned error {}: {}", status, error_text);
        }

        let response: OllamaChatResponse = response
            .json()
            .await
            .context("failed to parse Ollama prompt response")?;

        Ok(response.message.content.unwrap_or_default())
    }

    async fn complete_prompt_openai(&self, prompt: &str, max_tokens: u32) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "stream": false,
            "max_tokens": max_tokens
        });

        let response = self
            .authorized_request(
                self.client
                    .post(format!("{}/chat/completions", self.base_url))
                    .header("content-type", "application/json"),
            )?
            .json(&body)
            .send()
            .await
            .context("OpenAI prompt request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI returned error {}: {}", status, error_text);
        }

        let response: OpenAIChatResponse = response
            .json()
            .await
            .context("failed to parse OpenAI prompt response")?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("OpenAI returned no choices"))?;
        Ok(choice.message.content.unwrap_or_default())
    }

    fn authorized_request(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        match self.provider {
            Provider::Ollama => Ok(request),
            Provider::OpenAI => {
                let api_key = self
                    .api_key
                    .as_deref()
                    .ok_or_else(|| anyhow!("missing OpenAI API key"))?;
                Ok(request.bearer_auth(api_key))
            }
        }
    }

    /// Set model.
    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    /// Set max tokens.
    pub fn set_max_tokens(&mut self, max_tokens: u32) {
        self.max_tokens = max_tokens;
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        match self.provider {
            Provider::Ollama => self.list_models_ollama().await,
            Provider::OpenAI => self.list_models_openai().await,
        }
    }

    async fn list_models_ollama(&self) -> Result<Vec<String>> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .context("failed to fetch Ollama model tags")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned error {}: {}", status, error_text);
        }

        let response: OllamaTagsResponse = response
            .json()
            .await
            .context("failed to parse Ollama tag response")?;

        let mut models = response
            .models
            .into_iter()
            .map(|model| model.name)
            .collect::<Vec<_>>();
        models.sort();
        models.dedup();
        Ok(models)
    }

    async fn list_models_openai(&self) -> Result<Vec<String>> {
        let response = self
            .authorized_request(self.client.get(format!("{}/models", self.base_url)))?
            .send()
            .await
            .context("failed to fetch OpenAI models")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI returned error {}: {}", status, error_text);
        }

        let response: OpenAIModelsResponse = response
            .json()
            .await
            .context("failed to parse OpenAI models response")?;

        let mut models = response
            .data
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        models.sort();
        models.dedup();
        Ok(models)
    }

    pub fn persist_model_to_home(&self, model: &str) -> Result<PathBuf> {
        let home_path = Self::home_settings_path()?;
        Self::persist_model_to_path(&home_path, self.provider, &self.base_url, model)?;
        Ok(home_path)
    }

    fn persist_model_to_path(
        path: &Path,
        provider: Provider,
        base_url: &str,
        model: &str,
    ) -> Result<()> {
        let model = model.trim();
        if model.is_empty() {
            return Err(anyhow!("model must not be empty"));
        }

        let settings = if path.exists() {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read settings file: {}", path.display()))?;
            let mut settings: PersistedSettings = serde_json::from_str(&raw)
                .with_context(|| format!("invalid settings JSON: {}", path.display()))?;

            match provider {
                Provider::Ollama => {
                    let ollama = settings.ollama.get_or_insert(PersistedOllamaSettings {
                        url: base_url.to_string(),
                        model: model.to_string(),
                    });
                    if ollama.url.trim().is_empty() {
                        ollama.url = base_url.to_string();
                    }
                    ollama.model = model.to_string();
                }
                Provider::OpenAI => {
                    let openai = settings.openai.get_or_insert(OpenAISettings {
                        base_url: base_url.to_string(),
                        api_key: String::new(),
                        model: model.to_string(),
                    });
                    if openai.base_url.trim().is_empty() {
                        openai.base_url = base_url.to_string();
                    }
                    openai.model = model.to_string();
                }
            }

            settings
        } else {
            match provider {
                Provider::Ollama => PersistedSettings {
                    ollama: Some(PersistedOllamaSettings {
                        url: base_url.to_string(),
                        model: model.to_string(),
                    }),
                    openai: None,
                },
                Provider::OpenAI => PersistedSettings {
                    ollama: None,
                    openai: Some(OpenAISettings {
                        base_url: base_url.to_string(),
                        api_key: String::new(),
                        model: model.to_string(),
                    }),
                },
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create settings directory: {}", parent.display())
            })?;
        }

        let raw = serde_json::to_string_pretty(&settings)
            .context("failed to serialize updated settings")?;
        fs::write(path, raw)
            .with_context(|| format!("failed to write settings file: {}", path.display()))?;

        Ok(())
    }
}

fn validate_non_empty(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!("{label} must not be empty"));
    }
    Ok(())
}

fn build_agent_response(text: String, tool_uses: Vec<ToolUseCall>) -> AgentResponse {
    let stop_reason = if tool_uses.is_empty() {
        "end_turn".to_string()
    } else {
        "tool_use".to_string()
    };

    AgentResponse {
        text,
        stop_reason,
        tool_uses,
    }
}

fn map_message_for_openai(index: usize, message: &Value) -> Value {
    let role = message["role"].as_str().unwrap_or("user");
    match role {
        "assistant" => {
            let mut mapped = json!({
                "role": "assistant",
                "content": assistant_content_for_openai(message)
            });

            if let Some(tool_calls) = message["tool_calls"].as_array() {
                mapped["tool_calls"] = Value::Array(
                    tool_calls
                        .iter()
                        .enumerate()
                        .map(|(tool_index, tool_call)| {
                            let call_id = tool_call["id"]
                                .as_str()
                                .map(ToOwned::to_owned)
                                .unwrap_or_else(|| format!("call_{index}_{tool_index}"));
                            json!({
                                "id": call_id,
                                "type": "function",
                                "function": {
                                    "name": tool_call["function"]["name"],
                                    "arguments": stringify_tool_arguments(&tool_call["function"]["arguments"])
                                }
                            })
                        })
                        .collect(),
                );
            }

            mapped
        }
        "tool" => {
            if let Some(tool_call_id) = message["tool_call_id"].as_str() {
                json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": message["content"].as_str().unwrap_or_default()
                })
            } else {
                json!({
                    "role": "user",
                    "content": format!(
                        "Tool {} returned:\n{}",
                        message["tool_name"].as_str().unwrap_or("unknown"),
                        message["content"].as_str().unwrap_or_default()
                    )
                })
            }
        }
        _ => json!({
            "role": role,
            "content": message["content"].as_str().unwrap_or_default()
        }),
    }
}

fn assistant_content_for_openai(message: &Value) -> Value {
    let content = message["content"].as_str().unwrap_or_default();
    if content.is_empty() && message.get("tool_calls").is_some() {
        Value::Null
    } else {
        Value::String(content.to_string())
    }
}

fn stringify_tool_arguments(arguments: &Value) -> String {
    match arguments {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
    }
}

#[cfg(test)]
impl LLMClient {
    fn max_tokens(&self) -> u32 {
        self.max_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_settings_file_with_creates_default_settings_in_home() {
        let home = tempdir().unwrap();
        let path = LLMClient::ensure_settings_file_with(Some(home.path())).unwrap();

        assert_eq!(path, home.path().join(".localcoder/settings.json"));

        let settings = LLMClient::load_settings_from_path(&path).unwrap();
        let ollama = settings.ollama.unwrap();
        assert_eq!(ollama.url, DEFAULT_OLLAMA_URL);
        assert_eq!(ollama.model, DEFAULT_OLLAMA_MODEL);
    }

    #[test]
    fn ensure_settings_file_with_prefers_existing_home_settings() {
        let home = tempdir().unwrap();
        let home_settings = home.path().join(".localcoder/settings.json");

        fs::create_dir_all(home_settings.parent().unwrap()).unwrap();
        fs::write(
            &home_settings,
            r#"{"ollama":{"url":"http://remote-host:11434","model":"qwen2.5-coder:7b"}}"#,
        )
        .unwrap();

        let path = LLMClient::ensure_settings_file_with(Some(home.path())).unwrap();

        assert_eq!(path, home_settings);
    }

    #[test]
    fn resolve_settings_path_with_requires_home_settings() {
        let err = LLMClient::resolve_settings_path_with(None).unwrap_err();
        assert!(err.to_string().contains("$HOME/.localcoder/settings.json"));
    }

    #[test]
    fn load_settings_from_path_reads_ollama_values() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("setting.json");
        fs::write(
            &path,
            r#"{"ollama":{"url":"http://localhost:11434","model":"qwen2.5-coder:7b"}}"#,
        )
        .unwrap();

        let settings = LLMClient::load_settings_from_path(&path).unwrap();
        let ollama = settings.ollama.unwrap();
        assert_eq!(ollama.url, "http://localhost:11434");
        assert_eq!(ollama.model, "qwen2.5-coder:7b");
    }

    #[test]
    fn load_settings_from_path_prefers_openai_when_present() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("setting.json");
        fs::write(
            &path,
            r#"{
                "ollama":{"url":"http://localhost:11434","model":"qwen2.5-coder:7b"},
                "openai":{"base_url":"https://api.openai.com/v1","api_key":"sk-test","model":"gpt-4o-mini"}
            }"#,
        )
        .unwrap();

        let settings = LLMClient::load_settings_from_path(&path).unwrap();
        let openai = settings.openai.unwrap();
        assert_eq!(openai.base_url, "https://api.openai.com/v1");
        assert_eq!(openai.model, "gpt-4o-mini");
    }

    #[test]
    fn from_settings_sets_ollama_defaults() {
        let client = LLMClient::from_settings(LLMSettings {
            ollama: Some(OllamaSettings {
                url: "http://localhost:11434/".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            }),
            openai: None,
        });

        assert_eq!(client.provider, Provider::Ollama);
        assert_eq!(client.base_url(), "http://localhost:11434");
        assert_eq!(client.model(), "qwen2.5-coder:7b");
        assert_eq!(client.max_tokens(), 4096);
    }

    #[test]
    fn from_settings_prefers_openai() {
        let client = LLMClient::from_settings(LLMSettings {
            ollama: Some(OllamaSettings {
                url: "http://localhost:11434".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            }),
            openai: Some(OpenAISettings {
                base_url: "https://api.openai.com/v1/".to_string(),
                api_key: "sk-test".to_string(),
                model: "gpt-4o-mini".to_string(),
            }),
        });

        assert_eq!(client.provider, Provider::OpenAI);
        assert_eq!(client.base_url(), "https://api.openai.com/v1");
        assert_eq!(client.model(), "gpt-4o-mini");
    }

    #[test]
    fn set_model_updates_model() {
        let mut client = LLMClient::from_settings(LLMSettings {
            ollama: Some(OllamaSettings {
                url: "http://localhost:11434".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            }),
            openai: None,
        });

        client.set_model("llama3.2".to_string());
        assert_eq!(client.model(), "llama3.2");
    }

    #[test]
    fn set_max_tokens_updates_value() {
        let mut client = LLMClient::from_settings(LLMSettings {
            ollama: Some(OllamaSettings {
                url: "http://localhost:11434".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            }),
            openai: None,
        });

        client.set_max_tokens(2048);
        assert_eq!(client.max_tokens(), 2048);
    }

    #[test]
    fn persist_model_to_path_creates_ollama_settings() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".localcoder/settings.json");

        LLMClient::persist_model_to_path(
            &path,
            Provider::Ollama,
            "http://localhost:11434",
            "llama3.2",
        )
        .unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let settings: PersistedSettings = serde_json::from_str(&raw).unwrap();
        let ollama = settings.ollama.unwrap();
        assert_eq!(ollama.url, "http://localhost:11434");
        assert_eq!(ollama.model, "llama3.2");
    }

    #[test]
    fn persist_model_to_path_updates_openai_model() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".localcoder/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"openai":{"base_url":"https://api.openai.com/v1","api_key":"sk-test","model":"gpt-4o-mini"}}"#,
        )
        .unwrap();

        LLMClient::persist_model_to_path(
            &path,
            Provider::OpenAI,
            "https://api.openai.com/v1",
            "gpt-4.1-mini",
        )
        .unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let settings: PersistedSettings = serde_json::from_str(&raw).unwrap();
        let openai = settings.openai.unwrap();
        assert_eq!(openai.base_url, "https://api.openai.com/v1");
        assert_eq!(openai.api_key, "sk-test");
        assert_eq!(openai.model, "gpt-4.1-mini");
    }

    #[test]
    fn map_message_for_openai_preserves_tool_call_id() {
        let message = json!({
            "role": "tool",
            "tool_call_id": "call_123",
            "tool_name": "read",
            "content": "ok"
        });

        let mapped = map_message_for_openai(0, &message);
        assert_eq!(mapped["role"], "tool");
        assert_eq!(mapped["tool_call_id"], "call_123");
        assert_eq!(mapped["content"], "ok");
    }
}
