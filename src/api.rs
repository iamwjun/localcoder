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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LLMSettings {
    llm: LLMConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LLMConfig {
    #[serde(rename = "type")]
    provider: Provider,
    base_url: String,
    #[serde(default)]
    api_key: Option<String>,
    model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Provider {
    Ollama,
    LMStudio,
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
struct LMStudioModelsResponse {
    models: Vec<LMStudioModelEntry>,
}

#[derive(Debug, Deserialize)]
struct LMStudioModelEntry {
    key: String,
    #[serde(rename = "type")]
    model_type: Option<String>,
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
        let provider = settings.llm.provider;

        match provider {
            Provider::OpenAI => Self {
                client: Client::new(),
                provider: Provider::OpenAI,
                base_url: settings.llm.base_url.trim_end_matches('/').to_string(),
                api_key: settings.llm.api_key.filter(|key| !key.trim().is_empty()),
                model: settings.llm.model,
                max_tokens: 4096,
            },
            Provider::LMStudio => Self {
                client: Client::new(),
                provider: Provider::LMStudio,
                base_url: normalize_lmstudio_base_url(&settings.llm.base_url),
                api_key: settings.llm.api_key.filter(|key| !key.trim().is_empty()),
                model: settings.llm.model,
                max_tokens: 4096,
            },
            Provider::Ollama => Self {
                client: Client::new(),
                provider: Provider::Ollama,
                base_url: settings.llm.base_url.trim_end_matches('/').to_string(),
                api_key: None,
                model: settings.llm.model,
                max_tokens: 4096,
            },
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
            Self::migrate_settings_file_if_needed(&path)?;
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

    fn migrate_settings_file_if_needed(path: &Path) -> Result<()> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read settings file: {}", path.display()))?;
        let mut root: Value = match serde_json::from_str(&raw) {
            Ok(root) => root,
            Err(_) => return Ok(()),
        };

        if !Self::migrate_settings_value(&mut root)? {
            return Ok(());
        }

        let migrated =
            serde_json::to_string_pretty(&root).context("failed to serialize migrated settings")?;
        fs::write(path, migrated)
            .with_context(|| format!("failed to write migrated settings: {}", path.display()))?;
        Ok(())
    }

    fn migrate_settings_value(root: &mut Value) -> Result<bool> {
        let Some(root_obj) = root.as_object_mut() else {
            return Ok(false);
        };

        if root_obj.get("llm").and_then(Value::as_object).is_some() {
            let root_provider = root_obj
                .get("type")
                .and_then(Value::as_str)
                .and_then(parse_provider);
            let mut changed = false;
            {
                let llm_obj = root_obj
                    .get_mut("llm")
                    .and_then(Value::as_object_mut)
                    .expect("checked llm object above");
                let provider = llm_obj
                    .get("type")
                    .and_then(Value::as_str)
                    .and_then(parse_provider)
                    .or(root_provider)
                    .or_else(|| {
                        llm_obj
                            .get("base_url")
                            .and_then(Value::as_str)
                            .map(infer_provider_from_base_url)
                    })
                    .unwrap_or(Provider::Ollama);

                let provider_value =
                    serde_json::to_value(provider).context("failed to serialize provider")?;
                if llm_obj.get("type") != Some(&provider_value) {
                    llm_obj.insert("type".to_string(), provider_value);
                    changed = true;
                }

                if matches!(provider, Provider::Ollama) && llm_obj.remove("api_key").is_some() {
                    changed = true;
                }
            }

            for key in ["type", "ollama", "lmstudio", "openai"] {
                if root_obj.remove(key).is_some() {
                    changed = true;
                }
            }

            return Ok(changed);
        }

        let provider = root_obj
            .get("type")
            .and_then(Value::as_str)
            .and_then(parse_provider)
            .filter(|provider| legacy_section_exists(root_obj, *provider))
            .or_else(|| {
                [Provider::OpenAI, Provider::LMStudio, Provider::Ollama]
                    .into_iter()
                    .find(|provider| legacy_section_exists(root_obj, *provider))
            });

        let Some(provider) = provider else {
            return Ok(false);
        };

        let Some(llm) = build_llm_from_legacy(root_obj, provider) else {
            return Ok(false);
        };

        root_obj.insert(
            "llm".to_string(),
            serde_json::to_value(llm).context("failed to serialize llm config")?,
        );

        for key in ["type", "ollama", "lmstudio", "openai"] {
            root_obj.remove(key);
        }

        Ok(true)
    }

    fn load_settings_from_path(path: &Path) -> Result<LLMSettings> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read settings file: {}", path.display()))?;
        let settings: LLMSettings = serde_json::from_str(&raw)
            .with_context(|| format!("invalid settings JSON: {}", path.display()))?;
        let _provider = settings.llm.provider;
        validate_non_empty("settings.llm.base_url", &settings.llm.base_url)?;
        validate_non_empty("settings.llm.model", &settings.llm.model)?;

        Ok(settings)
    }

    fn default_settings_json() -> String {
        serde_json::to_string_pretty(&json!({
            "llm": {
                "type": "ollama",
                "base_url": DEFAULT_OLLAMA_URL,
                "model": DEFAULT_OLLAMA_MODEL
            }
        }))
        .expect("default settings json must serialize")
    }

    pub fn provider_name(&self) -> &'static str {
        match self.provider {
            Provider::Ollama => "Ollama",
            Provider::LMStudio => "LM Studio",
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
            Provider::LMStudio | Provider::OpenAI => {
                self.call_with_tools_openai_compatible(messages, tools)
                    .await
            }
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

    async fn call_with_tools_openai_compatible(
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
                    .post(self.chat_completions_url())
                    .header("content-type", "application/json"),
            )?
            .json(&body)
            .send()
            .await
            .with_context(|| format!("{} request failed", self.provider_name()))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "{} returned error {}: {}",
                self.provider_name(),
                status,
                error_text
            );
        }

        let response: OpenAIChatResponse = response
            .json()
            .await
            .with_context(|| format!("failed to parse {} response", self.provider_name()))?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("{} returned no choices", self.provider_name()))?;

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
            Provider::LMStudio | Provider::OpenAI => {
                self.complete_prompt_openai_compatible(prompt, max_tokens)
                    .await
            }
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

    async fn complete_prompt_openai_compatible(
        &self,
        prompt: &str,
        max_tokens: u32,
    ) -> Result<String> {
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
                    .post(self.chat_completions_url())
                    .header("content-type", "application/json"),
            )?
            .json(&body)
            .send()
            .await
            .with_context(|| format!("{} prompt request failed", self.provider_name()))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "{} returned error {}: {}",
                self.provider_name(),
                status,
                error_text
            );
        }

        let response: OpenAIChatResponse = response
            .json()
            .await
            .with_context(|| format!("failed to parse {} prompt response", self.provider_name()))?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("{} returned no choices", self.provider_name()))?;
        Ok(choice.message.content.unwrap_or_default())
    }

    fn authorized_request(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        match self.provider {
            Provider::Ollama => Ok(request),
            Provider::LMStudio => match self.api_key.as_deref() {
                Some(api_key) => Ok(request.bearer_auth(api_key)),
                None => Ok(request),
            },
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

    fn chat_completions_url(&self) -> String {
        match self.provider {
            Provider::LMStudio => format!("{}/v1/chat/completions", self.base_url),
            Provider::Ollama | Provider::OpenAI => format!("{}/chat/completions", self.base_url),
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        match self.provider {
            Provider::Ollama => self.list_models_ollama().await,
            Provider::LMStudio => self.list_models_lmstudio().await,
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

    async fn list_models_lmstudio(&self) -> Result<Vec<String>> {
        let response = self
            .authorized_request(self.client.get(format!("{}/api/v1/models", self.base_url)))?
            .send()
            .await
            .context("failed to fetch LM Studio models")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("LM Studio returned error {}: {}", status, error_text);
        }

        let response: LMStudioModelsResponse = response
            .json()
            .await
            .context("failed to parse LM Studio models response")?;

        let mut models = response
            .models
            .into_iter()
            .filter(|model| model.model_type.as_deref() != Some("embedding"))
            .map(|model| model.key)
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
            let mut settings: LLMSettings = serde_json::from_str(&raw)
                .with_context(|| format!("invalid settings JSON: {}", path.display()))?;
            if settings.llm.base_url.trim().is_empty() {
                settings.llm.base_url = base_url.to_string();
            }
            settings.llm.provider = provider;
            if matches!(provider, Provider::Ollama) {
                settings.llm.api_key = None;
            }
            settings.llm.model = model.to_string();
            settings
        } else {
            LLMSettings {
                llm: LLMConfig {
                    provider,
                    base_url: base_url.to_string(),
                    api_key: None,
                    model: model.to_string(),
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

fn parse_provider(value: &str) -> Option<Provider> {
    match value {
        "ollama" => Some(Provider::Ollama),
        "lmstudio" => Some(Provider::LMStudio),
        "openai" => Some(Provider::OpenAI),
        _ => None,
    }
}

fn legacy_section_exists(root: &serde_json::Map<String, Value>, provider: Provider) -> bool {
    let key = match provider {
        Provider::Ollama => "ollama",
        Provider::LMStudio => "lmstudio",
        Provider::OpenAI => "openai",
    };
    root.get(key).and_then(Value::as_object).is_some()
}

fn build_llm_from_legacy(
    root: &serde_json::Map<String, Value>,
    provider: Provider,
) -> Option<LLMConfig> {
    let key = match provider {
        Provider::Ollama => "ollama",
        Provider::LMStudio => "lmstudio",
        Provider::OpenAI => "openai",
    };
    let section = root.get(key)?.as_object()?;
    let base_url = section
        .get("base_url")
        .or_else(|| section.get("url"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let model = section
        .get("model")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if base_url.is_empty() || model.is_empty() {
        return None;
    }

    let api_key = match provider {
        Provider::Ollama => None,
        Provider::LMStudio | Provider::OpenAI => section
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(ToOwned::to_owned),
    };

    Some(LLMConfig {
        provider,
        base_url,
        api_key,
        model,
    })
}

fn infer_provider_from_base_url(base_url: &str) -> Provider {
    let normalized = normalize_lmstudio_base_url(base_url);

    if let Ok(url) = reqwest::Url::parse(&normalized) {
        match url.port_or_known_default() {
            Some(11434) => return Provider::Ollama,
            Some(1234) => return Provider::LMStudio,
            _ => {}
        }
    }

    if normalized.contains("11434") || normalized.contains("ollama") {
        Provider::Ollama
    } else if normalized.contains("1234") || normalized.contains("lmstudio") {
        Provider::LMStudio
    } else {
        Provider::OpenAI
    }
}

fn normalize_lmstudio_base_url(base_url: &str) -> String {
    let mut normalized = base_url.trim().trim_end_matches('/').to_string();

    for suffix in ["/v1", "/api/v1", "/api/v0"] {
        if normalized.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            break;
        }
    }

    normalized
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
        assert_eq!(settings.llm.provider, Provider::Ollama);
        assert_eq!(settings.llm.base_url, DEFAULT_OLLAMA_URL);
        assert_eq!(settings.llm.model, DEFAULT_OLLAMA_MODEL);
        assert_eq!(settings.llm.api_key, None);
    }

    #[test]
    fn ensure_settings_file_with_prefers_existing_home_settings() {
        let home = tempdir().unwrap();
        let home_settings = home.path().join(".localcoder/settings.json");

        fs::create_dir_all(home_settings.parent().unwrap()).unwrap();
        fs::write(
            &home_settings,
            r#"{"llm":{"type":"ollama","base_url":"http://remote-host:11434","model":"qwen2.5-coder:7b"}}"#,
        )
        .unwrap();

        let path = LLMClient::ensure_settings_file_with(Some(home.path())).unwrap();

        assert_eq!(path, home_settings);
    }

    #[test]
    fn ensure_settings_file_with_migrates_legacy_ollama_settings() {
        let home = tempdir().unwrap();
        let home_settings = home.path().join(".localcoder/settings.json");
        fs::create_dir_all(home_settings.parent().unwrap()).unwrap();
        fs::write(
            &home_settings,
            r#"{
                "ollama":{"url":"http://localhost:11434","model":"qwen2.5-coder:7b"},
                "ui":{"theme":"dark"}
            }"#,
        )
        .unwrap();

        LLMClient::ensure_settings_file_with(Some(home.path())).unwrap();

        let raw = fs::read_to_string(&home_settings).unwrap();
        let root: Value = serde_json::from_str(&raw).unwrap();
        assert!(root.get("ollama").is_none());
        assert_eq!(root["llm"]["type"], "ollama");
        assert_eq!(root["llm"]["base_url"], "http://localhost:11434");
        assert_eq!(root["llm"]["model"], "qwen2.5-coder:7b");
        assert_eq!(root["ui"]["theme"], "dark");
    }

    #[test]
    fn ensure_settings_file_with_migrates_legacy_openai_settings() {
        let home = tempdir().unwrap();
        let home_settings = home.path().join(".localcoder/settings.json");
        fs::create_dir_all(home_settings.parent().unwrap()).unwrap();
        fs::write(
            &home_settings,
            r#"{
                "openai":{"base_url":"https://api.openai.com/v1","api_key":"sk-test","model":"gpt-4o-mini"},
                "lsp":{"enabled":false}
            }"#,
        )
        .unwrap();

        LLMClient::ensure_settings_file_with(Some(home.path())).unwrap();

        let raw = fs::read_to_string(&home_settings).unwrap();
        let root: Value = serde_json::from_str(&raw).unwrap();
        assert!(root.get("openai").is_none());
        assert_eq!(root["llm"]["type"], "openai");
        assert_eq!(root["llm"]["base_url"], "https://api.openai.com/v1");
        assert_eq!(root["llm"]["api_key"], "sk-test");
        assert_eq!(root["llm"]["model"], "gpt-4o-mini");
        assert_eq!(root["lsp"]["enabled"], false);
    }

    #[test]
    fn ensure_settings_file_with_adds_type_to_llm_when_missing() {
        let home = tempdir().unwrap();
        let home_settings = home.path().join(".localcoder/settings.json");
        fs::create_dir_all(home_settings.parent().unwrap()).unwrap();
        fs::write(
            &home_settings,
            r#"{
                "llm":{"base_url":"http://localhost:1234","model":"deepseek-r1"},
                "type":"lmstudio"
            }"#,
        )
        .unwrap();

        LLMClient::ensure_settings_file_with(Some(home.path())).unwrap();

        let raw = fs::read_to_string(&home_settings).unwrap();
        let root: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(root["llm"]["type"], "lmstudio");
        assert!(root.get("type").is_none());
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
            r#"{"llm":{"type":"ollama","base_url":"http://localhost:11434","model":"qwen2.5-coder:7b"}}"#,
        )
        .unwrap();

        let settings = LLMClient::load_settings_from_path(&path).unwrap();
        assert_eq!(settings.llm.provider, Provider::Ollama);
        assert_eq!(settings.llm.base_url, "http://localhost:11434");
        assert_eq!(settings.llm.model, "qwen2.5-coder:7b");
    }

    #[test]
    fn load_settings_from_path_reads_openai_values() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("setting.json");
        fs::write(
            &path,
            r#"{"llm":{"type":"openai","base_url":"https://api.openai.com/v1","api_key":"sk-test","model":"gpt-4o-mini"}}"#,
        )
        .unwrap();

        let settings = LLMClient::load_settings_from_path(&path).unwrap();
        assert_eq!(settings.llm.provider, Provider::OpenAI);
        assert_eq!(settings.llm.base_url, "https://api.openai.com/v1");
        assert_eq!(settings.llm.api_key.as_deref(), Some("sk-test"));
        assert_eq!(settings.llm.model, "gpt-4o-mini");
    }

    #[test]
    fn load_settings_from_path_requires_llm() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("setting.json");
        fs::write(
            &path,
            r#"{"openai":{"base_url":"https://api.openai.com/v1"}}"#,
        )
        .unwrap();

        let err = LLMClient::load_settings_from_path(&path).unwrap_err();
        assert!(err.to_string().contains("invalid settings JSON"));
    }

    #[test]
    fn load_settings_from_path_requires_llm_type() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("setting.json");
        fs::write(
            &path,
            r#"{"llm":{"base_url":"http://localhost:11434","model":"qwen2.5-coder:7b"}}"#,
        )
        .unwrap();

        let err = LLMClient::load_settings_from_path(&path).unwrap_err();
        assert!(err.to_string().contains("invalid settings JSON"));
    }

    #[test]
    fn from_settings_sets_ollama_defaults() {
        let client = LLMClient::from_settings(LLMSettings {
            llm: LLMConfig {
                provider: Provider::Ollama,
                base_url: "http://localhost:11434/".to_string(),
                api_key: Some("ignored".to_string()),
                model: "qwen2.5-coder:7b".to_string(),
            },
        });

        assert_eq!(client.provider, Provider::Ollama);
        assert_eq!(client.base_url(), "http://localhost:11434");
        assert_eq!(client.model(), "qwen2.5-coder:7b");
        assert_eq!(client.max_tokens(), 4096);
    }

    #[test]
    fn from_settings_detects_lmstudio() {
        let client = LLMClient::from_settings(LLMSettings {
            llm: LLMConfig {
                provider: Provider::LMStudio,
                base_url: "http://localhost:1234/".to_string(),
                api_key: None,
                model: "deepseek-r1".to_string(),
            },
        });

        assert_eq!(client.provider, Provider::LMStudio);
        assert_eq!(client.base_url(), "http://localhost:1234");
        assert_eq!(client.model(), "deepseek-r1");
        assert_eq!(
            client.chat_completions_url(),
            "http://localhost:1234/v1/chat/completions"
        );
    }

    #[test]
    fn from_settings_detects_openai() {
        let client = LLMClient::from_settings(LLMSettings {
            llm: LLMConfig {
                provider: Provider::OpenAI,
                base_url: "https://api.openai.com/v1/".to_string(),
                api_key: Some("sk-test".to_string()),
                model: "gpt-4o-mini".to_string(),
            },
        });

        assert_eq!(client.provider, Provider::OpenAI);
        assert_eq!(client.base_url(), "https://api.openai.com/v1");
        assert_eq!(client.model(), "gpt-4o-mini");
    }

    #[test]
    fn from_settings_normalizes_lmstudio_v1_base_url() {
        let client = LLMClient::from_settings(LLMSettings {
            llm: LLMConfig {
                provider: Provider::LMStudio,
                base_url: "http://localhost:1234/v1/".to_string(),
                api_key: None,
                model: "deepseek-r1".to_string(),
            },
        });

        assert_eq!(client.base_url(), "http://localhost:1234");
        assert_eq!(
            client.chat_completions_url(),
            "http://localhost:1234/v1/chat/completions"
        );
    }

    #[test]
    fn set_model_updates_model() {
        let mut client = LLMClient::from_settings(LLMSettings {
            llm: LLMConfig {
                provider: Provider::Ollama,
                base_url: "http://localhost:11434".to_string(),
                api_key: None,
                model: "qwen2.5-coder:7b".to_string(),
            },
        });

        client.set_model("llama3.2".to_string());
        assert_eq!(client.model(), "llama3.2");
    }

    #[test]
    fn set_max_tokens_updates_value() {
        let mut client = LLMClient::from_settings(LLMSettings {
            llm: LLMConfig {
                provider: Provider::Ollama,
                base_url: "http://localhost:11434".to_string(),
                api_key: None,
                model: "qwen2.5-coder:7b".to_string(),
            },
        });

        client.set_max_tokens(2048);
        assert_eq!(client.max_tokens(), 2048);
    }

    #[test]
    fn persist_model_to_path_creates_llm_settings() {
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
        let settings: LLMSettings = serde_json::from_str(&raw).unwrap();
        assert_eq!(settings.llm.provider, Provider::Ollama);
        assert_eq!(settings.llm.base_url, "http://localhost:11434");
        assert_eq!(settings.llm.model, "llama3.2");
    }

    #[test]
    fn persist_model_to_path_updates_existing_llm_model() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".localcoder/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"llm":{"type":"lmstudio","base_url":"http://localhost:1234","api_key":"token","model":"deepseek-r1"}}"#,
        )
        .unwrap();

        LLMClient::persist_model_to_path(
            &path,
            Provider::LMStudio,
            "http://localhost:1234",
            "qwen3-coder",
        )
        .unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let settings: LLMSettings = serde_json::from_str(&raw).unwrap();
        assert_eq!(settings.llm.provider, Provider::LMStudio);
        assert_eq!(settings.llm.base_url, "http://localhost:1234");
        assert_eq!(settings.llm.api_key.as_deref(), Some("token"));
        assert_eq!(settings.llm.model, "qwen3-coder");
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
