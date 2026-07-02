//! P2P-aware consumer with automatic fallback to cloud relay.
//!
//! When P2P mode is preferred, the consumer first attempts a direct QUIC
//! connection to the supplier. If P2P fails within the deadline, it
//! transparently falls back to the existing cloud relay path.

use crate::database::ShareDb;
use crate::error::ShareError;
use crate::p2p::connection::P2PConnectionManager;
use crate::p2p::crypto;
use crate::p2p::key::P2PKeyManager;
use crate::p2p::protocol::{self, P2pFrame, P2pMessageType, P2pTaskRequest, P2pTaskResult};
use crate::p2p::session::P2PSession;
use crate::share::consumer::{ConsumeRequest, ConsumeResponse, Consumer};
use crate::share::protocol::TokenUsage;
use crate::share::signing;
use std::sync::Arc;
use std::time::Duration;

/// P2P dispatch response from the cloud server.
#[derive(Debug, Clone, serde::Deserialize)]
struct P2PDispatchResponse {
    mode: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    supplier_candidates: Option<Vec<String>>,
    #[serde(default)]
    supplier_pubkey: Option<String>,
    #[serde(default)]
    p2p_deadline_ms: Option<u64>,
    #[serde(default)]
    fallback_task_id: Option<String>,
    // Relay mode fields
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    usage: Option<TokenUsage>,
    #[serde(default)]
    error: Option<String>,
}

/// P2P-aware consumer that tries direct connection first, then falls back to cloud relay.
pub struct P2PConsumer {
    /// The underlying cloud relay consumer.
    relay: Consumer,
    /// P2P connection manager.
    conn_manager: Arc<P2PConnectionManager>,
    /// P2P key manager for E2E encryption.
    key_manager: Arc<P2PKeyManager>,
    /// HTTP client for P2P dispatch requests.
    http: reqwest::Client,
    /// Cloud base URL.
    base_url: String,
    /// Auth token.
    auth_token: String,
    /// HMAC secret.
    hmac_secret: Vec<u8>,
    /// P2P connection timeout.
    p2p_timeout: Duration,
}

impl P2PConsumer {
    /// Create a new P2P-aware consumer.
    pub fn new(
        db: Arc<ShareDb>,
        relay_config: crate::share::consumer::ConsumerConfig,
        conn_manager: Arc<P2PConnectionManager>,
        key_manager: Arc<P2PKeyManager>,
    ) -> Self {
        let timeout = Duration::from_secs(if relay_config.request_timeout_secs == 0 {
            60
        } else {
            relay_config.request_timeout_secs
        });
        let http = crate::http_client::shareplan_client_builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build");

        let relay = Consumer::new(db, relay_config);

        Self {
            relay,
            conn_manager,
            key_manager,
            http,
            base_url: String::new(), // Will be set from LocalServerState
            auth_token: String::new(),
            hmac_secret: Vec::new(),
            p2p_timeout: Duration::from_secs(5),
        }
    }

    /// Update configuration from LocalServerState.
    pub fn update_config(&mut self, base_url: &str, auth_token: &str, hmac_secret: &str) {
        self.base_url = base_url.to_string();
        self.auth_token = auth_token.to_string();
        self.hmac_secret = hmac_secret.as_bytes().to_vec();
    }

    /// Consume a request, trying P2P first and falling back to cloud relay.
    pub async fn consume(&self, request: ConsumeRequest) -> ConsumeResponse {
        // Try P2P first if we have connection info.
        if !self.base_url.is_empty() && !self.auth_token.is_empty() {
            match self.consume_p2p(&request).await {
                Ok(response) => {
                    log::info!("P2P consume succeeded for model={}", request.model);
                    return response;
                }
                Err(e) => {
                    log::warn!("P2P consume failed, falling back to relay: {}", e);
                    // Report P2P failure to cloud for monitoring
                    self.report_p2p_fallback(&request, &e.to_string()).await;
                }
            }
        }

        // Fallback to cloud relay.
        self.relay.consume(request).await
    }

    /// Attempt P2P direct connection.
    async fn consume_p2p(&self, request: &ConsumeRequest) -> Result<ConsumeResponse, ShareError> {
        // Step 1: Send P2P dispatch request to cloud to get supplier candidates.
        let dispatch_resp = self.request_p2p_dispatch(request).await?;

        if dispatch_resp.mode != "p2p" {
            // Cloud returned relay mode — not a P2P-capable supplier.
            return Err(ShareError::Connection("cloud returned relay mode".into()));
        }

        let session_id = dispatch_resp.session_id.ok_or_else(|| {
            ShareError::Connection("missing session_id in P2P dispatch response".into())
        })?;
        let candidates = dispatch_resp.supplier_candidates.ok_or_else(|| {
            ShareError::Connection("missing supplier_candidates in P2P dispatch response".into())
        })?;
        let supplier_pubkey_b64 = dispatch_resp.supplier_pubkey.ok_or_else(|| {
            ShareError::Connection("missing supplier_pubkey in P2P dispatch response".into())
        })?;

        // Step 2: Derive shared secret and task key.
        let peer_pubkey_bytes = base64_decode(&supplier_pubkey_b64)?;
        let peer_pubkey: [u8; 32] = peer_pubkey_bytes.try_into().map_err(|_| {
            ShareError::Connection("invalid supplier public key length".into())
        })?;
        let session = P2PSession::new(
            session_id.clone(),
            &self.key_manager,
            &supplier_pubkey_b64,
            self.conn_manager.clone(),
        )?;

        // Step 3: Connect to the supplier via QUIC.
        let conn = tokio::time::timeout(
            self.p2p_timeout,
            self.conn_manager.connect_to_peer(&candidates, &session_id),
        )
        .await
        .map_err(|_| ShareError::Connection("P2P connection timeout".into()))??;

        // Step 4: Encrypt and send the task request over QUIC.
        let messages_encrypted = session.encrypt(&request.messages.to_string().into_bytes())?;
        let params_encrypted = session.encrypt(&request.params.to_string().into_bytes())?;

        let p2p_request = P2pTaskRequest {
            session_id: session_id.clone(),
            task_id: uuid::Uuid::new_v4().to_string(),
            model: request.model.clone(),
            messages: messages_encrypted,
            stream: false,
            params: params_encrypted,
            est_prompt_tokens: request.est_prompt_tokens,
            max_output_tokens: request.max_output_tokens,
        };

        // Open a QUIC bidirectional stream.
        let (mut send_stream, mut recv_stream) = conn
            .open_bi()
            .await
            .map_err(|e| ShareError::Connection(format!("open QUIC stream: {e}")))?;

        // Send the frame.
        let frame = protocol::encode_message(P2pMessageType::TaskRequest, &p2p_request)?;
        send_stream
            .write_all(&frame.encode())
            .await
            .map_err(|e| ShareError::Connection(format!("write QUIC frame: {e}")))?;
        send_stream
            .finish()
            .map_err(|e| ShareError::Connection(format!("finish QUIC stream: {e}")))?;

        // Step 5: Read the response.
        let mut buf = vec![0u8; 64 * 1024];
        let n = recv_stream
            .read(&mut buf)
            .await
            .map_err(|e| ShareError::Connection(format!("read QUIC response: {e}")))?
            .ok_or_else(|| ShareError::Connection("QUIC stream closed".into()))?;

        let (p2p_frame, _) = P2pFrame::decode(&buf[..n])?
            .ok_or_else(|| ShareError::Connection("incomplete P2P response frame".into()))?;

        if p2p_frame.msg_type != P2pMessageType::TaskResult {
            return Err(ShareError::Connection(format!(
                "expected TaskResult, got {:?}",
                p2p_frame.msg_type
            )));
        }

        let p2p_result: P2pTaskResult = protocol::decode_message(&p2p_frame)?;

        // Step 6: Decrypt the response content.
        let content_decrypted = session.decrypt(&p2p_result.content)?;
        let content = String::from_utf8(content_decrypted)
            .map_err(|e| ShareError::Connection(format!("decrypt content UTF-8: {e}")))?;

        // Step 7: Report usage to cloud (best-effort, spawned).
        let http = self.http.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();
        let hmac_secret_str = String::from_utf8_lossy(&self.hmac_secret).to_string();
        let report = crate::p2p::report::UsageReport::new(
            session_id.clone(),
            p2p_request.task_id.clone(),
            request.model.clone(),
            request.est_prompt_tokens,
            p2p_result.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
            p2p_result.status.clone(),
            "consumer".to_string(),
        );

        tokio::spawn(async move {
            if let Err(e) = crate::p2p::report::submit_report(
                &http,
                &base_url,
                &auth_token,
                &hmac_secret_str,
                &report,
            ).await {
                log::warn!("P2P usage report submission failed: {}", e);
            }
        });

        Ok(ConsumeResponse {
            content,
            usage: p2p_result.usage.map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
            success: p2p_result.status == "completed",
            error: p2p_result.error,
            node_id: None, // P2P mode doesn't have a node_id from cloud
        })
    }

    /// Request P2P dispatch from the cloud server.
    async fn request_p2p_dispatch(
        &self,
        request: &ConsumeRequest,
    ) -> Result<P2PDispatchResponse, ShareError> {
        let url = format!("{}/api/v1/p2p/dispatch", self.base_url.trim_end_matches('/'));

        // Get local STUN candidates (for now, just use local addresses).
        let local_candidates = self.conn_manager.local_addr().await.unwrap_or_default();
        let candidate_strs: Vec<String> = local_candidates
            .iter()
            .map(|a| a.to_string())
            .collect();

        let body = serde_json::json!({
            "model": request.model,
            "consumer_candidates": candidate_strs,
            "consumer_pubkey": self.key_manager.public_key_base64(),
            "est_prompt_tokens": request.est_prompt_tokens,
            "max_output_tokens": request.max_output_tokens,
            "p2p_mode": "prefer",
        });

        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| ShareError::Connection(format!("encode P2P dispatch: {e}")))?;

        let mut req_builder = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body_bytes.clone());

        if !self.auth_token.is_empty() {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", self.auth_token));
        }

        if !self.hmac_secret.is_empty() {
            let timestamp = chrono::Utc::now().timestamp().to_string();
            let nonce = uuid::Uuid::new_v4().to_string();
            let body_hash = signing::body_hash_hex(&body_bytes);
            let canonical = format!("POST\n/p2p/dispatch\n{}\n{}\n{}", timestamp, nonce, body_hash);
            let signature = signing::sign(&self.hmac_secret, canonical.as_bytes());

            req_builder = req_builder
                .header(signing::HEADER_TIMESTAMP, &timestamp)
                .header(signing::HEADER_NONCE, &nonce)
                .header(signing::HEADER_SIGNATURE, &signature);
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| ShareError::Connection(format!("P2P dispatch HTTP: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let err_body = response.text().await.unwrap_or_default();
            return Err(ShareError::Connection(format!(
                "P2P dispatch failed: HTTP {} - {}",
                status.as_u16(),
                err_body
            )));
        }

        response
            .json::<P2PDispatchResponse>()
            .await
            .map_err(|e| ShareError::Connection(format!("P2P dispatch decode: {e}")))
    }

    /// Report P2P fallback to cloud (best-effort, for monitoring).
    async fn report_p2p_fallback(&self, _request: &ConsumeRequest, reason: &str) {
        log::info!("P2P fallback reported: {}", reason);
        // In a full implementation, this would POST to /api/v1/p2p/fallback
        // For now, just log it.
    }
}

/// Base64 decode helper.
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("base64 decode: {e}"))
}