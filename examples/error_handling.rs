/*!
 * Error Handling Example
 *
 * Demonstrates how to handle various API errors
 *
 * Run: cargo run --example error_handling
 */

use anyhow::Result;
use colored::*;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    println!("{}", "=== Example 5: Error Handling ===\n".cyan().bold());

    // Example 1: Invalid API Key
    println!("{}", "1️⃣  Testing invalid API Key...".yellow());
    test_invalid_api_key().await;
    println!();

    // Example 2: Empty message
    println!("{}", "2️⃣  Testing empty message...".yellow());
    test_empty_message().await;
    println!();

    // Example 3: Excessive tokens
    println!("{}", "3️⃣  Testing excessive max_tokens...".yellow());
    test_invalid_max_tokens().await;
    println!();

    println!("{}", "✅ All error handling tests complete!".green().bold());

    Ok(())
}

async fn test_invalid_api_key() {
    let client = reqwest::Client::new();

    let body = json!({
        "model": "claude-opus-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}]
    });

    match client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", "invalid-key")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                println!(
                    "{}",
                    format!("   ✓ Caught expected error: {} - {}", status, error_text).green()
                );
            }
        }
        Err(e) => {
            println!("{}", format!("   ✓ Caught network error: {}", e).green());
        }
    }
}

async fn test_empty_message() {
    let result = validate_message("");
    match result {
        Ok(_) => println!("{}", "   ✗ Should have rejected empty message".red()),
        Err(e) => println!("{}", format!("   ✓ Correctly rejected: {}", e).green()),
    }
}

async fn test_invalid_max_tokens() {
    let result = validate_max_tokens(1000000);
    match result {
        Ok(_) => println!("{}", "   ✗ Should have rejected excessive max_tokens".red()),
        Err(e) => println!("{}", format!("   ✓ Correctly rejected: {}", e).green()),
    }
}

fn validate_message(content: &str) -> Result<()> {
    if content.trim().is_empty() {
        anyhow::bail!("Message content cannot be empty");
    }
    Ok(())
}

fn validate_max_tokens(max_tokens: u32) -> Result<()> {
    const MAX_ALLOWED: u32 = 100000;
    if max_tokens > MAX_ALLOWED {
        anyhow::bail!("max_tokens cannot exceed {}", MAX_ALLOWED);
    }
    Ok(())
}
