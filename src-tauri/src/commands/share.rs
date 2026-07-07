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
            DaemonEvent::HealthUpdate { latency_ms } => {
                log::debug!("share_connect: health update latency={}ms", latency_ms);
                let payload = serde_json::json!({
                    "healthy": true,
                    "latency_ms": latency_ms,
                });
                let _ = app_for_events.emit(events::HEALTH_UPDATE, payload);
            }
        })
        .map_err(|e| {
            log::error!("share_connect: daemon start failed - {}", e);
            e.to_string()
        })?;

    log::info!("✓ share_connect: daemon started successfully for node_id={}", node_id);

    // Auto-start P2P endpoint alongside the share daemon.
    if !state.p2p_conn_manager.is_running().await {
        match state.p2p_conn_manager.start().await {
            Ok(()) => log::info!("share_connect: P2P endpoint auto-started"),
            Err(e) => log::warn!("share_connect: P2P endpoint auto-start failed (non-fatal): {e}"),
        }
    }

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

    // Auto-stop P2P endpoint alongside the share daemon.
    if state.p2p_conn_manager.is_running().await {
        state.p2p_conn_manager.shutdown().await;
        log::info!("share_disconnect: P2P endpoint auto-stopped");
    }

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

#[cfg(test)]
mod tests {
    use super::*;
}
