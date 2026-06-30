//! 消费者代理相关 Tauri 命令
//!
//! 把请求转发到云端共享池（cloud-server `/api/v1/dispatch`）。
//!
//! 这是 cc-switch ProviderRouter 集成的接缝：在 cc-switch 端，将虚拟
//! Provider `cc-share-pool` 的请求路由到这里，而不是修改 cc-share 内部。
//! 集成只需调用 `invoke("plugin:shareplan|share_consume", { request })` 即可。

use crate::share::client::ClientConfig;
use crate::share::consumer::{ConsumeRequest, ConsumeResponse, Consumer, ConsumerConfig};
use crate::share::protocol::TokenUsage;
use crate::ShareState;

/// 前端调用的消费请求参数（驼峰友好的子集）
#[derive(Debug, serde::Deserialize)]
pub struct ConsumeArgs {
    pub model: String,
    pub messages: serde_json::Value,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub est_prompt_tokens: u32,
    #[serde(default)]
    pub max_output_tokens: u32,
}

/// 返回给前端的精简响应
#[derive(Debug, serde::Serialize)]
pub struct ConsumeResult {
    pub success: bool,
    pub content: String,
    pub usage: Option<TokenUsage>,
    pub error: Option<String>,
    pub node_id: Option<String>,
}

impl From<ConsumeResponse> for ConsumeResult {
    fn from(r: ConsumeResponse) -> Self {
        Self {
            success: r.success,
            content: r.content,
            usage: r.usage,
            error: r.error,
            node_id: r.node_id,
        }
    }
}

/// 把本地请求路由到云端共享池
#[tauri::command]
pub async fn share_consume(
    state: tauri::State<'_, ShareState>,
    args: ConsumeArgs,
) -> Result<ConsumeResult, String> {
    log::info!("▶ share_consume: model={}, stream={}, est_prompt_tokens={}", args.model, args.stream, args.est_prompt_tokens);

    let cfg: ClientConfig = state.client_config.read().await.clone();
    if cfg.server_host.is_empty() {
        log::error!("share_consume failed: server_host not configured");
        return Err("服务器地址未配置".into());
    }
    if cfg.auth_token.is_empty() {
        log::error!("share_consume failed: auth_token not configured");
        return Err("认证令牌未配置".into());
    }

    // 把 server_host (域名或域名:端口) → https://host 或 http://host:port
    let base_url = host_to_http_base(&cfg.server_host, cfg.use_https);

    let consumer = Consumer::new(
        state.db.clone(),
        ConsumerConfig {
            base_url,
            auth_token: cfg.auth_token,
            hmac_secret: cfg.hmac_secret,
            request_timeout_secs: 60,
        },
    );

    let request = ConsumeRequest {
        model: args.model,
        messages: args.messages,
        stream: args.stream,
        params: args.params,
        est_prompt_tokens: args.est_prompt_tokens,
        max_output_tokens: args.max_output_tokens,
    };

    let response = consumer.consume(request).await;
    log::info!("share_consume: result success={}, error={:?}", response.success, response.error);
    Ok(response.into())
}

/// 返回云端 share_node_registry 缓存（前端模型选择器用）
#[tauri::command]
pub async fn list_share_nodes(
    state: tauri::State<'_, ShareState>,
) -> Result<Vec<crate::database::dao_credits::ShareNode>, String> {
    state
        .db
        .get_online_share_nodes()
        .map_err(|e| e.to_string())
}

/// Convert a user-provided server host (domain or domain:port) to an HTTP(S) base URL.
///
/// - Pure domain (`api.cc-share.com`) → `https://api.cc-share.com` (if use_https) or `http://api.cc-share.com` (default)
/// - Domain:port (`192.168.1.60:8080`) → `http://192.168.1.60:8080`
fn host_to_http_base(host: &str, use_https: bool) -> String {
    crate::url_utils::build_http_base_with_tls(host, use_https)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_to_http_base_pure_domain_no_tls() {
        assert_eq!(host_to_http_base("api.cc-share.com", false), "http://api.cc-share.com");
    }

    #[test]
    fn test_host_to_http_base_pure_domain_with_tls() {
        assert_eq!(host_to_http_base("api.cc-share.com", true), "https://api.cc-share.com");
    }

    #[test]
    fn test_host_to_http_base_with_port() {
        assert_eq!(host_to_http_base("192.168.1.60:8080", false), "http://192.168.1.60:8080");
    }

    #[test]
    fn test_host_to_http_base_strips_protocol() {
        assert_eq!(host_to_http_base("wss://api.cc-share.com/api/v1/agent/connect", false), "https://api.cc-share.com");
    }
}
