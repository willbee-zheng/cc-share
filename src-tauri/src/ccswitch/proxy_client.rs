//! cc-switch local proxy client.
//!
//! cc-share talks to cc-switch's local LLM proxy (default `127.0.0.1:15721`)
//! to (a) discover which provider app_types are currently active and
//! (b) forward LLM tasks. cc-share NEVER touches provider API keys —
//! cc-switch holds them and makes the upstream call.
//!
//! This module is the supplier-side data source (replaces the v1 plan's
//! direct `cc-switch.db` read). See `proxy_forwarder.rs` (Phase 5) for the
//! task forwarding path.

use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

/// Default cc-switch proxy address. Users can override via config.
pub const DEFAULT_PROXY_BASE: &str = "http://127.0.0.1:15721";

/// Error talking to cc-switch proxy.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("cc-switch proxy unreachable: {0}")]
    Unreachable(String),
    #[error("cc-switch proxy returned {0}: {1}")]
    HttpStatus(u16, String),
    #[error("cc-switch proxy response parse error: {0}")]
    Parse(String),
}

/// `/status` response (subset of cc-switch's `ProxyStatus`).
/// Only the fields cc-share needs are decoded; extras are ignored.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub current_provider: Option<String>,
    pub current_provider_id: Option<String>,
    #[serde(default)]
    pub active_targets: Vec<ActiveTarget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActiveTarget {
    pub app_type: String,
    pub provider_name: String,
    pub provider_id: String,
}

/// API format inferred from a cc-switch `app_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ApiFormat {
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
    GeminiNative,
}

impl ApiFormat {
    /// Map cc-switch app_type strings to a wire format.
    /// cc-switch uses "Claude"/"Codex"/"Gemini" (and variants); we normalize
    /// case-insensitively. Unknown → OpenAiChat as a safe default.
    pub fn from_app_type(app_type: &str) -> Self {
        let s = app_type.to_ascii_lowercase();
        if s.contains("claude") {
            Self::Anthropic
        } else if s.contains("gemini") {
            Self::GeminiNative
        } else {
            // Codex and anything OpenAI-shaped (DeepSeek/Moonshot/etc.) go here.
            Self::OpenAiChat
        }
    }

    /// Representative model names cc-share advertises to the cloud for each
    /// format. These names must match what the consumer-side `list_models`
    /// returns in `local_server/openai_compat.rs` so that dispatch can match
    /// requests to available nodes via Redis SET lookups.
    pub fn representative_models(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic => &["claude-sonnet-4", "claude-opus-4", "claude-haiku-4"],
            Self::OpenAiChat => &["gpt-4o", "gpt-4o-mini", "deepseek-chat"],
            Self::OpenAiResponses => &["gpt-5", "o1"],
            Self::GeminiNative => &["gemini-1.5-pro", "gemini-1.5-flash"],
        }
    }

    /// Environment variable names that map representative models to real upstream
    /// models, keyed by the representative model name. The value is the env var
    /// that holds the real model name, with a fallback to the generic model env var.
    pub fn model_env_mapping(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Anthropic => &[
                ("claude-sonnet-4", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
                ("claude-opus-4", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
                ("claude-haiku-4", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            ],
            Self::OpenAiChat => &[
                ("gpt-4o", "OPENAI_MODEL"),
                ("gpt-4o-mini", "OPENAI_MODEL"),
                ("deepseek-chat", "OPENAI_MODEL"),
            ],
            Self::OpenAiResponses => &[
                ("gpt-5", "OPENAI_MODEL"),
                ("o1", "OPENAI_MODEL"),
            ],
            Self::GeminiNative => &[
                ("gemini-1.5-pro", "GEMINI_MODEL"),
                ("gemini-1.5-flash", "GEMINI_MODEL"),
            ],
        }
    }

    /// The generic fallback env var for this format (e.g., ANTHROPIC_MODEL for Anthropic).
    pub fn generic_model_env(self) -> &'static str {
        match self {
            Self::Anthropic => "ANTHROPIC_MODEL",
            Self::OpenAiChat => "OPENAI_MODEL",
            Self::OpenAiResponses => "OPENAI_MODEL",
            Self::GeminiNative => "GEMINI_MODEL",
        }
    }

    /// Build an upstream_models mapping by parsing the provider's env config.
    /// Maps each representative model name to the real upstream model name.
    /// E.g., {"claude-sonnet-4": "glm-5.1:cloud"}
    /// `env` keys are compared case-insensitively.
    pub fn build_upstream_models(self, env: &HashMap<String, String>) -> HashMap<String, String> {
        let mut mapping = HashMap::new();
        let generic = self.generic_model_env().to_ascii_lowercase();
        let generic_value = env.get(&generic).map(|s| s.as_str()).unwrap_or("");
        for (rep_model, env_var) in self.model_env_mapping() {
            let env_key = env_var.to_ascii_lowercase();
            let real_model = env.get(&env_key)
                .map(|s| s.as_str())
                .unwrap_or(generic_value);
            if !real_model.is_empty() {
                mapping.insert(rep_model.to_string(), real_model.to_string());
            }
        }
        mapping
    }
}

/// HTTP client for cc-switch's local proxy.
#[derive(Clone)]
pub struct CcSwitchProxyClient {
    base_url: String,
    http: reqwest::Client,
}

impl Default for CcSwitchProxyClient {
    fn default() -> Self {
        Self::new(DEFAULT_PROXY_BASE)
    }
}

impl CcSwitchProxyClient {
    pub fn new(base_url: &str) -> Self {
        let http = crate::http_client::shareplan_client_builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client build");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Fetch `/status` from cc-switch proxy. Returns `ProxyError::Unreachable`
    /// when cc-switch is not running — callers should treat this as a soft
    /// warning, not a fatal error.
    pub async fn status(&self) -> Result<ProxyStatus, ProxyError> {
        let url = format!("{}/status", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ProxyError::Unreachable(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ProxyError::HttpStatus(status, body));
        }
        resp.json::<ProxyStatus>()
            .await
            .map_err(|e| ProxyError::Parse(e.to_string()))
    }

    /// Borrow the inner reqwest client for forwarding (non-streaming).
    /// This client has a 10s total timeout, suitable for status checks and
    /// short requests but NOT for long-running SSE streams.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Return a reqwest client suitable for long-running SSE streams.
    /// No total timeout — individual reads are bounded by the HTTP connection
    /// and the streaming SSE protocol's own keep-alive.
    pub fn http_streaming(&self) -> reqwest::Client {
        crate::http_client::shareplan_client_builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("reqwest streaming client build")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_type_mapping() {
        assert_eq!(ApiFormat::from_app_type("Claude"), ApiFormat::Anthropic);
        assert_eq!(ApiFormat::from_app_type("claude"), ApiFormat::Anthropic);
        assert_eq!(ApiFormat::from_app_type("Gemini"), ApiFormat::GeminiNative);
        assert_eq!(ApiFormat::from_app_type("Codex"), ApiFormat::OpenAiChat);
        assert_eq!(ApiFormat::from_app_type("DeepSeek"), ApiFormat::OpenAiChat);
    }

    #[test]
    fn representative_models_nonempty() {
        for fmt in [
            ApiFormat::Anthropic,
            ApiFormat::OpenAiChat,
            ApiFormat::OpenAiResponses,
            ApiFormat::GeminiNative,
        ] {
            assert!(!fmt.representative_models().is_empty());
        }
    }

    #[tokio::test]
    async fn status_unreachable_returns_soft_error() {
        // Port 1 is never bound — should get Unreachable, not panic.
        let client = CcSwitchProxyClient::new("http://127.0.0.1:1");
        let err = client.status().await.unwrap_err();
        assert!(matches!(err, ProxyError::Unreachable(_)), "got {:?}", err);
    }
}
