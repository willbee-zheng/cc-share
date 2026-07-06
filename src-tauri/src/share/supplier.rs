//! 供应者模式：接收并执行云端任务
//!
//! 接收云端下发的代刷任务，调用本地 Provider（通过 [`TaskExecutor`] trait 注入）
//! 执行请求并返回结果，同时执行内容审查、本地互斥与拟人化延迟。

use crate::content_filter::rules::ContentFilter;
use crate::database::ShareDb;
use crate::error::ShareError;
use crate::share::executor::{ExecuteRequest, SharedExecutor, StreamEvent, TaskExecutor};
use crate::share::humanizer::Humanizer;
use crate::share::mutex::MutexChecker;
use crate::share::protocol::{TaskPayload, TaskResult, TaskStatus, TokenUsage};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

/// 供应者任务执行器
///
/// 负责处理云端下发的任务：
/// 1. 内容安全审查（黑名单关键词）
/// 2. 本地互斥检查（避免与本地用户操作冲突）
/// 3. 调用注入的 [`TaskExecutor`] 执行真实请求
/// 4. 拟人化延迟（避免被风控）
/// 5. 记录任务日志到 share.db
pub struct Supplier {
    db: Arc<ShareDb>,
    content_filter: ContentFilter,
    mutex_checker: MutexChecker,
    humanizer: Humanizer,
    executor: SharedExecutor,
}

impl Supplier {
    /// 创建供应者任务执行器
    pub fn new(db: Arc<ShareDb>, executor: SharedExecutor) -> Self {
        Self {
            db,
            content_filter: ContentFilter::new(),
            mutex_checker: MutexChecker::new(),
            humanizer: Humanizer::with_defaults(),
            executor,
        }
    }

    /// 替换 humanizer（仅供测试 / 高级配置使用）
    pub fn with_humanizer(mut self, h: Humanizer) -> Self {
        self.humanizer = h;
        self
    }

    /// 处理云端下发的任务
    pub async fn handle_task(&self, payload: TaskPayload, provider_id: &str, upstream_model: Option<&str>) -> TaskResult {
        let start = Instant::now();
        log::info!("▶ handle_task: task_id={}, model={}, provider={}, stream={}",
            payload.task_id, payload.model, provider_id, payload.stream);

        // 1. 内容过滤
        if let Err(e) = self.check_content(&payload) {
            log::warn!("handle_task: task_id={} rejected by content filter: {}", payload.task_id, e);
            return self.log_and_return(
                &payload,
                TaskResult {
                    task_id: payload.task_id.clone(),
                    status: TaskStatus::Rejected,
                    content: String::new(),
                    usage: None,
                    error: Some(format!("内容过滤拦截: {e}")),
                    sequence: None,
                    r#final: None,
                },
                start,
                upstream_model,
            );
        }

        // 2. 互斥检查
        if self.mutex_checker.is_busy(provider_id) {
            log::warn!("handle_task: task_id={} busy, provider {} in use locally", payload.task_id, provider_id);
            return self.log_and_return(
                &payload,
                TaskResult {
                    task_id: payload.task_id.clone(),
                    status: TaskStatus::Busy,
                    content: String::new(),
                    usage: None,
                    error: Some("本地用户正在使用该 Provider".into()),
                    sequence: None,
                    r#final: None,
                },
                start,
                upstream_model,
            );
        }

        // 3. 标记忙碌
        self.mutex_checker.set_busy(provider_id, true);

        // 4. 执行请求
        let exec_result = self
            .executor
            .execute(ExecuteRequest {
                provider_id: provider_id.to_string(),
                model: payload.model.clone(),
                messages: payload.messages.clone(),
                stream: payload.stream,
                params: payload.params.clone(),
            })
            .await;

        // 5. 释放互斥
        self.mutex_checker.set_busy(provider_id, false);

        // 6. 拟人化冷却（不阻塞返回 — 在后台 sleep；调用方可决定是否 await）
        // 这里直接 await，是因为单个任务完成后再立即接下一个的概率不高，
        // cooldown 不会阻塞其它 provider 的并发任务（mutex 是 per-provider）。
        let cooldown = self.humanizer.next_cooldown();

        // 7. 转换结果
        let result = match exec_result {
            Ok(resp) => {
                log::info!(
                    "✓ handle_task: task_id={} completed, tokens={:?}",
                    payload.task_id,
                    resp.usage
                );
                TaskResult {
                    task_id: payload.task_id.clone(),
                    status: TaskStatus::Completed,
                    content: resp.content,
                    usage: resp.usage,
                    error: None,
                    sequence: None,
                    r#final: None,
                }
            }
            Err(e) => {
                log::warn!("✗ handle_task: task_id={} failed: {}", payload.task_id, e);
                TaskResult {
                    task_id: payload.task_id.clone(),
                    status: TaskStatus::Failed,
                    content: String::new(),
                    usage: None,
                    error: Some(e.to_string()),
                    sequence: None,
                    r#final: None,
                }
            }
        };

        let logged = self.log_and_return(&payload, result, start, upstream_model);
        // 在日志写完后再 sleep，避免阻塞数据库 IO
        tokio::time::sleep(cooldown).await;
        logged
    }

    fn check_content(&self, payload: &TaskPayload) -> Result<(), ShareError> {
        let messages_text = serde_json::to_string(&payload.messages)
            .unwrap_or_else(|_| payload.messages.to_string());
        self.content_filter.check(&messages_text)
    }

    fn log_and_return(
        &self,
        payload: &TaskPayload,
        result: TaskResult,
        start: Instant,
        upstream_model: Option<&str>,
    ) -> TaskResult {
        let latency_ms = start.elapsed().as_millis() as i32;
        let status_str = match &result.status {
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Rejected => "rejected",
            TaskStatus::Busy => "busy",
            TaskStatus::Pending | TaskStatus::Running => "running",
        };

        if let Err(e) = self
            .db
            .insert_p2p_task_log(&crate::database::dao_credits::P2PTaskLog {
                task_id: payload.task_id.clone(),
                direction: "supply".into(),
                model: payload.model.clone(),
                upstream_model: upstream_model.map(|s| s.to_string()),
                tokens_prompt: result
                    .usage
                    .as_ref()
                    .map(|u| u.prompt_tokens as i32)
                    .unwrap_or(0),
                tokens_completion: result
                    .usage
                    .as_ref()
                    .map(|u| u.completion_tokens as i32)
                    .unwrap_or(0),
                credits: 0.0, // 实际清算由云端进行
                latency_ms: Some(latency_ms),
                status: status_str.into(),
                error_message: result.error.clone(),
                created_at: chrono::Utc::now().timestamp(),
            })
        {
            log::warn!("记录任务日志失败: {e}");
        }
        result
    }

    /// 处理云端下发的流式任务。
    ///
    /// 与 `handle_task` 相同的前置检查（内容过滤、互斥），但通过
    /// `on_result` 回调逐 chunk 发送 `TaskResult`，实现端到端流式。
    /// 返回最终帧的 `TaskResult`（用于日志和 DaemonEvent）。
    pub async fn handle_task_stream<F>(
        &self,
        payload: TaskPayload,
        provider_id: &str,
        upstream_model: Option<&str>,
        mut on_result: F,
    ) -> TaskResult
    where
        F: FnMut(TaskResult) + Send,
    {
        let start = Instant::now();
        log::info!(
            "▶ handle_task_stream: task_id={}, model={}, provider={}, stream=true",
            payload.task_id, payload.model, provider_id
        );

        // 1. 内容过滤
        if let Err(e) = self.check_content(&payload) {
            log::warn!("handle_task_stream: task_id={} rejected by content filter: {}", payload.task_id, e);
            let result = TaskResult::terminal(
                &payload.task_id,
                TaskStatus::Rejected,
                String::new(),
                None,
                Some(format!("内容过滤拦截: {e}")),
            );
            on_result(result.clone());
            return self.log_and_return(&payload, result, start, upstream_model);
        }

        // 2. 互斥检查
        if self.mutex_checker.is_busy(provider_id) {
            log::warn!("handle_task_stream: task_id={} busy, provider {} in use locally", payload.task_id, provider_id);
            let result = TaskResult::terminal(
                &payload.task_id,
                TaskStatus::Busy,
                String::new(),
                None,
                Some("本地用户正在使用该 Provider".into()),
            );
            on_result(result.clone());
            return self.log_and_return(&payload, result, start, upstream_model);
        }

        // 3. 标记忙碌
        self.mutex_checker.set_busy(provider_id, true);

        // 4. 流式执行，逐 chunk 转发
        let mut seq: u64 = 0;
        let mut final_result = TaskResult::terminal(
            &payload.task_id,
            TaskStatus::Failed,
            String::new(),
            None,
            Some("stream ended without terminal event".into()),
        );

        let exec_result = self
            .executor
            .execute_stream(ExecuteRequest {
                provider_id: provider_id.to_string(),
                model: payload.model.clone(),
                messages: payload.messages.clone(),
                stream: true,
                params: payload.params.clone(),
            })
            .await;

        match exec_result {
            Ok(mut rx) => {
                while let Some(event) = rx.recv().await {
                    match event {
                        StreamEvent::Delta(text) => {
                            seq += 1;
                            on_result(TaskResult::running_chunk(&payload.task_id, text, seq));
                        }
                        StreamEvent::Usage(usage) => {
                            // Buffer usage for terminal frame.
                            final_result.usage = Some(usage);
                        }
                        StreamEvent::End => {
                            seq += 1;
                            final_result.status = TaskStatus::Completed;
                            final_result.sequence = Some(seq);
                            final_result.r#final = Some(true);
                            on_result(final_result.clone());
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("✗ handle_task_stream: task_id={} failed: {}", payload.task_id, e);
                final_result.status = TaskStatus::Failed;
                final_result.error = Some(e.to_string());
                final_result.r#final = Some(true);
                on_result(final_result.clone());
            }
        }

        // 5. 释放互斥
        self.mutex_checker.set_busy(provider_id, false);

        // 6. 拟人化冷却
        let cooldown = self.humanizer.next_cooldown();
        let logged = self.log_and_return(&payload, final_result, start, upstream_model);
        tokio::time::sleep(cooldown).await;
        logged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::executor::{ExecuteError, ExecuteResponse, NullExecutor};
    use crate::share::humanizer::HumanizerConfig;
    use crate::share::protocol::TokenUsage;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn create_test_db() -> Arc<ShareDb> {
        Arc::new(ShareDb::memory().expect("创建内存数据库失败"))
    }

    fn fast_humanizer() -> Humanizer {
        Humanizer::new(HumanizerConfig {
            min_cooldown_secs: 0,
            max_cooldown_secs: 0,
            enable_active_hours: false,
            active_hours_start: 0,
            active_hours_end: 24,
        })
    }

    /// 测试用 mock executor：返回固定响应或错误
    struct MockExecutor {
        calls: AtomicUsize,
        respond: Result<ExecuteResponse, ExecuteError>,
    }

    impl MockExecutor {
        fn ok(content: &str, prompt: u32, completion: u32) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                respond: Ok(ExecuteResponse {
                    content: content.into(),
                    usage: Some(TokenUsage {
                        prompt_tokens: prompt,
                        completion_tokens: completion,
                        total_tokens: prompt + completion,
                    }),
                }),
            })
        }
        fn fail(err: ExecuteError) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                respond: Err(err),
            })
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl TaskExecutor for MockExecutor {
        async fn execute(&self, _req: ExecuteRequest) -> Result<ExecuteResponse, ExecuteError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.respond.clone()
        }
    }

    #[test]
    fn test_supplier_new_with_null_executor() {
        let db = create_test_db();
        let _ = Supplier::new(db, Arc::new(NullExecutor));
    }

    #[tokio::test]
    async fn test_supplier_busy_short_circuits() {
        let db = create_test_db();
        let exec = MockExecutor::ok("ignored", 0, 0);
        let supplier = Supplier::new(db, exec.clone()).with_humanizer(fast_humanizer());

        supplier.mutex_checker.set_busy("p1", true);

        let payload = TaskPayload {
            task_id: "t-busy".into(),
            model: "claude-sonnet-4-6".into(),
            messages: serde_json::json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: serde_json::Value::Null,
        };
        let r = supplier.handle_task(payload, "p1", None).await;
        assert_eq!(r.status, TaskStatus::Busy);
        assert_eq!(exec.call_count(), 0, "busy 时不应调用 executor");
    }

    #[tokio::test]
    async fn test_supplier_completed_passes_through_executor() {
        let db = create_test_db();
        let exec = MockExecutor::ok("hello there", 10, 5);
        let supplier = Supplier::new(db, exec.clone()).with_humanizer(fast_humanizer());

        let payload = TaskPayload {
            task_id: "t-ok".into(),
            model: "claude-sonnet-4-6".into(),
            messages: serde_json::json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: serde_json::Value::Null,
        };
        let r = supplier.handle_task(payload, "p1", None).await;
        assert_eq!(r.status, TaskStatus::Completed);
        assert_eq!(r.content, "hello there");
        let usage = r.usage.unwrap();
        assert_eq!(usage.total_tokens, 15);
        assert_eq!(exec.call_count(), 1);
    }

    #[tokio::test]
    async fn test_supplier_executor_error_maps_to_failed() {
        let db = create_test_db();
        let exec = MockExecutor::fail(ExecuteError::Upstream {
            status: 429,
            body: "rate limit".into(),
        });
        let supplier = Supplier::new(db, exec).with_humanizer(fast_humanizer());

        let payload = TaskPayload {
            task_id: "t-err".into(),
            model: "claude-sonnet-4-6".into(),
            messages: serde_json::json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: serde_json::Value::Null,
        };
        let r = supplier.handle_task(payload, "p1", None).await;
        assert_eq!(r.status, TaskStatus::Failed);
        assert!(r.error.unwrap().contains("429"));
    }
}
