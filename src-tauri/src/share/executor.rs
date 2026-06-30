//! Provider 调用抽象
//!
//! 将 Supplier 与具体 Provider 实现解耦。cc-share 不直接依赖 cc-switch 内部 API；
//! 集成方（cc-switch、测试）通过实现 [`TaskExecutor`] trait 注入真实调用逻辑。

use crate::share::protocol::TokenUsage;
use async_trait::async_trait;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Provider 调用请求
#[derive(Debug, Clone)]
pub struct ExecuteRequest {
    /// 本地 Provider ID（cc-switch 数据库中的 provider key）
    pub provider_id: String,
    /// 目标模型
    pub model: String,
    /// 对话消息（OpenAI 格式 JSON 数组）
    pub messages: Value,
    /// 是否流式返回
    pub stream: bool,
    /// 透传参数（temperature/top_p/max_tokens 等）
    pub params: Value,
}

/// Provider 调用响应（非流式）
#[derive(Debug, Clone)]
pub struct ExecuteResponse {
    /// 完整响应内容
    pub content: String,
    /// Token 用量
    pub usage: Option<TokenUsage>,
}

/// 流式事件 — 由 execute_stream 逐 chunk 通过 channel 发送
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// 文本增量
    Delta(String),
    /// Token 用量
    Usage(TokenUsage),
    /// 流正常结束
    End,
}

/// 流式执行返回的接收端
pub type StreamReader = mpsc::UnboundedReceiver<StreamEvent>;

/// 调用错误
#[derive(Debug, Clone)]
pub enum ExecuteError {
    /// Provider 未找到或未配置
    ProviderNotFound(String),
    /// 上游 API 报错（包含状态码和原始 body）
    Upstream { status: u16, body: String },
    /// 网络/IO 错误
    Network(String),
    /// 其他错误
    Other(String),
}

impl fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecuteError::ProviderNotFound(id) => write!(f, "provider not found: {id}"),
            ExecuteError::Upstream { status, body } => {
                write!(f, "upstream {status}: {body}")
            }
            ExecuteError::Network(msg) => write!(f, "network: {msg}"),
            ExecuteError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ExecuteError {}

/// Provider 调用 trait
///
/// `execute` 用于非流式请求，返回完整响应。
/// `execute_stream` 用于流式请求，返回 channel 接收端供调用方逐 chunk 读取。
#[async_trait]
pub trait TaskExecutor: Send + Sync + 'static {
    /// 非流式执行 — 返回完整响应。
    async fn execute(&self, req: ExecuteRequest) -> Result<ExecuteResponse, ExecuteError>;

    /// 流式执行 — 返回 channel 接收端，调用方逐 chunk 读取 StreamEvent。
    ///
    /// 默认实现回退到 `execute()`，将完整内容作为单个 Delta 发出后发送 End。
    async fn execute_stream(&self, req: ExecuteRequest) -> Result<StreamReader, ExecuteError> {
        let resp = self.execute(req).await?;
        let (tx, rx) = mpsc::unbounded_channel();
        if !resp.content.is_empty() {
            let _ = tx.send(StreamEvent::Delta(resp.content));
        }
        if let Some(u) = resp.usage {
            let _ = tx.send(StreamEvent::Usage(u));
        }
        let _ = tx.send(StreamEvent::End);
        Ok(rx)
    }
}

/// 共享指针类型（在 ShareState 中持有）
pub type SharedExecutor = Arc<dyn TaskExecutor>;

/// 占位执行器 — 始终返回 ProviderNotFound
pub struct NullExecutor;

#[async_trait]
impl TaskExecutor for NullExecutor {
    async fn execute(&self, req: ExecuteRequest) -> Result<ExecuteResponse, ExecuteError> {
        Err(ExecuteError::ProviderNotFound(format!(
            "NullExecutor: no executor injected for provider {}",
            req.provider_id
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_executor_returns_provider_not_found() {
        let exec = NullExecutor;
        let req = ExecuteRequest {
            provider_id: "p1".into(),
            model: "claude-sonnet-4-6".into(),
            messages: serde_json::json!([]),
            stream: false,
            params: Value::Null,
        };
        let err = exec.execute(req).await.unwrap_err();
        assert!(matches!(err, ExecuteError::ProviderNotFound(_)));
    }

    #[test]
    fn execute_error_display() {
        let e = ExecuteError::Upstream {
            status: 429,
            body: "rate limit".into(),
        };
        assert_eq!(e.to_string(), "upstream 429: rate limit");
    }
}