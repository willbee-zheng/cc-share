//! Auth callback server — temporary HTTP server for browser-based login.
//!
//! When the user clicks "Sign in with browser", the desktop app starts a
//! temporary Axum server on `127.0.0.1:0` (OS-assigned port).  The browser
//! is opened to the cloud-dashboard's `/desktop-login` page with a `callback`
//! URL pointing back to this server.  After the user authenticates, the
//! cloud-dashboard redirects to the callback URL with the auth tokens in
//! query parameters.  The handler validates the state, persists the auth
//! state, and returns an HTML success page.

use crate::auth::token::{self, AuthState};
use crate::database::ShareDb;
use axum::extract::Query;
use axum::extract::State as AxumState;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use rand::Rng;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

/// Auth data received from the browser callback redirect.
#[derive(Debug, Clone, Deserialize)]
pub struct CallbackParams {
    pub state: String,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: i64,
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
}

/// Shared state for the callback server.
pub struct CallbackState {
    /// Expected `state` parameter for CSRF validation.
    pub expected_state: String,
    /// Database handle for persisting auth state.
    pub db: Arc<ShareDb>,
    /// Sender to notify the Tauri command of the result.
    pub result_tx: Mutex<Option<oneshot::Sender<Result<AuthState, String>>>>,
}

/// Result of starting the callback server.
pub struct CallbackHandle {
    /// The port the server is listening on.
    pub port: u16,
    /// The random state parameter included in the browser URL.
    pub state_param: String,
    /// Receiver for the auth result.
    pub result_rx: oneshot::Receiver<Result<AuthState, String>>,
    /// Handle to abort the server task.
    pub server_abort: tokio::task::JoinHandle<()>,
}

impl CallbackHandle {
    /// Abort the callback server.
    pub fn abort(self) {
        self.server_abort.abort();
    }
}

/// Start the callback server on a random port (async).
///
/// Returns a `CallbackHandle` with the port, state parameter, and result receiver.
pub async fn start_callback_server(db: Arc<ShareDb>) -> Result<CallbackHandle, String> {
    // Generate random state parameter for CSRF protection.
    let state_param = generate_state();

    let (result_tx, result_rx) = oneshot::channel();
    let callback_state = Arc::new(CallbackState {
        expected_state: state_param.clone(),
        db,
        result_tx: Mutex::new(Some(result_tx)),
    });

    // Bind to 127.0.0.1:0 to get a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind callback server: {e}"))?;

    let port = listener.local_addr().map_err(|e| format!("get local addr: {e}"))?.port();

    let app = Router::new()
        .route("/auth/callback", get(callback_handler))
        .with_state(callback_state);

    let server_abort = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    Ok(CallbackHandle {
        port,
        state_param,
        result_rx,
        server_abort,
    })
}

/// Axum handler for `GET /auth/callback`.
async fn callback_handler(
    AxumState(state): AxumState<Arc<CallbackState>>,
    Query(params): Query<CallbackParams>,
) -> impl IntoResponse {
    // Validate state parameter.
    if params.state != state.expected_state {
        return (
            StatusCode::BAD_REQUEST,
            Html(html_error_page("Invalid state parameter. Please try again.")),
        )
            .into_response();
    }

    // Construct auth state.
    let auth_state = AuthState {
        user_id: params.user_id,
        email: params.email,
        display_name: params.display_name,
        role: params.role,
        access_token: params.access_token,
        refresh_token: params.refresh_token,
        access_expires_at: params.access_expires_at,
    };

    // Persist to database.
    if let Err(e) = token::save_auth_state(&state.db, &auth_state) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(html_error_page(&format!("Failed to save auth state: {e}"))),
        )
            .into_response();
    }

    // Send result to the Tauri command.
    {
        let mut tx = state.result_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(Ok(auth_state.clone()));
        }
    }

    // Return success HTML.
    Html(html_success_page()).into_response()
}

/// Generate a random hex string for the state parameter.
fn generate_state() -> String {
    let mut rng = rand::thread_rng();
    (0..32).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
}

fn html_success_page() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>SharePlan - Sign In Successful</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
         display: flex; justify-content: center; align-items: center; min-height: 100vh;
         margin: 0; background: #f9fafb; color: #111827; }
  .card { text-align: center; padding: 2rem; background: white; border-radius: 0.75rem;
          box-shadow: 0 1px 3px rgba(0,0,0,0.1); max-width: 24rem; }
  .icon { font-size: 3rem; margin-bottom: 1rem; }
  h1 { font-size: 1.25rem; margin: 0 0 0.5rem; }
  p { color: #6b7280; font-size: 0.875rem; line-height: 1.5; }
</style>
</head>
<body>
<div class="card">
  <div class="icon">&#10003;</div>
  <h1>Sign In Successful</h1>
  <p>You can close this tab and return to the SharePlan app.</p>
</div>
</body>
</html>"#.to_string()
}

fn html_error_page(msg: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>SharePlan - Error</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
         display: flex; justify-content: center; align-items: center; min-height: 100vh;
         margin: 0; background: #f9fafb; color: #111827; }}
  .card {{ text-align: center; padding: 2rem; background: white; border-radius: 0.75rem;
           box-shadow: 0 1px 3px rgba(0,0,0,0.1); max-width: 24rem; }}
  .icon {{ font-size: 3rem; margin-bottom: 1rem; color: #ef4444; }}
  h1 {{ font-size: 1.25rem; margin: 0 0 0.5rem; }}
  p {{ color: #6b7280; font-size: 0.875rem; line-height: 1.5; }}
</style>
</head>
<body>
<div class="card">
  <div class="icon">&#10007;</div>
  <h1>Authentication Error</h1>
  <p>{msg}</p>
</div>
</body>
</html>"#,
        msg = msg,
    )
}