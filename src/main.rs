/*!
 * Localcoder Rust Implementation
 *
 * Based on Claude Code v2.1.88 source analysis
 *
 * Core features:
 * 1. Connect to Ollama
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
mod engine;
mod markdown;
mod repl;
mod tools;
mod types;

use anyhow::Result;
use colored::*;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables
    dotenv::dotenv().ok();

    api::LLMClient::ensure_settings_file()?;

    // Print welcome banner
    print_banner();

    println!("{}", "🦙 Using Ollama".green().bold());

    // Register tools
    let mut registry = tools::ToolRegistry::new();
    registry.register(tools::EchoTool);
    registry.register(tools::ReadTool);
    registry.register(tools::EditTool);
    registry.register(tools::WriteTool);
    registry.register(tools::GlobTool);
    registry.register(tools::GrepTool);
    registry.register(tools::BashTool);

    // Get command-line arguments
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        // Interactive REPL mode
        repl::start_repl(registry).await?;
    } else {
        // Single-shot query mode
        let prompt = args.join(" ");
        one_shot(&prompt, registry).await?;
    }

    Ok(())
}

/// Print welcome banner
fn print_banner() {
    println!(
        "{}",
        "╔════════════════════════════════════════════════════════════╗".cyan()
    );
    println!(
        "{}",
        "║         Localcoder Minimal Version (Rust) - CLI Interface  ║".cyan()
    );
    println!(
        "{}",
        "╚════════════════════════════════════════════════════════════╝".cyan()
    );
    println!();
}

/// Single-shot query mode
async fn one_shot(prompt: &str, registry: tools::ToolRegistry) -> Result<()> {
    println!("{} {}", "💬 User:".green().bold(), prompt);
    println!();

    let client = api::LLMClient::new()?;

    println!("{}", "🤖 Model is thinking...\n".yellow());

    let mut messages = vec![serde_json::json!({"role": "user", "content": prompt})];

    match engine::run_agent_loop(&client, &registry, &mut messages).await {
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
