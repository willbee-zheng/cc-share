//! Tauri IPC commands for auth operations.

use crate::auth::callback;
use crate::auth::{AuthClient, AuthState};
use crate::ShareState;
use tauri::{Emitter, State};

/// Get the current auth state from the database.
#[tauri::command]
pub fn get_auth_state(state: State<'_, ShareState>) -> Result<Option<AuthState>, String> {
    crate::auth::token::load_auth_state(&state.db).map_err(|e| e.to_string())
}

/// Register a new user account on the cloud server.
#[tauri::command]
pub async fn auth_register(
    app: tauri::AppHandle<tauri::Wry>,
    state: State<'_, ShareState>,
    server_host: String,
    email: String,
    password: String,
    display_name: Option<String>,
) -> Result<AuthState, String> {
    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    let (auth_state, _profile) = client
        .register(&email, &password, display_name.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    // Persist auth state to database.
    crate::auth::token::save_auth_state(&state.db, &auth_state)
        .map_err(|e| e.to_string())?;

    // Notify frontend of auth state change.
    let _ = app.emit(crate::events::AUTH_STATE_CHANGED, &auth_state);

    Ok(auth_state)
}

/// Login with email and password.
#[tauri::command]
pub async fn auth_login(
    app: tauri::AppHandle<tauri::Wry>,
    state: State<'_, ShareState>,
    server_host: String,
    email: String,
    password: String,
) -> Result<AuthState, String> {
    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    let auth_state = client
        .login(&email, &password)
        .await
        .map_err(|e| e.to_string())?;

    // Persist auth state.
    crate::auth::token::save_auth_state(&state.db, &auth_state)
        .map_err(|e| e.to_string())?;

    // Notify frontend of auth state change.
    let _ = app.emit(crate::events::AUTH_STATE_CHANGED, &auth_state);

    Ok(auth_state)
}

/// Logout by clearing auth state and revoking the refresh token.
#[tauri::command]
pub async fn auth_logout(
    app: tauri::AppHandle<tauri::Wry>,
    state: State<'_, ShareState>,
    server_host: String,
) -> Result<(), String> {
    // Load current auth state to get the refresh token.
    let auth_state = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| e.to_string())?;

    if let Some(st) = auth_state {
        let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
        let client = AuthClient::new(&base_url);
        // Best-effort: revoke refresh token on server.
        let _ = client.logout(&st.access_token, &st.refresh_token).await;
    }

    // Clear local auth state regardless of server result.
    crate::auth::token::clear_auth_state(&state.db).map_err(|e| e.to_string())?;

    // Notify frontend of auth state change (logged out = null).
    let _ = app.emit(crate::events::AUTH_STATE_CHANGED, serde_json::json!(null));

    Ok(())
}

/// Refresh the access token using the stored refresh token.
#[tauri::command]
pub async fn auth_refresh(
    state: State<'_, ShareState>,
    server_host: String,
) -> Result<AuthState, String> {
    let auth_state = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| e.to_string())?
        .ok_or("Not logged in")?;

    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    let mut new_state = client
        .refresh_token(&auth_state.refresh_token)
        .await
        .map_err(|e| e.to_string())?;

    // Preserve user info from the previous state.
    new_state.user_id = auth_state.user_id.clone();
    new_state.email = auth_state.email.clone();
    new_state.display_name = auth_state.display_name.clone();
    new_state.role = auth_state.role.clone();

    // Persist updated auth state.
    crate::auth::token::save_auth_state(&state.db, &new_state)
        .map_err(|e| e.to_string())?;

    Ok(new_state)
}

/// Change the user's password.
#[tauri::command]
pub async fn auth_change_password(
    state: State<'_, ShareState>,
    server_host: String,
    current_password: String,
    new_password: String,
) -> Result<(), String> {
    let auth_state = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| e.to_string())?
        .ok_or("Not logged in")?;

    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    client
        .change_password(&auth_state.access_token, &current_password, &new_password)
        .await
        .map_err(|e| e.to_string())
}

/// Get the user's profile from the cloud server.
#[tauri::command]
pub async fn auth_get_profile(
    state: State<'_, ShareState>,
    server_host: String,
) -> Result<crate::auth::UserProfile, String> {
    let auth_state = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| e.to_string())?
        .ok_or("Not logged in")?;

    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    client
        .get_profile(&auth_state.access_token)
        .await
        .map_err(|e| e.to_string())
}

/// Create a new API key.
#[tauri::command]
pub async fn auth_create_api_key(
    state: State<'_, ShareState>,
    server_host: String,
    name: String,
    permissions: Vec<String>,
) -> Result<crate::auth::CreateKeyResponse, String> {
    let auth_state = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| e.to_string())?
        .ok_or("Not logged in")?;

    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    let key_resp = client
        .create_api_key(&auth_state.access_token, &name, permissions)
        .await
        .map_err(|e| e.to_string())?;

    // Save the API key to client_config for connection use.
    {
        let mut config = state.client_config.write().await;
        config.auth_token = key_resp.key.clone();
        // Clear the old hmac_secret since API key replaces both.
        config.hmac_secret = String::new();
    }

    Ok(key_resp)
}

/// List all API keys for the current user.
#[tauri::command]
pub async fn auth_list_api_keys(
    state: State<'_, ShareState>,
    server_host: String,
) -> Result<Vec<crate::auth::ApiKeyInfo>, String> {
    let auth_state = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| e.to_string())?
        .ok_or("Not logged in")?;

    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    client
        .list_api_keys(&auth_state.access_token)
        .await
        .map_err(|e| e.to_string())
}

/// Revoke (delete) an API key.
#[tauri::command]
pub async fn auth_revoke_api_key(
    state: State<'_, ShareState>,
    server_host: String,
    key_id: String,
) -> Result<(), String> {
    let auth_state = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| e.to_string())?
        .ok_or("Not logged in")?;

    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    let client = AuthClient::new(&base_url);
    client
        .revoke_api_key(&auth_state.access_token, &key_id)
        .await
        .map_err(|e| e.to_string())
}

/// Initiate browser-based login.
///
/// Starts a local HTTP callback server, opens the browser to the
/// cloud-dashboard's desktop-login page, and waits for the callback.
/// Returns the auth state on success or an error on timeout.
///
/// The browser URL is derived from `server_host` by stripping the `api.`
/// prefix and defaulting to HTTP, since the dashboard is a separate web
/// app (e.g. `api.shareplan.com` → `http://shareplan.com/desktop-login`).
#[tauri::command]
pub async fn auth_browser_login(
    app: tauri::AppHandle<tauri::Wry>,
    state: State<'_, ShareState>,
    server_host: String,
) -> Result<AuthState, String> {
    let base_url = build_base_url(&server_host, state.client_config.read().await.use_https);
    if base_url.is_empty() {
        return Err("Server host not configured".to_string());
    }

    // Derive the dashboard URL from server_host by stripping the "api." prefix.
    // e.g. "api.shareplan.com" → "http://shareplan.com"
    let dashboard_url = crate::url_utils::build_dashboard_base(&server_host);
    if dashboard_url.is_empty() {
        return Err("Dashboard host could not be determined".to_string());
    }

    // Start the callback server (async — runs on the Tokio runtime).
    let handle = callback::start_callback_server(state.db.clone()).await?;
    let port = handle.port;
    let state_param = handle.state_param.clone();
    let result_rx = handle.result_rx;
    let server_abort = handle.server_abort;

    // Construct the browser URL — uses the dashboard URL, not the API URL,
    // because /desktop-login is a client-side route served by the dashboard app.
    let callback_url = format!("http://127.0.0.1:{port}/auth/callback");
    let browser_url = format!(
        "{dashboard_url}/desktop-login?callback={callback}&state={state}",
        dashboard_url = dashboard_url,
        callback = urlencoding::encode(&callback_url),
        state = state_param,
    );

    // Open the URL in the default browser.
    tauri_plugin_opener::open_url(&browser_url, None::<String>)
        .map_err(|e| format!("failed to open browser: {e}"))?;

    // Wait for the callback result with a 5-minute timeout.
    let mut result_rx = result_rx;
    let result = tokio::time::timeout(std::time::Duration::from_secs(300), &mut result_rx)
        .await
        .map_err(|_| "browser login timed out (5 minutes)".to_string())?
        .map_err(|_| "callback channel closed unexpectedly".to_string())?;

    // Shut down the callback server.
    server_abort.abort();

    // Emit event to notify frontend of auth state change.
    if let Ok(ref auth_state) = result {
        let _ = app.emit(crate::events::AUTH_STATE_CHANGED, auth_state);
    }

    result
}

/// Helper: build the HTTP base URL from a server host string.
/// Respects explicit `http://` or `https://` prefixes from the user.
/// When no prefix, uses the `use_https` flag from ClientConfig.
fn build_base_url(host: &str, use_https: bool) -> String {
    crate::url_utils::build_http_base_with_tls(host, use_https)
}