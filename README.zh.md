# Localcoder

## 📖 简介

Localcoder 是一个基于 Rust 实现的 Claude-like 命令行 AI 助手，提供：

- ✅ 基于 Ollama 的本地 LLM 调用
- ✅ 交互式 REPL 界面
- ✅ 文件操作工具（Read / Edit / Write）
- ✅ 轻量级（启动快、内存占用低）

> 相比 JavaScript 版本，Rust 版本启动时间快 **10 倍**，内存占用少 **10 倍**。

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

### 2. 启动 Ollama

确保本地 Ollama 服务已经启动，并且至少拉取了一个模型：

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

程序启动时会自动检查配置文件：

- 优先读取当前目录的 `.localcoder/settings.json`
- 如果当前目录没有，则读取 `$HOME/.localcoder/settings.json`
- 如果两处都没有，则在当前目录自动创建默认配置

默认配置格式如下：

```json
{
  "ollama": {
    "url": "http://localhost:11434",
    "model": "qwen3.5:4b"
  }
}
```

你也可以手动编辑这个文件，或在 REPL 中使用 `/model` 指令切换模型。

---

### 4. 运行

```bash
# REPL 交互模式
localcoder

# 单次查询（快速测试）
localcoder -- "你好，介绍一下你自己"
```

---

## 🛠️ 文件操作命令

本地 Ollama 模式下，你可以直接使用工具操作文件：

```bash
localcoder -- "读取 /etc/hosts 的前 5 行"
localcoder -- "在 /tmp/test.txt 中写入'hello world'"
localcoder -- "把 'hello' 替换成 'world'"
```

---

## 📝 REPL 命令

| 命令 | 描述 |
|------|------|
| `/help` | 显示可用命令列表 |
| `/clear` | 清空对话历史 |
| `/history` | 查看对话历史（JSON 格式） |
| `/model` | 从 `/api/tags` 获取模型列表并切换当前模型，同时更新 `$HOME/.localcoder/settings.json` |
| `/count` | 显示消息数量 |
| `/version` | 显示当前版本 |
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
├── docs/                # 设计文档
│   ├── S00-basic-chat.md      # 基础聊天实现
│   ├── S01-tool-system.md     # 工具系统架构
│   └── S02-file-tools.md      # 文件操作工具
├── examples/            # 示例代码
│   ├── basic.rs          # 基本 API 调用
│   ├── streaming.rs      # 流式响应
│   ├── conversation.rs   # 多轮对话
│   ├── custom_model.rs   # 自定义模型参数
│   └── error_handling.rs # 错误处理
└── src/                 # 源代码
    ├── main.rs           # 程序入口
    ├── api.rs            # Ollama 客户端与配置管理
    ├── types.rs          # 类型定义
    ├── engine.rs         # Agent 循环
    └── repl.rs           # REPL 界面
```

---

## 📋 技术栈

| 组件 | 技术选型 |
|------|----------|
| 异步运行时 | tokio 1.40 |
| HTTP 客户端 | reqwest 0.12 |
| JSON 处理 | serde + serde_json 1.0 |
| 命令行编辑 | rustyline 14.0 |
| 错误处理 | anyhow + thiserror |
| 终端彩色 | colored 2.1 |
| 流式处理 | futures 0.3 |

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
4. **CLI 开发** - rustyline REPL、命令行参数
5. **Ollama 集成** - `/api/chat`、`/api/tags`、模型配置管理

---

## 🤖 后续扩展方向

可以基于此项目继续扩展：

- 工具系统（Bash、Web 抓取等）
- 权限管理与沙箱
- 上下文压缩
- 子代理协作
- MCP 集成
- GUI 界面（egui/iced）
- WebAssembly（浏览器运行）

---

## 📄 License

MIT License
