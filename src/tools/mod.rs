/*!
 * Tool System — S01
 *
 * Corresponds to: src/Tool.ts:buildTool(), src/tools.ts
 *
 * Design:
 *   Tool trait    — unified interface every tool must implement
 *   ToolRegistry  — registry that dispatches Claude's tool_use calls
 *   EchoTool      — built-in smoke-test tool (always available)
 *
 * Note: Uses edition 2024 native async fn in traits (no async_trait crate needed)
 */

use anyhow::{anyhow, Result};
use colored::*;
use serde_json::{json, Value};
use std::collections::HashMap;

// ─── Tool trait ────────────────────────────────────────────────────────────

/// Unified interface for every tool.
///
/// Corresponds to: src/Tool.ts interface
/// Edition 2024 allows async fn in traits natively.
pub trait Tool: Send + Sync {
    /// The exact name Claude uses when it emits a tool_use block.
    fn name(&self) -> &str;

    /// One-line description shown to the user when listing tools.
    fn description(&self) -> &str;

    /// JSON Schema object passed to the Claude API in the `tools` array.
    /// Must be a valid `{"type":"object","properties":{...},"required":[...]}`.
    fn schema(&self) -> Value;

    /// Execute the tool with the parsed input from the API response.
    /// Returns a string result that is sent back as `tool_result` content.
    ///
    /// On error, returns Err; the engine converts this to an is_error tool_result.
    fn execute(&self, input: Value) -> impl Future<Output = Result<String>> + Send;
}

use std::future::Future;

// ─── ToolRegistry ──────────────────────────────────────────────────────────

/// Registry that owns all registered tools and routes Claude's tool_use calls.
///
/// Corresponds to: src/tools.ts (the tools array passed to query())
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn ToolBoxed>>,
}

/// Object-safe wrapper trait for dynamic dispatch.
/// (edition 2024 async fn in traits are not yet object-safe; use Box<dyn Future>)
trait ToolBoxed: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    fn execute_boxed(&self, input: Value) -> std::pin::Pin<Box<dyn Future<Output = Result<String>> + Send + '_>>;
}

impl<T: Tool> ToolBoxed for T {
    fn name(&self) -> &str {
        Tool::name(self)
    }
    fn description(&self) -> &str {
        Tool::description(self)
    }
    fn schema(&self) -> Value {
        Tool::schema(self)
    }
    fn execute_boxed(&self, input: Value) -> std::pin::Pin<Box<dyn Future<Output = Result<String>> + Send + '_>> {
        Box::pin(Tool::execute(self, input))
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Later registrations with the same name overwrite earlier ones.
    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
    }

    /// Collect JSON Schema objects for all registered tools.
    /// This slice is passed directly in the `tools` field of every API request.
    ///
    /// Corresponds to: src/tools.ts — building the tools array for the API
    pub fn get_schemas(&self) -> Vec<Value> {
        let mut schemas: Vec<Value> = self
            .tools
            .values()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                    "input_schema": t.schema()
                })
            })
            .collect();
        // Stable ordering for deterministic API payloads
        schemas.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
        schemas
    }

    /// Look up and execute a tool by name.
    pub async fn execute(&self, name: &str, input: Value) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow!("unknown tool: {}", name))?;
        tool.execute_boxed(input).await
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// List names of registered tools (sorted).
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── EchoTool ──────────────────────────────────────────────────────────────

/// Built-in smoke-test tool — echoes back whatever text Claude passes it.
pub struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo_tool"
    }

    fn description(&self) -> &str {
        "Echo back the provided text. Use for testing the tool execution pipeline."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to echo back"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, input: Value) -> Result<String> {
        let text = input["text"]
            .as_str()
            .ok_or_else(|| anyhow!("echo_tool: missing required field 'text'"))?;
        println!("{}", format!("  [echo_tool] → {}", text).dimmed());
        Ok(text.to_string())
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_tool_returns_input_text() {
        let tool = EchoTool;
        let result = tool.execute(json!({"text": "hello world"})).await.unwrap();
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn echo_tool_errors_on_missing_text() {
        let tool = EchoTool;
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("text"));
    }

    #[test]
    fn echo_tool_name() {
        assert_eq!(Tool::name(&EchoTool), "echo_tool");
    }

    #[test]
    fn echo_tool_schema_has_required_text() {
        let schema = Tool::schema(&EchoTool);
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["text"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "text"));
    }

    #[test]
    fn new_registry_is_empty() {
        let r = ToolRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn register_increments_len() {
        let mut r = ToolRegistry::new();
        r.register(EchoTool);
        assert_eq!(r.len(), 1);
        assert!(!r.is_empty());
    }

    #[test]
    fn register_same_name_overwrites() {
        let mut r = ToolRegistry::new();
        r.register(EchoTool);
        r.register(EchoTool);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn names_returns_sorted_names() {
        let mut r = ToolRegistry::new();
        r.register(EchoTool);
        assert_eq!(r.names(), vec!["echo_tool"]);
    }

    #[test]
    fn get_schemas_includes_name_and_description() {
        let mut r = ToolRegistry::new();
        r.register(EchoTool);
        let schemas = r.get_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"], "echo_tool");
        assert!(schemas[0]["description"].is_string());
        assert!(schemas[0]["input_schema"].is_object());
    }

    #[tokio::test]
    async fn execute_known_tool_succeeds() {
        let mut r = ToolRegistry::new();
        r.register(EchoTool);
        let result = r.execute("echo_tool", json!({"text": "ping"})).await.unwrap();
        assert_eq!(result, "ping");
    }

    #[tokio::test]
    async fn execute_unknown_tool_errors() {
        let r = ToolRegistry::new();
        let result = r.execute("nonexistent", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown tool"));
    }
}
