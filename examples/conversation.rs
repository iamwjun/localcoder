/*!
 * Example 3: Multi-turn Conversation
 *
 * Demonstrates how to maintain conversation context
 *
 * Run: cargo run --example conversation
 */

use anyhow::Result;
use colored::*;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    println!(
        "{}",
        "=== Example 3: Multi-turn Conversation ===\n".cyan().bold()
    );

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("Please set the ANTHROPIC_API_KEY environment variable");

    let client = reqwest::Client::new();

    // Conversation history
    let mut messages = Vec::new();

    // First turn
    messages.push(json!({
        "role": "user",
        "content": "My name is Alice, and I'm learning Rust"
    }));

    let mut body = json!({
        "model": "claude-opus-4-20250514",
        "max_tokens": 1024,
        "messages": messages.clone()
    });

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let result: serde_json::Value = response.json().await?;
    let assistant_response = result["content"][0]["text"].as_str().unwrap();

    println!(
        "{} My name is Alice, and I'm learning Rust",
        "User:".blue().bold()
    );
    println!("{} {}\n", "Claude:".green().bold(), assistant_response);

    // Add assistant response to history
    messages.push(json!({
        "role": "assistant",
        "content": assistant_response
    }));

    // Second turn - test context memory
    messages.push(json!({
        "role": "user",
        "content": "Do you remember my name? What am I learning?"
    }));

    body = json!({
        "model": "claude-opus-4-20250514",
        "max_tokens": 1024,
        "messages": messages
    });

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let result: serde_json::Value = response.json().await?;
    let assistant_response = result["content"][0]["text"].as_str().unwrap();

    println!(
        "{} Do you remember my name? What am I learning?",
        "User:".blue().bold()
    );
    println!("{} {}\n", "Claude:".green().bold(), assistant_response);

    println!(
        "{}",
        "✅ Conversation context preserved successfully!".green()
    );

    Ok(())
}
