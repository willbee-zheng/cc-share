//! P2P session state machine.
//!
//! Tracks the lifecycle of a P2P session from signaling through
//! QUIC connection establishment, task execution, and completion.
//! Emits Tauri events on each state transition so the frontend can
//! display real-time P2P connection status.

use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::AppHandle;

use super::connection::P2PConnectionManager;
use super::crypto;
use super::events::{P2PSessionEvent, P2PSessionState, emit_session_state};
use super::key::P2PKeyManager;
use super::protocol::{self, P2pFrame, P2pMessageType, P2pTaskRequest, P2pTaskResult};

/// P2P session state.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// Waiting for supplier to respond to offer
    AwaitingAnswer,
    /// Exchanging candidates, attempting QUIC connection
    Connecting,
    /// QUIC connection established, ready for tasks
    Connected,
    /// Task in progress over P2P
    Executing,
    /// Task finished, reporting usage
    Completed,
    /// Connection or execution failed
    Failed,
}

impl SessionState {
    /// Convert to the serializable event type.
    fn to_event_state(&self) -> P2PSessionState {
        match self {
            SessionState::AwaitingAnswer => P2PSessionState::AwaitingAnswer,
            SessionState::Connecting => P2PSessionState::Connecting,
            SessionState::Connected => P2PSessionState::Connected,
            SessionState::Executing => P2PSessionState::Executing,
            SessionState::Completed => P2PSessionState::Completed,
            SessionState::Failed => P2PSessionState::Failed,
        }
    }
}

/// A P2P session tracks the full lifecycle of a direct connection.
pub struct P2PSession {
    /// Session ID assigned by the cloud server during signaling.
    pub session_id: String,
    /// Current state.
    pub state: Arc<Mutex<SessionState>>,
    /// E2E encryption key derived from X25519 DH + HKDF.
    task_key: [u8; 32],
    /// QUIC connection manager.
    conn_manager: Arc<P2PConnectionManager>,
    /// Optional Tauri app handle for emitting state events.
    app_handle: Option<AppHandle>,
    /// Optional model name for event payloads.
    model: Option<String>,
}

impl P2PSession {
    /// Create a new P2P session with derived task key.
    pub fn new(
        session_id: String,
        key_manager: &P2PKeyManager,
        peer_pubkey_base64: &str,
        conn_manager: Arc<P2PConnectionManager>,
    ) -> Result<Self, String> {
        // Decode peer's public key from base64.
        let peer_pubkey = base64_decode(peer_pubkey_base64)?;
        let peer_pubkey: [u8; 32] = peer_pubkey
            .try_into()
            .map_err(|_| "peer public key must be 32 bytes".to_string())?;

        // Derive shared secret and task key.
        let shared_secret = key_manager.diffie_hellman(&peer_pubkey);
        let task_key = crypto::derive_task_key(&shared_secret, &session_id);

        Ok(Self {
            session_id,
            state: Arc::new(Mutex::new(SessionState::AwaitingAnswer)),
            task_key,
            conn_manager,
            app_handle: None,
            model: None,
        })
    }

    /// Set the Tauri app handle for emitting state events.
    pub fn set_app_handle(&mut self, handle: AppHandle) {
        self.app_handle = Some(handle);
    }

    /// Set the model name for event payloads.
    pub fn set_model(&mut self, model: String) {
        self.model = Some(model);
    }

    /// Transition to a new state, logging and emitting an event.
    async fn transition(&self, new_state: SessionState, peer_address: Option<String>, error: Option<String>) {
        let mut state = self.state.lock().await;
        let prev = state.clone();
        *state = new_state.clone();
        drop(state);

        log::info!(
            "P2P session {}: {} -> {}",
            self.session_id,
            format!("{:?}", prev).to_lowercase(),
            format!("{:?}", new_state).to_lowercase(),
        );

        if let Some(app) = &self.app_handle {
            emit_session_state(app, P2PSessionEvent {
                session_id: self.session_id.clone(),
                state: new_state.to_event_state(),
                peer_address,
                model: self.model.clone(),
                error,
            });
        }
    }

    /// Send an encrypted task request over a QUIC connection.
    pub async fn send_task_request(
        &self,
        conn: &quinn::Connection,
        request: P2pTaskRequest,
    ) -> Result<(), String> {
        // Serialize the request with MessagePack.
        let frame = protocol::encode_message(P2pMessageType::TaskRequest, &request)?;

        // Open a new QUIC bidirectional stream.
        let (mut send_stream, _recv_stream) = conn
            .open_bi()
            .await
            .map_err(|e| format!("open QUIC stream: {e}"))?;

        // Send the frame.
        send_stream
            .write_all(&frame.encode())
            .await
            .map_err(|e| format!("write QUIC frame: {e}"))?;

        send_stream
            .finish()
            .map_err(|e| format!("finish QUIC stream: {e}"))?;

        // Update state.
        let peer = conn.remote_address().to_string();
        self.transition(SessionState::Executing, Some(peer), None).await;

        Ok(())
    }

    /// Read a task result from a QUIC stream.
    pub async fn read_task_result(
        &self,
        recv_stream: &mut quinn::RecvStream,
    ) -> Result<P2pTaskResult, String> {
        let mut buf = vec![0u8; 64 * 1024]; // 64KB read buffer
        let n = recv_stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("read QUIC stream: {e}"))?
            .ok_or("QUIC stream closed".to_string())?;

        let (frame, _) = P2pFrame::decode(&buf[..n])?
            .ok_or("incomplete frame".to_string())?;

        match frame.msg_type {
            P2pMessageType::TaskResult => {
                let result: P2pTaskResult = protocol::decode_message(&frame)?;
                Ok(result)
            }
            P2pMessageType::SessionClose => {
                let close: protocol::P2pSessionClose = protocol::decode_message(&frame)?;
                Err(format!("session closed: {}", close.reason))
            }
            other => Err(format!("unexpected message type: {:?}", other)),
        }
    }

    /// Read a stream of task result chunks from a QUIC stream.
    /// Returns a channel that produces chunks until the terminal frame.
    pub async fn read_task_stream(
        &self,
        mut recv_stream: quinn::RecvStream,
    ) -> tokio::sync::mpsc::Receiver<Result<P2pTaskResult, String>> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let task_key = self.task_key;

        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match recv_stream.read(&mut buf).await {
                    Ok(Some(n)) => {
                        match P2pFrame::decode(&buf[..n]) {
                            Ok(Some((frame, _))) => match frame.msg_type {
                                P2pMessageType::TaskResult => {
                                    match protocol::decode_message::<P2pTaskResult>(&frame) {
                                        Ok(result) => {
                                            let is_final = result.r#final.unwrap_or(false);
                                            if tx.send(Ok(result)).await.is_err() {
                                                break;
                                            }
                                            if is_final {
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Err(e)).await;
                                            break;
                                        }
                                    }
                                }
                                P2pMessageType::SessionClose => {
                                    let close: protocol::P2pSessionClose =
                                        match protocol::decode_message(&frame) {
                                            Ok(c) => c,
                                            Err(e) => {
                                                let _ = tx.send(Err(e)).await;
                                                break;
                                            }
                                        };
                                    let _ =
                                        tx.send(Err(format!("session closed: {}", close.reason)))
                                            .await;
                                    break;
                                }
                                other => {
                                    let _ =
                                        tx.send(Err(format!("unexpected message type: {:?}", other)))
                                            .await;
                                    break;
                                }
                            },
                            Ok(None) => {
                                // Need more data — continue reading
                                continue;
                            }
                            Err(e) => {
                                let _ = tx.send(Err(format!("frame decode: {e}"))).await;
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        // Stream closed
                        break;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("QUIC read: {e}"))).await;
                        break;
                    }
                }
            }
        });

        rx
    }

    /// Encrypt a plaintext message with the session's task key.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        crypto::encrypt_payload(&self.task_key, plaintext)
    }

    /// Decrypt a ciphertext with the session's task key.
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        crypto::decrypt_payload(&self.task_key, ciphertext)
    }

    /// Mark the session as completed.
    pub async fn mark_completed(&self) {
        self.transition(SessionState::Completed, None, None).await;
    }

    /// Mark the session as failed.
    pub async fn mark_failed(&self, error: Option<String>) {
        self.transition(SessionState::Failed, None, error).await;
    }

    /// Transition to Connecting state (attempting QUIC connection).
    pub async fn mark_connecting(&self) {
        self.transition(SessionState::Connecting, None, None).await;
    }

    /// Transition to Connected state (QUIC connection established).
    pub async fn mark_connected(&self, peer_address: String) {
        self.transition(SessionState::Connected, Some(peer_address), None).await;
    }
}

/// Decode a base64 string to bytes.
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("base64 decode: {e}"))
}