//! Supplier-side P2P task handler.
//!
//! Accepts incoming QUIC connections, reads encrypted P2P task requests,
//! executes them via the ProxyExecutor, and sends encrypted results back.

use std::collections::HashMap;
use std::sync::Arc;

use crate::database::ShareDb;
use crate::p2p::connection::P2PConnectionManager;
use crate::p2p::crypto;
use crate::p2p::key::P2PKeyManager;
use crate::p2p::protocol::{self, P2pFrame, P2pMessageType, P2pTaskRequest, P2pTaskResult};
use crate::share::executor::{ExecuteRequest, SharedExecutor};
use crate::share::protocol::{TaskStatus, TokenUsage};
use tokio::sync::RwLock;

/// Tracks P2P sessions so the supplier can derive the correct task key
/// when an incoming QUIC connection arrives. Maps session_id → consumer_pubkey.
pub struct P2PSessionStore {
    sessions: RwLock<HashMap<String, String>>,
}

impl P2PSessionStore {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Register a session with the consumer's public key.
    pub async fn register(&self, session_id: String, consumer_pubkey: String) {
        self.sessions.write().await.insert(session_id, consumer_pubkey);
    }

    /// Look up the consumer's public key for a session.
    pub async fn get(&self, session_id: &str) -> Option<String> {
        self.sessions.read().await.get(session_id).cloned()
    }

    /// Remove a session after it completes.
    pub async fn remove(&self, session_id: &str) {
        self.sessions.write().await.remove(session_id);
    }
}

/// Handle an incoming QUIC connection from a consumer.
///
/// Reads P2P task requests, decrypts them, executes via ProxyExecutor,
/// and sends encrypted results back over QUIC.
pub async fn handle_incoming_connection(
    conn: quinn::Connection,
    key_manager: Arc<P2PKeyManager>,
    executor: SharedExecutor,
    db: Arc<ShareDb>,
    session_store: Arc<P2PSessionStore>,
) {
    let peer = conn.remote_address().to_string();
    log::info!("P2P supplier: incoming QUIC connection from {}", peer);

    loop {
        // Accept a bidirectional stream from the consumer.
        let stream_result = conn.accept_bi().await;
        match stream_result {
            Ok((mut send_stream, mut recv_stream)) => {
                let peer = peer.clone();
                let key_manager = key_manager.clone();
                let executor = executor.clone();
                let db = db.clone();
                let session_store = session_store.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_task_stream(
                        &mut send_stream,
                        &mut recv_stream,
                        &key_manager,
                        &executor,
                        &db,
                        &session_store,
                    )
                    .await
                    {
                        log::error!("P2P supplier: task handling failed for {}: {}", peer, e);
                    }
                });
            }
            Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                log::info!("P2P supplier: connection closed by peer {}", peer);
                break;
            }
            Err(e) => {
                log::warn!("P2P supplier: connection error from {}: {}", peer, e);
                break;
            }
        }
    }
}

/// Read a P2P task request from a QUIC stream, execute it, and send the result back.
async fn handle_task_stream(
    send_stream: &mut quinn::SendStream,
    recv_stream: &mut quinn::RecvStream,
    key_manager: &P2PKeyManager,
    executor: &SharedExecutor,
    db: &Arc<ShareDb>,
    session_store: &Arc<P2PSessionStore>,
) -> Result<(), String> {
    // Read the request frame.
    let mut buf = vec![0u8; 256 * 1024]; // 256KB buffer for large requests
    let n = recv_stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("read QUIC stream: {e}"))?
        .ok_or("QUIC stream closed before reading request")?;

    let (frame, _) = P2pFrame::decode(&buf[..n])?
        .ok_or("incomplete P2P request frame")?;

    if frame.msg_type != P2pMessageType::TaskRequest {
        return Err(format!("expected TaskRequest, got {:?}", frame.msg_type));
    }

    let request: P2pTaskRequest = protocol::decode_message(&frame)?;

    log::info!(
        "P2P supplier: received task session={} model={} stream={}",
        request.session_id,
        request.model,
        request.stream,
    );

    // Derive the task key using X25519 DH with the consumer's public key.
    // The consumer_pubkey was stored in the session store when the P2POffer was received.
    let consumer_pubkey = session_store
        .get(&request.session_id)
        .await
        .ok_or_else(|| format!("no consumer pubkey for session {}", request.session_id))?;

    let task_key = derive_task_key(key_manager, &consumer_pubkey, &request.session_id)?;

    // Decrypt the messages and params.
    let messages_decrypted = crypto::decrypt_payload(&task_key, &request.messages)
        .map_err(|e| format!("decrypt messages: {e}"))?;
    let messages_str = String::from_utf8(messages_decrypted)
        .map_err(|e| format!("messages UTF-8: {e}"))?;
    let messages: serde_json::Value = serde_json::from_str(&messages_str)
        .unwrap_or_else(|_| serde_json::Value::Array(vec![]));

    let params_decrypted = crypto::decrypt_payload(&task_key, &request.params)
        .map_err(|e| format!("decrypt params: {e}"))?;
    let params_str = String::from_utf8(params_decrypted)
        .unwrap_or_else(|_| "{}".to_string());
    let params: serde_json::Value = serde_json::from_str(&params_str)
        .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

    // Execute the task via ProxyExecutor.
    let exec_request = ExecuteRequest {
        provider_id: "default".to_string(),
        model: request.model.clone(),
        messages,
        stream: false, // P2P non-streaming for now
        params,
    };

    let result = executor.execute(exec_request).await;

    // Build the P2P task result.
    let (status, content, usage, error) = match result {
        Ok(resp) => (
            TaskStatus::Completed,
            resp.content,
            resp.usage,
            None,
        ),
        Err(e) => (
            TaskStatus::Failed,
            String::new(),
            None,
            Some(e.to_string()),
        ),
    };

    // Encrypt the content.
    let content_encrypted = crypto::encrypt_payload(&task_key, content.as_bytes())
        .map_err(|e| format!("encrypt result: {e}"))?;

    let p2p_result = P2pTaskResult {
        session_id: request.session_id.clone(),
        task_id: request.task_id.clone(),
        status: format!("{:?}", status).to_lowercase(),
        content: content_encrypted,
        usage: usage.as_ref().map(|u| crate::p2p::protocol::TokenUsageEncrypted {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }),
        error,
        sequence: None,
        r#final: Some(true),
    };

    // Send the result frame.
    let result_frame = protocol::encode_message(P2pMessageType::TaskResult, &p2p_result)?;
    send_stream
        .write_all(&result_frame.encode())
        .await
        .map_err(|e| format!("write QUIC result: {e}"))?;
    send_stream
        .finish()
        .map_err(|e| format!("finish QUIC stream: {e}"))?;

    log::info!(
        "P2P supplier: task completed session={} task_id={} status={:?}",
        request.session_id,
        request.task_id,
        status,
    );

    // Clean up the session.
    session_store.remove(&request.session_id).await;

    // Report usage to cloud (best-effort).
    let _ = report_usage(db, &request, &status, &usage);

    Ok(())
}

/// Derive the task key for a P2P session using X25519 DH.
///
/// Both consumer and supplier derive the same task key:
///   shared_secret = X25519(our_private, their_public)
///   task_key = HKDF-SHA256(shared_secret, "shareplan-task-key-{session_id}")
fn derive_task_key(
    key_manager: &P2PKeyManager,
    consumer_pubkey_b64: &str,
    session_id: &str,
) -> Result<[u8; 32], String> {
    use base64::Engine;
    let peer_pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(consumer_pubkey_b64)
        .map_err(|e| format!("base64 decode consumer pubkey: {e}"))?;
    let peer_pubkey: [u8; 32] = peer_pubkey_bytes
        .try_into()
        .map_err(|_| "consumer public key must be 32 bytes".to_string())?;

    let shared_secret = key_manager.diffie_hellman(&peer_pubkey);
    let task_key = crypto::derive_task_key(&shared_secret, session_id);
    Ok(task_key)
}

/// Report P2P task usage to the cloud server (best-effort).
fn report_usage(
    _db: &Arc<ShareDb>,
    request: &P2pTaskRequest,
    status: &TaskStatus,
    usage: &Option<TokenUsage>,
) {
    log::info!(
        "P2P supplier: usage report session={} task_id={} status={:?} usage={:?}",
        request.session_id,
        request.task_id,
        status,
        usage,
    );
    // Full reporting will be wired in once the supplier has access to
    // auth_token and hmac_secret from ShareState.
}

/// Start accepting incoming P2P connections in a background task.
///
/// This should be called when the supplier starts sharing. It continuously
/// accepts QUIC connections and spawns a handler for each.
pub async fn accept_loop(
    conn_manager: Arc<P2PConnectionManager>,
    key_manager: Arc<P2PKeyManager>,
    executor: SharedExecutor,
    db: Arc<ShareDb>,
    session_store: Arc<P2PSessionStore>,
) {
    log::info!("P2P supplier: starting QUIC accept loop");

    loop {
        match conn_manager.accept_incoming().await {
            Ok(incoming) => {
                let conn = match incoming.await {
                    Ok(conn) => conn,
                    Err(e) => {
                        log::warn!("P2P supplier: incoming connection failed: {}", e);
                        conn_manager.peer_disconnected(None);
                        continue;
                    }
                };

                log::info!("P2P supplier: accepted connection from {}", conn.remote_address());
                let key_manager = key_manager.clone();
                let executor = executor.clone();
                let db = db.clone();
                let conn_manager = conn_manager.clone();
                let session_store = session_store.clone();
                let peer_addr = conn.remote_address().to_string();

                tokio::spawn(async move {
                    handle_incoming_connection(conn, key_manager, executor, db, session_store).await;
                    conn_manager.peer_disconnected(Some(peer_addr));
                });
            }
            Err(e) => {
                log::error!("P2P supplier: accept_incoming failed: {}", e);
                // If the endpoint is shut down, exit the loop.
                if !conn_manager.is_running().await {
                    log::info!("P2P supplier: QUIC endpoint shut down, exiting accept loop");
                    break;
                }
                // Brief backoff before retrying.
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}