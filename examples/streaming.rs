/*!
 * Example 2: Streaming Response
 *
 * Demonstrates how to handle streaming responses (Server-Sent Events)
 *
 * Run: cargo run --example streaming
 */

use anyhow::Result;
use colored::*;
use futures::stream::StreamExt;
use serde_json::json;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    println!("{}", "=== Example 2: Streaming Response ===\n".cyan().bold());

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("Please set the ANTHROPIC_API_KEY environment variable");

    let client = reqwest::Client::new();

    let body = json!({
        "model": "claude-opus-4-20250514",
        "max_tokens": 1024,
        "messages": [
            {
                "role": "user",
                "content": "Write a short poem about Rust programming (4 lines)"
            }
        ],
        "stream": true  // Enable streaming response
    });

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    println!("{}", "Claude: ".green().bold());

    // Handle streaming response
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete SSE events
        while let Some(pos) = buffer.find("\n\n") {
            let event = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            // Parse "data: {...}" format
            for line in event.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        continue;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        // Extract text delta
                        if json["type"] == "content_block_delta" {
                            if let Some(text) = json["delta"]["text"].as_str() {
                                print!("{}", text);
                                io::stdout().flush().ok();
                            }
                        }
                    }
                }
            }
        }
    }

    println!("\n");
    Ok(())
}
