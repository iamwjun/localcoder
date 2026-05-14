# S21 · REPL 斜杠命令面板

> 在 REPL 中，当用户输入行首第一个字符为 `/` 时，提供弱提示式命令候选与下拉选择，降低命令发现成本，并兼容现有内建命令与 `/<skill>` 技能命令。

**建议新增文件**: `src/repl_completion.rs`  
**建议修改文件**: `src/repl.rs`, `src/skills.rs`  
**参考模块**: `src/repl.rs`, `src/skills.rs`, `Cargo.toml`

---

## 一、目标

S21 优先增强 `repl` 的可发现性，而不是增加新的 Agent 能力。

当前问题很明确：

- 命令很多，但入口主要靠记忆
- `print_help()` 和启动提示只覆盖“看得到”，不覆盖“输入时发现”
- `/<skill>` 已经存在，但用户几乎只能通过 `/skills` 先看列表

因此 S21 的目标是：

- 当输入行首第一个字符是 `/` 时，进入斜杠命令发现模式
- 给出弱提示式候选，而不是等用户输完整命令后才报 unknown command
- 同时覆盖内建命令与用户技能命令
- 保持现有命令优先级不变：内建命令优先，其次是技能命令

这不是 one-shot 能力，也不是新的工具系统；它就是一个 REPL 输入层增强。

---

## 二、目标交互

### 2.1 触发条件

仅在以下条件下激活：

- 当前输入去掉前导空白后，第一个字符是 `/`
- 光标仍位于第一个 token 内
- 仅 `repl` 模式生效，one-shot 模式不做这层交互增强

这意味着普通文本里的 `/tmp/foo`、URL、路径片段不应被误判成命令模式。

### 2.2 用户体验

期望交互如下：

1. 用户输入 `/`
2. REPL 立即给出轻量提示，例如“`Tab` 查看命令”
3. 用户继续输入 `/co`
4. 候选收敛为 `/commit`、`/compact`、`/config`
5. 用户按 `Tab` 展开候选列表，并可继续选择
6. 选中命令后，把命令写回输入框，而不是直接执行

补全规则：

- 无参数命令：补全为 `/help`、`/clear`
- 需要参数的命令：补全后自动追加空格，例如 `/web `、`/fetch `、`/commit `
- 技能命令：补全为 `/<skill-name>`；如果技能声明了参数提示，可在展示中带出 hint

### 2.3 `/` 单独输入时的行为

当用户输入的内容只有 `/` 时，不应该直接落到 unknown command。

建议改为：

- 首先显示弱提示
- 如果用户直接按 `Enter`，打开一个轻量命令选择菜单
- 菜单选择结果只回填命令，不直接执行

最后这一点很重要。像 `/clear`、`/commit`、`/server stop` 这类命令不应因为误触而直接运行。

---

## 三、候选来源

S21 不应继续在多个位置重复硬编码命令名。建议抽一层统一元数据：

```rust
pub struct ReplCommandSpec {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub takes_args: bool,
    pub source: ReplCommandSource,
}

pub enum ReplCommandSource {
    Builtin,
    Skill,
}
```

候选来源分两类：

- 内建命令：`/exit`、`/quit`、`/clear`、`/history`、`/resume`、`/compact`、`/diff`、`/review`、`/commit`、`/memory`、`/output-style`、`/web`、`/fetch`、`/server`、`/plan`、`/skills`、`/config`、`/model`、`/help`、`/count`、`/version`
- 技能命令：来自 `SkillManager` 中 `user_invocable=true` 的技能

排序建议：

1. 前缀完全匹配优先
2. 内建命令优先于技能命令
3. 更短、更常用的命令优先
4. 展示顺序尽量与 `/help` 输出保持一致

这样可以保持 REPL 行为、帮助文档和补全菜单一致，而不是三套列表各自漂移。

---

## 四、落地方案

### 4.1 先把输入增强从 `repl.rs` 拆出去

当前 `src/repl.rs` 已经同时承担：

- 输入循环
- 命令分发
- 配置菜单
- session 恢复
- server 控制
- 技能调度

S21 不应继续把补全逻辑塞进同一个文件。建议新增：

- `src/repl_completion.rs`

职责只放：

- `ReplCommandSpec`
- 命令元数据构建
- slash 模式匹配
- `rustyline` helper / completer
- picker fallback

### 4.2 基于 `rustyline` 的第一版实现

当前仓库已使用 `rustyline = "14.0"`，它已经具备：

- `Completer`
- `Helper`
- `Hinter`
- `CompletionType::List`
- 自定义 key binding

因此 S21 v1 不需要先换掉整套 REPL。

建议把：

```rust
let mut rl = DefaultEditor::new()?;
```

改成带 helper 的 editor，并启用 list completion：

```rust
let config = rustyline::Config::builder()
    .completion_type(rustyline::CompletionType::List)
    .build();
let mut rl = Editor::<ReplHelper, DefaultHistory>::with_config(config)?;
rl.set_helper(Some(ReplHelper::new(...)));
```

`ReplHelper` 负责两件事：

- `Completer`：当输入以 `/` 开头时返回候选
- `Hinter`：在仅输入 `/` 或部分前缀时显示弱提示

### 4.3 自动弹出与现实约束

这里需要把“想要的体验”和“第一版可稳定落地的实现”分开。

理想体验是：用户刚输入 `/`，候选列表立即像下拉框一样出现。

但基于当前 `rustyline`，单个按键绑定更适合做：

- 插入字符
- 或触发 complete

不适合在一次 `/` 按键中同时可靠完成“插入 `/` + 立即展开完整列表”两步动作。

所以 S21 v1 建议采用分层方案：

- 输入 `/` 时，先显示弱提示，不强行抢焦点
- 按 `Tab` 时，展开命令列表
- 当输入只有 `/` 且按 `Enter` 时，再进入一个轻量 picker 兜底

这已经能显著提升 discoverability，同时不需要引入更重的 TUI 依赖。

如果后续确实需要“真自动下拉”，再评估两条路径：

- 启用更强的 fuzzy / menu 能力
- 为 slash command 单独做一个小型 TUI 选择器

但这不应阻塞 S21 第一版。

### 4.4 picker fallback

建议增加一个轻量回退入口：

```rust
fn show_slash_command_picker(...) -> Result<Option<String>>
```

触发时机：

- 当前输入精确等于 `/`
- 用户按下 `Enter`

行为：

- 列出全部命令和简短说明
- 允许选择一项
- 选择结果只回填到输入框

例如：

```text
> /

Select command:
  1. /help          Show available commands
  2. /clear         Clear conversation history
  3. /commit        Generate and run git commit
  4. /web           Search the web
```

这样即使终端对列表补全体验一般，用户仍然有一个稳定的“命令发现入口”。

### 4.5 与现有技能系统对齐

当前 `repl` 中对技能命令的判断是：

- 先检查内建命令
- 再用 `parse_slash_skill()` 解析
- 再调用 `SkillManager::has_user_invocable()`

S21 必须沿用这个优先级。

也就是说：

- 如果存在 `/plan` 内建命令和同名 skill，菜单中应先展示内建命令
- 执行时也必须继续以内建命令为准

不要为了补全菜单改掉当前命令分发语义。

---

## 五、边界

S21 第一版明确不做：

- one-shot 模式的斜杠命令菜单
- 普通自然语言中的 `/` 自动补全
- 鼠标交互
- 多列复杂 TUI
- 命令执行前自动确认框统一化
- 模糊搜索全局命令面板

S21 的定位很克制：先把 REPL 中“输入 `/` 后我有哪些命令可用”这件事解决。

---

## 六、测试建议

建议新增的测试覆盖：

- `/` 开头时返回内建命令候选
- `/co` 只匹配 `/commit`、`/compact`、`/config`
- 普通文本里的 `/tmp/foo` 不触发 slash completion
- skill 候选会被带入结果集
- 内建命令与同名 skill 冲突时，排序和执行优先级保持内建优先
- 需要参数的命令补全后自动补一个空格
- 输入仅为 `/` 时走 picker fallback，而不是 unknown command

手动验证场景：

```bash
cargo run
```

重点检查：

- 输入 `/`
- 输入 `/co`
- 输入 `/web`
- 输入 `   /pl`
- 输入普通文本 `请解释 /tmp 目录`

---

## 七、阶段结论

S21 不追求“命令系统重写”，而是一个很聚焦的 REPL 交互增强：

- 让 `/` 成为命令发现入口
- 让内建命令和技能命令都可被补全
- 先用 `rustyline` 的 helper/completion 能力完成 v1
- 对“真自动下拉”保持清醒，作为后续增强项

如果只做一个最小版本，建议优先级如下：

1. 统一命令元数据
2. `/` 前缀补全
3. bare `/` 的 picker fallback
4. 技能命令并入候选集

这样投入小、收益直接，而且能明显提升 `repl` 的可用性。
