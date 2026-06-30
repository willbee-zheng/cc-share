//! 本地 WebSocket 桥接 — 连接桌面端 cc-share 与浏览器扩展
//!
//! 在 `127.0.0.1:<port>` 监听一个 WebSocket 服务，扩展用一次性 pairing token 配对。
//! 配对成功后：
//! - cc-share 把 `web:*` provider 的任务通过此通道转发给扩展
//! - 扩展把任务结果或活动状态（busy/idle）回送过来
//!
//! 协议（与 `browser-extension/background/bridge.js` 对齐）：
//!
//! ```text
//! ext→host pair       { type: "pair", token, agent }
//! host→ext paired     { type: "paired", node_id, providers }
//! host→ext task       { type: "task", task_id, provider_id, model, messages, params, stream }
//! ext→host result     { type: "task_result", task_id, status, content, usage, error }
//! ext→host status     { type: "web_status", provider_id, state }
//! both    heartbeat   { type: "heartbeat" }
//! host→ext pong       { type: "pong" }
//! ```

use crate::error::ShareError;
use crate::share::protocol::{TaskPayload, TaskResult, TaskStatus, TokenUsage};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

/// 监听地址（默认本机 19829）
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:19829";

/// 单个 web provider 在桥上的状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebProviderState {
    Idle,
    Busy,
    Offline,
}

/// 桥接服务的配置
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub bind_addr: String,
    pub pairing_token: String,
    pub task_timeout_secs: u64,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            bind_addr: DEFAULT_BIND_ADDR.into(),
            pairing_token: String::new(),
            task_timeout_secs: 60,
        }
    }
}

/// 共享内部状态：当前唯一已配对会话 + pending 任务等待表 + provider 状态
#[derive(Default)]
struct BridgeShared {
    /// 已配对会话的 outbound 通道。多次配对时旧会话被踢出。
    paired_tx: tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<HostToExt>>>,
    /// 等待结果的任务 oneshot 通道
    pending: Mutex<HashMap<String, oneshot::Sender<TaskResult>>>,
    /// provider_id → 当前状态（来自扩展上报）
    provider_states: RwLock<HashMap<String, WebProviderState>>,
}

/// 桥接服务句柄。`run()` 后台跑监听循环；外部通过它派发任务。
pub struct WebBridge {
    config: BridgeConfig,
    shared: Arc<BridgeShared>,
    server_handle: Option<JoinHandle<()>>,
}

/// 主进程发送给扩展的消息
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostToExt {
    Paired {
        node_id: String,
        providers: Vec<String>,
    },
    Task(TaskFrame),
    Pong,
}

/// 扁平化的 task 帧（`{type:"task", task_id, ...}`）
#[derive(Debug, Clone, Serialize)]
struct TaskFrame {
    #[serde(rename = "type")]
    ty: &'static str,
    task_id: String,
    provider_id: String,
    model: String,
    messages: Value,
    stream: bool,
    params: Value,
}

/// 扩展发送给主进程的消息
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExtToHost {
    Pair { token: String, #[serde(default)] agent: String },
    TaskResult(TaskResultFrame),
    WebStatus { provider_id: String, state: WebProviderState },
    Heartbeat,
}

#[derive(Debug, Deserialize)]
struct TaskResultFrame {
    task_id: String,
    status: TaskStatus,
    #[serde(default)]
    content: String,
    #[serde(default)]
    usage: Option<TokenUsage>,
    #[serde(default)]
    error: Option<String>,
}

impl WebBridge {
    pub fn new(config: BridgeConfig) -> Self {
        Self {
            config,
            shared: Arc::new(BridgeShared::default()),
            server_handle: None,
        }
    }

    /// 启动监听循环（后台 tokio 任务）。多次调用是幂等的。
    pub async fn start(&mut self, node_id: String) -> Result<(), ShareError> {
        if self.server_handle.is_some() {
            return Ok(());
        }
        let addr: SocketAddr = self
            .config
            .bind_addr
            .parse()
            .map_err(|e| ShareError::Connection(format!("bind addr invalid: {e}")))?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| ShareError::Connection(format!("bind {addr}: {e}")))?;

        let shared = self.shared.clone();
        let token = self.config.pairing_token.clone();
        let node_id_arc = Arc::new(node_id);
        let handle = tokio::spawn(async move {
            log::info!("CC-Share bridge listening on {addr}");
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        log::info!("bridge: accepted from {peer}");
                        let shared = shared.clone();
                        let token = token.clone();
                        let node_id = node_id_arc.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, shared, token, node_id).await
                            {
                                log::warn!("bridge: connection ended - {e}");
                            }
                        });
                    }
                    Err(e) => {
                        log::warn!("bridge: accept failed - {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });
        self.server_handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(h) = self.server_handle.take() {
            h.abort();
            let _ = h.await;
        }
        self.shared.pending.lock().await.clear();
        self.shared.provider_states.write().await.clear();
        *self.shared.paired_tx.lock().await = None;
    }

    /// 是否有扩展已配对
    pub async fn is_paired(&self) -> bool {
        self.shared.paired_tx.lock().await.is_some()
    }

    /// 当前 provider 状态快照
    pub async fn provider_state(&self, provider_id: &str) -> WebProviderState {
        self.shared
            .provider_states
            .read()
            .await
            .get(provider_id)
            .copied()
            .unwrap_or(WebProviderState::Offline)
    }

    /// 派发任务给扩展，等待结果
    ///
    /// 返回 `Err` 仅在没有配对扩展或超时；其他失败（rejected/failed）通过
    /// `Ok(TaskResult)` 的 `status` 字段表示。
    pub async fn dispatch(
        &self,
        task: TaskPayload,
        provider_id: &str,
    ) -> Result<TaskResult, ShareError> {
        // 检查 provider 当前可用
        match self.provider_state(provider_id).await {
            WebProviderState::Busy => {
                return Ok(TaskResult {
                    task_id: task.task_id,
                    status: TaskStatus::Busy,
                    content: String::new(),
                    usage: None,
                    error: Some("web provider busy (user active)".into()),
                    sequence: None,
                    r#final: None,
                });
            }
            WebProviderState::Offline => {
                return Ok(TaskResult {
                    task_id: task.task_id,
                    status: TaskStatus::Failed,
                    content: String::new(),
                    usage: None,
                    error: Some(format!("web provider {provider_id} offline")),
                    sequence: None,
                    r#final: None,
                });
            }
            WebProviderState::Idle => {}
        }

        let tx_opt = { self.shared.paired_tx.lock().await.clone() };
        let tx = match tx_opt {
            Some(t) => t,
            None => {
                return Err(ShareError::Connection("no paired browser extension".into()))
            }
        };

        let (result_tx, result_rx) = oneshot::channel::<TaskResult>();
        {
            let mut pending = self.shared.pending.lock().await;
            pending.insert(task.task_id.clone(), result_tx);
        }

        let frame = TaskFrame {
            ty: "task",
            task_id: task.task_id.clone(),
            provider_id: provider_id.to_string(),
            model: task.model,
            messages: task.messages,
            stream: task.stream,
            params: task.params,
        };

        if tx.send(HostToExt::Task(frame)).is_err() {
            let _ = self.shared.pending.lock().await.remove(&task.task_id);
            return Err(ShareError::Connection(
                "extension disconnected before task dispatched".into(),
            ));
        }

        let timeout = Duration::from_secs(self.config.task_timeout_secs);
        match tokio::time::timeout(timeout, result_rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => {
                Err(ShareError::Connection("result channel closed".into()))
            }
            Err(_) => {
                let _ = self.shared.pending.lock().await.remove(&task.task_id);
                Ok(TaskResult {
                    task_id: task.task_id,
                    status: TaskStatus::Failed,
                    content: String::new(),
                    usage: None,
                    error: Some("extension task timeout".into()),
                    sequence: None,
                    r#final: None,
                })
            }
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    shared: Arc<BridgeShared>,
    token: String,
    node_id: Arc<String>,
) -> Result<(), ShareError> {
    let ws = accept_async(stream)
        .await
        .map_err(|e| ShareError::Connection(format!("ws accept: {e}")))?;

    let (mut sink, mut stream) = ws.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<HostToExt>();

    // 等待 pair 帧
    let pair_frame = match stream.next().await {
        Some(Ok(Message::Text(txt))) => txt,
        _ => return Err(ShareError::Connection("expected pair frame".into())),
    };
    let pair: ExtToHost = serde_json::from_str(&pair_frame)
        .map_err(|e| ShareError::Connection(format!("pair decode: {e}")))?;
    let provided_token = match pair {
        ExtToHost::Pair { token: t, .. } => t,
        _ => return Err(ShareError::Connection("first frame must be pair".into())),
    };

    if token.is_empty() || provided_token != token {
        let _ = sink.send(Message::Close(None)).await;
        return Err(ShareError::Connection("bad pairing token".into()));
    }

    // 配对成功 — 替换之前的 paired_tx
    {
        let mut guard = shared.paired_tx.lock().await;
        *guard = Some(out_tx.clone());
    }

    let paired = HostToExt::Paired {
        node_id: node_id.as_str().to_string(),
        providers: vec!["web:chatgpt".into(), "web:claude".into()],
    };
    let raw =
        serde_json::to_string(&paired).map_err(|e| ShareError::Connection(format!("ser: {e}")))?;
    sink.send(Message::Text(raw.into()))
        .await
        .map_err(|e| ShareError::Connection(format!("write paired: {e}")))?;

    // 主循环：读 ext→host 消息 / 写 host→ext 消息
    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(txt))) => {
                        if let Err(e) = handle_ext_message(&txt, &shared, &out_tx).await {
                            log::warn!("bridge: handle ext msg - {e}");
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => { /* binary/ping/pong: ignore */ }
                    Some(Err(e)) => {
                        log::warn!("bridge: ws read - {e}");
                        break;
                    }
                }
            }
            outgoing = out_rx.recv() => {
                match outgoing {
                    Some(msg) => {
                        let raw = match serde_json::to_string(&msg) {
                            Ok(s) => s,
                            Err(e) => {
                                log::warn!("bridge: serialize host→ext - {e}");
                                continue;
                            }
                        };
                        if sink.send(Message::Text(raw.into())).await.is_err() { break; }
                    }
                    None => break,
                }
            }
        }
    }

    // 清理：仅在自己仍是当前 paired_tx 时才清空
    {
        let mut guard = shared.paired_tx.lock().await;
        if let Some(cur) = guard.as_ref() {
            if cur.same_channel(&out_tx) {
                *guard = None;
            }
        }
    }
    log::info!("bridge: session closed");
    Ok(())
}

async fn handle_ext_message(
    raw: &str,
    shared: &Arc<BridgeShared>,
    out_tx: &tokio::sync::mpsc::UnboundedSender<HostToExt>,
) -> Result<(), ShareError> {
    let msg: ExtToHost = serde_json::from_str(raw)
        .map_err(|e| ShareError::Connection(format!("decode: {e}")))?;
    match msg {
        ExtToHost::Heartbeat => {
            let _ = out_tx.send(HostToExt::Pong);
        }
        ExtToHost::Pair { .. } => {
            // 配对应只发生一次；忽略后续 pair
        }
        ExtToHost::WebStatus { provider_id, state } => {
            shared.provider_states.write().await.insert(provider_id, state);
        }
        ExtToHost::TaskResult(frame) => {
            let task_id = frame.task_id.clone();
            let result = TaskResult {
                task_id: frame.task_id,
                status: frame.status,
                content: frame.content,
                usage: frame.usage,
                error: frame.error,
                sequence: None,
                r#final: None,
            };
            let waiter = { shared.pending.lock().await.remove(&task_id) };
            if let Some(tx) = waiter {
                let _ = tx.send(result);
            } else {
                log::debug!("bridge: orphan task_result {task_id}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_bind_addr() {
        let cfg = BridgeConfig::default();
        assert_eq!(cfg.bind_addr, DEFAULT_BIND_ADDR);
        assert_eq!(cfg.task_timeout_secs, 60);
    }

    #[test]
    fn test_web_provider_state_serde() {
        for s in [
            WebProviderState::Idle,
            WebProviderState::Busy,
            WebProviderState::Offline,
        ] {
            let raw = serde_json::to_string(&s).unwrap();
            let back: WebProviderState = serde_json::from_str(&raw).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_task_frame_serializes_with_type_tag() {
        let frame = TaskFrame {
            ty: "task",
            task_id: "t1".into(),
            provider_id: "web:chatgpt".into(),
            model: "gpt-4o".into(),
            messages: serde_json::json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: Value::Null,
        };
        let s = serde_json::to_string(&frame).unwrap();
        assert!(s.starts_with(r#"{"type":"task""#));
        assert!(s.contains(r#""task_id":"t1""#));
        assert!(s.contains(r#""provider_id":"web:chatgpt""#));
    }

    #[test]
    fn test_ext_to_host_decode_status() {
        let raw =
            r#"{"type":"web_status","provider_id":"web:chatgpt","state":"busy"}"#;
        let msg: ExtToHost = serde_json::from_str(raw).unwrap();
        match msg {
            ExtToHost::WebStatus { provider_id, state } => {
                assert_eq!(provider_id, "web:chatgpt");
                assert_eq!(state, WebProviderState::Busy);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_ext_to_host_decode_task_result() {
        let raw = r#"{"type":"task_result","task_id":"t1","status":"completed","content":"hi","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;
        let msg: ExtToHost = serde_json::from_str(raw).unwrap();
        match msg {
            ExtToHost::TaskResult(f) => {
                assert_eq!(f.task_id, "t1");
                assert_eq!(f.status, TaskStatus::Completed);
                assert_eq!(f.usage.unwrap().total_tokens, 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn test_bridge_dispatch_no_extension_returns_error() {
        let bridge = WebBridge::new(BridgeConfig::default());
        let task = TaskPayload {
            task_id: "t1".into(),
            model: "gpt-4o".into(),
            messages: serde_json::json!([]),
            stream: false,
            params: Value::Null,
        };
        let err = bridge.dispatch(task, "web:chatgpt").await;
        // provider_state 默认 Offline，所以会返回 Ok(Failed)（不是 Err）
        match err.unwrap() {
            r if r.status == TaskStatus::Failed => {
                assert!(r.error.unwrap().contains("offline"));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bridge_provider_state_default_offline() {
        let bridge = WebBridge::new(BridgeConfig::default());
        let s = bridge.provider_state("web:chatgpt").await;
        assert_eq!(s, WebProviderState::Offline);
    }
}
