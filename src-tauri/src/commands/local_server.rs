//! Tauri IPC commands for the local OpenAI-compatible consumer server.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::local_server::{serve, LocalServerState};
use crate::ShareState;

/// Managed state: the local server's shared state + a handle to abort it.
pub struct LocalServerHandle {
    pub state: Arc<LocalServerState>,
    pub addr: Mutex<Option<SocketAddr>>,
    pub shutdown: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl LocalServerHandle {
    pub fn new(state: Arc<LocalServerState>) -> Arc<Self> {
        Arc::new(Self {
            state,
            addr: Mutex::new(None),
            shutdown: Mutex::new(None),
        })
    }
}

/// Default bind address for the local consumer server.
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8081";

/// Start the local consumer server. Idempotent: re-starting while running
/// returns the existing address.
#[tauri::command]
pub async fn start_local_server(
    handle: tauri::State<'_, Arc<LocalServerHandle>>,
    state: tauri::State<'_, ShareState>,
    bind_addr: Option<String>,
) -> Result<String, String> {
    // Already running?
    {
        let addr = handle.addr.lock().await;
        if let Some(a) = *addr {
            return Ok(a.to_string());
        }
    }

    // Sync cloud base URL + auth from the current ClientConfig.
    let cfg = state.client_config.read().await.clone();
    let cloud_base = host_to_http_base(&cfg.server_host, cfg.use_https);
    if cloud_base.is_empty() {
        return Err("cloud server host not configured".into());
    }

    // Determine auth credentials: prefer ClientConfig.auth_token (API key),
    // fall back to the logged-in user's access_token from AuthState.
    let auth_token = if !cfg.auth_token.is_empty() {
        cfg.auth_token.clone()
    } else {
        crate::auth::token::load_auth_state(&state.db)
            .ok()
            .flatten()
            .map(|s| s.access_token)
            .unwrap_or_default()
    };

    // Determine HMAC secret: prefer ClientConfig.hmac_secret,
    // fall back to empty (access_token uses Bearer auth, not HMAC).
    let hmac_secret = cfg.hmac_secret.clone();

    // Push live config into the server state so it picks up edits.
    *handle.state.cloud_base_url.write().await = cloud_base;
    *handle.state.auth_token.write().await = auth_token;
    *handle.state.hmac_secret.write().await = hmac_secret;

    let addr: SocketAddr = bind_addr
        .unwrap_or_else(|| DEFAULT_BIND_ADDR.into())
        .parse()
        .map_err(|e: std::net::AddrParseError| format!("invalid bind addr: {e}"))?;

    let server_state = handle.state.clone();
    let join = tokio::spawn(async move {
        if let Err(e) = serve(addr, server_state).await {
            log::error!("local server stopped: {e}");
        }
    });

    *handle.addr.lock().await = Some(addr);
    *handle.shutdown.lock().await = Some(join);
    log::info!("✓ local OpenAI server listening on http://{addr}/v1");
    Ok(addr.to_string())
}

/// Stop the local consumer server.
#[tauri::command]
pub async fn stop_local_server(
    handle: tauri::State<'_, Arc<LocalServerHandle>>,
) -> Result<(), String> {
    let join = {
        let mut shutdown = handle.shutdown.lock().await;
        shutdown.take()
    };
    if let Some(j) = join {
        j.abort();
    }
    *handle.addr.lock().await = None;
    Ok(())
}

/// Get the current local server address (empty if not running).
#[tauri::command]
pub async fn get_local_server_addr(
    handle: tauri::State<'_, Arc<LocalServerHandle>>,
) -> Result<String, String> {
    let addr = handle.addr.lock().await;
    Ok(addr.map(|a| a.to_string()).unwrap_or_default())
}

/// Convert a user-provided server host (domain or domain:port) to an HTTP(S) base URL.
///
/// Respects the `use_https` flag for plain domains; explicit scheme prefixes
/// always override the flag.
fn host_to_http_base(host: &str, use_https: bool) -> String {
    crate::url_utils::build_http_base_with_tls(host, use_https)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_to_http_pure_domain_no_tls() {
        assert_eq!(host_to_http_base("api.cc-share.com", false), "http://api.cc-share.com");
    }

    #[test]
    fn host_to_http_pure_domain_with_tls() {
        assert_eq!(host_to_http_base("api.cc-share.com", true), "https://api.cc-share.com");
    }

    #[test]
    fn host_to_http_with_port() {
        assert_eq!(host_to_http_base("192.168.1.60:8080", false), "http://192.168.1.60:8080");
    }

    #[test]
    fn host_to_http_strips_protocol() {
        assert_eq!(host_to_http_base("wss://api.cc-share.com/api/v1/agent/connect", true), "https://api.cc-share.com");
    }

    #[test]
    fn host_to_http_empty() {
        assert_eq!(host_to_http_base("", false), "");
    }

    #[test]
    fn host_to_http_with_port_in_domain() {
        assert_eq!(host_to_http_base("share.example.com:8443", true), "https://share.example.com:8443");
    }
}
