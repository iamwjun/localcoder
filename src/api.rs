/*!
 * LLM Client Module
 *
 * Pure Ollama client implementation with tool-calling support.
 */

use crate::types::{AgentResponse, OllamaChatResponse, ToolUseCall};
use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen3.5:4b";

#[derive(Debug, Clone, Deserialize)]
struct LLMSettings {
    ollama: OllamaSettings,
}

#[derive(Debug, Clone, Deserialize)]
struct OllamaSettings {
    url: String,
    model: String,
}

/// Ollama client used by the REPL and agent loop.
pub struct LLMClient {
    client: Client,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl LLMClient {
    /// Create a client from `.localcoder/setting.json`.
    pub fn new() -> Result<Self> {
        let settings = Self::load_settings()?;
        Ok(Self::from_settings(settings))
    }

    /// Ensure the settings file exists before command handling starts.
    pub fn ensure_settings_file() -> Result<PathBuf> {
        let cwd = env::current_dir().context("failed to resolve current working directory")?;
        let home = env::var_os("HOME").map(PathBuf::from);
        Self::ensure_settings_file_with(&cwd, home.as_deref())
    }

    fn from_settings(settings: LLMSettings) -> Self {
        Self {
            client: Client::new(),
            base_url: settings.ollama.url.trim_end_matches('/').to_string(),
            model: settings.ollama.model,
            max_tokens: 4096,
        }
    }

    fn load_settings() -> Result<LLMSettings> {
        let path = Self::resolve_settings_path()?;
        Self::load_settings_from_path(&path)
    }

    fn resolve_settings_path() -> Result<PathBuf> {
        let cwd = env::current_dir().context("failed to resolve current working directory")?;
        let home = env::var_os("HOME").map(PathBuf::from);
        Self::resolve_settings_path_with(&cwd, home.as_deref())
    }

    fn resolve_settings_path_with(cwd: &Path, home: Option<&Path>) -> Result<PathBuf> {
        let cwd_path = cwd.join(".localcoder/settings.json");
        if cwd_path.exists() {
            return Ok(cwd_path);
        }

        if let Some(home) = home {
            let home_path = home.join(".localcoder/settings.json");
            if home_path.exists() {
                return Ok(home_path);
            }
        }

        Err(anyhow!(
            "missing .localcoder/settings.json; checked current directory and $HOME"
        ))
    }

    fn ensure_settings_file_with(cwd: &Path, home: Option<&Path>) -> Result<PathBuf> {
        if let Ok(path) = Self::resolve_settings_path_with(cwd, home) {
            return Ok(path);
        }

        let path = cwd.join(".localcoder/settings.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create settings directory: {}",
                    parent.display()
                )
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

        if settings.ollama.url.trim().is_empty() {
            return Err(anyhow!("settings.ollama.url must not be empty"));
        }
        if settings.ollama.model.trim().is_empty() {
            return Err(anyhow!("settings.ollama.model must not be empty"));
        }

        Ok(settings)
    }

    fn default_settings_json() -> String {
        json!({
            "ollama": {
                "url": DEFAULT_OLLAMA_URL,
                "model": DEFAULT_OLLAMA_MODEL,
            }
        })
        .to_string()
    }

    /// Send a tool-aware chat request to Ollama.
    pub async fn call_with_tools(
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
                name: tool_call.function.name,
                arguments: tool_call.function.arguments,
            })
            .collect::<Vec<_>>();

        let stop_reason = if tool_uses.is_empty() {
            "end_turn".to_string()
        } else {
            "tool_use".to_string()
        };

        Ok(AgentResponse {
            text,
            stop_reason,
            tool_uses,
        })
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
    fn ensure_settings_file_with_creates_default_settings_in_cwd() {
        let temp = tempdir().unwrap();
        let path = LLMClient::ensure_settings_file_with(temp.path(), None).unwrap();

        assert_eq!(path, temp.path().join(".localcoder/settings.json"));

        let settings = LLMClient::load_settings_from_path(&path).unwrap();
        assert_eq!(settings.ollama.url, DEFAULT_OLLAMA_URL);
        assert_eq!(settings.ollama.model, DEFAULT_OLLAMA_MODEL);
    }

    #[test]
    fn ensure_settings_file_with_prefers_existing_home_settings() {
        let cwd = tempdir().unwrap();
        let home = tempdir().unwrap();
        let home_settings = home.path().join(".localcoder/settings.json");

        fs::create_dir_all(home_settings.parent().unwrap()).unwrap();
        fs::write(
            &home_settings,
            r#"{"ollama":{"url":"http://remote-host:11434","model":"qwen2.5-coder:7b"}}"#,
        )
        .unwrap();

        let path = LLMClient::ensure_settings_file_with(cwd.path(), Some(home.path())).unwrap();

        assert_eq!(path, home_settings);
        assert!(!cwd.path().join(".localcoder/settings.json").exists());
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
        assert_eq!(settings.ollama.url, "http://localhost:11434");
        assert_eq!(settings.ollama.model, "qwen2.5-coder:7b");
    }

    #[test]
    fn from_settings_sets_defaults() {
        let client = LLMClient::from_settings(LLMSettings {
            ollama: OllamaSettings {
                url: "http://localhost:11434/".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            },
        });

        assert_eq!(client.base_url(), "http://localhost:11434");
        assert_eq!(client.model(), "qwen2.5-coder:7b");
        assert_eq!(client.max_tokens(), 4096);
    }

    #[test]
    fn set_model_updates_model() {
        let mut client = LLMClient::from_settings(LLMSettings {
            ollama: OllamaSettings {
                url: "http://localhost:11434".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            },
        });

        client.set_model("llama3.2".to_string());
        assert_eq!(client.model(), "llama3.2");
    }

    #[test]
    fn set_max_tokens_updates_value() {
        let mut client = LLMClient::from_settings(LLMSettings {
            ollama: OllamaSettings {
                url: "http://localhost:11434".to_string(),
                model: "qwen2.5-coder:7b".to_string(),
            },
        });

        client.set_max_tokens(2048);
        assert_eq!(client.max_tokens(), 2048);
    }
}
