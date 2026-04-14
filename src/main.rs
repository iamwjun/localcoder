/*!
 * Localcoder Rust Implementation
 */

mod api;
mod compact;
mod config;
mod engine;
mod git;
mod markdown;
mod memory;
mod plan;
mod repl;
mod session;
mod skills;
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
    let cwd = env::current_dir()?;

    print_banner();
    println!("{}", "🦙 Using Ollama".green().bold());

    let plan_manager = plan::PlanManager::new(&cwd)?;
    let skill_manager = skills::SkillManager::new(&cwd)?;
    let mut registry = tools::ToolRegistry::new();
    registry.attach_plan_manager(plan_manager.clone());
    registry.attach_skill_manager(skill_manager.clone());
    registry.register(tools::EchoTool);
    registry.register(tools::ReadTool);
    registry.register(tools::EditTool);
    registry.register(tools::WriteTool);
    registry.register(tools::GlobTool);
    registry.register(tools::GrepTool);
    registry.register(tools::BashTool);
    registry.register(tools::EnterPlanModeTool::new(plan_manager.clone()));
    registry.register(tools::ExitPlanModeTool::new(plan_manager.clone()));
    registry.register(tools::TodoWriteTool::new(plan_manager.clone()));
    registry.register(tools::SkillTool::new(skill_manager.clone()));

    let args: Vec<String> = env::args().skip(1).collect();
    let (resume_target, prompt_args) = parse_args(args)?;

    if prompt_args.is_empty() {
        repl::start_repl(registry, resume_target).await?;
    } else {
        let prompt = prompt_args.join(" ");
        one_shot(&prompt, registry, plan_manager, skill_manager).await?;
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

async fn one_shot(
    prompt: &str,
    registry: tools::ToolRegistry,
    plan_manager: plan::PlanManager,
    skill_manager: skills::SkillManager,
) -> Result<()> {
    if prompt.trim() == "/plan" {
        println!("{}", "📋 Plan Status:".cyan().bold());
        println!("{}", plan_manager.render_status());
        return Ok(());
    }

    if prompt.trim() == "/skills" {
        println!("{}", "🧩 Available skills:".cyan().bold());
        println!("{}", skill_manager.render_user_invocable_list()?);
        return Ok(());
    }

    let client = api::LLMClient::new()?;
    let cwd = env::current_dir()?;
    let mut memory_store = memory::MemoryStore::new(&cwd, 0)?;
    let mut effective_prompt = prompt.trim().to_string();

    if let Some((skill_name, args)) = parse_slash_skill(prompt) {
        if skill_manager.has_user_invocable(skill_name)? {
            let resolved = skill_manager.resolve_and_activate(skill_name, args)?;
            effective_prompt = resolved.default_user_message(args);
            println!(
                "{} {} {}",
                "🧩 Skill:".cyan().bold(),
                resolved.name.white(),
                format!("[{} / {}]", resolved.loaded_from, resolved.context).dimmed()
            );
        }
    }

    println!("{} {}", "💬 User:".green().bold(), effective_prompt);
    println!();

    let system_prompt = merge_system_prompts([
        memory_store.build_system_prompt()?,
        skill_manager.build_system_prompt()?,
    ]);
    println!("{}", "🤖 Model is thinking...\n".yellow());

    let mut messages = vec![serde_json::json!({"role": "user", "content": effective_prompt})];

    match engine::run_agent_loop_with_system_prompt(
        &client,
        &registry,
        &mut messages,
        system_prompt.as_deref(),
    )
    .await
    {
        Ok(_) => {
            let saved = memory_store.extract_and_save(&client, &messages).await?;
            if !saved.is_empty() {
                println!(
                    "{} {}",
                    "🧠 Saved memories:".cyan().bold(),
                    saved
                        .iter()
                        .map(|m| format!("[{}] {}", m.memory_type, m.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
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

    #[test]
    fn parse_slash_skill_extracts_name_and_args() {
        assert_eq!(
            parse_slash_skill("/simplify src/main.rs"),
            Some(("simplify", "src/main.rs"))
        );
    }

    #[test]
    fn parse_slash_skill_handles_name_only() {
        assert_eq!(parse_slash_skill("/simplify"), Some(("simplify", "")));
    }

    #[test]
    fn parse_slash_skill_rejects_non_slash_input() {
        assert_eq!(parse_slash_skill("simplify"), None);
    }
}

fn merge_system_prompts<const N: usize>(parts: [Option<String>; N]) -> Option<String> {
    let joined = parts
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if joined.trim().is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn parse_slash_skill(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }

    match rest.split_once(char::is_whitespace) {
        Some((name, args)) => Some((name, args.trim())),
        None => Some((rest, "")),
    }
}
