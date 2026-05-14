/*!
 * Localcoder Rust Implementation
 */

mod api;
mod compact;
mod config;
mod engine;
mod git;
mod memory;
mod output_style;
mod plan;
mod repl;
mod repl_completion;
mod runtime;
mod server;
mod services;
mod session;
mod skills;
mod terminal_style;
mod tools;
mod types;

use anyhow::{Result, anyhow};
use repl::ResumeTarget;
use std::env;

use crate::terminal_style::StyleExt;

#[tokio::main]
async fn main() -> Result<()> {
    api::LLMClient::ensure_settings_file()?;
    let cwd = env::current_dir()?;
    let client = api::LLMClient::new()?;
    let runtime = runtime::build_runtime(&cwd)?;

    print_banner();
    println!(
        "{} {}",
        "🤖 Using".green().bold(),
        client.provider_name().green().bold()
    );

    let args: Vec<String> = env::args().skip(1).collect();
    let (resume_target, prompt_args) = parse_args(args)?;

    if prompt_args.is_empty() {
        repl::start_repl(runtime.registry, resume_target).await?;
    } else {
        let prompt = prompt_args.join(" ");
        one_shot(
            &prompt,
            runtime.registry,
            runtime.output_style_manager,
            runtime.plan_manager,
            runtime.skill_manager,
        )
        .await?;
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
    output_style_manager: output_style::OutputStyleManager,
    plan_manager: plan::PlanManager,
    skill_manager: skills::SkillManager,
) -> Result<()> {
    let cwd = env::current_dir()?;
    let mut app_config = config::AppConfig::load(&cwd)?;
    if let Some(server_args) = parse_command_arg(prompt, "/server") {
        match server::parse_server_command(server_args)? {
            server::ServerCommand::Start(config) => {
                server::run_server_foreground(config, &cwd).await?;
            }
            server::ServerCommand::Status | server::ServerCommand::Stop => {
                anyhow::bail!("one-shot mode only supports /server or /server <host:port>");
            }
        }
        return Ok(());
    }

    if prompt.trim() == "/plan" {
        println!("{}", "📋 Plan Status:".cyan().bold());
        println!("{}", plan_manager.render_status());
        return Ok(());
    }

    if let Some(style_name) = parse_command_arg(prompt, "/output-style") {
        if style_name.is_empty() {
            println!("{}", "🎨 Available output styles:".cyan().bold());
            println!(
                "{}",
                output_style_manager.render_style_list(&app_config.output_style)?
            );
        } else if output_style_manager.has_style(style_name)? {
            app_config.output_style = style_name.trim().to_string();
            let path = app_config.save(&cwd)?;
            println!(
                "{} {} ({})",
                "✅ Output style updated:".green(),
                app_config.output_style.white(),
                path.display()
            );
        } else {
            anyhow::bail!("unknown output style: {}", style_name);
        }
        return Ok(());
    }

    if prompt.trim() == "/skills" {
        println!("{}", "🧩 Available skills:".cyan().bold());
        println!("{}", skill_manager.render_user_invocable_list()?);
        return Ok(());
    }

    if let Some(query) = parse_command_arg(prompt, "/web") {
        println!("{}", "🌐 Web search:".cyan().bold());
        println!("{}", tools::web_search::search_web(query, None, 5).await?);
        return Ok(());
    }

    if let Some(url) = parse_command_arg(prompt, "/fetch") {
        println!("{}", "🌐 Web fetch:".cyan().bold());
        println!("{}", tools::web_fetch::fetch_url(url, None, 12_000).await?);
        return Ok(());
    }

    let client = api::LLMClient::new()?;
    let memory_store = memory::MemoryStore::new(&cwd, 0)?;
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

    let system_prompt = runtime::build_base_system_prompt(
        &memory_store,
        &output_style_manager,
        &app_config.output_style,
        Some(&skill_manager),
    )?;
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
        Ok(response) => {
            if engine::response_needs_trailing_newline(&response) {
                println!();
            }
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

    #[test]
    fn parse_command_arg_extracts_argument() {
        assert_eq!(
            parse_command_arg("/web rust async", "/web"),
            Some("rust async")
        );
    }

    #[test]
    fn parse_command_arg_rejects_partial_prefix() {
        assert_eq!(parse_command_arg("/webbing x", "/web"), None);
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

fn parse_command_arg<'a>(input: &'a str, command: &str) -> Option<&'a str> {
    if !input.starts_with(command) {
        return None;
    }

    let rest = &input[command.len()..];
    if rest.is_empty() {
        return Some("");
    }

    if rest.starts_with(char::is_whitespace) {
        return Some(rest.trim());
    }

    None
}
