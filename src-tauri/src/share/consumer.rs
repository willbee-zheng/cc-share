//! 消费者模式：通过 HTTP 调用云端 /api/v1/dispatch
//!
//! 当用户切换到 CC-Share Pool 作为 Provider 时，本地请求由 [`Consumer`] 转发到
//! 云端调度服务器。云端撮合最优供应者节点，执行后返回完整响应。
//!
//! 本 MVP 走 **HTTP 同步**（非流式）。Phase 4 将引入流式 SSE。

use crate::database::ShareDb;
use crate::share::protocol::TokenUsage;
use crate::share::signing;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// 消费者代理
pub struct Consumer {
    db: Arc<ShareDb>,
    http: reqwest::Client,
    base_url: String,
    auth_token: String,
    hmac_secret: Vec<u8>,
    request_timeout: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct ConsumerConfig {
    pub base_url: String,
    pub auth_token: String,
    /// HMAC-SHA256 签名密钥（与 cloud-server 的 `auth.hmac_secret` 一致）
    pub hmac_secret: String,
    pub request_timeout_secs: u64,
}

/// 消费者请求参数
#[derive(Debug, Clone)]
pub struct ConsumeRequest {
    pub model: String,
    pub messages: serde_json::Value,
    pub stream: bool,
    pub params: serde_json::Value,
    pub est_prompt_tokens: u32,
    pub max_output_tokens: u32,
}

/// 消费者响应
#[derive(Debug, Clone)]
pub struct ConsumeResponse {
    pub content: String,
    pub usage: Option<TokenUsage>,
    pub success: bool,
    pub error: Option<String>,
    pub node_id: Option<String>,
    /// 云端计费信息（总费用）
    pub credits_spent: Option<f64>,
}

/// 与 cloud-server 的 dispatch.go DispatchRequestBody 字段对齐
#[derive(Debug, Serialize)]
struct DispatchBody<'a> {
    model: &'a str,
    messages: &'a serde_json::Value,
    stream: bool,
    params: &'a serde_json::Value,
    est_prompt_tokens: u32,
    max_output_tokens: u32,
}

/// 与 cloud-server 的 200 响应对齐
#[derive(Debug, Deserialize)]
struct DispatchResponse {
    node_id: Option<String>,
    task_id: Option<String>,
    content: String,
    usage: Option<TokenUsage>,
    billing: Option<DispatchBilling>,
}

/// 云端返回的计费信息
#[derive(Debug, Deserialize)]
struct DispatchBilling {
    supplier: Option<String>,
    platform: Option<String>,
    total: Option<String>,
    frozen: Option<String>,
}

/// 错误响应
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

impl Consumer {
    pub fn new(db: Arc<ShareDb>, config: ConsumerConfig) -> Self {
        let timeout = Duration::from_secs(if config.request_timeout_secs == 0 {
            60
        } else {
            config.request_timeout_secs
        });
        let http = crate::http_client::shareplan_client_builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build");
        Self {
            db,
            http,
            base_url: config.base_url,
            auth_token: config.auth_token,
            hmac_secret: config.hmac_secret.into_bytes(),
            request_timeout: timeout,
        }
    }

    /// 发送请求到共享池。返回完整结果或错误。
    pub async fn consume(&self, request: ConsumeRequest) -> ConsumeResponse {
        let task_id = uuid::Uuid::new_v4().to_string();
        let start = std::time::Instant::now();

        let url = format!("{}/api/v1/dispatch", self.base_url.trim_end_matches('/'));
        let body = DispatchBody {
            model: &request.model,
            messages: &request.messages,
            stream: false, // MVP: 不走流式
            params: &request.params,
            est_prompt_tokens: request.est_prompt_tokens,
            max_output_tokens: if request.max_output_tokens == 0 {
                1024
            } else {
                request.max_output_tokens
            },
        };

        // 序列化 body 一次，复用给签名 + 发送（避免 reqwest .json() 重新序列化产生不同字节序）
        let body_bytes = match serde_json::to_vec(&body) {
            Ok(b) => b,
            Err(e) => {
                log::error!(
                    "consume [{}]: failed to encode request body for model={}: {}",
                    task_id, request.model, e
                );
                let resp = ConsumeResponse {
                    content: String::new(),
                    usage: None,
                    success: false,
                    error: Some(format!("encode body: {e}")),
                    node_id: None,
                    credits_spent: None,
                };
                self.log(&task_id, &request, &resp, start);
                return resp;
            }
        };

        log::info!(
            "consume [{}]: POST {} model={} stream={} est_prompt_tokens={} max_output_tokens={} body_size={}B",
            task_id, url, request.model, request.stream, request.est_prompt_tokens,
            if request.max_output_tokens == 0 { 1024 } else { request.max_output_tokens },
            body_bytes.len()
        );

        let mut req_builder = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body_bytes.clone());
        if !self.auth_token.is_empty() {
            req_builder =
                req_builder.header("Authorization", format!("Bearer {}", self.auth_token));
        }
        if !self.hmac_secret.is_empty() {
            // /api/v1/dispatch 的 path 必须与服务端 c.Request.URL.Path 完全一致
            let now = chrono::Utc::now().timestamp();
            let nonce = uuid::Uuid::new_v4().to_string();
            let (ts, n, sig) = signing::build_headers(
                &self.hmac_secret,
                "POST",
                "/api/v1/dispatch",
                &body_bytes,
                now,
                &nonce,
            );
            req_builder = req_builder
                .header(signing::HEADER_TIMESTAMP, ts.to_string())
                .header(signing::HEADER_NONCE, n)
                .header(signing::HEADER_SIGNATURE, sig);
        }

        let response = match req_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                log::error!(
                    "consume [{}]: HTTP request failed for model={}: {}",
                    task_id, request.model, e
                );
                let resp = ConsumeResponse {
                    content: String::new(),
                    usage: None,
                    success: false,
                    error: Some(format!("dispatch http: {e}")),
                    node_id: None,
                    credits_spent: None,
                };
                self.log(&task_id, &request, &resp, start);
                return resp;
            }
        };

        let status = response.status();
        log::info!("consume [{}]: response status={}", task_id, status.as_u16());
        if !status.is_success() {
            let err = response
                .json::<ErrorResponse>()
                .await
                .map(|e| e.error)
                .unwrap_or_else(|_| format!("HTTP {}", status.as_u16()));
            log::warn!("consume [{}]: error response {} - {}", task_id, status.as_u16(), err);
            let resp = ConsumeResponse {
                content: String::new(),
                usage: None,
                success: false,
                error: Some(err),
                node_id: None,
                credits_spent: None,
            };
            self.log(&task_id, &request, &resp, start);
            return resp;
        }

        let parsed: DispatchResponse = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                log::error!("consume [{}]: failed to decode response JSON: {}", task_id, e);
                let resp = ConsumeResponse {
                    content: String::new(),
                    usage: None,
                    success: false,
                    error: Some(format!("decode response: {e}")),
                    node_id: None,
                    credits_spent: None,
                };
                self.log(&task_id, &request, &resp, start);
                return resp;
            }
        };

        // Extract credits from billing response
        let credits_spent = parsed.billing.as_ref().and_then(|b| {
            b.total.as_ref().map(|t| t.parse::<f64>().unwrap_or(0.0))
        });

        let resp = ConsumeResponse {
            content: parsed.content,
            usage: parsed.usage,
            success: true,
            error: None,
            node_id: parsed.node_id,
            credits_spent,
        };
        log::info!(
            "consume [{}]: success, node_id={:?}, usage={:?}, latency={}ms",
            task_id, resp.node_id, resp.usage, start.elapsed().as_millis()
        );
        self.log(&task_id, &request, &resp, start);
        resp
    }

    fn log(
        &self,
        task_id: &str,
        request: &ConsumeRequest,
        resp: &ConsumeResponse,
        start: std::time::Instant,
    ) {
        let latency_ms = start.elapsed().as_millis() as i32;
        if let Err(e) = self
            .db
            .insert_p2p_task_log(&crate::database::dao_credits::P2PTaskLog {
                task_id: task_id.to_string(),
                direction: "consume".into(),
                model: request.model.clone(),
                upstream_model: None, // Consumer does not know the real upstream model
                tokens_prompt: resp
                    .usage
                    .as_ref()
                    .map(|u| u.prompt_tokens as i32)
                    .unwrap_or(0),
                tokens_completion: resp
                    .usage
                    .as_ref()
                    .map(|u| u.completion_tokens as i32)
                    .unwrap_or(0),
                credits: resp.credits_spent.unwrap_or(0.0),
                latency_ms: Some(latency_ms),
                status: if resp.success { "completed" } else { "failed" }.into(),
                error_message: resp.error.clone(),
                created_at: chrono::Utc::now().timestamp(),
            })
        {
            log::warn!("记录消费者任务日志失败: {e}");
        }
    }

    /// Expose timeout for diagnostics / tests.
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> Arc<ShareDb> {
        Arc::new(ShareDb::memory().expect("创建内存数据库失败"))
    }

    #[test]
    fn test_consumer_new_default_timeout() {
        let db = create_test_db();
        let c = Consumer::new(
            db,
            ConsumerConfig {
                base_url: "https://example.com".into(),
                auth_token: "x".into(),
                request_timeout_secs: 0,
                hmac_secret: String::new(),
            },
        );
        assert_eq!(c.request_timeout(), Duration::from_secs(60));
    }

    #[tokio::test]
    async fn test_consume_unreachable_returns_error() {
        let db = create_test_db();
        let consumer = Consumer::new(
            db,
            ConsumerConfig {
                // 端口 1 几乎不可达
                base_url: "http://127.0.0.1:1".into(),
                auth_token: "tok".into(),
                request_timeout_secs: 1,
                hmac_secret: String::new(),
            },
        );

        let request = ConsumeRequest {
            model: "claude-sonnet-4-6".into(),
            messages: serde_json::json!([{"role": "user", "content": "hi"}]),
            stream: false,
            params: serde_json::Value::Null,
            est_prompt_tokens: 5,
            max_output_tokens: 100,
        };
        let response = consumer.consume(request).await;
        assert!(!response.success);
        assert!(response.error.is_some());
    }
}
