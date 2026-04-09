/*!
 * Agent Loop Engine — S01
 *
 * Corresponds to: src/query.ts — the while loop that runs until stop_reason != "tool_use"
 *
 * Core flow:
 *   loop {
 *     response = api.call_with_tools(messages, tool_schemas)
 *     append assistant message to messages
 *     if stop_reason == "tool_use" {
 *         for each tool_use call → execute tool → append tool_result
 *         continue
 *     } else {
 *         return final text
 *     }
 *   }
 */

use anyhow::Result;
use colored::*;
use serde_json::{Value, json};

use crate::api::LLMClient;
use crate::tools::ToolRegistry;
use crate::types::ToolUseCall;

/// Run the agent loop until the model reaches a terminal stop reason.
///
/// `messages` is mutated in-place: the function appends assistant messages and
/// tool_result messages as the conversation progresses, so the caller's history
/// stays up-to-date after the call returns.
///
pub async fn run_agent_loop(
    client: &LLMClient,
    registry: &ToolRegistry,
    messages: &mut Vec<Value>,
) -> Result<String> {
    let tools = registry.get_schemas();
    let mut final_text = String::new();

    loop {
        // ── 1. Call the model ─────────────────────────────────────────────
        let response = client.call_with_tools(messages, &tools).await?;
        final_text = response.text.clone();

        // ── 2. Append assistant message to conversation history ───────────
        messages.push(build_assistant_message(&response.text, &response.tool_uses));

        // ── 3. Check stop reason ──────────────────────────────────────────
        if response.stop_reason != "tool_use" || response.tool_uses.is_empty() {
            break;
        }

        // ── 4. Execute tool calls and collect results ─────────────────────
        println!();
        let mut tool_results: Vec<Value> = Vec::new();

        for call in &response.tool_uses {
            println!("{}", format!("▶ Tool: {}", call.name).cyan());

            let (content, is_error) =
                match registry.execute(&call.name, call.arguments.clone()).await {
                    Ok(result) => (result, false),
                    Err(e) => {
                        eprintln!("{} {}", "  ✗ Tool error:".red(), e);
                        (e.to_string(), true)
                    }
                };

            tool_results.push(json!({
                "role": "tool",
                "tool_name": call.name,
                "content": content,
                "is_error": is_error
            }));
        }

        // ── 5. Append tool results and loop ───────────────────────────────
        messages.extend(tool_results);
    }

    Ok(final_text)
}

/// Build the assistant JSON message from an Ollama response.
fn build_assistant_message(text: &str, tool_uses: &[ToolUseCall]) -> Value {
    let mut message = json!({
        "role": "assistant",
        "content": text
    });

    if !tool_uses.is_empty() {
        let tool_calls: Vec<Value> = tool_uses
            .iter()
            .map(|call| {
                json!({
                    "function": {
                        "name": call.name,
                        "arguments": call.arguments
                    }
                })
            })
            .collect();
        message["tool_calls"] = Value::Array(tool_calls);
    }

    message
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolUseCall;

    #[test]
    fn build_assistant_message_text_only() {
        let msg = build_assistant_message("hello", &[]);
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "hello");
        assert!(msg.get("tool_calls").is_none());
    }

    #[test]
    fn build_assistant_message_tool_only() {
        let calls = vec![ToolUseCall {
            name: "echo_tool".into(),
            arguments: json!({"text":"hi"}),
        }];
        let msg = build_assistant_message("", &calls);
        let tool_calls = msg["tool_calls"].as_array().unwrap();
        assert_eq!(msg["content"], "");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "echo_tool");
        assert_eq!(tool_calls[0]["function"]["arguments"]["text"], "hi");
    }

    #[test]
    fn build_assistant_message_text_and_tool() {
        let calls = vec![ToolUseCall {
            name: "echo_tool".into(),
            arguments: json!({"text":"world"}),
        }];
        let msg = build_assistant_message("I'll echo this:", &calls);
        let tool_calls = msg["tool_calls"].as_array().unwrap();
        assert_eq!(msg["content"], "I'll echo this:");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "echo_tool");
    }
}
