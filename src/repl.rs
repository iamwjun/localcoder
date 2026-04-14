/*!
 * REPL Interactive Interface Module
 */

use crate::api::LLMClient;
use crate::compact;
use crate::config::{AppConfig, Theme};
use crate::engine;
use crate::git;
use crate::memory::MemoryStore;
use crate::session::SessionStore;
use crate::tools::ToolRegistry;
use crate::types::ConversationHistory;
use anyhow::{Context, Result};
use colored::*;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde_json::{Value, json};
use std::env;

#[derive(Debug, Clone)]
pub enum ResumeTarget {
    New,
    ContinueLatest,
    ResumeId(String),
}

/// Start the REPL interactive interface
pub async fn start_repl(registry: ToolRegistry, resume: ResumeTarget) -> Result<()> {
    let mut client = LLMClient::new()?;
    let cwd = env::current_dir().context("failed to resolve current directory")?;
    let mut app_config = AppConfig::load(&cwd)?;
    print_instructions(&client, &app_config);
    let (mut session, mut messages) = init_session(&cwd, resume)?;
    let mut memory_store = MemoryStore::new(&cwd, visible_message_count(&messages))?;
    if let Some(s) = &session {
        println!("{} {}", "🧾 Session:".cyan().bold(), s.id.as_str().white());
    } else {
        println!("{}", "🧾 Session: (not started yet)".cyan().bold());
    }

    let mut display_history = rebuild_display_history(&messages);

    let mut rl = DefaultEditor::new()?;

    loop {
        let readline = rl.readline(&format!("\n{} ", "💬 You >".green().bold()));

        match readline {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(input);

                if input.starts_with('/') {
                    if command_arg(input, "/resume").is_some() {
                        if let Err(e) =
                            handle_resume_command(
                                &mut rl,
                                &cwd,
                                &mut memory_store,
                                &mut session,
                                &mut messages,
                                &mut display_history,
                            )
                        {
                            eprintln!("\n{} {}", "❌ Resume failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if command_arg(input, "/model").is_some() {
                        if let Err(e) = select_model(&mut rl, &mut client).await {
                            eprintln!("\n{} {}", "❌ Failed to update model:".red().bold(), e);
                        }
                        continue;
                    }

                    if command_arg(input, "/config").is_some() {
                        if let Err(e) = handle_config_command(&mut rl, &cwd, &mut app_config) {
                            eprintln!("\n{} {}", "❌ Config failed:".red().bold(), e);
                        } else {
                            print_instructions(&client, &app_config);
                        }
                        continue;
                    }

                    if command_arg(input, "/compact").is_some() {
                        if let Err(e) = handle_manual_compact(
                            &client,
                            &cwd,
                            &mut session,
                            &mut messages,
                            &mut display_history,
                        )
                        .await
                        {
                            eprintln!("\n{} {}", "❌ Compact failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if command_arg(input, "/diff").is_some() {
                        if let Err(e) = handle_diff_command(&cwd) {
                            eprintln!("\n{} {}", "❌ Diff failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if command_arg(input, "/review").is_some() {
                        if let Err(e) = handle_review_command(&client, &cwd).await {
                            eprintln!("\n{} {}", "❌ Review failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if let Some(title) = command_arg(input, "/commit") {
                        let title = if title.is_empty() { None } else { Some(title) };
                        if let Err(e) = handle_commit_command(&mut rl, &client, &cwd, title).await {
                            eprintln!("\n{} {}", "❌ Commit failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if command_arg(input, "/memory").is_some() {
                        if let Err(e) = handle_memory_command(&memory_store) {
                            eprintln!("\n{} {}", "❌ Memory failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if handle_command(input, &mut display_history).await {
                        break;
                    }
                    continue;
                }

                messages.push(json!({"role": "user", "content": input}));
                display_history.add_user_message(input);
                ensure_session_started(&cwd, &mut session)?;
                session
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("session was not initialized"))?
                    .append_message(
                    messages
                        .last()
                        .ok_or_else(|| anyhow::anyhow!("missing just-added user message"))?,
                )?;

                println!("\n{}", "🤖 Model is thinking...\n".yellow());

                maybe_auto_compact(&client, &mut session, &mut messages, &mut display_history).await?;

                let before_len = messages.len();
                let system_prompt = memory_store.build_system_prompt()?;

                match engine::run_agent_loop_with_system_prompt(
                    &client,
                    &registry,
                    &mut messages,
                    system_prompt.as_deref(),
                )
                .await
                {
                    Ok(response) => {
                        if let Some(s) = &session {
                            s.append_messages(&messages[before_len..])?;
                        }
                        display_history = rebuild_display_history(&messages);
                        let saved_memories = memory_store.extract_and_save(&client, &messages).await?;
                        if !saved_memories.is_empty() {
                            println!(
                                "\n{} {}",
                                "🧠 Saved memories:".cyan().bold(),
                                saved_memories
                                    .iter()
                                    .map(|m| format!("[{}] {}", m.memory_type, m.name))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                        if response.is_empty() {
                            println!();
                        }
                    }
                    Err(e) => {
                        eprintln!("\n{} {}", "❌ Error:".red().bold(), e);
                        display_history = rebuild_display_history(&messages);
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

fn init_session(
    cwd: &std::path::Path,
    resume: ResumeTarget,
) -> Result<(Option<SessionStore>, Vec<Value>)> {
    match resume {
        ResumeTarget::New => Ok((None, Vec::new())),
        ResumeTarget::ContinueLatest => {
            if let Some(store) = SessionStore::load_latest(cwd)? {
                let messages = store.load_messages()?;
                println!(
                    "{} {}",
                    "🔄 Resumed latest session:".cyan().bold(),
                    store.id.as_str().white()
                );
                Ok((Some(store), messages))
            } else {
                println!(
                    "{}",
                    "ℹ️ No previous session found; session will start on first message".yellow()
                );
                Ok((None, Vec::new()))
            }
        }
        ResumeTarget::ResumeId(id) => {
            let store = SessionStore::load(cwd, &id)?;
            let messages = store.load_messages()?;
            println!(
                "{} {}",
                "🔄 Resumed session:".cyan().bold(),
                id.as_str().white()
            );
            Ok((Some(store), messages))
        }
    }
}

fn rebuild_display_history(messages: &[Value]) -> ConversationHistory {
    let mut history = ConversationHistory::new();
    for msg in messages {
        let Some(role) = msg["role"].as_str() else {
            continue;
        };
        let content = msg["content"].as_str().unwrap_or_default();
        match role {
            "user" => history.add_user_message(content),
            "assistant" => history.add_assistant_message(content),
            _ => {}
        }
    }
    history
}

fn visible_message_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|msg| matches!(msg["role"].as_str(), Some("user" | "assistant")))
        .count()
}

fn print_instructions(client: &LLMClient, app_config: &AppConfig) {
    println!("{}", "📝 Instructions:".cyan().bold());
    println!("  - Type a message and press Enter to send");
    if app_config.tips {
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
        println!("  - Type {} to resume previous session", "/resume".yellow());
        println!("  - Type {} to compact long context", "/compact".yellow());
        println!("  - Type {} to show git diff", "/diff".yellow());
        println!("  - Type {} to review current diff", "/review".yellow());
        println!("  - Type {} to generate and run git commit", "/commit".yellow());
        println!("  - Type {} to list saved memories", "/memory".yellow());
        println!("  - Type {} to open config menu", "/config".yellow());
        println!("  - Type {} to switch Ollama model", "/model".yellow());
        println!("  - Type {} to show help", "/help".yellow());
    } else {
        println!("  - Tips are disabled. Use {} for all commands", "/help".yellow());
    }
    println!();

    println!("{} {}", "🔧 Model:".cyan().bold(), client.model().white());
    println!(
        "{} theme={} tips={}",
        "⚙️  UI:".cyan().bold(),
        app_config.theme.to_string().white(),
        if app_config.tips { "on".green() } else { "off".red() }
    );
    println!(
        "{} {}",
        "🌐 Endpoint:".cyan().bold(),
        client.base_url().white()
    );
    println!();
}

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
    println!("  {}           - List and resume a previous session", "/resume".yellow());
    println!("  {}           - Manually compact conversation context", "/compact".yellow());
    println!("  {}              - Show current git diff", "/diff".yellow());
    println!("  {}            - Review current git diff with the model", "/review".yellow());
    println!("  {}    - Generate a commit message and commit", "/commit [title]".yellow());
    println!("  {}           - List saved memories", "/memory".yellow());
    println!(
        "  {}           - Configure UI settings (Theme / Tips)",
        "/config".yellow()
    );
    println!("  {}           - Select and persist Ollama model", "/model".yellow());
    println!("  {}             - Show this help", "/help".yellow());
    println!("  {}            - Show message count", "/count".yellow());
    println!("  {}          - Show current version", "/version".yellow());
    println!();
}

async fn maybe_auto_compact(
    client: &LLMClient,
    session: &mut Option<SessionStore>,
    messages: &mut Vec<Value>,
    display_history: &mut ConversationHistory,
) -> Result<()> {
    if compact::maybe_compact(client, messages).await? {
        if let Some(store) = session {
            store.replace_messages(messages)?;
        }
        *display_history = rebuild_display_history(messages);
        println!(
            "{} {}",
            "⚡ Context compacted automatically. Estimated tokens:".cyan().bold(),
            compact::estimate_tokens(messages)
        );
    }
    Ok(())
}

fn handle_memory_command(memory_store: &MemoryStore) -> Result<()> {
    println!("\n{}", "🧠 Saved memories:".cyan().bold());
    println!("{}", memory_store.render_memory_list()?);
    Ok(())
}

async fn handle_manual_compact(
    client: &LLMClient,
    cwd: &std::path::Path,
    session: &mut Option<SessionStore>,
    messages: &mut Vec<Value>,
    display_history: &mut ConversationHistory,
) -> Result<()> {
    ensure_session_started(cwd, session)?;

    if compact::force_compact(client, messages).await? {
        if let Some(store) = session {
            store.replace_messages(messages)?;
        }
        *display_history = rebuild_display_history(messages);
        println!(
            "{} {}",
            "✅ Context compacted. Estimated tokens:".green(),
            compact::estimate_tokens(messages)
        );
    } else {
        println!("{}", "ℹ️ Not enough history to compact yet".yellow());
    }

    Ok(())
}

fn handle_config_command(
    rl: &mut DefaultEditor,
    cwd: &std::path::Path,
    app_config: &mut AppConfig,
) -> Result<()> {
    println!("\n{}", "⚙️ Config Menu:".cyan().bold());
    println!("  1. Theme");
    println!("  2. Tips");
    println!();

    let input = rl.readline(&format!("{} ", "Select option (Enter to cancel) >".cyan().bold()))?;
    let input = input.trim();
    if input.is_empty() {
        println!("{}", "Config cancelled".yellow());
        return Ok(());
    }

    match input {
        "1" => configure_theme(rl, cwd, app_config)?,
        "2" => configure_tips(rl, cwd, app_config)?,
        _ => println!("{}", "Unknown config option".yellow()),
    }

    Ok(())
}

fn configure_theme(
    rl: &mut DefaultEditor,
    cwd: &std::path::Path,
    app_config: &mut AppConfig,
) -> Result<()> {
    println!("\n{}", "🎨 Theme:".cyan().bold());
    println!("  1. default");
    println!("  2. light");
    println!("  3. dark");
    println!("  current: {}", app_config.theme.to_string().white());
    println!();

    let input = rl.readline(&format!("{} ", "Select theme (Enter to cancel) >".cyan().bold()))?;
    let input = input.trim();
    if input.is_empty() {
        println!("{}", "Theme change cancelled".yellow());
        return Ok(());
    }

    let theme = match input {
        "1" => Theme::Default,
        "2" => Theme::Light,
        "3" => Theme::Dark,
        _ => {
            println!("{}", "Unknown theme option".yellow());
            return Ok(());
        }
    };

    app_config.theme = theme;
    let path = app_config.save(cwd)?;
    println!(
        "{} {} ({})",
        "✅ Theme updated:".green(),
        app_config.theme.to_string().white(),
        path.display()
    );
    Ok(())
}

fn configure_tips(
    rl: &mut DefaultEditor,
    cwd: &std::path::Path,
    app_config: &mut AppConfig,
) -> Result<()> {
    println!("\n{}", "💡 Tips:".cyan().bold());
    println!("  1. on");
    println!("  2. off");
    println!(
        "  current: {}",
        if app_config.tips { "on".green() } else { "off".red() }
    );
    println!();

    let input = rl.readline(&format!(
        "{} ",
        "Select tips mode (Enter to cancel) >".cyan().bold()
    ))?;
    let input = input.trim();
    if input.is_empty() {
        println!("{}", "Tips change cancelled".yellow());
        return Ok(());
    }

    app_config.tips = match input {
        "1" => true,
        "2" => false,
        _ => {
            println!("{}", "Unknown tips option".yellow());
            return Ok(());
        }
    };

    let path = app_config.save(cwd)?;
    println!(
        "{} {} ({})",
        "✅ Tips updated:".green(),
        if app_config.tips { "on".green() } else { "off".red() },
        path.display()
    );
    Ok(())
}

fn handle_resume_command(
    rl: &mut DefaultEditor,
    cwd: &std::path::Path,
    memory_store: &mut MemoryStore,
    session: &mut Option<SessionStore>,
    messages: &mut Vec<Value>,
    display_history: &mut ConversationHistory,
) -> Result<()> {
    let sessions = SessionStore::list(cwd)?;
    if sessions.is_empty() {
        println!("\n{}", "ℹ️ No saved sessions for current project".yellow());
        return Ok(());
    }

    println!("\n{}", "📚 Available sessions:".cyan().bold());
    for (index, s) in sessions.iter().enumerate() {
        let preview = session_last_user_preview(s).unwrap_or_else(|_| "(failed to load)".to_string());
        println!("  {}. {} {}", index + 1, preview.white(), format!("[{}]", s.id).dimmed());
    }
    println!();

    let prompt = format!(
        "{} ",
        "Select session number (Enter to cancel) >".cyan().bold()
    );
    let input = rl.readline(&prompt)?;
    let input = input.trim();
    if input.is_empty() {
        println!("{}", "Resume cancelled".yellow());
        return Ok(());
    }

    let index = input
        .parse::<usize>()
        .context("please enter a valid session number")?;
    if index == 0 || index > sessions.len() {
        anyhow::bail!("session number out of range");
    }

    let selected = sessions[index - 1].clone();
    let loaded_messages = selected.load_messages()?;

    *session = Some(selected);
    *messages = loaded_messages;
    memory_store.set_processed_visible_messages(visible_message_count(messages));
    *display_history = rebuild_display_history(messages);

    println!(
        "{} {}",
        "🔄 Resumed session:".green().bold(),
        session
            .as_ref()
            .map(|s| s.id.as_str())
            .unwrap_or("unknown")
            .white()
    );
    print_loaded_history(messages);

    Ok(())
}

fn session_last_user_preview(session: &SessionStore) -> Result<String> {
    let messages = session.load_messages()?;
    let last_user = messages
        .iter()
        .rev()
        .find(|m| m["role"].as_str() == Some("user"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or("(no user message)");

    Ok(truncate_preview(last_user, 48))
}

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    if out.is_empty() {
        "(empty)".to_string()
    } else {
        out
    }
}

fn ensure_session_started(cwd: &std::path::Path, session: &mut Option<SessionStore>) -> Result<()> {
    if session.is_none() {
        let created = SessionStore::create(cwd)?;
        println!(
            "{} {}",
            "🧾 Session started:".cyan().bold(),
            created.id.as_str().white()
        );
        *session = Some(created);
    }
    Ok(())
}

fn print_loaded_history(messages: &[Value]) {
    println!("\n{}", "📜 Loaded conversation history:".cyan().bold());
    if messages.is_empty() {
        println!("  {}", "(empty)".dimmed());
        return;
    }

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("unknown");
        let content = msg["content"].as_str().unwrap_or_default();
        match role {
            "user" => println!("  {} {}", "You:".green().bold(), content),
            "assistant" => println!("  {} {}", "Assistant:".blue().bold(), content),
            "tool" => println!(
                "  {} {}",
                "Tool:".yellow().bold(),
                msg["tool_name"].as_str().unwrap_or("unknown")
            ),
            _ => {}
        }
    }
}

fn command_arg<'a>(input: &'a str, command: &str) -> Option<&'a str> {
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

fn handle_diff_command(cwd: &std::path::Path) -> Result<()> {
    git::ensure_git_repo(cwd)?;
    let diff = git::get_combined_diff(cwd)?;
    if diff.trim().is_empty() {
        println!("{}", "ℹ️ No git changes to show".yellow());
    } else {
        println!("\n{}", "📄 Current diff:".cyan().bold());
        println!("{}", diff);
    }
    Ok(())
}

async fn handle_review_command(client: &LLMClient, cwd: &std::path::Path) -> Result<()> {
    git::ensure_git_repo(cwd)?;
    let diff = git::get_combined_diff(cwd)?;
    if diff.trim().is_empty() {
        println!("{}", "ℹ️ No git changes to review".yellow());
        return Ok(());
    }

    println!("{}", "🔍 Reviewing diff...\n".yellow());
    let prompt = format!(
        "请审查以下代码变更，重点关注：\n1. 潜在 bug 和行为回归\n2. 安全风险\n3. 测试缺口\n4. 可维护性问题\n\n请先给出 findings，再给出简短总结。\n\n变更内容：\n{}",
        truncate_preview(&diff, 12_000)
    );
    let review = client.complete_prompt(&prompt, 1200).await?;
    println!("{}", review);
    Ok(())
}

async fn handle_commit_command(
    rl: &mut DefaultEditor,
    client: &LLMClient,
    cwd: &std::path::Path,
    title: Option<&str>,
) -> Result<()> {
    git::ensure_git_repo(cwd)?;

    let mut diff = git::get_staged_diff(cwd)?;
    let had_staged = !diff.trim().is_empty();
    if !had_staged {
        diff = git::get_working_diff(cwd)?;
    }

    if diff.trim().is_empty() {
        println!("{}", "ℹ️ No git changes to commit".yellow());
        return Ok(());
    }

    let suggested = if let Some(title) = title {
        title.trim().to_string()
    } else {
        println!("{}", "📝 Generating commit message...\n".yellow());
        let prompt = format!(
            "请根据以下 git diff 生成一个简洁的 Conventional Commit 风格提交消息。只返回一行提交消息，不要解释。\n\nDiff:\n{}",
            truncate_preview(&diff, 12_000)
        );
        client
            .complete_prompt(&prompt, 120)
            .await?
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    };

    if suggested.is_empty() {
        anyhow::bail!("model returned an empty commit message");
    }

    println!("{} {}", "Suggested commit message:".cyan().bold(), suggested.white());
    let confirm = rl.readline(&format!("{} ", "Commit with this message? [Y/n/e] >".cyan().bold()))?;
    let confirm = confirm.trim().to_lowercase();

    let final_message = match confirm.as_str() {
        "" | "y" | "yes" => suggested,
        "e" | "edit" => {
            let edited = rl.readline(&format!("{} ", "Enter commit message >".cyan().bold()))?;
            let edited = edited.trim().to_string();
            if edited.is_empty() {
                println!("{}", "Commit cancelled".yellow());
                return Ok(());
            }
            edited
        }
        _ => {
            println!("{}", "Commit cancelled".yellow());
            return Ok(());
        }
    };

    if !had_staged {
        git::stage_all(cwd)?;
    }
    git::commit(cwd, &final_message)?;
    println!("{} {}", "✅ Committed:".green(), final_message.white());
    Ok(())
}

async fn select_model(rl: &mut DefaultEditor, client: &mut LLMClient) -> Result<()> {
    let models = client.list_models().await?;
    if models.is_empty() {
        println!("\n{}", "⚠️ No models found from /api/tags".yellow());
        return Ok(());
    }

    println!("\n{}", "📦 Available Ollama models:".cyan().bold());
    for (index, model) in models.iter().enumerate() {
        let current = if model == client.model() {
            " (current)".green().to_string()
        } else {
            String::new()
        };
        println!("  {}. {}{}", index + 1, model.white(), current);
    }

    println!();
    let prompt = format!(
        "{} ",
        "Select model number (Enter to cancel) >".cyan().bold()
    );
    let input = rl.readline(&prompt)?;
    let input = input.trim();
    if input.is_empty() {
        println!("{}", "Model selection cancelled".yellow());
        return Ok(());
    }

    let index = input
        .parse::<usize>()
        .context("please enter a valid model number")?;
    if index == 0 || index > models.len() {
        anyhow::bail!("model number out of range");
    }

    let model = models[index - 1].clone();
    let path = client.persist_model_to_home(&model)?;
    *client = LLMClient::new()?;

    println!(
        "{} {}",
        "✅ Active model updated:".green(),
        client.model().white()
    );
    println!("{} {}", "📝 Saved to:".cyan().bold(), path.display());

    Ok(())
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

    #[test]
    fn rebuild_display_history_only_user_assistant() {
        let messages = vec![
            json!({"role":"user","content":"u"}),
            json!({"role":"assistant","content":"a"}),
            json!({"role":"tool","content":"t"}),
        ];
        let h = rebuild_display_history(&messages);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn command_arg_matches_exact_command() {
        assert_eq!(command_arg("/commit", "/commit"), Some(""));
    }

    #[test]
    fn command_arg_extracts_argument() {
        assert_eq!(command_arg("/commit feat: add git", "/commit"), Some("feat: add git"));
    }

    #[test]
    fn command_arg_rejects_prefix_without_separator() {
        assert_eq!(command_arg("/commitment", "/commit"), None);
    }
}
