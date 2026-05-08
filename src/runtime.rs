/*!
 * Runtime Bootstrap Helpers
 */

use crate::memory::MemoryStore;
use crate::output_style::OutputStyleManager;
use crate::plan::PlanManager;
use crate::skills::SkillManager;
use crate::tools::{
    self, BashTool, EditTool, EnterPlanModeTool, ExitPlanModeTool, GlobTool, GrepTool, LspTool,
    ReadTool, SkillTool, TodoWriteTool, ToolRegistry, WebFetchTool, WebSearchTool, WriteTool,
};
use anyhow::Result;
use std::path::Path;

pub struct RuntimeContext {
    pub output_style_manager: OutputStyleManager,
    pub plan_manager: PlanManager,
    pub skill_manager: SkillManager,
    pub registry: ToolRegistry,
}

pub fn build_runtime(cwd: &Path) -> Result<RuntimeContext> {
    let output_style_manager = OutputStyleManager::new(cwd);
    let plan_manager = PlanManager::new(cwd)?;
    let skill_manager = SkillManager::new(cwd)?;
    let registry = build_registry_with(cwd, plan_manager.clone(), skill_manager.clone())?;

    Ok(RuntimeContext {
        output_style_manager,
        plan_manager,
        skill_manager,
        registry,
    })
}

pub fn build_registry(cwd: &Path) -> Result<ToolRegistry> {
    let plan_manager = PlanManager::new(cwd)?;
    let skill_manager = SkillManager::new(cwd)?;
    build_registry_with(cwd, plan_manager, skill_manager)
}

pub fn build_base_system_prompt(
    memory_store: &MemoryStore,
    output_style_manager: &OutputStyleManager,
    output_style_name: &str,
    skill_manager: Option<&SkillManager>,
) -> Result<Option<String>> {
    let memory_prompt = memory_store.build_system_prompt()?;
    let skill_prompt = match skill_manager {
        Some(manager) => manager.build_system_prompt()?,
        None => None,
    };

    let joined = [memory_prompt, skill_prompt]
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let base_prompt = if joined.trim().is_empty() {
        None
    } else {
        Some(joined)
    };

    output_style_manager.apply_selected_style(output_style_name, base_prompt)
}

fn build_registry_with(
    cwd: &Path,
    plan_manager: PlanManager,
    skill_manager: SkillManager,
) -> Result<ToolRegistry> {
    let enter_plan_manager = plan_manager.clone();
    let exit_plan_manager = plan_manager.clone();
    let todo_plan_manager = plan_manager.clone();
    let mut registry = tools::ToolRegistry::new();
    registry.attach_plan_manager(plan_manager.clone());
    registry.attach_skill_manager(skill_manager.clone());
    registry.register(tools::EchoTool);
    registry.register(ReadTool);
    registry.register(EditTool);
    registry.register(WriteTool);
    registry.register(GlobTool);
    registry.register(GrepTool);
    registry.register(BashTool);
    registry.register(LspTool::new(cwd)?);
    registry.register(WebFetchTool);
    registry.register(WebSearchTool);
    registry.register(EnterPlanModeTool::new(enter_plan_manager));
    registry.register(ExitPlanModeTool::new(exit_plan_manager));
    registry.register(TodoWriteTool::new(todo_plan_manager));
    registry.register(SkillTool::new(skill_manager));
    Ok(registry)
}
