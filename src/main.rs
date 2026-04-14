/*!
 * Localcoder Rust Implementation
 */

mod api;
mod config;
mod engine;
mod markdown;
mod repl;
mod session;
mod tools;
mod types;

use anyhow::{Result, anyhow};
use colored::*;
use repl::ResumeTarget;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    api::LLMClient::ensure_settings_file()?;

    print_banner();
    println!("{}", "🦙 Using Ollama".green().bold());

    let mut registry = tools::ToolRegistry::new();
    registry.register(tools::EchoTool);
    registry.register(tools::ReadTool);
    registry.register(tools::EditTool);
    registry.register(tools::WriteTool);
    registry.register(tools::GlobTool);
    registry.register(tools::GrepTool);
    registry.register(tools::BashTool);

    let args: Vec<String> = env::args().skip(1).collect();
    let (resume_target, prompt_args) = parse_args(args)?;

    if prompt_args.is_empty() {
        repl::start_repl(registry, resume_target).await?;
    } else {
        let prompt = prompt_args.join(" ");
        one_shot(&prompt, registry).await?;
    }

    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<(ResumeTarget, Vec<String>)> {
    let mut prompt_args: Vec<String> = Vec::new();
    let mut resume_target = ResumeTarget::New;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--continue" => {
                resume_target = ResumeTarget::ContinueLatest;
                i += 1;
            }
            "--resume" => {
                let id = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--resume requires a session id"))?;
                resume_target = ResumeTarget::ResumeId(id.clone());
                i += 2;
            }
            other => {
                prompt_args.push(other.to_string());
                i += 1;
            }
        }
    }

    Ok((resume_target, prompt_args))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_continue() {
        let (resume, prompt) = parse_args(vec!["--continue".into()]).unwrap();
        assert!(matches!(resume, ResumeTarget::ContinueLatest));
        assert!(prompt.is_empty());
    }

    #[test]
    fn parse_args_resume_with_id() {
        let (resume, prompt) = parse_args(vec!["--resume".into(), "abc123".into()]).unwrap();
        match resume {
            ResumeTarget::ResumeId(id) => assert_eq!(id, "abc123"),
            _ => panic!("expected ResumeId"),
        }
        assert!(prompt.is_empty());
    }

    #[test]
    fn parse_args_prompt_only() {
        let (resume, prompt) = parse_args(vec!["hello".into(), "world".into()]).unwrap();
        assert!(matches!(resume, ResumeTarget::New));
        assert_eq!(prompt, vec!["hello", "world"]);
    }
}
