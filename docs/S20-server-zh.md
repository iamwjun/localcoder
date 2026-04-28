# S20 · Server 模式

> 当前未实现。本文定义 S20 的落地方案：使用 Axum 为 `localcoder` 增加 `/server` 指令，使当前进程可以启动一个本地 Web 服务，接收 HTTP 和 WebSocket 消息，并复用现有 agent loop 返回结果。  
> 这里的 WebSocket 指 `ws`；如果需要 `wss`，建议交给反向代理或 TLS 终止层处理。

**建议修改文件**: `Cargo.toml`, `src/main.rs`, `src/repl.rs`, `src/engine.rs`  
**建议新增文件**: `src/server.rs`  
**参考模块**: `src/session.rs`, `src/memory.rs`, `src/tools/`

---

## 一、目标

S20 不是新增一个“网络工具”，而是让 `localcoder` 本身进入一个可对外提供服务的模式。

目标很明确：

- 在 REPL 中增加 `/server` 指令
- 在 one-shot 模式下也支持 `cargo run -- "/server"` 直接启动服务
- 使用 Axum 提供 HTTP 路由和 WebSocket 升级
- 对外接收消息后，复用现有 `LLMClient + ToolRegistry + engine` 生成回复
- 支持最小会话续接能力，而不是每次请求都只能做单轮问答

S20 的重点是“把现有 CLI 能力包装成一个本地服务端入口”，而不是重新设计一套新的 Agent 架构。

---

## 二、为什么用 Axum

Axum 适合这个仓库，原因很直接：

- 已经在 `tokio` 运行时上，接入成本低
- HTTP 路由、JSON 提取、错误返回、状态共享都足够直接
- WebSocket 支持成熟，适合 `/ws` 入口
- 不需要为了一个本地服务模式引入过重的框架

因此 S20 直接用 Axum 即可，不需要再额外抽象一层 Web 框架适配。

---

## 三、用户命令设计

### 3.1 REPL 内命令

建议支持下面三种形式：

- `/server`：使用默认地址启动服务，默认 `127.0.0.1:3000`
- `/server status`：查看当前服务状态
- `/server stop`：停止当前服务

可选扩展：

- `/server 127.0.0.1:4000`
- `/server 0.0.0.0:3000`

其中：

- 在 REPL 内执行 `/server` 时，应后台启动 Axum 服务，不阻塞 REPL 主循环
- 如果服务已经在运行，再次执行 `/server` 应返回状态，而不是重复绑定端口

### 3.2 one-shot 模式

`src/main.rs` 里已经对 `/web`、`/fetch` 等命令做了单独分发，S20 可以沿用同样方式：

```bash
cargo run -- "/server"
cargo run -- "/server 127.0.0.1:4000"
```

在 one-shot 模式下：

- 服务器前台运行
- 直到收到 `Ctrl+C` 才退出
- 退出时做优雅关闭

---

## 四、HTTP 与 WebSocket 协议

### 4.1 路由

建议先只做三个入口：

- `GET /healthz`
- `POST /v1/message`
- `GET /v1/ws`

职责：

- `GET /healthz`：健康检查
- `POST /v1/message`：单次 HTTP 请求-响应
- `GET /v1/ws`：升级为 WebSocket，接收多轮消息

### 4.2 HTTP 请求体

`POST /v1/message` 请求体建议保持最小化：

```json
{
  "message": "解释一下 src/main.rs 的职责",
  "session_id": "optional-session-id",
  "output_style": "optional-style"
}
```

规则：

- `message` 必填，不能为空
- `session_id` 为空时自动创建新会话
- `output_style` 为空时沿用当前默认输出风格

### 4.3 HTTP 响应体

建议返回：

```json
{
  "session_id": "sess_abc123",
  "reply": "src/main.rs 主要负责启动配置、注册工具并进入 REPL 或 one-shot 模式。",
  "model": "qwen3.5:4b"
}
```

错误时返回标准 JSON：

```json
{
  "error": "message must not be empty"
}
```

### 4.4 WebSocket 消息格式

客户端发：

```json
{
  "type": "message",
  "message": "继续上一轮，给我列出 main.rs 的关键函数",
  "session_id": "sess_abc123"
}
```

服务端回：

```json
{
  "type": "assistant",
  "session_id": "sess_abc123",
  "reply": "关键函数包括 main、parse_args、one_shot 和 parse_command_arg。"
}
```

错误消息：

```json
{
  "type": "error",
  "message": "invalid request json"
}
```

v1 建议限制为：

- 一条 WebSocket 消息对应一次完整 agent 执行
- 暂不做 token streaming
- 同一个连接里，客户端应串行发送请求，等上一条响应结束后再发下一条

这样可以先把协议和实现复杂度压住。

---

## 五、核心实现方案

### 5.1 新增 `src/server.rs`

建议在 `src/server.rs` 中集中放服务端逻辑，而不是把 Axum 代码散落到 `main.rs` 或 `repl.rs`。

最小结构可以是：

- `ServerConfig`
- `ServerState`
- `ServerHandle`
- `start_server(...)`
- `stop_server(...)`
- `build_router(...)`
- `handle_http_message(...)`
- `handle_ws(...)`

### 5.2 `ServerConfig`

最小字段：

- `host: String`
- `port: u16`

默认值：

- `127.0.0.1:3000`

S20 第一版建议先不做复杂配置菜单，也不急着加很多 server 级设置；先把命令参数和默认值跑通。

### 5.3 `ServerState`

Axum 需要共享状态，建议统一封装为：

- `client: LLMClient`
- `registry: Arc<ToolRegistry>`
- `cwd: PathBuf`
- `output_style_manager: OutputStyleManager`

如果后续需要保存运行状态，还可以加：

- `active_sessions`
- `shutdown_tx`

### 5.4 `ServerHandle`

REPL 内后台服务需要可停止，因此应返回一个句柄，至少包含：

- 监听地址
- 后台任务 `JoinHandle`
- 优雅关闭信号

这样 `/server stop` 才有明确的生命周期控制，而不是依赖进程退出。

---

## 六、必须先做的引擎拆分

这是 S20 最关键的一点。

当前 `src/engine.rs` 在执行过程中会直接输出终端内容，比如：

- tool 名称
- 换行分隔
- 中间过程渲染

这对 REPL 没问题，但对 HTTP / WebSocket 服务端是错误的抽象，因为：

- 服务端需要的是结构化返回值
- 不应该把响应过程耦合到终端输出
- Web 请求不应携带 REPL 的渲染副作用

所以在做 Axum 之前，建议先把 `engine` 拆成两层：

- 纯执行层：只负责跑 agent loop，返回最终文本和消息历史
- 展示层：只在 REPL 中打印 tool 调用过程和分隔符

一种简单做法是给 `run_agent_loop_with_system_prompt` 增加一个“是否静默”的参数，或者提炼一个 `EngineHooks` / `EngineObserver` 接口。  
只要做到 server 模式可以完全静默执行，就足够了。

---

## 七、会话与并发

Server 模式和 REPL 最大的差异，是它天然会遇到并发请求。

因此不要直接复用 REPL 里那套“一个循环里持有可变消息数组”的写法，而要改成“按请求加载、按请求执行、按请求写回”。

建议规则：

- HTTP 请求如果没有 `session_id`，就创建新会话
- HTTP / WebSocket 如果带了 `session_id`，就从 `SessionStore` 加载历史消息
- 本轮完成后，把新增消息追加回对应 session

为了避免同一个 `session_id` 被并发写坏，建议：

- 对同一个 session 做串行化处理
- 可以在内存里维护 `HashMap<String, Mutex<()>>`
- 同一 session 的请求先拿锁，再读写 JSONL

这能避免两个 HTTP 请求同时续写同一个 session 文件。

---

## 八、与现有模块的衔接

### 8.1 `src/main.rs`

需要新增 `/server` 分发逻辑，和 `/web`、`/fetch` 并列。

### 8.2 `src/repl.rs`

需要：

- 在命令分发处识别 `/server`
- 在 `print_instructions` 与 `/help` 中加入 `/server`
- 持有一个可选 `ServerHandle`

### 8.3 `src/session.rs`

现有 `SessionStore` 可继续复用，但 server 模式需要额外处理并发访问。

### 8.4 `src/memory.rs`

S20 第一版建议不要默认接入自动 memory 抽取，原因是：

- HTTP 请求通常更短更碎
- 多连接下自动抽取会带来额外模型调用
- 容易让服务端响应时延和副作用变得不透明

更稳妥的策略是：

- 先复用 session
- 先不自动抽取 memory
- 等 server 模式稳定后再决定是否引入

---

## 九、安全边界

S20 第一版建议严格保守：

- 默认只监听 `127.0.0.1`
- 不默认绑定 `0.0.0.0`
- 不默认开启跨域
- 不承诺公网部署

原因很简单：

- `localcoder` 可以执行 Bash、读写文件、调用网络工具
- 当前仓库还没有完整的远程鉴权和权限隔离

所以 S20 的合理定位是：

- 本地桌面助手的 HTTP / WebSocket 入口
- 给浏览器插件、本地 GUI、脚本或其他本机进程调用

如果后续要开放到局域网或公网，至少还要补：

- Bearer Token 鉴权
- 更严格的权限系统
- 请求级审计
- TLS / `wss`

---

## 十、建议依赖

`Cargo.toml` 需要增加 Axum 相关依赖。S20 只需要最小集合：

- `axum`，启用 `ws` 能力

如果后续要补跨域、trace 或静态文件，再考虑增加额外依赖。  
第一版不要一开始就把 Web 栈铺得太大。

---

## 十一、测试建议

实现后至少补这些测试：

- `/server` 参数解析测试
- `GET /healthz` 返回 200
- `POST /v1/message` 在空消息时返回 400
- `POST /v1/message` 能创建新 session 并返回 reply
- 传入已有 `session_id` 时能续接历史
- WebSocket 非法 JSON 返回 `error`
- REPL 内重复启动 `/server` 不会重复绑定端口
- `/server stop` 能优雅关闭后台任务

---

## 十二、结论

S20 最合理的落地方式，不是把 REPL 代码硬塞进一个路由里，而是：

1. 先把 `engine` 拆成“纯执行”和“终端渲染”两层
2. 用 Axum 新增 `src/server.rs`
3. 在 `main.rs` 和 `repl.rs` 中接入 `/server`
4. 先只支持本地 HTTP + WebSocket
5. 先只做完整消息响应，不做流式 token 推送

这样实现量可控，和当前仓库结构也最兼容。
