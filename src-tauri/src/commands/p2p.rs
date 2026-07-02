//! P2P status and management IPC commands for the Tauri frontend.
//!
//! These commands allow the UI to:
//! - Check P2P connection status (running, port, active connections)
//! - Get the local P2P endpoint address
//! - Get the P2P public key
//! - Start/stop the P2P connection manager

use crate::p2p::connection::{P2PConnectionManager, DEFAULT_P2P_PORT};
use crate::p2p::key::P2PKeyManager;
use crate::ShareState;
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

    Ok(serde_json::json!({
        "enabled": true,
        "running": is_running,
        "port": DEFAULT_P2P_PORT,
        "public_key": state.p2p_key_manager.public_key_base64(),
        "local_addresses": local_addrs,
        "active_connections": active,
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