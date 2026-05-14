/*!
 * REPL Slash Command Completion — S21
 */

use crate::skills::{Skill, SkillManager};
use anyhow::Result;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hint, Hinter};
use rustyline::line_buffer::LineBuffer;
use rustyline::validate::Validator;
use rustyline::{Changeset, Context, Helper};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplCommandSource {
    Builtin,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplCommandSpec {
    pub name: String,
    pub usage: String,
    pub summary: String,
    pub takes_args: bool,
    pub source: ReplCommandSource,
}

impl ReplCommandSpec {
    pub fn replacement(&self) -> String {
        if self.takes_args {
            format!("{} ", self.name)
        } else {
            self.name.clone()
        }
    }

    fn display(&self) -> String {
        format!("{:<24} {}", self.usage, self.summary)
    }
}

#[derive(Debug, Clone)]
pub struct ReplHelper {
    commands: Arc<Mutex<Vec<ReplCommandSpec>>>,
}

impl ReplHelper {
    pub fn new(commands: Vec<ReplCommandSpec>) -> Self {
        Self {
            commands: Arc::new(Mutex::new(commands)),
        }
    }

    pub fn set_commands(&mut self, commands: Vec<ReplCommandSpec>) {
        *self.commands.lock().expect("repl command lock poisoned") = commands;
    }

    fn commands(&self) -> Vec<ReplCommandSpec> {
        self.commands
            .lock()
            .expect("repl command lock poisoned")
            .clone()
    }
}

#[derive(Debug, Clone)]
pub struct ReplHint {
    display: String,
    completion: Option<String>,
}

impl Hint for ReplHint {
    fn display(&self) -> &str {
        &self.display
    }

    fn completion(&self) -> Option<&str> {
        self.completion.as_deref()
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let Some(ctx) = slash_token_context(line, pos) else {
            return Ok((0, Vec::new()));
        };

        let candidates = matching_commands(&self.commands(), ctx.prefix)
            .into_iter()
            .map(|spec| Pair {
                display: spec.display(),
                replacement: spec.replacement(),
            })
            .collect::<Vec<_>>();

        Ok((ctx.start, candidates))
    }

    fn update(&self, line: &mut LineBuffer, start: usize, elected: &str, cl: &mut Changeset) {
        let end = line.as_str()[start..]
            .find(char::is_whitespace)
            .map(|offset| start + offset)
            .unwrap_or(line.len());
        line.replace(start..end, elected, cl);
    }
}

impl Hinter for ReplHelper {
    type Hint = ReplHint;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        let ctx = slash_token_context(line, pos)?;
        let matches = matching_commands(&self.commands(), ctx.prefix);
        if matches.is_empty() {
            return None;
        }

        if ctx.prefix == "/" {
            return Some(ReplHint {
                display: "  Tab to list commands, Enter to open picker".to_string(),
                completion: None,
            });
        }

        if matches.len() == 1 {
            let command = &matches[0];
            if ctx.prefix == command.name {
                if command.takes_args {
                    return Some(ReplHint {
                        display: " ".to_string(),
                        completion: Some(" ".to_string()),
                    });
                }
                return None;
            }

            return command.name.strip_prefix(ctx.prefix).map(|rest| ReplHint {
                display: rest.to_string(),
                completion: Some(rest.to_string()),
            });
        }

        Some(ReplHint {
            display: format!("  Tab to list {} matches", matches.len()),
            completion: None,
        })
    }
}

impl Helper for ReplHelper {}
impl Highlighter for ReplHelper {}
impl Validator for ReplHelper {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SlashTokenContext<'a> {
    start: usize,
    prefix: &'a str,
}

pub fn is_bare_slash_input(input: &str) -> bool {
    input.trim() == "/"
}

pub fn build_slash_commands(skill_manager: Option<&SkillManager>) -> Result<Vec<ReplCommandSpec>> {
    let mut commands = builtin_commands();
    let mut seen = commands
        .iter()
        .map(|command| command.name.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();

    if let Some(manager) = skill_manager {
        let mut skills = manager.user_invocable_skills()?;
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        for skill in skills {
            let name = format!("/{}", skill.name);
            let key = name.to_ascii_lowercase();
            if seen.insert(key) {
                commands.push(skill_to_command(skill));
            }
        }
    }

    Ok(commands)
}

fn skill_to_command(skill: Skill) -> ReplCommandSpec {
    let hint = skill
        .argument_hint
        .as_deref()
        .map(str::trim)
        .filter(|hint| !hint.is_empty());
    let usage = hint
        .map(|hint| format!("/{} {}", skill.name, hint))
        .unwrap_or_else(|| format!("/{}", skill.name));

    ReplCommandSpec {
        name: format!("/{}", skill.name),
        usage,
        summary: skill.description,
        takes_args: hint.is_some(),
        source: ReplCommandSource::Skill,
    }
}

fn builtin_commands() -> Vec<ReplCommandSpec> {
    [
        ("/clear", "/clear", "Clear conversation history", false),
        (
            "/commit",
            "/commit [title]",
            "Generate and run git commit",
            true,
        ),
        (
            "/compact",
            "/compact",
            "Compact long context manually",
            false,
        ),
        ("/config", "/config", "Open config menu", false),
        ("/count", "/count", "Show message count", false),
        ("/diff", "/diff", "Show current git diff", false),
        ("/exit", "/exit", "Exit the program", false),
        ("/fetch", "/fetch <url>", "Fetch a public web page", true),
        ("/help", "/help", "Show available commands", false),
        ("/history", "/history", "View conversation history", false),
        ("/memory", "/memory", "List saved memories", false),
        ("/model", "/model", "Select active model", false),
        (
            "/output-style",
            "/output-style [name]",
            "List or switch output styles",
            true,
        ),
        (
            "/plan",
            "/plan [on|off|clear]",
            "Show or toggle plan mode",
            true,
        ),
        ("/quit", "/quit", "Exit the program", false),
        ("/resume", "/resume", "Resume previous session", false),
        ("/review", "/review", "Review current git diff", false),
        (
            "/server",
            "/server [status|stop|host:port]",
            "Start, stop, or inspect the local server",
            true,
        ),
        ("/skills", "/skills", "List available user skills", false),
        ("/version", "/version", "Show current version", false),
        ("/web", "/web <query>", "Search the web directly", true),
    ]
    .into_iter()
    .map(|(name, usage, summary, takes_args)| ReplCommandSpec {
        name: name.to_string(),
        usage: usage.to_string(),
        summary: summary.to_string(),
        takes_args,
        source: ReplCommandSource::Builtin,
    })
    .collect()
}

fn matching_commands(commands: &[ReplCommandSpec], prefix: &str) -> Vec<ReplCommandSpec> {
    let mut matches = commands
        .iter()
        .filter(|command| command.name.starts_with(prefix))
        .cloned()
        .collect::<Vec<_>>();

    matches.sort_by(|a, b| {
        let a_exact = a.name == prefix;
        let b_exact = b.name == prefix;
        b_exact
            .cmp(&a_exact)
            .then_with(|| source_weight(a.source).cmp(&source_weight(b.source)))
            .then_with(|| a.name.cmp(&b.name))
    });

    matches
}

fn source_weight(source: ReplCommandSource) -> u8 {
    match source {
        ReplCommandSource::Builtin => 0,
        ReplCommandSource::Skill => 1,
    }
}

fn slash_token_context(line: &str, pos: usize) -> Option<SlashTokenContext<'_>> {
    if pos > line.len() {
        return None;
    }

    let start = line.find(|ch: char| !ch.is_whitespace())?;
    if !line[start..].starts_with('/') {
        return None;
    }

    let token_end = line[start..]
        .find(char::is_whitespace)
        .map(|offset| start + offset)
        .unwrap_or(line.len());

    if pos < start || pos > token_end {
        return None;
    }

    let token = &line[start..token_end];
    let suffix = token.strip_prefix('/')?;
    if suffix.is_empty() {
        return Some(SlashTokenContext {
            start,
            prefix: &line[start..pos],
        });
    }

    if suffix.contains('/') || suffix.contains(':') || suffix.contains('.') {
        return None;
    }

    if !suffix
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }

    Some(SlashTokenContext {
        start,
        prefix: &line[start..pos],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn bare_slash_input_is_detected() {
        assert!(is_bare_slash_input("/"));
        assert!(is_bare_slash_input("   /   "));
        assert!(!is_bare_slash_input("/help"));
    }

    #[test]
    fn slash_context_accepts_leading_whitespace() {
        let ctx = slash_token_context("   /pl", 6).unwrap();
        assert_eq!(ctx.start, 3);
        assert_eq!(ctx.prefix, "/pl");
    }

    #[test]
    fn slash_context_ignores_non_command_paths() {
        assert!(slash_token_context("请解释 /tmp 目录", 17).is_none());
        assert!(slash_token_context("/tmp/foo", 8).is_none());
        assert!(slash_token_context("/tmp.txt", 8).is_none());
    }

    #[test]
    fn matching_commands_filters_expected_candidates() {
        let matches = matching_commands(&builtin_commands(), "/co");
        let names = matches
            .into_iter()
            .map(|item| item.name)
            .collect::<Vec<_>>();
        assert_eq!(&names[..3], ["/commit", "/compact", "/config"]);
        assert!(names.contains(&"/count".to_string()));
    }

    #[test]
    fn build_slash_commands_includes_user_invocable_skills() {
        let project = TempDir::new().unwrap();
        let skill_dir = project.path().join(".claude/skills/explain");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: explain\ndescription: Explain a file\nargument-hint: \"<path>\"\nuser-invocable: true\n---\n\nExplain it.\n",
        )
        .unwrap();

        let manager = SkillManager::new(project.path()).unwrap();
        let commands = build_slash_commands(Some(&manager)).unwrap();
        let explain = commands
            .into_iter()
            .find(|command| command.name == "/explain")
            .unwrap();
        assert_eq!(explain.usage, "/explain <path>");
        assert!(explain.takes_args);
        assert_eq!(explain.source, ReplCommandSource::Skill);
    }

    #[test]
    fn builtin_commands_shadow_same_named_skills() {
        let project = TempDir::new().unwrap();
        let skill_dir = project.path().join(".claude/skills/help");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: help\ndescription: Shadow builtin\nuser-invocable: true\n---\n\nnoop\n",
        )
        .unwrap();

        let manager = SkillManager::new(project.path()).unwrap();
        let commands = build_slash_commands(Some(&manager)).unwrap();
        assert_eq!(
            commands
                .iter()
                .filter(|command| command.name == "/help")
                .count(),
            1
        );
        assert_eq!(
            commands
                .iter()
                .find(|command| command.name == "/help")
                .unwrap()
                .source,
            ReplCommandSource::Builtin
        );
    }
}
