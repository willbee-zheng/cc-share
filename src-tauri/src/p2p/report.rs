//! P2P usage report submission to cloud server.
//!
//! After a P2P task completes, both consumer and supplier independently
//! report their token usage to the cloud for billing reconciliation.

use crate::error::ShareError;
use crate::share::signing;
use serde::{Deserialize, Serialize};

/// P2P usage report submitted to the cloud server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageReport {
    /// Session ID from cloud signaling
    pub session_id: String,
    /// Task ID
    pub task_id: String,
    /// Model used
    pub model: String,
    /// Upstream model (resolved by supplier)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_model: Option<String>,
    /// Prompt tokens consumed
    pub prompt_tokens: u32,
    /// Completion tokens generated
    pub completion_tokens: u32,
    /// Task status: "completed", "failed", "rejected", "busy"
    pub status: String,
    /// Reporter role: "consumer" or "supplier"
    pub reporter: String,
    /// Supplier node ID (for supplier reports)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplier_node_id: Option<String>,
    /// Unix timestamp of the report
    pub timestamp: i64,
    /// HMAC signature of the report fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl UsageReport {
    /// Create a new usage report with the current timestamp.
    pub fn new(
        session_id: String,
        task_id: String,
        model: String,
        prompt_tokens: u32,
        completion_tokens: u32,
        status: String,
        reporter: String,
    ) -> Self {
        Self {
            session_id,
            task_id,
            model,
            upstream_model: None,
            prompt_tokens,
            completion_tokens,
            status,
            reporter,
            supplier_node_id: None,
            timestamp: chrono::Utc::now().timestamp(),
            signature: None,
        }
    }

    /// Sign the report with HMAC-SHA256 using the shared secret.
    pub fn sign(&mut self, hmac_secret: &str) {
        let body = serde_json::to_string(self)
            .unwrap_or_default();
        let body_hash = signing::body_hash_hex(body.as_bytes());
        let timestamp = self.timestamp.to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let canonical = format!("POST\n/p2p/report\n{}\n{}\n{}", timestamp, nonce, body_hash);
        let sig = signing::sign(hmac_secret.as_bytes(), canonical.as_bytes());
        self.signature = Some(sig);
    }
}

/// Submit a P2P usage report to the cloud server.
pub async fn submit_report(
    client: &reqwest::Client,
    base_url: &str,
    auth_token: &str,
    hmac_secret: &str,
    report: &UsageReport,
) -> Result<(), ShareError> {
    let url = format!("{}/api/v1/p2p/report", base_url.trim_end_matches('/'));

    let mut signed_report = report.clone();
    signed_report.sign(hmac_secret);

    let body = serde_json::to_string(&signed_report)
        .map_err(|e| ShareError::Connection(format!("serialize report: {e}")))?;

    let timestamp = chrono::Utc::now().timestamp().to_string();
    let nonce = uuid::Uuid::new_v4().to_string();
    let body_hash = signing::body_hash_hex(body.as_bytes());
    let canonical = format!("POST\n/p2p/report\n{}\n{}\n{}", timestamp, nonce, body_hash);
    let signature = signing::sign(hmac_secret.as_bytes(), canonical.as_bytes());

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", auth_token))
        .header("Content-Type", "application/json")
        .header("X-Shareplan-Timestamp", &timestamp)
        .header("X-Shareplan-Nonce", &nonce)
        .header("X-Shareplan-Signature", &signature)
        .body(body)
        .send()
        .await
        .map_err(|e| ShareError::Connection(format!("submit P2P report: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(ShareError::Connection(format!(
            "P2P report submission failed: {} - {}",
            status, body
        )));
    }

    log::info!(
        "P2P: usage report submitted (session={}, reporter={})",
        report.session_id,
        report.reporter
    );

    Ok(())
}