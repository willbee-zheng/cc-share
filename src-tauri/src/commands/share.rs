//! 共享设置相关 Tauri 命令
//!
//! 前端通过这些命令管理供应者共享策略、连接状态等。

use crate::database::dao_share::ShareSettings;
use crate::events;
use crate::share::client::{ClientConfig, ConnectionState};
use crate::share::daemon::DaemonEvent;
use crate::share::protocol::{NodeState, NodeStatus};
use crate::ShareState;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime};

#[tauri::command]
pub async fn get_share_settings(
    state: tauri::State<'_, ShareState>,
    provider_id: String,
    app_type: String,
) -> Result<Option<ShareSettings>, String> {
    state
        .db
        .get_share_settings(&provider_id, &app_type)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn upsert_share_settings(
    state: tauri::State<'_, ShareState>,
    settings: ShareSettings,
) -> Result<(), String> {
    state
        .db
        .upsert_share_settings(&settings)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_all_sharing_providers(
    state: tauri::State<'_, ShareState>,
) -> Result<Vec<ShareSettings>, String> {
    state
        .db
        .get_all_sharing_providers()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_share_settings(
    state: tauri::State<'_, ShareState>,
    provider_id: String,
    app_type: String,
) -> Result<(), String> {
    state
        .db
        .delete_share_settings(&provider_id, &app_type)
        .map_err(|e| e.to_string())
}

/// 读取持久化的客户端配置（前端首次加载用）
#[tauri::command]
pub async fn get_client_config(
    state: tauri::State<'_, ShareState>,
) -> Result<ClientConfig, String> {
    let saved = state.db.load_client_config().map_err(|e| e.to_string())?;
    let cfg = saved.unwrap_or_default();
    // 同步到内存缓存
    *state.client_config.write().await = cfg.clone();
    Ok(cfg)
}

/// 持久化客户端配置；同时刷新内存缓存
#[tauri::command]
pub async fn set_client_config(
    state: tauri::State<'_, ShareState>,
    config: ClientConfig,
) -> Result<(), String> {
    state.db.save_client_config(&config).map_err(|e| e.to_string())?;
    *state.client_config.write().await = config;
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct ShareConnectArgs {
    /// 节点 ID（默认从配置读取）
    pub node_id: Option<String>,
    /// 当前可分享的模型列表
    #[serde(default)]
    pub available_models: Vec<String>,
    /// 最大并发任务数（默认 1）
    pub max_concurrency: Option<u32>,
}

#[tauri::command]
pub async fn share_connect<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, ShareState>,
    provider_state: tauri::State<'_, crate::commands::providers::ProviderState>,
    args: ShareConnectArgs,
) -> Result<(), String> {
    log::info!("▶ share_connect: node_id={:?}, available_models={:?}, max_concurrency={:?}",
        args.node_id, args.available_models, args.max_concurrency);

    let config: ClientConfig = state.client_config.read().await.clone();
    if config.server_host.is_empty() {
        log::error!("share_connect failed: server_host not configured");
        return Err("服务器地址未配置".into());
    }
    if config.auth_token.is_empty() {
        log::error!("share_connect failed: auth_token not configured");
        return Err("认证令牌未配置".into());
    }

    let node_id = args
        .node_id
        .unwrap_or_else(|| config.node_id.clone())
        .trim()
        .to_string();
    if node_id.is_empty() {
        log::error!("share_connect failed: node_id is empty");
        return Err("node_id 未配置".into());
    }

    // If the frontend didn't pass available_models, auto-derive from the
    // cc-switch proxy discovery snapshot, filtered by the saved whitelist.
    // Also extract upstream model mappings (representative → real model names).
    let (available_models, upstream_models) = if args.available_models.is_empty() {
        let snap = provider_state.registry.snapshot().await;
        let whitelist = provider_state
            .db
            .load_whitelist()
            .map_err(|e| e.to_string())?;
        log::info!(
            "share_connect: snapshot — reachable={}, running={}, providers={}, models={:?}, from_db={}, whitelist={:?}",
            snap.reachable,
            snap.running,
            snap.providers.len(),
            snap.available_models,
            snap.from_db,
            whitelist,
        );
        let models = crate::commands::providers::filter_models(snap.available_models, whitelist);
        log::info!("share_connect: auto-derived available_models={:?}", models);
        (models, snap.upstream_models)
    } else {
        (args.available_models, HashMap::new())
    };

    if available_models.is_empty() {
        log::error!("share_connect: no models available — cc-switch may not be running");
        return Err("无法获取可共享的模型列表，请确认 cc-switch 已启动并配置了供应商".into());
    }

    let status = NodeStatus {
        node_id: node_id.clone(),
        state: NodeState::Idle,
        available_models,
        upstream_models,
        current_concurrency: 0,
        max_concurrency: args.max_concurrency.unwrap_or(1),
        p2p_public_key: None,
    };

    log::info!("share_connect: connecting node_id={}, max_concurrency={}", node_id, status.max_concurrency);

    {
        let mut conn_state = state.connection_state.write().await;
        *conn_state = ConnectionState::Connecting;
    }

    let app_for_events = app.clone();
    let conn_state_for_cb: Arc<tokio::sync::RwLock<ConnectionState>> =
        state.connection_state.clone();

    let mut daemon = state.daemon.lock().await;
    daemon
        .start(config, status, move |evt| match &evt {
            DaemonEvent::ConnectionState(s) => {
                log::info!("share_connect: connection state changed to {:?}", s);
                if let Ok(mut guard) = conn_state_for_cb.try_write() {
                    *guard = s.clone();
                }
                let _ = app_for_events.emit(events::CONNECTION_STATE, s.clone());
            }
            DaemonEvent::ConnectionError { category, message } => {
                log::warn!("share_connect: connection error [{}]: {}", category, message);
                let payload = serde_json::json!({
                    "category": category,
                    "message": message,
                });
                let _ = app_for_events.emit(events::CONNECTION_ERROR, payload);
            }
            DaemonEvent::TaskFinished {
                task_id,
                status,
                latency_ms,
            } => {
                log::info!("share_connect: task {} finished with status={:?}, latency={}ms", task_id, status, latency_ms);
                let payload = serde_json::json!({
                    "task_id": task_id,
                    "status": status,
                    "latency_ms": latency_ms,
                });
                let _ = app_for_events.emit(events::TASK_FINISHED, payload);
            }
        })
        .map_err(|e| {
            log::error!("share_connect: daemon start failed - {}", e);
            e.to_string()
        })?;

    log::info!("✓ share_connect: daemon started successfully for node_id={}", node_id);
    Ok(())
}

#[tauri::command]
pub async fn share_disconnect<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, ShareState>,
) -> Result<(), String> {
    log::info!("◀ share_disconnect: stopping daemon");
    {
        let mut conn_state = state.connection_state.write().await;
        *conn_state = ConnectionState::Disconnected;
    }
    // Notify frontend immediately
    let _ = app.emit(events::CONNECTION_STATE, ConnectionState::Disconnected);
    let mut daemon = state.daemon.lock().await;
    daemon.stop().await;
    log::info!("✓ share_disconnect: daemon stopped successfully");
    Ok(())
}

#[tauri::command]
pub async fn share_get_status(state: tauri::State<'_, ShareState>) -> Result<String, String> {
    let conn_state = state.connection_state.read().await;
    let status = format!("{:?}", *conn_state).to_lowercase();
    log::debug!("share_get_status: {}", status);
    Ok(status)
}

/// 检查云端服务器健康状态
///
/// 向 server_url 的 health 端点发送 HTTP GET 请求，
/// 返回 { healthy: bool, latency_ms: u64, error: Option<String> }。
#[tauri::command]
pub async fn check_server_health(
    state: tauri::State<'_, ShareState>,
) -> Result<serde_json::Value, String> {
    let config = state.client_config.read().await.clone();
    if config.server_host.is_empty() {
        return Ok(serde_json::json!({
            "healthy": false,
            "latency_ms": 0,
            "error": "server_host not configured"
        }));
    }

    // Derive health check URL from the server host.
    // e.g. api.cc-share.com -> https://api.cc-share.com/health
    //      192.168.1.60:8080 -> http://192.168.1.60:8080/health
    let health_url = derive_health_url(&config.server_host, config.use_https);

    let start = std::time::Instant::now();
    let client = crate::http_client::shareplan_client_builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    match client.get(&health_url).send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let status = resp.status();
            if status.is_success() {
                Ok(serde_json::json!({
                    "healthy": true,
                    "latency_ms": latency_ms,
                    "error": null
                }))
            } else {
                Ok(serde_json::json!({
                    "healthy": false,
                    "latency_ms": latency_ms,
                    "error": format!("HTTP {}", status.as_u16())
                }))
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            Ok(serde_json::json!({
                "healthy": false,
                "latency_ms": latency_ms,
                "error": e.to_string()
            }))
        }
    }
}

/// 根据用户填写的服务器地址推导 health 检查 URL。
///
/// Respects explicit `http://` or `https://` prefixes.
/// When no prefix, `use_https` controls the scheme.
fn derive_health_url(host: &str, use_https: bool) -> String {
    let base = crate::url_utils::build_http_base_with_tls(host, use_https);
    if base.is_empty() {
        return String::new();
    }
    format!("{base}/health")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_health_url_pure_domain_no_tls() {
        let result = derive_health_url("api.cc-share.com", false);
        assert_eq!(result, "http://api.cc-share.com/health");
    }

    #[test]
    fn test_derive_health_url_pure_domain_with_tls() {
        let result = derive_health_url("api.cc-share.com", true);
        assert_eq!(result, "https://api.cc-share.com/health");
    }

    #[test]
    fn test_derive_health_url_with_port() {
        let result = derive_health_url("192.168.1.60:8080", false);
        assert_eq!(result, "http://192.168.1.60:8080/health");
    }

    #[test]
    fn test_derive_health_url_explicit_http() {
        let result = derive_health_url("http://test.local", false);
        assert_eq!(result, "http://test.local/health");
    }

    #[test]
    fn test_derive_health_url_explicit_https() {
        let result = derive_health_url("https://api.cc-share.com", false);
        assert_eq!(result, "https://api.cc-share.com/health");
    }

    #[test]
    fn test_derive_health_url_strips_protocol_prefix() {
        let result = derive_health_url("wss://api.cc-share.com/api/v1/agent/connect", false);
        assert_eq!(result, "https://api.cc-share.com/health");
    }

    #[test]
    fn test_derive_health_url_empty() {
        assert_eq!(derive_health_url("", false), "");
    }
}
