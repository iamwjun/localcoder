/*!
 * REPL Interactive Interface Module
 */

use crate::api::LLMClient;
use crate::compact;
use crate::config::{AppConfig, Theme};
use crate::engine;
use crate::git;
use crate::memory::MemoryStore;
use crate::output_style::OutputStyleManager;
use crate::plan::PlanManager;
use crate::repl_completion::{ReplCommandSpec, build_slash_commands};
use crate::runtime;
use crate::server::{self, ServerHandle};
use crate::session::SessionStore;
use crate::skills::SkillManager;
use crate::terminal_style::StyleExt;
use crate::tools::ToolRegistry;
use crate::tools::web_fetch::fetch_url;
use crate::tools::web_search::search_web;
use crate::types::ConversationHistory;
use anyhow::{Context, Result};
use oxink::input::{
    InputAction, InputOption, InputRenderer, InputTheme, KeyCode, KeyEvent, SlashInput,
    TerminalColor,
};
use oxink::styles::ANSI_STYLES;
use serde_json::{Value, json};
use std::env;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum ResumeTarget {
    New,
    ContinueLatest,
    ResumeId(String),
}

struct ReplEditor {
    slash_commands: Vec<ReplCommandSpec>,
    theme: InputTheme,
}

enum PromptSignal {
    Submitted(String),
    Interrupted,
    Eof,
}

#[derive(Clone, Copy)]
enum SuggestionBehavior {
    KeepEditing,
    ReturnImmediately,
}

enum AppEvent {
    Interrupted,
    Eof,
    Key(KeyEvent),
}

struct TerminalMode {
    original_state: String,
}

struct SessionInit {
    session: Option<SessionStore>,
    messages: Vec<Value>,
    status: String,
}

impl TerminalMode {
    fn enter() -> io::Result<Self> {
        let original_state = run_stty_capture(["-g"])?;
        run_stty(["raw", "-echo"])?;

        let mut tty = open_tty_writer()?;
        write!(tty, "\x1B[?25h")?;
        tty.flush()?;

        Ok(Self {
            original_state: original_state.trim().to_string(),
        })
    }
}

impl Drop for TerminalMode {
    fn drop(&mut self) {
        let _ = run_stty([self.original_state.as_str()]);
        if let Ok(mut tty) = open_tty_writer() {
            let _ = write!(tty, "\x1B[?25h");
            let _ = tty.flush();
        }
    }
}

/// Start the REPL interactive interface
pub async fn start_repl(registry: ToolRegistry, resume: ResumeTarget) -> Result<()> {
    let mut client = LLMClient::new()?;
    let cwd = env::current_dir().context("failed to resolve current directory")?;
    let mut app_config = AppConfig::load(&cwd)?;
    let output_style_manager = OutputStyleManager::new(&cwd);
    let session_init = init_session(&cwd, resume)?;
    let mut session = session_init.session;
    let mut messages = session_init.messages;
    let mut memory_store = MemoryStore::new(&cwd, visible_message_count(&messages))?;
    let plan_manager = registry.plan_manager();
    let skill_manager = registry.skill_manager();
    if let Some(manager) = &skill_manager {
        manager.set_session_id(session.as_ref().map(|s| s.id.as_str()));
    }
    print_startup_intro(&client, &app_config, &session_init.status);

    let mut display_history = rebuild_display_history(&messages);
    let mut server_handle: Option<ServerHandle> = None;
    let mut rl = build_repl_editor(skill_manager.as_ref())?;

    loop {
        refresh_repl_commands(&mut rl, skill_manager.as_ref());
        let footer_lines = build_main_prompt_footer_lines(&client);
        match rl.read_main_input(&footer_lines) {
            Ok(PromptSignal::Submitted(line)) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                if input.starts_with('/') {
                    if command_arg(input, "/resume").is_some() {
                        if let Err(e) = handle_resume_command(
                            &mut rl,
                            &cwd,
                            skill_manager.as_ref(),
                            &mut memory_store,
                            &mut session,
                            &mut messages,
                            &mut display_history,
                        ) {
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

                    if let Some(style_name) = command_arg(input, "/output-style") {
                        match handle_output_style_command(
                            &cwd,
                            &output_style_manager,
                            &mut app_config,
                            style_name,
                        ) {
                            Ok(_changed) => {}
                            Err(e) => {
                                eprintln!("\n{} {}", "❌ Output style failed:".red().bold(), e);
                            }
                        }
                        continue;
                    }

                    if let Some(query) = command_arg(input, "/web") {
                        if let Err(e) = handle_web_search_command(query).await {
                            eprintln!("\n{} {}", "❌ Web search failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if let Some(url) = command_arg(input, "/fetch") {
                        if let Err(e) = handle_web_fetch_command(url).await {
                            eprintln!("\n{} {}", "❌ Web fetch failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if let Some(args) = command_arg(input, "/server") {
                        if let Err(e) = handle_server_command(&cwd, args, &mut server_handle).await
                        {
                            eprintln!("\n{} {}", "❌ Server command failed:".red().bold(), e);
                        }
                        continue;
                    }

                    if let Some(args) = command_arg(input, "/plan") {
                        if let Some(manager) = plan_manager.as_ref() {
                            if let Err(e) = handle_plan_command(manager, args) {
                                eprintln!("\n{} {}", "❌ Plan failed:".red().bold(), e);
                            }
                        } else {
                            eprintln!("\n{}", "❌ Plan mode is not initialized".red().bold());
                        }
                        continue;
                    }

                    if command_arg(input, "/skills").is_some() {
                        if let Some(manager) = skill_manager.as_ref() {
                            if let Err(e) = handle_skills_command(manager) {
                                eprintln!("\n{} {}", "❌ Skills failed:".red().bold(), e);
                            }
                        } else {
                            eprintln!("\n{}", "❌ Skills are not initialized".red().bold());
                        }
                        continue;
                    }

                    if let Some(manager) = skill_manager.as_ref() {
                        if let Some((skill_name, skill_args)) = parse_slash_skill(input) {
                            match manager.has_user_invocable(skill_name) {
                                Ok(true) => {
                                    if let Err(e) = handle_skill_command(
                                        &client,
                                        &registry,
                                        &output_style_manager,
                                        &app_config,
                                        manager,
                                        &cwd,
                                        &mut session,
                                        &mut memory_store,
                                        &mut messages,
                                        &mut display_history,
                                        skill_name,
                                        skill_args,
                                    )
                                    .await
                                    {
                                        eprintln!("\n{} {}", "❌ Skill failed:".red().bold(), e);
                                    }
                                    continue;
                                }
                                Ok(false) => {}
                                Err(e) => {
                                    eprintln!("\n{} {}", "❌ Skill lookup failed:".red().bold(), e);
                                    continue;
                                }
                            }
                        }
                    }

                    if handle_command(input, &mut display_history).await {
                        break;
                    }
                    continue;
                }

                if let Err(e) = run_user_turn(
                    &client,
                    &registry,
                    &output_style_manager,
                    &app_config,
                    skill_manager.as_ref(),
                    &cwd,
                    &mut session,
                    &mut memory_store,
                    &mut messages,
                    &mut display_history,
                    input,
                )
                .await
                {
                    eprintln!("\n{} {}", "❌ Error:".red().bold(), e);
                }
            }
            Ok(PromptSignal::Interrupted) => {
                break;
            }
            Ok(PromptSignal::Eof) => {
                break;
            }
            Err(err) => {
                eprintln!("{} {}", "❌ Error reading input:".red().bold(), err);
                break;
            }
        }
    }

    if let Some(handle) = server_handle.take() {
        let _ = handle.stop().await;
    }

    Ok(())
}

fn build_repl_editor(skill_manager: Option<&SkillManager>) -> Result<ReplEditor> {
    Ok(ReplEditor {
        slash_commands: build_slash_commands(skill_manager)?,
        theme: InputTheme::new()
            .with_border_color(TerminalColor::Ansi256(240))
            .with_text_color(TerminalColor::Ansi256(255))
            .with_background_color(TerminalColor::Ansi256(236)),
    })
}

fn refresh_repl_commands(rl: &mut ReplEditor, skill_manager: Option<&SkillManager>) {
    rl.slash_commands = match build_slash_commands(skill_manager) {
        Ok(commands) => commands,
        Err(err) => {
            eprintln!(
                "\n{} {}",
                "⚠️ Failed to refresh slash commands:".yellow().bold(),
                err
            );
            build_slash_commands(None).unwrap_or_default()
        }
    };
}

impl ReplEditor {
    fn read_main_input(&mut self, footer_lines: &[String]) -> Result<PromptSignal> {
        let options = self
            .slash_commands
            .iter()
            .map(|command| {
                let description = if command.takes_args {
                    format!("{}  {}", command.usage, command.summary)
                } else {
                    command.summary.clone()
                };
                InputOption::new(command.name.clone(), description)
            })
            .collect::<Vec<_>>();
        self.run_input_session(
            &[],
            footer_lines,
            None,
            &options,
            SuggestionBehavior::KeepEditing,
        )
    }

    fn read_line_with_initial(&mut self, title: &str, initial: &str) -> Result<String> {
        let header_lines = vec![title.to_string()];
        match self.run_input_session(
            &header_lines,
            &[],
            Some(initial),
            &[],
            SuggestionBehavior::KeepEditing,
        )? {
            PromptSignal::Submitted(value) => Ok(value),
            PromptSignal::Interrupted | PromptSignal::Eof => Ok(String::new()),
        }
    }

    fn select_option(&mut self, title: &str, options: &[InputOption]) -> Result<Option<String>> {
        let header_lines = vec![title.to_string()];
        match self.run_input_session(
            &header_lines,
            &[],
            Some("/"),
            options,
            SuggestionBehavior::ReturnImmediately,
        )? {
            PromptSignal::Submitted(value) => Ok(Some(trim_selection_value(&value))),
            PromptSignal::Interrupted | PromptSignal::Eof => Ok(None),
        }
    }

    fn run_input_session(
        &mut self,
        header_lines: &[String],
        footer_lines: &[String],
        initial: Option<&str>,
        options: &[InputOption],
        suggestion_behavior: SuggestionBehavior,
    ) -> Result<PromptSignal> {
        let _terminal_mode = TerminalMode::enter()?;
        let terminal_columns = terminal_columns();
        let mut stdout = open_tty_writer()?;
        let mut stdin = stdout.try_clone()?;
        let fitted_options = fit_options_to_terminal(options, terminal_columns);
        let mut renderer = InputRenderer::new(terminal_columns);
        let mut input = SlashInput::new(fitted_options)
            .with_header_lines(header_lines.iter().cloned())
            .with_theme(self.theme.clone());

        if let Some(initial) = initial.filter(|value| !value.is_empty()) {
            input.handle_paste(initial);
        }

        render_input_frame(
            &mut renderer,
            &mut stdout,
            &input,
            footer_lines,
            terminal_columns,
        )?;

        let outcome = loop {
            let Some(event) = read_event(&mut stdin)? else {
                continue;
            };

            match event {
                AppEvent::Interrupted => break PromptSignal::Interrupted,
                AppEvent::Eof => break PromptSignal::Eof,
                AppEvent::Key(event) => match input.handle_key(event) {
                    InputAction::None => {}
                    InputAction::CopyRequested(_) => {}
                    InputAction::PasteRequested => {
                        if let Some(text) = read_clipboard_text() {
                            input.handle_paste(text);
                        }
                    }
                    InputAction::SuggestionApplied(value) => {
                        if matches!(suggestion_behavior, SuggestionBehavior::ReturnImmediately) {
                            break PromptSignal::Submitted(value);
                        }
                    }
                    InputAction::Submitted(value) => break PromptSignal::Submitted(value),
                },
            }

            render_input_frame(
                &mut renderer,
                &mut stdout,
                &input,
                footer_lines,
                terminal_columns,
            )?;
        };

        renderer.clear(&mut stdout)?;
        Ok(outcome)
    }
}

fn render_input_frame<W: Write>(
    renderer: &mut InputRenderer,
    output: &mut W,
    input: &SlashInput,
    footer_lines: &[String],
    terminal_columns: usize,
) -> io::Result<()> {
    let mut view = input.render_with_terminal_width(terminal_columns);

    if footer_lines.is_empty() || input.is_dropdown_visible() {
        return renderer.render_view(output, &view);
    }

    view.lines.extend(footer_lines.iter().cloned());
    renderer.render_view(output, &view)
}

fn trim_selection_value(value: &str) -> String {
    value.trim().trim_start_matches('/').trim_end().to_string()
}

fn read_event<R: Read>(reader: &mut R) -> io::Result<Option<AppEvent>> {
    let byte = match read_byte(reader)? {
        Some(byte) => byte,
        None => return Ok(None),
    };

    let event = match byte {
        0x04 => AppEvent::Eof,
        0x03 => AppEvent::Interrupted,
        0x16 => AppEvent::Key(KeyEvent::ctrl(KeyCode::Char('v'))),
        b'\r' | b'\n' => AppEvent::Key(KeyEvent::plain(KeyCode::Enter)),
        b'\t' => AppEvent::Key(KeyEvent::plain(KeyCode::Tab)),
        0x7f | 0x08 => AppEvent::Key(KeyEvent::plain(KeyCode::Backspace)),
        0x1b => match read_escape_sequence(reader)? {
            Some(event) => AppEvent::Key(event),
            None => return Ok(None),
        },
        byte if byte.is_ascii_control() => return Ok(None),
        byte => AppEvent::Key(KeyEvent::plain(KeyCode::Char(read_char(reader, byte)?))),
    };

    Ok(Some(event))
}

fn read_escape_sequence<R: Read>(reader: &mut R) -> io::Result<Option<KeyEvent>> {
    let Some(prefix) = read_byte(reader)? else {
        return Ok(Some(KeyEvent::plain(KeyCode::Esc)));
    };

    let key = match prefix {
        b'[' => read_csi_sequence(reader)?,
        b'O' => read_ss3_sequence(reader)?,
        _ => KeyEvent::plain(KeyCode::Esc),
    };

    Ok(Some(key))
}

fn read_csi_sequence<R: Read>(reader: &mut R) -> io::Result<KeyEvent> {
    let Some(byte) = read_byte(reader)? else {
        return Ok(KeyEvent::plain(KeyCode::Esc));
    };

    let key = match byte {
        b'A' => KeyEvent::plain(KeyCode::Up),
        b'B' => KeyEvent::plain(KeyCode::Down),
        b'C' => KeyEvent::plain(KeyCode::Right),
        b'D' => KeyEvent::plain(KeyCode::Left),
        b'H' => KeyEvent::plain(KeyCode::Home),
        b'F' => KeyEvent::plain(KeyCode::End),
        b'1' | b'3' | b'4' | b'7' | b'8' => {
            let Some(suffix) = read_byte(reader)? else {
                return Ok(KeyEvent::plain(KeyCode::Esc));
            };

            match (byte, suffix) {
                (b'1', b'~') | (b'7', b'~') => KeyEvent::plain(KeyCode::Home),
                (b'3', b'~') => KeyEvent::plain(KeyCode::Delete),
                (b'4', b'~') | (b'8', b'~') => KeyEvent::plain(KeyCode::End),
                _ => KeyEvent::plain(KeyCode::Esc),
            }
        }
        _ => KeyEvent::plain(KeyCode::Esc),
    };

    Ok(key)
}

fn read_ss3_sequence<R: Read>(reader: &mut R) -> io::Result<KeyEvent> {
    let Some(byte) = read_byte(reader)? else {
        return Ok(KeyEvent::plain(KeyCode::Esc));
    };

    let key = match byte {
        b'H' => KeyEvent::plain(KeyCode::Home),
        b'F' => KeyEvent::plain(KeyCode::End),
        _ => KeyEvent::plain(KeyCode::Esc),
    };

    Ok(key)
}

fn read_byte<R: Read>(reader: &mut R) -> io::Result<Option<u8>> {
    let mut byte = [0u8; 1];
    match reader.read(&mut byte)? {
        0 => Ok(None),
        _ => Ok(Some(byte[0])),
    }
}

fn read_char<R: Read>(reader: &mut R, first_byte: u8) -> io::Result<char> {
    if first_byte.is_ascii() {
        return Ok(first_byte as char);
    }

    let width = utf8_width(first_byte)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid utf-8 lead byte"))?;
    let mut buffer = vec![0; width];
    buffer[0] = first_byte;

    if width > 1 {
        reader.read_exact(&mut buffer[1..])?;
    }

    let text = std::str::from_utf8(&buffer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    text.chars()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty utf-8 sequence"))
}

fn utf8_width(first_byte: u8) -> Option<usize> {
    match first_byte {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn terminal_columns() -> usize {
    read_terminal_columns().unwrap_or(80)
}

fn read_terminal_columns() -> io::Result<usize> {
    let output = run_stty_capture(["size"])?;
    parse_terminal_columns(&output)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid terminal size"))
}

fn parse_terminal_columns(output: &str) -> Option<usize> {
    let mut parts = output.split_whitespace();
    let _rows = parts.next()?.parse::<usize>().ok()?;
    let columns = parts.next()?.parse::<usize>().ok()?;
    Some(columns.max(1))
}

fn fit_options_to_terminal(options: &[InputOption], terminal_columns: usize) -> Vec<InputOption> {
    let max_command_width = options
        .iter()
        .map(|option| option.command.chars().count())
        .max()
        .unwrap_or(0);
    let max_description_width = terminal_columns.saturating_sub(max_command_width + 5);

    options
        .iter()
        .map(|option| {
            let description = truncate_for_width(&option.description, max_description_width);
            InputOption::new(option.command.clone(), description)
        })
        .collect()
}

fn truncate_for_width(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars - 3 {
            break;
        }
        out.push(ch);
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out.push_str("...");
    out
}

fn run_stty<I, S>(args: I) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let tty = open_tty_reader()?;
    let status = Command::new("stty")
        .args(args.into_iter().map(|arg| arg.as_ref().to_string()))
        .stdin(Stdio::from(tty))
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "stty failed with status {status}"
        )))
    }
}

fn run_stty_capture<I, S>(args: I) -> io::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let tty = open_tty_reader()?;
    let output = Command::new("stty")
        .args(args.into_iter().map(|arg| arg.as_ref().to_string()))
        .stdin(Stdio::from(tty))
        .stdout(Stdio::piped())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "stty failed with status {}",
            output.status
        )));
    }

    String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn read_clipboard_text() -> Option<String> {
    clipboard_command_candidates()
        .into_iter()
        .find_map(|candidate| {
            let output = Command::new(candidate.0).args(candidate.1).output().ok()?;
            if !output.status.success() {
                return None;
            }
            let text = String::from_utf8(output.stdout).ok()?;
            let trimmed = text.trim_end_matches(['\n', '\r']);
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

fn clipboard_command_candidates() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("pbpaste", &[]),
        ("wl-paste", &["-n"]),
        ("xclip", &["-o", "-selection", "clipboard"]),
    ]
}

fn open_tty_reader() -> io::Result<File> {
    File::open("/dev/tty")
}

fn open_tty_writer() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn init_session(cwd: &std::path::Path, resume: ResumeTarget) -> Result<SessionInit> {
    match resume {
        ResumeTarget::New => Ok(SessionInit {
            session: None,
            messages: Vec::new(),
            status: "new conversation".to_string(),
        }),
        ResumeTarget::ContinueLatest => {
            if let Some(store) = SessionStore::load_latest(cwd)? {
                let messages = store.load_messages()?;
                let status = format!("continued {}", store.id);
                Ok(SessionInit {
                    session: Some(store),
                    messages,
                    status,
                })
            } else {
                Ok(SessionInit {
                    session: None,
                    messages: Vec::new(),
                    status: "new conversation (no previous session found)".to_string(),
                })
            }
        }
        ResumeTarget::ResumeId(id) => {
            let store = SessionStore::load(cwd, &id)?;
            let messages = store.load_messages()?;
            Ok(SessionInit {
                session: Some(store),
                messages,
                status: format!("resumed {}", id),
            })
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

fn print_startup_intro(client: &LLMClient, app_config: &AppConfig, session_status: &str) {
    crate::print_banner(&[
        format!("{} {}", "session".dimmed(), session_status.white()),
        format!(
            "{} theme={} output={}",
            "ui".dimmed(),
            app_config.theme.to_string().white(),
            app_config.output_style.white()
        ),
        format!("{} {}", "endpoint".dimmed(), client.base_url().white()),
    ]);

    if app_config.tips {
        print_startup_tip();
    }
}

fn print_startup_tip() {
    const STARTUP_TIPS: &[&str] = &[
        "Enter submits the current message",
        "Type / to open slash commands",
        "/resume switches to a saved conversation",
        "/help shows all commands",
    ];
    let index = choose_startup_tip_index(STARTUP_TIPS.len());
    println!("{} {}", "tip".dimmed(), STARTUP_TIPS[index].white());
    println!();
}

fn choose_startup_tip_index(len: usize) -> usize {
    if len <= 1 {
        return 0;
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seed =
        std::collections::hash_map::RandomState::new().hash_one((nanos, std::process::id(), len));
    (seed as usize) % len
}

fn build_main_prompt_footer_lines(client: &LLMClient) -> Vec<String> {
    vec![format!(
        "{} {}  {} {}",
        "llm".dimmed(),
        client.provider_name().white(),
        "model".dimmed(),
        client.model().white()
    )]
}

async fn handle_command(command: &str, history: &mut ConversationHistory) -> bool {
    match command.to_lowercase().as_str() {
        "/exit" | "/quit" => {
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
    println!(
        "  {}           - List and resume a previous session",
        "/resume".yellow()
    );
    println!(
        "  {}           - Manually compact conversation context",
        "/compact".yellow()
    );
    println!(
        "  {}              - Show current git diff",
        "/diff".yellow()
    );
    println!(
        "  {}            - Review current git diff with the model",
        "/review".yellow()
    );
    println!(
        "  {}    - Generate a commit message and commit",
        "/commit [title]".yellow()
    );
    println!("  {}           - List saved memories", "/memory".yellow());
    println!(
        "  {}      - List or switch output styles",
        "/output-style [name]".yellow()
    );
    println!(
        "  {}          - Search the web directly",
        "/web <query>".yellow()
    );
    println!(
        "  {}         - Fetch a public web page",
        "/fetch <url>".yellow()
    );
    println!(
        "  {} - Start, stop, or inspect the local server",
        "/server [status|stop|host:port]".yellow()
    );
    println!("  {}             - Show plan status", "/plan".yellow());
    println!(
        "  {}        - Enable plan mode manually",
        "/plan on".yellow()
    );
    println!(
        "  {}       - Disable plan mode manually",
        "/plan off".yellow()
    );
    println!(
        "  {}     - Clear persisted todo list",
        "/plan clear".yellow()
    );
    println!(
        "  {}           - List available user skills",
        "/skills".yellow()
    );
    println!(
        "  {}     - Invoke a user skill by slash command",
        "/<skill-name> [args]".yellow()
    );
    println!(
        "  {}           - Configure UI settings (Theme / Tips)",
        "/config".yellow()
    );
    println!(
        "  {}           - Select and persist active model",
        "/model".yellow()
    );
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
            "⚡ Context compacted automatically. Estimated tokens:"
                .cyan()
                .bold(),
            compact::estimate_tokens(messages)
        );
    }
    Ok(())
}

async fn run_user_turn(
    client: &LLMClient,
    registry: &ToolRegistry,
    output_style_manager: &OutputStyleManager,
    app_config: &AppConfig,
    skill_manager: Option<&SkillManager>,
    cwd: &std::path::Path,
    session: &mut Option<SessionStore>,
    memory_store: &mut MemoryStore,
    messages: &mut Vec<Value>,
    display_history: &mut ConversationHistory,
    user_input: &str,
) -> Result<()> {
    let result = async {
        messages.push(json!({"role": "user", "content": user_input}));
        display_history.add_user_message(user_input);
        ensure_session_started(cwd, session)?;

        if let Some(manager) = skill_manager {
            manager.set_session_id(session.as_ref().map(|s| s.id.as_str()));
        }

        session
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("session was not initialized"))?
            .append_message(
                messages
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("missing just-added user message"))?,
            )?;

        println!("\n{}", "🤖 Model is thinking...\n".yellow());

        maybe_auto_compact(client, session, messages, display_history).await?;

        let before_len = messages.len();
        let system_prompt = runtime::build_base_system_prompt(
            memory_store,
            output_style_manager,
            &app_config.output_style,
            skill_manager,
        )?;

        match engine::run_agent_loop_with_system_prompt(
            client,
            registry,
            messages,
            system_prompt.as_deref(),
        )
        .await
        {
            Ok(response) => {
                if engine::response_needs_trailing_newline(&response) {
                    println!();
                }
                if let Some(s) = session {
                    s.append_messages(&messages[before_len..])?;
                }
                *display_history = rebuild_display_history(messages);
                memory_store.spawn_extract_and_save(client.clone(), messages.clone());
                Ok(())
            }
            Err(err) => {
                *display_history = rebuild_display_history(messages);
                Err(err)
            }
        }
    }
    .await;

    registry.clear_active_skill();
    result
}

async fn handle_skill_command(
    client: &LLMClient,
    registry: &ToolRegistry,
    output_style_manager: &OutputStyleManager,
    app_config: &AppConfig,
    skill_manager: &SkillManager,
    cwd: &std::path::Path,
    session: &mut Option<SessionStore>,
    memory_store: &mut MemoryStore,
    messages: &mut Vec<Value>,
    display_history: &mut ConversationHistory,
    skill_name: &str,
    skill_args: &str,
) -> Result<()> {
    ensure_session_started(cwd, session)?;
    skill_manager.set_session_id(session.as_ref().map(|s| s.id.as_str()));

    let resolved = skill_manager.resolve_and_activate(skill_name, skill_args)?;
    println!(
        "\n{} {} {}",
        "🧩 Skill:".cyan().bold(),
        resolved.name.white(),
        format!("[{} / {}]", resolved.loaded_from, resolved.context).dimmed()
    );
    if !resolved.allowed_tools.is_empty() {
        println!(
            "{} {}",
            "🔒 Allowed tools:".cyan().bold(),
            resolved.allowed_tools.join(", ")
        );
    }

    let user_message = resolved.default_user_message(skill_args);
    run_user_turn(
        client,
        registry,
        output_style_manager,
        app_config,
        Some(skill_manager),
        cwd,
        session,
        memory_store,
        messages,
        display_history,
        &user_message,
    )
    .await
}

fn handle_memory_command(memory_store: &MemoryStore) -> Result<()> {
    println!("\n{}", "🧠 Saved memories:".cyan().bold());
    println!("{}", memory_store.render_memory_list()?);
    Ok(())
}

fn handle_output_style_command(
    cwd: &std::path::Path,
    output_style_manager: &OutputStyleManager,
    app_config: &mut AppConfig,
    style_name: &str,
) -> Result<bool> {
    let style_name = style_name.trim();
    if style_name.is_empty() {
        println!("\n{}", "🎨 Available output styles:".cyan().bold());
        println!(
            "{}",
            output_style_manager.render_style_list(&app_config.output_style)?
        );
        return Ok(false);
    }

    if !output_style_manager.has_style(style_name)? {
        anyhow::bail!("unknown output style: {}", style_name);
    }

    app_config.output_style = style_name.to_string();
    let path = app_config.save(cwd)?;
    println!(
        "\n{} {} ({})",
        "✅ Output style updated:".green(),
        app_config.output_style.white(),
        path.display()
    );
    Ok(true)
}

async fn handle_web_search_command(query: &str) -> Result<()> {
    if query.trim().is_empty() {
        anyhow::bail!("please provide a search query");
    }
    println!("\n{}", "🌐 Web search results:".cyan().bold());
    println!("{}", search_web(query, None, 5).await?);
    Ok(())
}

async fn handle_web_fetch_command(url: &str) -> Result<()> {
    if url.trim().is_empty() {
        anyhow::bail!("please provide a URL");
    }
    println!("\n{}", "🌐 Web page fetch:".cyan().bold());
    println!("{}", fetch_url(url, None, 12_000).await?);
    Ok(())
}

async fn handle_server_command(
    cwd: &std::path::Path,
    args: &str,
    server_handle: &mut Option<ServerHandle>,
) -> Result<()> {
    if server_handle
        .as_ref()
        .map(ServerHandle::is_finished)
        .unwrap_or(false)
    {
        if let Some(handle) = server_handle.take() {
            match handle.stop().await {
                Ok(()) => println!(
                    "\n{}",
                    "ℹ️ Previous background server already stopped".yellow()
                ),
                Err(err) => eprintln!(
                    "\n{} {}",
                    "⚠️ Background server exited unexpectedly:".yellow().bold(),
                    err
                ),
            }
        }
    }

    match server::parse_server_command(args)? {
        server::ServerCommand::Status => {
            if let Some(handle) = server_handle.as_ref() {
                println!(
                    "\n{} http://{}",
                    "🌐 Server is running at".cyan().bold(),
                    handle.addr()
                );
            } else {
                println!("\n{}", "ℹ️ Server is not running".yellow());
            }
        }
        server::ServerCommand::Stop => {
            if let Some(handle) = server_handle.take() {
                handle.stop().await?;
                println!("\n{}", "🛑 Server stopped".green());
            } else {
                println!("\n{}", "ℹ️ Server is not running".yellow());
            }
        }
        server::ServerCommand::Start(config) => {
            if let Some(handle) = server_handle.as_ref() {
                println!(
                    "\n{} http://{}",
                    "🌐 Server is already running at".cyan().bold(),
                    handle.addr()
                );
                return Ok(());
            }

            let handle = server::start_server(config, cwd.to_path_buf()).await?;
            println!(
                "\n{} http://{}",
                "🌐 Server started at".green().bold(),
                handle.addr()
            );
            *server_handle = Some(handle);
        }
    }

    Ok(())
}

fn handle_plan_command(plan_manager: &PlanManager, args: &str) -> Result<()> {
    let arg = args.trim().to_ascii_lowercase();
    match arg.as_str() {
        "" => {
            println!("\n{}", "📋 Plan Status:".cyan().bold());
            println!("{}", plan_manager.render_status());
        }
        "on" | "enter" => {
            println!("\n{}", "📋 Plan Mode Enabled".cyan().bold());
            println!("{}", plan_manager.enter_mode(None)?);
        }
        "off" | "exit" => {
            println!("\n{}", "📋 Plan Mode Disabled".cyan().bold());
            println!("{}", plan_manager.exit_mode(None)?);
        }
        "clear" => {
            println!("\n{}", "📋 Todo List Cleared".cyan().bold());
            println!("{}", plan_manager.clear_todos()?);
        }
        _ => {
            println!(
                "{}",
                "Unknown /plan option. Use /plan, /plan on, /plan off, or /plan clear".yellow()
            );
        }
    }
    Ok(())
}

fn handle_skills_command(skill_manager: &SkillManager) -> Result<()> {
    println!("\n{}", "🧩 Available skills:".cyan().bold());
    println!("{}", skill_manager.render_user_invocable_list()?);
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
    rl: &mut ReplEditor,
    cwd: &std::path::Path,
    app_config: &mut AppConfig,
) -> Result<()> {
    let options = vec![
        InputOption::new("theme", "Configure UI theme"),
        InputOption::new("tips", "Toggle startup tips"),
    ];
    let Some(input) = rl.select_option("⚙️ Config Menu", &options)? else {
        println!("{}", "Config cancelled".yellow());
        return Ok(());
    };

    match input.as_str() {
        "theme" => configure_theme(rl, cwd, app_config)?,
        "tips" => configure_tips(rl, cwd, app_config)?,
        _ => println!("{}", "Unknown config option".yellow()),
    }

    Ok(())
}

fn configure_theme(
    rl: &mut ReplEditor,
    cwd: &std::path::Path,
    app_config: &mut AppConfig,
) -> Result<()> {
    let options = vec![
        InputOption::new("default", "Standard theme"),
        InputOption::new("light", "Light terminal palette"),
        InputOption::new("dark", "Dark terminal palette"),
    ];
    let Some(input) = rl.select_option("🎨 Theme", &options)? else {
        println!("{}", "Theme change cancelled".yellow());
        return Ok(());
    };

    let theme = match input.as_str() {
        "default" => Theme::Default,
        "light" => Theme::Light,
        "dark" => Theme::Dark,
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
    rl: &mut ReplEditor,
    cwd: &std::path::Path,
    app_config: &mut AppConfig,
) -> Result<()> {
    let options = vec![
        InputOption::new("on", "Show startup tips and help lines"),
        InputOption::new("off", "Keep startup quieter"),
    ];
    let Some(input) = rl.select_option("💡 Tips", &options)? else {
        println!("{}", "Tips change cancelled".yellow());
        return Ok(());
    };

    app_config.tips = match input.as_str() {
        "on" => true,
        "off" => false,
        _ => {
            println!("{}", "Unknown tips option".yellow());
            return Ok(());
        }
    };

    let path = app_config.save(cwd)?;
    println!(
        "{} {} ({})",
        "✅ Tips updated:".green(),
        if app_config.tips {
            "on".green()
        } else {
            "off".red()
        },
        path.display()
    );
    Ok(())
}

fn handle_resume_command(
    rl: &mut ReplEditor,
    cwd: &std::path::Path,
    skill_manager: Option<&SkillManager>,
    memory_store: &mut MemoryStore,
    session: &mut Option<SessionStore>,
    messages: &mut Vec<Value>,
    display_history: &mut ConversationHistory,
) -> Result<()> {
    let sessions = SessionStore::list(cwd)?;
    if sessions.is_empty() {
        println!("\n{}", "No saved sessions for current project".dimmed());
        return Ok(());
    }

    let options = sessions
        .iter()
        .map(|session_store| {
            let preview = session_last_user_preview(session_store)
                .unwrap_or_else(|_| "(failed to load)".to_string());
            InputOption::new(session_store.id.clone(), preview)
        })
        .collect::<Vec<_>>();

    let Some(selected_id) = rl.select_option("Resume Session", &options)? else {
        println!("{}", "Resume cancelled".yellow());
        return Ok(());
    };

    let selected = sessions
        .iter()
        .find(|store| store.id == selected_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("selected session not found: {}", selected_id))?;
    let loaded_messages = selected.load_messages()?;

    *session = Some(selected);
    *messages = loaded_messages;
    memory_store.set_processed_visible_messages(visible_message_count(messages));
    *display_history = rebuild_display_history(messages);
    if let Some(manager) = skill_manager {
        manager.set_session_id(session.as_ref().map(|s| s.id.as_str()));
    }

    println!(
        "{} {}",
        "session resumed".white().bold(),
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
            "session started".white().bold(),
            created.id.as_str().white()
        );
        *session = Some(created);
    }
    Ok(())
}

fn print_loaded_history(messages: &[Value]) {
    println!("\n{}", "Loaded conversation history".white().bold());
    println!();
    if messages.is_empty() {
        println!("{}", "(empty)".dimmed());
        return;
    }

    let card_width = terminal_columns().saturating_sub(1).max(20);
    let text_width = card_width.saturating_sub(4).max(16);
    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("unknown");
        let content = msg["content"].as_str().unwrap_or_default();
        match role {
            "user" => print_loaded_user_message(content, card_width),
            "assistant" => print_loaded_assistant_message(content, text_width),
            "tool" => println!(
                "{} {}",
                "[tool]".dimmed(),
                msg["tool_name"].as_str().unwrap_or("unknown")
            ),
            _ => {}
        }
    }
}

fn print_loaded_user_message(content: &str, card_width: usize) {
    let card_width = card_width.max(6);
    let content_width = card_width.saturating_sub(4).max(1);
    let lines = wrap_message_lines(content, content_width);
    let blank = gray_message_surface(&" ".repeat(card_width));

    println!("{blank}");
    for line in lines {
        let padding = content_width.saturating_sub(text_display_width(&line));
        let padded = format!("  {}{}  ", line, " ".repeat(padding));
        println!("{}", gray_message_surface(&padded));
    }
    println!("{blank}");
    println!();
}

fn print_loaded_assistant_message(content: &str, max_width: usize) {
    for line in wrap_message_lines(content, max_width) {
        println!("{}", line);
    }
    println!();
}

fn gray_message_surface(content: &str) -> String {
    format!(
        "{}{}{}{}{}",
        ANSI_STYLES.bg_color.ansi256(236),
        ANSI_STYLES.color.ansi256(255),
        content,
        ANSI_STYLES.color.close,
        ANSI_STYLES.bg_color.close
    )
}

fn wrap_message_lines(content: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut wrapped = Vec::new();

    for raw_line in content.lines() {
        if raw_line.is_empty() {
            wrapped.push(String::new());
            continue;
        }

        let mut current = String::new();
        let mut current_width = 0usize;
        for ch in raw_line.chars() {
            let ch_width = char_display_width(ch);
            if current_width > 0 && current_width + ch_width > max_width {
                wrapped.push(current);
                current = String::new();
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
        wrapped.push(current);
    }

    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
}

fn text_display_width(text: &str) -> usize {
    text.chars().map(char_display_width).sum()
}

fn char_display_width(ch: char) -> usize {
    if ch.is_control() {
        return 0;
    }

    if matches!(
        ch,
        '\u{1100}'..='\u{115F}'
            | '\u{2329}'..='\u{232A}'
            | '\u{2E80}'..='\u{A4CF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE10}'..='\u{FE19}'
            | '\u{FE30}'..='\u{FE6F}'
            | '\u{FF00}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}'
            | '\u{1F300}'..='\u{1FAFF}'
            | '\u{20000}'..='\u{2FFFD}'
            | '\u{30000}'..='\u{3FFFD}'
    ) {
        2
    } else {
        1
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
    rl: &mut ReplEditor,
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

    println!(
        "{} {}",
        "Suggested commit message:".cyan().bold(),
        suggested.white()
    );
    let confirm_options = vec![
        InputOption::new("yes", "Commit with the suggested message"),
        InputOption::new("edit", "Edit the commit message first"),
        InputOption::new("no", "Cancel this commit"),
    ];
    let confirm = rl
        .select_option("Commit With This Message?", &confirm_options)?
        .unwrap_or_else(|| "no".to_string());

    let final_message = match confirm.as_str() {
        "yes" => suggested,
        "edit" => {
            let edited = rl.read_line_with_initial("Enter Commit Message", &suggested)?;
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

async fn select_model(rl: &mut ReplEditor, client: &mut LLMClient) -> Result<()> {
    let models = client.list_models().await?;
    if models.is_empty() {
        println!("\n{}", "⚠️ No models found from provider endpoint".yellow());
        return Ok(());
    }

    let options = models
        .iter()
        .map(|model| {
            let description = if model == client.model() {
                "Current model".to_string()
            } else {
                "Available model".to_string()
            };
            InputOption::new(model.clone(), description)
        })
        .collect::<Vec<_>>();

    let Some(model) = rl.select_option("📦 Select Model", &options)? else {
        println!("{}", "Model selection cancelled".yellow());
        return Ok(());
    };

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
        assert_eq!(
            command_arg("/commit feat: add git", "/commit"),
            Some("feat: add git")
        );
    }

    #[test]
    fn command_arg_rejects_prefix_without_separator() {
        assert_eq!(command_arg("/commitment", "/commit"), None);
    }

    #[test]
    fn parse_slash_skill_extracts_name_and_args() {
        assert_eq!(
            parse_slash_skill("/simplify src/lib.rs"),
            Some(("simplify", "src/lib.rs"))
        );
    }

    #[test]
    fn parse_slash_skill_handles_name_only() {
        assert_eq!(parse_slash_skill("/simplify"), Some(("simplify", "")));
    }

    #[test]
    fn parse_terminal_columns_reads_stty_size_output() {
        assert_eq!(parse_terminal_columns("24 120\n"), Some(120));
    }

    #[test]
    fn fit_options_to_terminal_truncates_long_descriptions() {
        let options = vec![InputOption::new(
            "commit",
            "/commit [title]  Generate and run git commit",
        )];
        let fitted = fit_options_to_terminal(&options, 30);
        assert_eq!(fitted[0].command, "commit");
        assert_eq!(fitted[0].description, "/commit [title]...");
    }

    #[test]
    fn choose_startup_tip_index_stays_in_range() {
        assert_eq!(choose_startup_tip_index(0), 0);
        assert_eq!(choose_startup_tip_index(1), 0);

        for _ in 0..32 {
            assert!(choose_startup_tip_index(4) < 4);
        }
    }
}
