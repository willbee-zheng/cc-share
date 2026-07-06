//! P2P status and management IPC commands for the Tauri frontend.
//!
//! These commands allow the UI to:
//! - Check P2P connection status (running, port, active connections)
//! - Get the local P2P endpoint address
//! - Get the P2P public key
//! - Start/stop the P2P connection manager
//! - Get/set P2P configuration (hole punch retries, STUN server, etc.)

use crate::database::dao_config::P2PConfig;
use crate::p2p::connection::{P2PConnectionManager, DEFAULT_P2P_PORT};
use crate::p2p::key::P2PKeyManager;
use crate::p2p::stun_client;
use crate::ShareState;
use std::time::Duration;
use tauri::State;

/// Get the P2P public key (base64-encoded X25519 public key).
#[tauri::command]
pub fn p2p_get_public_key(state: State<'_, ShareState>) -> String {
    state.p2p_key_manager.public_key_base64()
}

/// Get the P2P connection status.
///
/// Returns whether the QUIC endpoint is running, the listening port,
/// the public key, local addresses, and the number of active peer connections.
#[tauri::command]
pub async fn p2p_get_status(state: State<'_, ShareState>) -> Result<serde_json::Value, String> {
    let is_running = state.p2p_conn_manager.is_running().await;
    let active = state.p2p_conn_manager.active_connection_count();

    let local_addrs = if is_running {
        state.p2p_conn_manager.local_addr().await
            .map(|addrs| addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let p2p_config = state.db.load_p2p_config().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "enabled": p2p_config.enabled,
        "running": is_running,
        "port": p2p_config.p2p_port,
        "public_key": state.p2p_key_manager.public_key_base64(),
        "local_addresses": local_addrs,
        "active_connections": active,
        "hole_punch_retries": p2p_config.hole_punch_retries,
        "hole_punch_delay_ms": p2p_config.hole_punch_delay_ms,
        "stun_server": p2p_config.stun_server,
    }))
}

/// Start the P2P QUIC endpoint.
#[tauri::command]
pub async fn p2p_start(state: State<'_, ShareState>) -> Result<(), String> {
    state
        .p2p_conn_manager
        .start()
        .await
        .map_err(|e| format!("P2P start failed: {e}"))
}

/// Stop the P2P QUIC endpoint.
#[tauri::command]
pub async fn p2p_stop(state: State<'_, ShareState>) -> Result<(), String> {
    state.p2p_conn_manager.shutdown().await;
    Ok(())
}

/// Get the P2P configuration.
#[tauri::command]
pub fn p2p_get_config(state: State<'_, ShareState>) -> Result<P2PConfig, String> {
    state.db.load_p2p_config().map_err(|e| e.to_string())
}

/// Update the P2P configuration.
#[tauri::command]
pub fn p2p_set_config(state: State<'_, ShareState>, config: P2PConfig) -> Result<(), String> {
    state.db.save_p2p_config(&config).map_err(|e| e.to_string())
}

/// Discover the public IP:port via STUN.
///
/// Uses the configured STUN server (or derives it from the cloud server URL).
/// Returns the public SocketAddr or an error.
#[tauri::command]
pub async fn p2p_discover_public_addr(state: State<'_, ShareState>) -> Result<serde_json::Value, String> {
    let p2p_config = state.db.load_p2p_config().map_err(|e| e.to_string())?;
    let client_config = state.client_config.read().await;

    // Derive STUN server from cloud server host if not explicitly configured.
    let stun_server = if p2p_config.stun_server.is_empty() {
        format!("{}:7890", client_config.server_host)
    } else {
        p2p_config.stun_server.clone()
    };

    match stun_client::discover_public_addr(
        &stun_server,
        p2p_config.p2p_port,
        Duration::from_secs(5),
    ).await {
        Ok(addr) => Ok(serde_json::json!({
            "public_addr": addr.to_string(),
            "stun_server": stun_server,
        })),
        Err(e) => Err(format!("STUN discovery failed: {e}")),
    }
}