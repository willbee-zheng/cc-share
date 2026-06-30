//! [`TaskExecutor`] 实现：把 `web:*` provider_id 的任务转发给浏览器扩展
//!
//! 通常作为多路 executor 的一员使用：API-key 类的 provider 走 cc-switch
//! 的真实 ProviderRouter，`web:chatgpt` / `web:claude` 走这里 → 经 [`WebBridge`]
//! → 浏览器扩展 → 调用官方 API。
//!
//! 用法：
//! ```ignore
//! let bridge = Arc::new(WebBridge::new(cfg));
//! bridge.start("node-1".into()).await?;
//! let exec: SharedExecutor = Arc::new(WebExecutor::new(bridge));
//! // (legacy plugin pattern removed in v2 — cc-share now uses ProxyExecutor directly)
//! ```

use crate::share::executor::{ExecuteError, ExecuteRequest, ExecuteResponse, TaskExecutor};
use crate::share::protocol::{TaskPayload, TaskStatus};
use crate::share::web_bridge::WebBridge;
use async_trait::async_trait;
use std::sync::Arc;

const PROVIDER_PREFIX: &str = "web:";

/// 仅处理 `web:*` provider_id 的执行器
pub struct WebExecutor {
    bridge: Arc<WebBridge>,
}

impl WebExecutor {
    pub fn new(bridge: Arc<WebBridge>) -> Self {
        Self { bridge }
    }

    pub fn supports(provider_id: &str) -> bool {
        provider_id.starts_with(PROVIDER_PREFIX)
    }
}

#[async_trait]
impl TaskExecutor for WebExecutor {
    async fn execute(&self, req: ExecuteRequest) -> Result<ExecuteResponse, ExecuteError> {
        if !Self::supports(&req.provider_id) {
            return Err(ExecuteError::ProviderNotFound(req.provider_id));
        }

        let task_id = uuid::Uuid::new_v4().to_string();
        let payload = TaskPayload {
            task_id: task_id.clone(),
            model: req.model,
            messages: req.messages,
            stream: req.stream,
            params: req.params,
        };

        let result = self
            .bridge
            .dispatch(payload, &req.provider_id)
            .await
            .map_err(|e| ExecuteError::Network(e.to_string()))?;

        match result.status {
            TaskStatus::Completed => Ok(ExecuteResponse {
                content: result.content,
                usage: result.usage,
            }),
            TaskStatus::Busy => Err(ExecuteError::Other(
                result.error.unwrap_or_else(|| "web provider busy".into()),
            )),
            TaskStatus::Rejected => Err(ExecuteError::Other(
                result.error.unwrap_or_else(|| "rejected by extension".into()),
            )),
            TaskStatus::Failed | TaskStatus::Pending | TaskStatus::Running => {
                Err(ExecuteError::Other(
                    result.error.unwrap_or_else(|| "extension task failed".into()),
                ))
            }
        }
    }
}

/// 多路 executor：把请求按 provider_id 前缀路由到不同后端
pub struct MultiplexExecutor {
    web: Option<Arc<WebExecutor>>,
    fallback: Arc<dyn TaskExecutor>,
}

impl MultiplexExecutor {
    pub fn new(fallback: Arc<dyn TaskExecutor>) -> Self {
        Self { web: None, fallback }
    }

    pub fn with_web(mut self, web: Arc<WebExecutor>) -> Self {
        self.web = Some(web);
        self
    }
}

#[async_trait]
impl TaskExecutor for MultiplexExecutor {
    async fn execute(&self, req: ExecuteRequest) -> Result<ExecuteResponse, ExecuteError> {
        if WebExecutor::supports(&req.provider_id) {
            match &self.web {
                Some(w) => w.execute(req).await,
                None => Err(ExecuteError::ProviderNotFound(format!(
                    "web executor not configured for {}",
                    req.provider_id
                ))),
            }
        } else {
            self.fallback.execute(req).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::executor::NullExecutor;
    use crate::share::web_bridge::BridgeConfig;
    use serde_json::Value;

    fn req(provider: &str) -> ExecuteRequest {
        ExecuteRequest {
            provider_id: provider.into(),
            model: "gpt-4o".into(),
            messages: serde_json::json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: Value::Null,
        }
    }

    #[test]
    fn test_supports_prefix() {
        assert!(WebExecutor::supports("web:chatgpt"));
        assert!(WebExecutor::supports("web:claude"));
        assert!(!WebExecutor::supports("anthropic"));
        assert!(!WebExecutor::supports(""));
    }

    #[tokio::test]
    async fn test_web_executor_rejects_non_web_provider() {
        let bridge = Arc::new(WebBridge::new(BridgeConfig::default()));
        let exec = WebExecutor::new(bridge);
        let err = exec.execute(req("openai")).await.unwrap_err();
        assert!(matches!(err, ExecuteError::ProviderNotFound(_)));
    }

    #[tokio::test]
    async fn test_web_executor_offline_provider_returns_other() {
        let bridge = Arc::new(WebBridge::new(BridgeConfig::default()));
        let exec = WebExecutor::new(bridge);
        let err = exec.execute(req("web:chatgpt")).await.unwrap_err();
        // 没有扩展配对：bridge 返回 status=Failed，executor 映射成 Other
        assert!(matches!(err, ExecuteError::Other(_)));
    }

    #[tokio::test]
    async fn test_multiplex_routes_web_and_falls_back() {
        let bridge = Arc::new(WebBridge::new(BridgeConfig::default()));
        let web = Arc::new(WebExecutor::new(bridge));
        let multi = MultiplexExecutor::new(Arc::new(NullExecutor)).with_web(web);

        // web:* → web 路径（offline → Other）
        let err1 = multi.execute(req("web:chatgpt")).await.unwrap_err();
        assert!(matches!(err1, ExecuteError::Other(_)));

        // 非 web → fallback 路径（NullExecutor → ProviderNotFound）
        let err2 = multi.execute(req("openai")).await.unwrap_err();
        assert!(matches!(err2, ExecuteError::ProviderNotFound(_)));
    }

    #[tokio::test]
    async fn test_multiplex_without_web_returns_provider_not_found() {
        let multi = MultiplexExecutor::new(Arc::new(NullExecutor));
        let err = multi.execute(req("web:chatgpt")).await.unwrap_err();
        assert!(matches!(err, ExecuteError::ProviderNotFound(_)));
    }
}
