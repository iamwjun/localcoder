/*!
 * REPL Slash Command Metadata — S21
 */

use crate::skills::{Skill, SkillManager};
use anyhow::Result;

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
            if seen.insert(name.to_ascii_lowercase()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
