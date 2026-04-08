/*!
 * API Client Module
 *
 * Corresponds to: src/services/api/claude.ts
 *
 * Main features:
 * - Call Anthropic Messages API
 * - Handle streaming responses (Server-Sent Events)
 * - Manage conversation history
 */

use crate::markdown::MarkdownRenderer;
use crate::types::*;
use anyhow::{Context, Result};
use futures::stream::StreamExt;
use reqwest::Client;
use serde_json::json;
use std::env;
use std::io::{self, Write};

/// Claude API client (now supports local Ollama)
pub struct ClaudeClient {
    api_key: Option<String>,
    client: Client,
    base_url: String,
    model: String,
    max_tokens: u32,
    markdown_enabled: bool,
}

impl ClaudeClient {
    /// Create a new client instance (auto-detect Ollama or Claude)
    /// Priority: USE_OLLAMA env var > api_key empty > api_key present
    pub fn new(api_key: &str) -> Result<Self> {
        // Check if USE_OLLAMA is set (highest priority)
        let use_ollama = env::var("USE_OLLAMA").is_ok() || api_key.is_empty();

        let (base_url, model, api_key) = if use_ollama {
            // Use Ollama (local)
            let ollama_url = env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
            let ollama_model = env::var("OLLAMA_MODEL")
                .unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());

            (ollama_url, ollama_model, None)
        } else {
            // Use Claude API
            (
                "https://api.anthropic.com/v1".to_string(),
                "claude-4.5-sonnet".to_string(),
                Some(api_key.to_string()),
            )
        };

        // Check if markdown rendering is enabled (default: true)
        let markdown_enabled = env::var("DISABLE_MARKDOWN").is_err();

        Ok(Self {
            api_key,
            client: Client::new(),
            base_url,
            model,
            max_tokens: 4096,
            markdown_enabled,
        })
    }

    /// Create a new Ollama client with custom endpoint
    pub fn new_ollama(model: &str, base_url: Option<&str>) -> Result<Self> {
        let markdown_enabled = env::var("DISABLE_MARKDOWN").is_err();

        Ok(Self {
            api_key: None,
            client: Client::new(),
            base_url: base_url
                .unwrap_or("http://localhost:11434/v1")
                .to_string(),
            model: model.to_string(),
            max_tokens: 4096,
            markdown_enabled,
        })
    }

    /// Query Claude API (streaming response)
    ///
    /// Corresponds to: src/services/api/claude.ts:864
    /// ```typescript
    /// await anthropic.beta.messages.create({
    ///   model: 'claude-opus-4',
    ///   max_tokens: 4096,
    ///   messages: messages,
    ///   stream: true
    /// })
    /// ```
    pub async fn query_streaming(
        &self,
        prompt: &str,
        history: &[Message],
    ) -> Result<String> {
        // Build messages array
        let mut messages: Vec<serde_json::Value> = history
            .iter()
            .map(|msg| {
                json!({
                    "role": msg.role,
                    "content": msg.content
                })
            })
            .collect();

        // Add current user message
        messages.push(json!({
            "role": "user",
            "content": prompt
        }));

        // Build request body
        let body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": messages,
            "stream": true
        });

        // Send request with appropriate headers
        let mut request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("content-type", "application/json");

        // Add API key if using Claude API
        if let Some(ref api_key) = self.api_key {
            request = request
                .header("Authorization", format!("Bearer {}", api_key))
                .header("anthropic-version", "2023-06-01");
        }

        let response = request
            .json(&body)
            .send()
            .await
            .context("API request failed")?;

        // Check response status
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("API returned error {}: {}", status, error_text);
        }

        // Handle streaming response
        let mut full_response = String::new();
        let mut stream = response.bytes_stream();
        let is_ollama = self.api_key.is_none();
        let mut markdown_renderer = if self.markdown_enabled {
            Some(MarkdownRenderer::new())
        } else {
            None
        };

        // Process Server-Sent Events (SSE) stream
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read stream data")?;
            let text = String::from_utf8_lossy(&chunk);

            // Parse SSE format: "data: {...}\n\n"
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break;
                    }

                    if is_ollama {
                        // Parse OpenAI-compatible format (Ollama)
                        if let Ok(chunk) = serde_json::from_str::<crate::types::OpenAIStreamChunk>(data) {
                            for choice in chunk.choices {
                                if let Some(content) = choice.delta.content {
                                    // Render with markdown if enabled
                                    let output = if let Some(ref mut renderer) = markdown_renderer {
                                        renderer.process_chunk(&content)
                                    } else {
                                        content.clone()
                                    };

                                    print!("{}", output);
                                    io::stdout().flush().ok();
                                    full_response.push_str(&content);
                                }
                                if choice.finish_reason.is_some() {
                                    break;
                                }
                            }
                        }
                    } else {
                        // Parse Claude format
                        if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                            match event.event_type.as_str() {
                                "content_block_delta" => {
                                    if let Some(delta) = event.delta {
                                        if delta.delta_type == "text_delta" {
                                            if let Some(text) = delta.text {
                                                // Render with markdown if enabled
                                                let output = if let Some(ref mut renderer) = markdown_renderer {
                                                    renderer.process_chunk(&text)
                                                } else {
                                                    text.clone()
                                                };

                                                print!("{}", output);
                                                io::stdout().flush().ok();
                                                full_response.push_str(&text);
                                            }
                                        }
                                    }
                                }
                                "message_start" => {
                                    // Message started
                                }
                                "message_stop" => {
                                    // Message ended
                                    break;
                                }
                                _ => {
                                    // Other event types
                                }
                            }
                        }
                    }
                }
            }
        }

        // Flush any remaining markdown content
        if let Some(ref mut renderer) = markdown_renderer {
            let remaining = renderer.flush();
            if !remaining.is_empty() {
                print!("{}", remaining);
                io::stdout().flush().ok();
            }
        }

        Ok(full_response)
    }

    /// Query Claude API (non-streaming, returns all at once)
    pub async fn query(&self, prompt: &str, history: &[Message]) -> Result<ApiResponse> {
        let mut messages: Vec<serde_json::Value> = history
            .iter()
            .map(|msg| {
                json!({
                    "role": msg.role,
                    "content": msg.content
                })
            })
            .collect();

        messages.push(json!({
            "role": "user",
            "content": prompt
        }));

        let body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": messages,
            "stream": false
        });

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url));

        let response = if let Some(ref api_key) = self.api_key {
            response
                .header("Authorization", format!("Bearer {}", api_key))
                .header("anthropic-version", "2023-06-01")
        } else {
            response
        };

        let response = response
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("API request failed")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("API returned error {}: {}", status, error_text);
        }

        let api_response: ApiResponse = response.json().await.context("Failed to parse response")?;
        Ok(api_response)
    }

    /// Set model
    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    /// Set max tokens
    pub fn set_max_tokens(&mut self, max_tokens: u32) {
        self.max_tokens = max_tokens;
    }
}

#[cfg(test)]
impl ClaudeClient {
    fn model(&self) -> &str {
        &self.model
    }

    fn max_tokens(&self) -> u32 {
        self.max_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_client_has_default_model() {
        let client = ClaudeClient::new("test-key").unwrap();
        assert_eq!(client.model(), "claude-4.5-sonnet");
    }

    #[test]
    fn new_client_has_default_max_tokens() {
        let client = ClaudeClient::new("test-key").unwrap();
        assert_eq!(client.max_tokens(), 4096);
    }

    #[test]
    fn set_model_updates_model() {
        let mut client = ClaudeClient::new("test-key").unwrap();
        client.set_model("claude-opus-4-20250514".to_string());
        assert_eq!(client.model(), "claude-opus-4-20250514");
    }

    #[test]
    fn set_max_tokens_updates_value() {
        let mut client = ClaudeClient::new("test-key").unwrap();
        client.set_max_tokens(2048);
        assert_eq!(client.max_tokens(), 2048);
    }
}
