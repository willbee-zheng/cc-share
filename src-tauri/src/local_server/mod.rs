//! Local OpenAI-compatible HTTP server (consumer entrypoint).
//!
//! cc-share listens on `127.0.0.1:8081` (configurable). Users point any
//! OpenAI-compatible client (including cc-switch itself, by adding a custom
//! provider) at `http://127.0.0.1:8081/v1`. The server forwards requests to
//! the SharePlan cloud-server `/api/v1/dispatch` endpoint, signing with the
//! user's JWT + HMAC.
//!
//! Supported endpoints:
//! - `GET  /v1/models`            — static model catalog
//! - `POST /v1/chat/completions`  — OpenAI Chat Completions format
//! - `POST /v1/messages`          — Anthropic Messages API format
//! - `GET  /health`               — liveness check
//!
//! Non-streaming: one JSON response in the respective API shape.
//! Streaming: SSE in the respective API shape (OpenAI chunks or Anthropic events).

pub mod anthropic_compat;
pub mod openai_compat;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, routing::post, Router};
use tokio::sync::RwLock;

use crate::database::ShareDb;

/// Local server shared state: cloud base URL + auth + http client.
/// `cloud_base_url` / `auth_token` are read from the live ClientConfig so
/// users editing config in the UI take effect without restarting the server.
/// When `auth_token` is empty, the handler falls back to the logged-in user's
/// access_token from AuthState (via `db`).
pub struct LocalServerState {
    pub cloud_base_url: RwLock<String>,
    pub auth_token: RwLock<String>,
    pub hmac_secret: RwLock<String>,
    pub http: reqwest::Client,
    /// Database handle for reading AuthState when auth_token is empty.
    pub db: Arc<ShareDb>,
}

impl LocalServerState {
    pub fn new(cloud_base_url: String, auth_token: String, hmac_secret: String, db: Arc<ShareDb>) -> Arc<Self> {
        Arc::new(Self {
            cloud_base_url: RwLock::new(cloud_base_url),
            auth_token: RwLock::new(auth_token),
            hmac_secret: RwLock::new(hmac_secret),
            http: crate::http_client::shareplan_client(),
            db,
        })
    }
}

/// Build the axum router for the local OpenAI-compatible server.
pub fn router(state: Arc<LocalServerState>) -> Router {
    Router::new()
        .route("/v1/models", get(openai_compat::list_models))
        .route("/v1/chat/completions", post(openai_compat::chat_completions))
        .route("/v1/messages", post(anthropic_compat::messages))
        .route("/health", get(|| async { "ok" }))
        .with_state(state)
}

/// Start the local server bound to `addr`. Returns a JoinHandle; caller
/// decides when to abort.
pub async fn serve(addr: SocketAddr, state: Arc<LocalServerState>) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;
    let app = router(state);
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("serve: {e}"))
}