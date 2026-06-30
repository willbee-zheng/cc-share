//! Auto token refresh — background task that proactively refreshes the access token
//! before it expires, matching the pattern used by the cloud-dashboard.

use crate::auth::AuthClient;
use crate::database::ShareDb;
use std::sync::Arc;
use tauri::Emitter;
use tokio::time::{self, Duration};

/// How often to check whether the token needs refreshing.
const CHECK_INTERVAL_SECS: u64 = 60;

/// Refresh the token if it will expire within this many seconds.
const REFRESH_BUFFER_SECS: i64 = 300; // 5 minutes

/// Start the auto-refresh background task.
///
/// This should be spawned as a Tokio task during app setup. It runs indefinitely,
/// checking every [`CHECK_INTERVAL_SECS`] seconds whether the stored auth token
/// is about to expire and refreshing it if so.
pub async fn start_auto_refresh(app: tauri::AppHandle<tauri::Wry>, db: Arc<ShareDb>) {
    let mut interval = time::interval(Duration::from_secs(CHECK_INTERVAL_SECS));

    loop {
        interval.tick().await;

        if let Err(e) = maybe_refresh(&app, &db).await {
            log::warn!("auto-refresh error: {e}");
        }
    }
}

/// Check if the token needs refreshing and refresh it if so.
async fn maybe_refresh(
    app: &tauri::AppHandle<tauri::Wry>,
    db: &Arc<ShareDb>,
) -> Result<(), String> {
    let state = crate::auth::token::load_auth_state(db)?;
    let Some(state) = state else {
        // Not logged in — nothing to do.
        return Ok(());
    };

    // Check if the token will expire within the buffer.
    if !crate::auth::token::is_token_expired(&state, REFRESH_BUFFER_SECS) {
        return Ok(());
    }

    log::info!("access token expiring soon, refreshing...");

    // Build the base URL from the client config.
    let config_str = db
        .get_config("client_config_v1")
        .map_err(|e| format!("read config: {e}"))?;
    let config_json = config_str
        .as_ref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let server_host = config_json
        .and_then(|v| v.get("server_host").and_then(|h| h.as_str().map(String::from)))
        .unwrap_or_default();
    let use_https = config_str
        .as_ref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("use_https").and_then(|u| u.as_bool()))
        .unwrap_or(false);

    if server_host.is_empty() {
        return Err("no server_host configured, cannot refresh".to_string());
    }

    let base_url = build_base_url(&server_host, use_https);
    let client = AuthClient::new(&base_url);

    match client.refresh_token(&state.refresh_token).await {
        Ok(mut new_state) => {
            // Preserve user info from the previous state (refresh response may not include it).
            new_state.user_id = state.user_id.clone();
            new_state.email = state.email.clone();
            new_state.display_name = state.display_name.clone();
            new_state.role = state.role.clone();

            crate::auth::token::save_auth_state(db, &new_state)?;
            let _ = app.emit(crate::events::AUTH_STATE_CHANGED, &new_state);
            log::info!("access token refreshed successfully");
            Ok(())
        }
        Err(e) => {
            log::warn!("token refresh failed: {e}");
            // If refresh fails (e.g. refresh token revoked), clear auth state
            // so the user is prompted to log in again.
            crate::auth::token::clear_auth_state(db)?;
            let _ = app.emit(
                crate::events::AUTH_STATE_CHANGED,
                serde_json::json!(null),
            );
            Err(format!("refresh failed: {e}"))
        }
    }
}

/// Build the HTTP base URL from a server host string.
/// Respects explicit http:// or https:// prefixes.
fn build_base_url(host: &str, use_https: bool) -> String {
    crate::url_utils::build_http_base_with_tls(host, use_https)
}