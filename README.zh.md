# Localcoder

> 说明：本仓库在开发过程中使用了 Claude Code 和 Codex。如果这让你感到不适，抱歉。

英文版：[README.md](./README.md)

## 📖 简介

Localcoder 是一个基于 Rust 实现、以本地优先为主的命令行编码助手，当前已经包括：

- ✅ 面向 Ollama、OpenAI 兼容接口和 LM Studio 的流式对话与单次查询
- ✅ 文件、搜索、Bash、Web、LSP 等工具调用运行时
- ✅ 基于 `oxink` 输入组件的交互式 REPL，支持模型切换、会话恢复、配置菜单和输出风格
- ✅ 本地 Server 模式，支持 HTTP 和 WebSocket 入口
- ✅ 上下文压缩、Git 工作流、记忆提取、计划模式和技能系统
- ✅ 轻量级（启动快、内存占用低）

> 相比 JavaScript 版本，Rust 版本启动时间快 **10 倍**，内存占用少 **10 倍**。

---

## 📊 实现状态

[`docs/P00-plan.md`](./docs/P00-plan.md) 中的阶段路线图大部分已经落地。当前进度：**21 个阶段中已完成 16 个**。

| 阶段 | 模块 | 状态 | 核心交付物 |
|------|------|------|------|
| S00 | 基础对话循环 | ✅ | REPL、流式 API、单次查询 |
| S01 | 工具系统架构 | ✅ | `Tool` trait、注册表、工具分发循环 |
| S02 | 文件工具 | ✅ | `Read` / `Edit` / `Write` |
| S03 | 搜索工具 | ✅ | `Glob` / `Grep` |
| S04 | 命令执行 | ✅ | 带安全检查的 `Bash` 工具 |
| S05 | 会话持久化 | ✅ | JSONL 会话存储、`--continue`、`--resume`、`/resume` |
| S06 | 配置系统 | ✅ | `settings.json`、`/config`、持久化 UI 配置 |
| S07 | 权限系统 | ❌ | 规则引擎和用户确认尚未实现 |
| S08 | 上下文压缩 | ✅ | 自动压缩、token 估算、`/compact` |
| S09 | Git 集成 | ✅ | `/diff`、`/review`、`/commit` |
| S10 | 记忆系统 | ✅ | 四种记忆类型和自动提取 |
| S11 | 子代理 | ❌ | fork 子代理和 worktree 隔离尚未实现 |
| S12 | 计划模式 | ✅ | `EnterPlanMode`、`ExitPlanMode`、`TodoWrite`、`/plan` |
| S13 | 技能系统 | ✅ | `SKILL.md`、`skill_tool`、`/skills`、`/<skill-name>` |
| S14 | 网络工具 | ✅ | `WebSearch`、`WebFetch`、`/web`、`/fetch` |
| S15 | 费用追踪 | ❌ | token 计费统计和 `/cost` 尚未实现 |
| S16 | 多平台支持 | ❌ | Bedrock / Vertex / Foundry 尚未实现 |
| S17 | MCP 集成 | ❌ | MCP 客户端和多传输支持尚未实现 |
| S18 | 输出样式 | ✅ | 输出样式加载和 `/output-style` |
| S19 | LSP 集成 | ✅ | 基于语言服务器的代码导航 `Lsp` |
| S20 | Server 模式 | ✅ | 基于 Axum 的本地 HTTP / WebSocket 服务与 `/server` |

---

## 🚀 快速开始

### 1. 安装二进制

**方法一：使用官方安装脚本**

```bash
curl -fsSL https://raw.githubusercontent.com/iamwjun/localcoder/main/install.sh | bash
```

支持平台：
- macOS (arm64 / x86_64)
- Linux (x86_64 / aarch64)

**方法二：手动编译**

```bash
git clone https://github.com/iamwjun/localcoder.git
cd localcoder
cargo build --release
```

---

### 2. 配置 Provider

首次启动时，Localcoder 会确保 `$HOME/.localcoder/settings.json` 存在。

LLM 配置从这个 home 级配置文件读取。示例：

**Ollama**

```json
{
  "llm": {
    "type": "ollama",
    "base_url": "http://localhost:11434",
    "model": "qwen3.5:4b"
  }
}
```

**LM Studio**

```json
{
  "llm": {
    "type": "lmstudio",
    "base_url": "http://localhost:1234",
    "model": "qwen/qwen3-coder-30b"
  }
}
```

**OpenAI 兼容服务**

```json
{
  "llm": {
    "type": "openai",
    "base_url": "https://api.openai.com/v1",
    "api_key": "sk-...",
    "model": "gpt-4o-mini"
  }
}
```

项目内也可以放 `.localcoder/settings.json` 做局部覆盖。当前这一路径尤其适合放 `ui` 和 `lsp` 配置：

```json
{
  "ui": {
    "theme": "default",
    "tips": true,
    "output_style": "default"
  },
  "lsp": {
    "enabled": true,
    "servers": [
      {
        "name": "rust-analyzer",
        "command": "rust-analyzer",
        "extensions": [".rs"],
        "language_id": "rust"
      }
    ]
  }
}
```

如果你使用 Ollama，请确保本地服务已经启动，并且至少拉取了一个模型：

```bash
ollama serve
ollama pull qwen3.5:4b
```

---

### 3. 首次运行

```bash
# REPL 交互模式
localcoder
```

启动时，REPL 会显示一个紧凑的 banner，包含会话状态、UI 状态和当前 endpoint。启用 tips 时，还会随机显示一条启动提示；当前激活的 `llm` / `model` 会显示在输入框下方。

你可以手动编辑 `$HOME/.localcoder/settings.json`，也可以在 REPL 中使用 `/model` 切换模型。

---

### 4. 运行

```bash
# REPL 交互模式
localcoder

# 单次查询（快速测试）
localcoder -- "你好，介绍一下你自己"

# 继续当前项目最近一次会话
localcoder --continue

# 恢复指定会话
localcoder --resume s1712345678-12345

# 前台启动本地服务
localcoder -- "/server"

# 使用自定义地址启动本地服务
localcoder -- "/server 127.0.0.1:4000"
```

一些有用的交互细节：

- `Ctrl-C`、`Ctrl-D`、`/exit`、`/quit` 都可以退出主 REPL
- `/resume` 会打开会话选择器，并重新渲染已加载的历史对话
- `/config` 用来管理主题和启动提示开关
- `/output-style` 可以切换当前回复风格，而不用手改 JSON

---

## 🌐 Server 模式

Localcoder 也可以作为本地 HTTP / WebSocket 服务运行。默认监听地址为 `127.0.0.1:3000`。

可以通过两种方式启动：

```bash
# 在 REPL 中后台启动，同时继续使用 REPL
/server
/server status
/server stop

# 在 one-shot 模式下前台运行
localcoder -- "/server"
localcoder -- "/server 127.0.0.1:4000"
```

当前可用路由：

- `GET /healthz`
- `POST /v1/message`
- `GET /v1/ws`

HTTP 请求示例：

```bash
curl -X POST http://127.0.0.1:3000/v1/message \
  -H "content-type: application/json" \
  -d '{
    "message": "解释一下 src/main.rs 的职责",
    "session_id": "",
    "output_style": "default"
  }'
```

响应示例：

```json
{
  "session_id": "s1746690000000-12345-0",
  "reply": "src/main.rs 负责初始化配置、注册工具，并决定进入 REPL 还是 one-shot 执行流程。",
  "model": "qwen3.5:4b"
}
```

WebSocket 消息同样使用 JSON；当前一条请求对应一次完整 agent 执行：

```json
{
  "type": "message",
  "message": "继续上一轮，并总结一下 main.rs",
  "session_id": "s1746690000000-12345-0"
}
```

当前服务模式的定位是本地优先：

- 默认只监听 `127.0.0.1`
- 暂时没有内建认证和 TLS
- 如果需要 `wss`，建议通过反向代理处理

---

## 🛠️ 内置工具

当前内置工具包括：

- 文件工具：`Read`、`Edit`、`Write`
- 搜索工具：`Glob`、`Grep`
- Shell 执行：`Bash`
- Web 访问：`WebSearch`、`WebFetch`
- 代码智能：`Lsp`

示例提示词：

```bash
localcoder -- "读取 src/main.rs 的前 5 行"
localcoder -- "在 /tmp/test.txt 中写入'hello world'"
localcoder -- "搜索 process_chunk 函数"
localcoder -- "在项目根目录运行 rg \"SessionStore\""
localcoder -- "抓取 https://www.rust-lang.org/"
```

---

## 📝 REPL 命令

| 命令 | 描述 |
|------|------|
| `/resume` | 列出并恢复历史会话 |
| `/compact` | 手动压缩过长的对话上下文 |
| `/diff` | 显示当前 Git diff |
| `/review` | 使用模型审查当前 Git diff |
| `/commit [title]` | 生成提交信息并执行 git commit |
| `/memory` | 列出已保存的记忆 |
| `/output-style [name]` | 列出或切换输出风格 |
| `/web <query>` | 直接搜索公网内容 |
| `/fetch <url>` | 抓取公开网页 |
| `/server [status\|stop\|host:port]` | 启动、停止或查看本地 HTTP / WebSocket 服务 |
| `/plan` | 查看计划模式状态 |
| `/plan on` | 手动启用计划模式 |
| `/plan off` | 手动关闭计划模式 |
| `/plan clear` | 清除持久化 todo 列表 |
| `/skills` | 列出可直接调用的技能 |
| `/<skill-name> [args]` | 直接调用某个技能 |
| `/config` | 配置 UI 设置，如主题和提示开关 |
| `/help` | 显示可用命令列表 |
| `/clear` | 清空对话历史 |
| `/history` | 查看对话历史（JSON 格式） |
| `/model` | 从当前 provider 的模型接口获取列表并切换当前模型，同时更新 `$HOME/.localcoder/settings.json` |
| `/count` | 显示消息数量 |
| `/version` | 显示当前版本 |
| `/quit` | 退出 REPL |
| `/exit` | 退出 REPL |

---

## 📦 项目结构

```text
localcoder/
├── install.sh           # 安装脚本（自动检测平台）
├── Cargo.toml           # Rust 项目配置
├── CHANGELOG.md         # 版本变更日志
├── README.md            # 英文说明
├── README.zh.md         # 中文说明
├── docs/                # 路线图与分阶段实现文档
│   ├── P00-plan.md      # 总体阶段计划
│   └── S00-S20*.md      # 各阶段详细说明
├── examples/            # 示例代码
│   ├── basic.rs          # 基本 API 调用
│   ├── streaming.rs      # 流式响应
│   ├── conversation.rs   # 多轮对话
│   ├── custom_model.rs   # 自定义模型参数
│   └── error_handling.rs # 错误处理
└── src/                 # 源代码
    ├── main.rs           # 程序入口
    ├── api.rs            # 多 provider 客户端与流式请求
    ├── compact.rs        # 上下文压缩
    ├── config.rs         # REPL/UI 配置加载与持久化
    ├── engine.rs         # Agent 循环与工具分发
    ├── git.rs            # Git 工作流辅助
    ├── memory.rs         # 记忆提取与存储
    ├── output_style.rs   # 输出风格加载与 prompt 注入
    ├── plan.rs           # 计划模式状态与 todo 管理
    ├── repl.rs           # 交互式 REPL
    ├── runtime.rs        # 共享运行时与启动辅助
    ├── server.rs         # 本地 HTTP / WebSocket 服务模式
    ├── session.rs        # JSONL 会话持久化
    ├── skills.rs         # SKILL.md 加载与激活
    ├── tools/            # 内置工具
    ├── services/lsp/     # 语言服务器集成
    └── types.rs          # 共享类型
```

---

## 📋 技术栈

| 组件 | 技术选型 |
|------|----------|
| 异步运行时 | tokio 1.40 |
| HTTP 客户端 | reqwest 0.12 |
| 本地服务 | axum 0.8 |
| JSON 处理 | serde + serde_json 1.0 |
| Prompt / 输入 UI | oxink 0.1.5 |
| 错误处理 | anyhow |
| 语言工具 | 内置 LSP manager + 外部语言服务器 |

---

## 📈 性能对比

| 指标 | JavaScript | Rust | 提升 |
|------|------------|------|------|
| 启动时间 | ~100ms | ~10ms | **10x** |
| 内存占用 | ~50MB | ~5MB | **10x** |
| 二进制大小 | N/A | 5-8MB | 独立部署 |

---

## 📚 学习价值

通过这个项目，你可以学到：

1. **Rust 异步编程** - tokio 运行时、async/await、Stream 处理
2. **HTTP 客户端** - reqwest、JSON API 调用
3. **系统编程** - 错误处理、所有权、类型安全
4. **CLI 开发** - 终端交互体验、prompt 渲染、命令行工作流
5. **Provider 集成** - Ollama、OpenAI 兼容接口、LM Studio 的模型管理

---

## 🤖 后续扩展方向

可以基于此项目继续扩展：

- 权限管理与沙箱
- 子代理协作
- Token 费用追踪
- Bedrock / Vertex / Foundry 等多平台后端
- MCP 集成
- GUI 界面（egui/iced）
- WebAssembly（浏览器运行）

---

## 📄 License

MIT License
