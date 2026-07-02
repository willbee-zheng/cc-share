//! P2P event emission helpers.
//!
//! These functions emit Tauri events to the frontend so the UI can display
//! real-time P2P connection status. They are thin wrappers around
//! `tauri::Emitter::emit` that serialize structured payloads.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::events;

/// P2P session state values — must match the frontend `P2PSessionState` type.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum P2PSessionState {
    AwaitingAnswer,
    Connecting,
    Connected,
    Executing,
    Completed,
    Failed,
}

/// Payload for `share:p2p-session-state` events.
#[derive(Debug, Clone, Serialize)]
pub struct P2PSessionEvent {
    pub session_id: String,
    pub state: P2PSessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// P2P connection status values.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum P2PConnStatus {
    Started,
    Stopped,
    PeerConnected,
    PeerDisconnected,
}

/// Payload for `share:p2p-connection-status` events.
#[derive(Debug, Clone, Serialize)]
pub struct P2PConnectionEvent {
    pub status: P2PConnStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_address: Option<String>,
    pub active_connections: usize,
}

/// Emit a P2P session state change event to the frontend.
pub fn emit_session_state(app: &AppHandle, event: P2PSessionEvent) {
    if let Err(e) = app.emit(events::P2P_SESSION_STATE, &event) {
        log::warn!("P2P: failed to emit session state event: {e}");
    }
}

/// Emit a P2P connection status change event to the frontend.
pub fn emit_connection_status(app: &AppHandle, event: P2PConnectionEvent) {
    if let Err(e) = app.emit(events::P2P_CONNECTION_STATUS, &event) {
        log::warn!("P2P: failed to emit connection status event: {e}");
    }
}