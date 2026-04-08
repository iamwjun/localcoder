/*!
 * Agent Loop Engine — S01
 *
 * Corresponds to: src/query.ts — the while loop that runs until stop_reason != "tool_use"
 *
 * Core flow:
 *   loop {
 *     response = api.call_with_tools(messages, tool_schemas)   // stream → print text
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
use serde_json::{json, Value};

use crate::api::ClaudeClient;
use crate::tools::ToolRegistry;
use crate::types::ToolUseCall;

/// Run the agent loop until Claude reaches a terminal stop reason.
///
/// `messages` is mutated in-place: the function appends assistant messages and
/// tool_result messages as the conversation progresses, so the caller's history
/// stays up-to-date after the call returns.
///
/// Returns the final text response (already printed to stdout via streaming).
///
/// Corresponds to: src/query.ts:query() — the outer while loop
pub async fn run_agent_loop(
    client: &ClaudeClient,
    registry: &ToolRegistry,
    messages: &mut Vec<Value>,
) -> Result<String> {
    let tools = registry.get_schemas();
    let mut final_text = String::new();

    loop {
        // ── 1. Call Claude (streams text to stdout in real-time) ──────────
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
            let input: Value = serde_json::from_str(&call.input_json)
                .unwrap_or(Value::Null);

            println!(
                "{}",
                format!("▶ Tool: {} ({})", call.name, call.id).cyan()
            );

            let (content, is_error) = match registry.execute(&call.name, input).await {
                Ok(result) => (result, false),
                Err(e) => {
                    eprintln!("{} {}", "  ✗ Tool error:".red(), e);
                    (e.to_string(), true)
                }
            };

            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": call.id,
                "content": content,
                "is_error": is_error
            }));
        }

        // ── 5. Append tool results as user message and loop ───────────────
        messages.push(json!({
            "role": "user",
            "content": tool_results
        }));
    }

    Ok(final_text)
}

/// Build the assistant JSON message from a streaming response.
///
/// The content array must include both text blocks and tool_use blocks so that
/// Claude can match tool_use IDs in subsequent tool_result messages.
fn build_assistant_message(text: &str, tool_uses: &[ToolUseCall]) -> Value {
    let mut content: Vec<Value> = Vec::new();

    if !text.is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }

    for call in tool_uses {
        let input: Value = serde_json::from_str(&call.input_json).unwrap_or(Value::Null);
        content.push(json!({
            "type": "tool_use",
            "id": call.id,
            "name": call.name,
            "input": input
        }));
    }

    json!({"role": "assistant", "content": content})
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
        let content = msg["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
    }

    #[test]
    fn build_assistant_message_tool_only() {
        let calls = vec![ToolUseCall {
            id: "toolu_01".into(),
            name: "echo_tool".into(),
            input_json: r#"{"text":"hi"}"#.into(),
        }];
        let msg = build_assistant_message("", &calls);
        let content = msg["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["id"], "toolu_01");
        assert_eq!(content[0]["name"], "echo_tool");
        assert_eq!(content[0]["input"]["text"], "hi");
    }

    #[test]
    fn build_assistant_message_text_and_tool() {
        let calls = vec![ToolUseCall {
            id: "toolu_02".into(),
            name: "echo_tool".into(),
            input_json: r#"{"text":"world"}"#.into(),
        }];
        let msg = build_assistant_message("I'll echo this:", &calls);
        let content = msg["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "tool_use");
    }

    #[test]
    fn build_assistant_message_invalid_json_input() {
        let calls = vec![ToolUseCall {
            id: "toolu_03".into(),
            name: "echo_tool".into(),
            input_json: "not-json".into(),
        }];
        let msg = build_assistant_message("", &calls);
        let content = msg["content"].as_array().unwrap();
        assert_eq!(content[0]["input"], Value::Null);
    }
}
