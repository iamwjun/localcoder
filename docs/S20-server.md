# S20 · Server Mode

> Not implemented yet. This document defines the S20 implementation plan: add a `/server` command to `localcoder` using Axum so the current process can start a local web server, receive HTTP and WebSocket messages, and reuse the existing agent loop to return replies.  
> Here, WebSocket means `ws`. If `wss` is needed, it should be handled by a reverse proxy or a TLS termination layer.

**Suggested files to modify**: `Cargo.toml`, `src/main.rs`, `src/repl.rs`, `src/engine.rs`  
**Suggested new file**: `src/server.rs`  
**Related modules**: `src/session.rs`, `src/memory.rs`, `src/tools/`

---

## 1. Goal

S20 is not just another "network tool". It puts `localcoder` itself into a mode where it can serve requests.

The goals are straightforward:

- Add a `/server` command in the REPL
- Support starting the server directly in one-shot mode via `cargo run -- "/server"`
- Use Axum to provide HTTP routes and WebSocket upgrades
- Reuse the existing `LLMClient + ToolRegistry + engine` flow to generate replies
- Support minimal session continuation instead of forcing every request to be single-turn

The focus of S20 is to wrap the existing CLI capability as a local server entrypoint, not to redesign the agent architecture.

---

## 2. Why Axum

Axum fits this repository for simple reasons:

- The project already runs on `tokio`, so integration cost is low
- HTTP routing, JSON extraction, error responses, and shared state are all straightforward
- WebSocket support is mature and fits a `/ws` entrypoint well
- There is no need to introduce a heavier web framework for a local server mode

So S20 should use Axum directly without adding another web abstraction layer.

---

## 3. User Command Design

### 3.1 Commands inside the REPL

Suggested command forms:

- `/server`: start the service on the default address, `127.0.0.1:3000`
- `/server status`: show the current server status
- `/server stop`: stop the current server

Optional extension:

- `/server 127.0.0.1:4000`
- `/server 0.0.0.0:3000`

Behavior:

- Running `/server` inside the REPL should start the Axum service in the background and should not block the REPL loop
- If the server is already running, calling `/server` again should return status instead of binding the port again

### 3.2 One-shot mode

`src/main.rs` already dispatches commands like `/web` and `/fetch` separately. S20 should follow the same pattern:

```bash
cargo run -- "/server"
cargo run -- "/server 127.0.0.1:4000"
```

In one-shot mode:

- The server runs in the foreground
- It exits only on `Ctrl+C`
- Shutdown should be graceful

---

## 4. HTTP and WebSocket Protocol

### 4.1 Routes

Start with only three routes:

- `GET /healthz`
- `POST /v1/message`
- `GET /v1/ws`

Responsibilities:

- `GET /healthz`: health check
- `POST /v1/message`: single HTTP request/response
- `GET /v1/ws`: upgrade to WebSocket and handle multi-turn messages

### 4.2 HTTP request body

Suggested minimal request body for `POST /v1/message`:

```json
{
  "message": "Explain the role of src/main.rs",
  "session_id": "optional-session-id",
  "output_style": "optional-style"
}
```

Rules:

- `message` is required and must not be empty
- If `session_id` is empty, create a new session automatically
- If `output_style` is empty, use the current default output style

### 4.3 HTTP response body

Suggested response:

```json
{
  "session_id": "sess_abc123",
  "reply": "src/main.rs is mainly responsible for bootstrapping config, registering tools, and entering REPL or one-shot mode.",
  "model": "qwen3.5:4b"
}
```

On error, return standard JSON:

```json
{
  "error": "message must not be empty"
}
```

### 4.4 WebSocket message format

Client sends:

```json
{
  "type": "message",
  "message": "Continue from the previous turn and list the key functions in main.rs",
  "session_id": "sess_abc123"
}
```

Server replies:

```json
{
  "type": "assistant",
  "session_id": "sess_abc123",
  "reply": "The key functions are main, parse_args, one_shot, and parse_command_arg."
}
```

Error message:

```json
{
  "type": "error",
  "message": "invalid request json"
}
```

Suggested v1 limits:

- One WebSocket message maps to one full agent execution
- No token streaming yet
- Within one connection, the client should send requests serially and wait for the previous response to finish

This keeps protocol and implementation complexity under control.

---

## 5. Core Implementation Plan

### 5.1 Add `src/server.rs`

The server logic should live in `src/server.rs` rather than scattering Axum code across `main.rs` and `repl.rs`.

Suggested minimum structure:

- `ServerConfig`
- `ServerState`
- `ServerHandle`
- `start_server(...)`
- `stop_server(...)`
- `build_router(...)`
- `handle_http_message(...)`
- `handle_ws(...)`

### 5.2 `ServerConfig`

Minimum fields:

- `host: String`
- `port: u16`

Default:

- `127.0.0.1:3000`

For S20 v1, do not add a complex config menu or too many server-level settings. Just support command arguments and sensible defaults.

### 5.3 `ServerState`

Axum needs shared state. Suggested shared fields:

- `client: LLMClient`
- `registry: Arc<ToolRegistry>`
- `cwd: PathBuf`
- `output_style_manager: OutputStyleManager`

If runtime state is needed later, add:

- `active_sessions`
- `shutdown_tx`

### 5.4 `ServerHandle`

The background server started from the REPL must be stoppable, so it should return a handle containing at least:

- Bound address
- Background task `JoinHandle`
- Graceful shutdown signal

Without that, `/server stop` has no clean lifecycle control and would depend on process exit.

---

## 6. Engine Refactor Required First

This is the most important part of S20.

Right now `src/engine.rs` prints directly to the terminal during execution, including things like:

- Tool names
- Line breaks between phases
- Intermediate output rendering

That is acceptable for the REPL, but it is the wrong abstraction for HTTP and WebSocket server mode because:

- The server needs structured return values
- The response path should not be coupled to terminal output
- Web requests should not inherit REPL rendering side effects

So before adding Axum, `engine` should be split into two layers:

- Execution layer: runs the agent loop and returns final text plus updated message history
- Presentation layer: prints tool calls and separators only in the REPL

A simple approach is to add a "silent" flag to `run_agent_loop_with_system_prompt`, or introduce something like `EngineHooks` / `EngineObserver`.  
As long as server mode can run fully silently, that is enough for v1.

---

## 7. Sessions and Concurrency

The biggest difference between server mode and the REPL is that server mode naturally faces concurrent requests.

So do not directly reuse the REPL pattern of mutating one in-memory message vector inside one loop. Instead, use a per-request flow:

- Load session state for the request
- Run the turn
- Persist the updated state

Suggested rules:

- If an HTTP request has no `session_id`, create a new session
- If HTTP or WebSocket includes `session_id`, load history from `SessionStore`
- After the turn finishes, append the new messages back to that session

To avoid concurrent corruption of the same `session_id`, use per-session serialization:

- Maintain something like `HashMap<String, Mutex<()>>` in memory
- Acquire the lock for a session before reading or writing its JSONL file

That prevents two HTTP requests from appending to the same session file at the same time.

---

## 8. Integration with Existing Modules

### 8.1 `src/main.rs`

Add `/server` dispatch logic alongside `/web` and `/fetch`.

### 8.2 `src/repl.rs`

It needs to:

- Recognize `/server` in command dispatch
- Add `/server` to `print_instructions` and `/help`
- Hold an optional `ServerHandle`

### 8.3 `src/session.rs`

The existing `SessionStore` can still be reused, but server mode needs extra concurrency protection around it.

### 8.4 `src/memory.rs`

For S20 v1, automatic memory extraction should not be enabled by default, because:

- HTTP requests are often shorter and more fragmented
- Automatic extraction would add extra model calls under multi-connection usage
- It would make server response latency and side effects less predictable

A safer first step is:

- Reuse sessions first
- Do not auto-extract memory yet
- Revisit memory extraction after server mode is stable

---

## 9. Security Boundary

S20 v1 should stay conservative:

- Listen on `127.0.0.1` by default
- Do not bind `0.0.0.0` by default
- Do not enable CORS by default
- Do not treat this as internet-facing deployment

The reason is simple:

- `localcoder` can execute Bash, read and write files, and call network tools
- The repository does not yet have full remote authentication or permission isolation

So the correct positioning for S20 is:

- A local HTTP / WebSocket entrypoint for a desktop assistant
- Something that can be called by a browser extension, local GUI, script, or another local process

If it is later exposed to LAN or public internet, it should at least add:

- Bearer token authentication
- A stricter permission system
- Request-level auditing
- TLS / `wss`

---

## 10. Suggested Dependencies

`Cargo.toml` needs Axum-related dependencies. For S20, keep it minimal:

- `axum` with `ws` enabled

If CORS, tracing, or static file serving are needed later, add those separately.  
Do not overbuild the web stack in v1.

---

## 11. Test Suggestions

At minimum, add tests for:

- `/server` argument parsing
- `GET /healthz` returns 200
- `POST /v1/message` returns 400 for an empty message
- `POST /v1/message` can create a new session and return a reply
- Passing an existing `session_id` continues prior history
- Invalid WebSocket JSON returns `error`
- Re-running `/server` in the REPL does not bind the port twice
- `/server stop` shuts down the background task gracefully

---

## 12. Conclusion

The right way to implement S20 is not to stuff REPL code into an HTTP route. It should be:

1. Split `engine` into execution and terminal presentation layers
2. Add `src/server.rs` with Axum
3. Integrate `/server` into `main.rs` and `repl.rs`
4. Support local HTTP + WebSocket first
5. Return complete message responses first, without token streaming

That keeps the implementation size under control and stays compatible with the current repository structure.
