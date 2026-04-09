/*!
 * REPL Interactive Interface Module
 *
 * Corresponds to: src/main.tsx - REPL implementation
 *
 * Features:
 * - Interactive command-line interface
 * - Conversation history management
 * - Command handling
 */

use crate::api::LLMClient;
use crate::engine;
use crate::tools::ToolRegistry;
use crate::types::ConversationHistory;
use anyhow::Result;
use colored::*;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde_json::{Value, json};
/// Start the REPL interactive interface
pub async fn start_repl(registry: ToolRegistry) -> Result<()> {
    // Print usage instructions
    let client = LLMClient::new()?;
    print_instructions(&client);

    // Conversation history as Vec<Value> (supports array content for tool calls)
    let mut messages: Vec<Value> = Vec::new();

    // Legacy text-only history for /history command display
    let mut display_history = ConversationHistory::new();

    // Create readline editor
    let mut rl = DefaultEditor::new()?;

    // Main loop
    loop {
        // Read user input
        let readline = rl.readline(&format!("\n{} ", "💬 You >".green().bold()));

        match readline {
            Ok(line) => {
                let input = line.trim();

                // Skip empty input
                if input.is_empty() {
                    continue;
                }

                // Add to readline history
                let _ = rl.add_history_entry(input);

                // Handle commands
                if input.starts_with('/') {
                    if handle_command(input, &mut display_history).await {
                        break; // Exit
                    }
                    continue;
                }

                // Append user message to conversation
                messages.push(json!({"role": "user", "content": input}));
                display_history.add_user_message(input);

                println!("\n{}", "🤖 Model is thinking...\n".yellow());

                // Run agent loop (handles tool calls internally)
                match engine::run_agent_loop(&client, &registry, &mut messages).await {
                    Ok(response) => {
                        display_history.add_assistant_message(&response);
                        println!();
                    }
                    Err(e) => {
                        eprintln!("\n{} {}", "❌ Error:".red().bold(), e);
                        // Pop the user message we just added to keep history consistent
                        messages.pop();
                        display_history.clear(); // rebuild from scratch on error
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("\n{}", "Use /exit or /quit to exit".yellow());
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("\n{}", "👋 Goodbye!".cyan());
                break;
            }
            Err(err) => {
                eprintln!("{} {:?}", "❌ Error reading input:".red().bold(), err);
                break;
            }
        }
    }

    Ok(())
}

/// Print usage instructions
fn print_instructions(client: &LLMClient) {
    println!("{}", "📝 Instructions:".cyan().bold());
    println!("  - Type a message and press Enter to send");
    println!(
        "  - Type {} or {} to exit",
        "/exit".yellow(),
        "/quit".yellow()
    );
    println!(
        "  - Type {} to clear conversation history",
        "/clear".yellow()
    );
    println!(
        "  - Type {} to view conversation history",
        "/history".yellow()
    );
    println!("  - Type {} to show help", "/help".yellow());
    println!();

    println!("{} {}", "🔧 Model:".cyan().bold(), client.model().white());
    println!(
        "{} {}",
        "🌐 Endpoint:".cyan().bold(),
        client.base_url().white()
    );
    println!();
}

/// Handle commands
/// Returns true to exit
async fn handle_command(command: &str, history: &mut ConversationHistory) -> bool {
    match command.to_lowercase().as_str() {
        "/exit" | "/quit" => {
            println!("\n{}", "👋 Goodbye!".cyan());
            return true;
        }

        "/clear" => {
            history.clear();
            println!("\n{}", "✅ Conversation history cleared".green());
        }

        "/history" => {
            println!("\n{}", "📜 Conversation history:".cyan().bold());
            if history.is_empty() {
                println!("  {}", "(empty)".dimmed());
            } else {
                match history.to_json() {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("{} {}", "❌ Serialization failed:".red(), e),
                }
            }
        }

        "/help" => {
            print_help();
        }

        "/count" => {
            println!("\n{} {}", "📊 Message count:".cyan().bold(), history.len());
        }

        "/version" => {
            println!(
                "\n{} {}",
                "📦 Version:".cyan().bold(),
                env!("CARGO_PKG_VERSION").white()
            );
        }

        _ => {
            println!("\n{} {}", "❌ Unknown command:".red().bold(), command);
            println!("Type {} to see available commands", "/help".yellow());
        }
    }

    false
}

/// Print help information
fn print_help() {
    println!("\n{}", "📖 Available commands:".cyan().bold());
    println!();
    println!("  {}          - Exit the program", "/exit, /quit".yellow());
    println!(
        "  {}            - Clear conversation history",
        "/clear".yellow()
    );
    println!(
        "  {}          - View conversation history (JSON format)",
        "/history".yellow()
    );
    println!("  {}             - Show this help", "/help".yellow());
    println!("  {}            - Show message count", "/count".yellow());
    println!("  {}          - Show current version", "/version".yellow());
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConversationHistory;

    #[tokio::test]
    async fn exit_command_returns_true() {
        let mut h = ConversationHistory::new();
        assert!(handle_command("/exit", &mut h).await);
    }

    #[tokio::test]
    async fn quit_command_returns_true() {
        let mut h = ConversationHistory::new();
        assert!(handle_command("/quit", &mut h).await);
    }

    #[tokio::test]
    async fn clear_command_empties_history() {
        let mut h = ConversationHistory::new();
        h.add_user_message("hello");
        h.add_assistant_message("hi");
        handle_command("/clear", &mut h).await;
        assert!(h.is_empty());
    }

    #[tokio::test]
    async fn clear_command_returns_false() {
        let mut h = ConversationHistory::new();
        assert!(!handle_command("/clear", &mut h).await);
    }

    #[tokio::test]
    async fn history_command_preserves_history() {
        let mut h = ConversationHistory::new();
        h.add_user_message("test");
        let result = handle_command("/history", &mut h).await;
        assert!(!result);
        assert_eq!(h.len(), 1);
    }

    #[tokio::test]
    async fn count_command_preserves_history() {
        let mut h = ConversationHistory::new();
        h.add_user_message("a");
        h.add_assistant_message("b");
        let result = handle_command("/count", &mut h).await;
        assert!(!result);
        assert_eq!(h.len(), 2);
    }

    #[tokio::test]
    async fn help_command_returns_false() {
        let mut h = ConversationHistory::new();
        assert!(!handle_command("/help", &mut h).await);
    }

    #[tokio::test]
    async fn version_command_returns_false() {
        let mut h = ConversationHistory::new();
        assert!(!handle_command("/version", &mut h).await);
    }

    #[tokio::test]
    async fn unknown_command_returns_false() {
        let mut h = ConversationHistory::new();
        assert!(!handle_command("/unknown", &mut h).await);
    }

    #[tokio::test]
    async fn unknown_command_does_not_modify_history() {
        let mut h = ConversationHistory::new();
        handle_command("/bogus", &mut h).await;
        assert!(h.is_empty());
    }

    #[tokio::test]
    async fn commands_are_case_insensitive() {
        let mut h = ConversationHistory::new();
        assert!(handle_command("/EXIT", &mut h).await);
    }
}
