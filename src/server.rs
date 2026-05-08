/*!
 * Server Mode — S20
 */

use crate::api::LLMClient;
use crate::config::AppConfig;
use crate::engine::{self, EngineOptions};
use crate::memory::MemoryStore;
use crate::output_style::OutputStyleManager;
use crate::runtime;
use crate::session::SessionStore;
use anyhow::{Context, Result, anyhow};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
        }
    }
}

impl ServerConfig {
    pub fn parse(value: &str) -> Result<Self> {
        let addr = SocketAddr::from_str(value.trim())
            .with_context(|| format!("invalid server address: {}", value.trim()))?;
        Ok(Self {
            host: addr.ip().to_string(),
            port: addr.port(),
        })
    }

    pub fn socket_addr(&self) -> Result<SocketAddr> {
        let addr = if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        };
        SocketAddr::from_str(&addr)
            .with_context(|| format!("invalid server address: {}:{}", self.host, self.port))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerCommand {
    Start(ServerConfig),
    Status,
    Stop,
}

pub fn parse_server_command(args: &str) -> Result<ServerCommand> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(ServerCommand::Start(ServerConfig::default()));
    }

    if trimmed.eq_ignore_ascii_case("status") {
        return Ok(ServerCommand::Status);
    }

    if trimmed.eq_ignore_ascii_case("stop") {
        return Ok(ServerCommand::Stop);
    }

    Ok(ServerCommand::Start(ServerConfig::parse(trimmed)?))
}

#[derive(Clone)]
pub struct ServerState {
    cwd: PathBuf,
    output_style_manager: OutputStyleManager,
    session_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl ServerState {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        let cwd = cwd.into();
        Self {
            output_style_manager: OutputStyleManager::new(&cwd),
            cwd,
            session_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn process_message(&self, request: MessageRequest) -> ApiResult<MessageResponse> {
        let message = request.message.trim();
        if message.is_empty() {
            return Err(ApiError::bad_request("message must not be empty"));
        }

        let session_id = request
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        let store = match session_id.as_deref() {
            Some(existing) => SessionStore::load(&self.cwd, existing)
                .map_err(|err| ApiError::not_found(err.to_string()))?,
            None => SessionStore::create(&self.cwd).map_err(ApiError::internal)?,
        };
        let session_id = store.id.clone();

        let lock = self.session_lock(&session_id).await;
        let _guard = lock.lock().await;

        let mut messages = if request
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        {
            store.load_messages().map_err(ApiError::internal)?
        } else {
            Vec::new()
        };

        let client = LLMClient::new().map_err(ApiError::internal)?;
        let registry = runtime::build_registry(&self.cwd).map_err(ApiError::internal)?;
        let skill_manager = registry.skill_manager();
        if let Some(manager) = skill_manager.as_ref() {
            manager.set_session_id(Some(&session_id));
        }

        let output_style = self.resolve_output_style(request.output_style.as_deref())?;
        let memory_store = MemoryStore::new(&self.cwd, visible_message_count(&messages))
            .map_err(ApiError::internal)?;
        let system_prompt = runtime::build_base_system_prompt(
            &memory_store,
            &self.output_style_manager,
            &output_style,
            skill_manager.as_ref(),
        )
        .map_err(ApiError::internal)?;

        messages.push(json!({
            "role": "user",
            "content": message,
        }));
        store
            .append_message(
                messages.last().ok_or_else(|| {
                    ApiError::internal(anyhow!("missing just-added user message"))
                })?,
            )
            .map_err(ApiError::internal)?;

        let assistant_start = messages.len();
        let reply = engine::run_agent_loop_with_system_prompt_and_options(
            &client,
            &registry,
            &mut messages,
            system_prompt.as_deref(),
            EngineOptions { silent: true },
        )
        .await
        .map_err(ApiError::internal)?;

        store
            .append_messages(&messages[assistant_start..])
            .map_err(ApiError::internal)?;

        Ok(MessageResponse {
            session_id,
            reply,
            model: client.model().to_string(),
        })
    }

    async fn session_lock(&self, session_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.session_locks.lock().await;
        locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn resolve_output_style(&self, requested_style: Option<&str>) -> ApiResult<String> {
        let app_config = AppConfig::load(&self.cwd).map_err(ApiError::internal)?;
        let requested = requested_style
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&app_config.output_style);

        if !self
            .output_style_manager
            .has_style(requested)
            .map_err(ApiError::internal)?
        {
            return Err(ApiError::bad_request(format!(
                "unknown output style: {}",
                requested
            )));
        }

        Ok(requested.to_string())
    }
}

pub struct ServerHandle {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<()>>,
}

impl ServerHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub async fn stop(mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        self.task
            .await
            .context("server task join failed")?
            .context("server task failed")
    }
}

#[derive(Debug, Deserialize)]
struct MessageRequest {
    message: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    output_style: Option<String>,
}

#[derive(Debug, Serialize)]
struct MessageResponse {
    session_id: String,
    reply: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct WsRequest {
    #[serde(rename = "type")]
    message_type: String,
    message: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    output_style: Option<String>,
}

#[derive(Debug, Serialize)]
struct WsAssistantResponse {
    #[serde(rename = "type")]
    message_type: &'static str,
    session_id: String,
    reply: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct WsErrorResponse {
    #[serde(rename = "type")]
    message_type: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(error: impl Into<anyhow::Error>) -> Self {
        let error = error.into();
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

pub fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/message", post(handle_http_message))
        .route("/v1/ws", get(handle_ws_upgrade))
        .with_state(state)
}

pub async fn start_server(config: ServerConfig, cwd: PathBuf) -> Result<ServerHandle> {
    let listener = TcpListener::bind(config.socket_addr()?)
        .await
        .context("failed to bind server socket")?;
    let addr = listener
        .local_addr()
        .context("failed to resolve bound server address")?;
    let app = build_router(ServerState::new(cwd));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        serve(listener, app, async move {
            let _ = shutdown_rx.await;
        })
        .await
    });

    Ok(ServerHandle {
        addr,
        shutdown_tx: Some(shutdown_tx),
        task,
    })
}

pub async fn run_server_foreground(config: ServerConfig, cwd: &Path) -> Result<()> {
    let listener = TcpListener::bind(config.socket_addr()?)
        .await
        .context("failed to bind server socket")?;
    let addr = listener
        .local_addr()
        .context("failed to resolve bound server address")?;

    println!("🌐 Server listening on http://{}", addr);
    println!("Press Ctrl+C to stop");

    serve(listener, build_router(ServerState::new(cwd)), async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

async fn serve(
    listener: TcpListener,
    app: Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("server exited with error")
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn handle_http_message(
    State(state): State<ServerState>,
    Json(request): Json<MessageRequest>,
) -> ApiResult<Json<MessageResponse>> {
    Ok(Json(state.process_message(request).await?))
}

async fn handle_ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: ServerState) {
    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Text(text)) => {
                let payload = match serde_json::from_str::<WsRequest>(&text) {
                    Ok(payload) => payload,
                    Err(_) => {
                        if send_ws_error(&mut socket, "invalid request json")
                            .await
                            .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                };

                if payload.message_type != "message" {
                    if send_ws_error(&mut socket, "unsupported websocket message type")
                        .await
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }

                let request = MessageRequest {
                    message: payload.message,
                    session_id: payload.session_id,
                    output_style: payload.output_style,
                };

                match state.process_message(request).await {
                    Ok(response) => {
                        let reply = WsAssistantResponse {
                            message_type: "assistant",
                            session_id: response.session_id,
                            reply: response.reply,
                            model: response.model,
                        };
                        if send_ws_json(&mut socket, &reply).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        if send_ws_error(&mut socket, &err.message).await.is_err() {
                            break;
                        }
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

async fn send_ws_error(socket: &mut WebSocket, message: &str) -> Result<()> {
    send_ws_json(
        socket,
        &WsErrorResponse {
            message_type: "error",
            message: message.to_string(),
        },
    )
    .await
}

async fn send_ws_json<T: Serialize>(socket: &mut WebSocket, payload: &T) -> Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(payload)?.into()))
        .await
        .context("failed to send websocket message")
}

fn visible_message_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|msg| matches!(msg["role"].as_str(), Some("user" | "assistant")))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use tempfile::TempDir;

    #[test]
    fn parse_server_command_defaults_to_localhost() {
        assert_eq!(
            parse_server_command("").unwrap(),
            ServerCommand::Start(ServerConfig::default())
        );
    }

    #[test]
    fn parse_server_command_supports_status_and_stop() {
        assert_eq!(
            parse_server_command("status").unwrap(),
            ServerCommand::Status
        );
        assert_eq!(parse_server_command("stop").unwrap(), ServerCommand::Stop);
    }

    #[test]
    fn parse_server_command_parses_socket_addr() {
        assert_eq!(
            parse_server_command("127.0.0.1:4000").unwrap(),
            ServerCommand::Start(ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 4000,
            })
        );
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        let response = healthz().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn post_message_rejects_empty_body_message() {
        let cwd = TempDir::new().unwrap();
        let state = ServerState::new(cwd.path());
        let err = handle_http_message(
            State(state),
            Json(MessageRequest {
                message: "   ".to_string(),
                session_id: None,
                output_style: None,
            }),
        )
        .await
        .unwrap_err();

        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
