/*!
 * Claude Code Rust Implementation
 *
 * Based on Claude Code v2.1.88 source analysis
 *
 * Core features:
 * 1. Connect to Claude API
 * 2. Streaming response handling
 * 3. Conversation history management
 * 4. REPL interactive interface
 *
 * Source references:
 * - src/services/api/claude.ts - API client implementation
 * - src/query.ts - Main query loop
 * - src/QueryEngine.ts - Session engine
 */

mod api;
mod markdown;
mod repl;
mod types;

use anyhow::Result;
use colored::*;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables
    dotenv::dotenv().ok();

    // Print welcome banner
    print_banner();

    // Get API Key (optional for Ollama)
    let api_key = env::var("ANTHROPIC_AUTH_TOKEN").unwrap_or_default();

    // Check if using Ollama
    let use_ollama = api_key.is_empty() || env::var("USE_OLLAMA").is_ok();

    if use_ollama {
        println!(
            "{} {}",
            "🦙 Using Ollama:".green().bold(),
            "Local model at http://localhost:11434"
        );
    }

    // Get command-line arguments
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        // Interactive REPL mode
        repl::start_repl(&api_key).await?;
    } else {
        // Single-shot query mode
        let prompt = args.join(" ");
        one_shot(&api_key, &prompt).await?;
    }

    Ok(())
}

/// Print welcome banner
fn print_banner() {
    println!("{}", "╔════════════════════════════════════════════════════════════╗".cyan());
    println!("{}", "║      Claude Code Minimal Version (Rust) - CLI Interface    ║".cyan());
    println!("{}", "╚════════════════════════════════════════════════════════════╝".cyan());
    println!();
}

/// Single-shot query mode
async fn one_shot(api_key: &str, prompt: &str) -> Result<()> {
    println!("{} {}", "💬 User:".green().bold(), prompt);
    println!();

    let client = api::ClaudeClient::new(api_key)?;

    println!("{}", "🤖 Claude is thinking...\n".yellow());

    match client.query_streaming(prompt, &[]).await {
        Ok(_) => {
            println!("\n");
            println!("{}", "✅ Done".green());
            Ok(())
        }
        Err(e) => {
            eprintln!("{} {}", "❌ Error:".red().bold(), e);
            Err(e)
        }
    }
}
