# Localcoder

Chinese version: [README.zh.md](./README.zh.md)

## 📖 Overview

Localcoder is a Claude-like command-line AI assistant implemented in Rust. It provides:

- ✅ Local LLM calls via Ollama
- ✅ Interactive REPL interface
- ✅ File operation tools (`Read` / `Edit` / `Write`)
- ✅ Lightweight runtime with fast startup and low memory usage

> Compared with the JavaScript version, the Rust version starts about **10x faster** and uses about **10x less** memory.

---

## 🚀 Quick Start

### 1. Install the Binary

**Option 1: Use the install script**

```bash
curl -fsSL https://raw.githubusercontent.com/iamwjun/localcoder/main/install.sh | bash
```

Supported platforms:
- macOS (arm64 / x86_64)
- Linux (x86_64 / aarch64)

**Option 2: Build from source**

```bash
git clone https://github.com/iamwjun/localcoder.git
cd localcoder
cargo build --release
```

---

### 2. Start Ollama

Make sure your local Ollama service is running and that at least one model has been pulled:

```bash
ollama serve
ollama pull qwen3.5:4b
```

---

### 3. First Run

```bash
# Start the interactive REPL
localcoder
```

On startup, Localcoder automatically checks for a settings file:

- It first looks for `.localcoder/settings.json` in the current directory
- If that file does not exist, it falls back to `$HOME/.localcoder/settings.json`
- If neither exists, it creates a default config in the current directory

The default config format is:

```json
{
  "ollama": {
    "url": "http://localhost:11434",
    "model": "qwen3.5:4b"
  }
}
```

You can edit this file manually, or switch models from the REPL with the `/model` command.

---

### 4. Run

```bash
# Interactive REPL mode
localcoder

# One-shot query
localcoder -- "Hello, introduce yourself"
```

---

## 🛠️ File Operations

In Ollama mode, you can directly use built-in tools to work with files:

```bash
localcoder -- "Read the first 5 lines of /etc/hosts"
localcoder -- "Write 'hello world' into /tmp/test.txt"
localcoder -- "Replace 'hello' with 'world'"
```

---

## 📝 REPL Commands

| Command | Description |
|------|------|
| `/help` | Show the available commands |
| `/clear` | Clear conversation history |
| `/history` | Show conversation history in JSON format |
| `/model` | Fetch models from `/api/tags`, switch the active model, and update `$HOME/.localcoder/settings.json` |
| `/count` | Show the message count |
| `/version` | Show the current version |
| `/exit` | Exit the REPL |

---

## 📦 Project Structure

```text
localcoder/
├── install.sh           # Install script with platform detection
├── Cargo.toml           # Rust project manifest
├── CHANGELOG.md         # Release notes
├── README.md            # English documentation
├── README.zh.md         # Chinese documentation
├── docs/                # Design notes
│   ├── S00-basic-chat.md      # Basic chat implementation
│   ├── S01-tool-system.md     # Tool system architecture
│   └── S02-file-tools.md      # File operation tools
├── examples/            # Example programs
│   ├── basic.rs          # Basic API usage
│   ├── streaming.rs      # Streaming responses
│   ├── conversation.rs   # Multi-turn conversation
│   ├── custom_model.rs   # Custom model parameters
│   └── error_handling.rs # Error handling
└── src/                 # Source code
    ├── main.rs           # Program entry point
    ├── api.rs            # Ollama client and config management
    ├── types.rs          # Shared types
    ├── engine.rs         # Agent loop
    └── repl.rs           # REPL interface
```

---

## 📋 Tech Stack

| Component | Selection |
|------|----------|
| Async runtime | tokio 1.40 |
| HTTP client | reqwest 0.12 |
| JSON handling | serde + serde_json 1.0 |
| Line editing | rustyline 14.0 |
| Error handling | anyhow + thiserror |
| Terminal colors | colored 2.1 |
| Streaming utilities | futures 0.3 |

---

## 📈 Performance

| Metric | JavaScript | Rust | Improvement |
|------|------------|------|------|
| Startup time | ~100ms | ~10ms | **10x** |
| Memory usage | ~50MB | ~5MB | **10x** |
| Binary size | N/A | 5-8MB | Standalone deployment |

---

## 📚 What You Can Learn

This project is useful for learning:

1. **Async Rust**: tokio, async/await, and stream handling
2. **HTTP clients**: reqwest and JSON-based APIs
3. **Systems programming**: error handling, ownership, and type safety
4. **CLI development**: rustyline REPL and command-line workflows
5. **Ollama integration**: `/api/chat`, `/api/tags`, and model configuration management

---

## 🤖 Possible Extensions

You can continue extending this project with:

- More tools such as Bash execution or web fetching
- Permission management and sandboxing
- Context compression
- Multi-agent collaboration
- MCP integration
- GUI frontends with `egui` or `iced`
- WebAssembly support for running in the browser

---

## 📄 License

MIT License
