//! WebSocket 长连接客户端
//!
//! 负责与云端调度服务器建立和维持 WebSocket 连接，
//! 处理断线重连（指数退避），完整的 send/recv 循环、心跳、超时检测。

use crate::error::ShareError;
use crate::share::protocol::{NodeStatus, SettlementReceipt, TaskPayload, TaskResult};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async, tungstenite, MaybeTlsStream, WebSocketStream,
};

/// 连接状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

/// 连接错误信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ConnectionErrorInfo {
    /// 错误类别：auth, path, network, tls, other
    pub category: String,
    /// 人类可读的错误描述
    pub message: String,
}

/// 云端 Agent WebSocket 端点的固定路径
pub const AGENT_CONNECT_PATH: &str = "/api/v1/agent/connect";

/// 根据用户填写的服务器地址（域名 或 域名:端口）拼出完整的 WebSocket URL。
///
/// 推断规则：
/// - 无协议前缀 → 根据 `use_https` 配置决定 `ws://` 或 `wss://`
/// - 用户若填写了 `wss://` / `ws://` / `http://` / `https://` 等协议前缀，自动按前缀决定协议
pub fn build_server_url(host: &str, use_https: bool) -> String {
    let base = crate::url_utils::build_ws_base_with_tls(host, use_https);
    if base.is_empty() {
        return String::new();
    }
    format!("{base}{AGENT_CONNECT_PATH}")
}

/// WebSocket 客户端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// 服务器地址：域名（如 `api.cc-share.com`）或域名:端口（如 `192.168.1.60:8080`）
    ///
    /// 完整的 WebSocket URL 在连接时由 [`build_server_url`] 推导，用户无需关心协议与路径。
    pub server_host: String,
    pub heartbeat_interval_secs: u64,
    pub max_reconnect_interval_secs: u64,
    pub auth_token: String,
    /// 节点 ID（连接 URL 上 ?node_id= 查询参数由调用方组装）
    #[serde(default)]
    pub node_id: String,
    /// HMAC-SHA256 签名密钥（与 cloud-server `auth.hmac_secret` 一致）
    #[serde(default)]
    pub hmac_secret: String,
    /// 是否使用 HTTPS/WSS 连接云端。本地开发用 HTTP，生产部署开启此项。
    #[serde(default)]
    pub use_https: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_host: String::new(),
            heartbeat_interval_secs: 30,
            max_reconnect_interval_secs: 60,
            auth_token: String::new(),
            node_id: String::new(),
            hmac_secret: String::new(),
            use_https: false,
        }
    }
}

/// WebSocket 客户端发送的消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingMessage {
    NodeStatus(NodeStatus),
    TaskResult(TaskResult),
    Heartbeat,
    /// P2P 信令：回复 P2P offer
    P2pAnswer(crate::share::protocol::P2PAnswer),
}

/// WebSocket 客户端接收的消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncomingMessage {
    /// 云端下发的任务（扁平字段 — 与 cloud-server 的 EncodeTask 对应）
    Task(TaskPayload),
    /// 云端心跳回执，携带服务端处理延迟（毫秒）
    Pong { latency_ms: u64 },
    /// P2P 信令：云端请求本节点接受直连
    P2pOffer(crate::share::protocol::P2POffer),
    /// 结算回执：云端 Finalize 后推送的计费结果
    SettlementReceipt(crate::share::protocol::SettlementReceipt),
}

/// P2P 客户端，管理与云端调度服务器的 WebSocket 长连接
pub struct P2PClient {
    config: ClientConfig,
    outgoing_tx: mpsc::UnboundedSender<OutgoingMessage>,
    outgoing_rx: Option<mpsc::UnboundedReceiver<OutgoingMessage>>,
    running: Arc<AtomicBool>,
    error_callback: Option<Box<dyn Fn(ConnectionErrorInfo) + Send + Sync>>,
    /// Callback invoked with heartbeat round-trip latency (ms) on each pong.
    health_callback: Option<Arc<dyn Fn(u64) + Send + Sync>>,
    /// Channel for P2P offer messages received from the cloud.
    p2p_offer_tx: Option<mpsc::UnboundedSender<crate::share::protocol::P2POffer>>,
    /// Channel for settlement receipts received from the cloud.
    settlement_receipt_tx: Option<mpsc::UnboundedSender<SettlementReceipt>>,
}

type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, tungstenite::Message>;

impl P2PClient {
    /// 创建新的 P2P 客户端
    pub fn new(config: ClientConfig) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            config,
            outgoing_tx: tx,
            outgoing_rx: Some(rx),
            running: Arc::new(AtomicBool::new(false)),
            error_callback: None,
            health_callback: None,
            p2p_offer_tx: None,
            settlement_receipt_tx: None,
        }
    }

    /// Set the channel for receiving P2P offer messages from the cloud.
    pub fn set_p2p_offer_channel(&mut self, tx: mpsc::UnboundedSender<crate::share::protocol::P2POffer>) {
        self.p2p_offer_tx = Some(tx);
    }

    /// Set the channel for receiving settlement receipts from the cloud.
    pub fn set_settlement_receipt_channel(&mut self, tx: mpsc::UnboundedSender<SettlementReceipt>) {
        self.settlement_receipt_tx = Some(tx);
    }

    /// 获取发送端，用于从其他模块发送消息
    pub fn sender(&self) -> mpsc::UnboundedSender<OutgoingMessage> {
        self.outgoing_tx.clone()
    }

    /// 设置连接错误回调，用于将错误详情推送到前端
    pub fn set_error_callback(&mut self, cb: Box<dyn Fn(ConnectionErrorInfo) + Send + Sync>) {
        self.error_callback = Some(cb);
    }

    /// 设置健康回调，每次收到心跳 pong 时调用，参数为往返延迟（毫秒）。
    pub fn set_health_callback(&mut self, cb: Box<dyn Fn(u64) + Send + Sync>) {
        self.health_callback = Some(Arc::from(cb));
    }

    /// 启动客户端连接循环
    ///
    /// 一旦连接成功，run() 会并发运行三个子任务：
    /// 1. **读循环**：解析云端帧，将 Task 推到 `task_tx`，Pong 仅用于刷新超时
    /// 2. **写循环**：从 outgoing_rx 取消息序列化发送
    /// 3. **心跳定时器**：每 heartbeat_interval_secs 注入一条 Heartbeat
    ///
    /// 任一子任务返回错误或 stop() 被调用时，整个会话关闭并按指数退避重连。
    pub async fn run(
        &mut self,
        task_tx: mpsc::UnboundedSender<TaskPayload>,
        state_callback: Box<dyn Fn(ConnectionState) + Send + Sync>,
    ) -> Result<(), ShareError> {
        self.running.store(true, Ordering::SeqCst);
        let mut reconnect_delay_secs: u64 = 1;

        log::info!(
            "CC-Share client: starting connection loop to host={} (auth_token={}...)",
            self.config.server_host,
            if self.config.auth_token.is_empty() { "EMPTY".into() } else { format!("{}chars", self.config.auth_token.len()) }
        );

        // outgoing_rx 只能被 take 一次。重连时把 rx 还给 self。
        let mut outgoing_rx = self
            .outgoing_rx
            .take()
            .ok_or_else(|| ShareError::Connection("outgoing_rx 已被消费".into()))?;

        while self.running.load(Ordering::SeqCst) {
            state_callback(ConnectionState::Connecting);

            let request =
                build_connect_request(&self.config.server_host, &self.config.auth_token, &self.config.node_id, self.config.use_https)?;
            log::info!("CC-Share: connecting to {} (node_id={})", self.config.server_host, self.config.node_id);

            match connect_async(request).await {
                Ok((ws_stream, _)) => {
                    log::info!("✓ CC-Share: 已连接到 {}", self.config.server_host);
                    state_callback(ConnectionState::Connected);
                    reconnect_delay_secs = 1;

                    let health_cb_clone = self.health_callback.clone();
                    let session_result = run_session(
                        ws_stream,
                        &mut outgoing_rx,
                        &task_tx,
                        self.p2p_offer_tx.as_ref(),
                        self.settlement_receipt_tx.as_ref(),
                        self.config.heartbeat_interval_secs,
                        self.running.clone(),
                        health_cb_clone,
                    )
                    .await;
                    if let Err(e) = session_result {
                        log::warn!("CC-Share: 会话结束 - {e}");
                    }
                }
                Err(e) => {
                    log::warn!(
                        "CC-Share: 连接失败 - {e}，{reconnect_delay_secs}s 后重连"
                    );
                    // Categorize the error and provide actionable context
                    let err_str = format!("{e}");
                    let error_info = if err_str.contains("InvalidContentType") {
                        let info = ConnectionErrorInfo {
                            category: "auth".into(),
                            message: format!(
                                "认证失败或服务器响应异常（非 WebSocket 响应）。请检查服务器地址和 auth_token 是否正确。原始错误: {e}"
                            ),
                        };
                        log::error!("CC-Share: {}", info.message);
                        info
                    } else if err_str.contains("404") {
                        let info = ConnectionErrorInfo {
                            category: "path".into(),
                            message: format!(
                                "服务器地址不正确或服务暂不可用（404）。请确认服务器地址填写无误。原始错误: {e}"
                            ),
                        };
                        log::error!("CC-Share: {}", info.message);
                        info
                    } else if err_str.contains("timed out") || err_str.contains("connection refused") || err_str.contains("No route to host") {
                        ConnectionErrorInfo {
                            category: "network".into(),
                            message: format!(
                                "网络连接失败 — 服务器不可达或端口未开放。请检查服务器地址和网络连接。原始错误: {e}"
                            ),
                        }
                    } else if err_str.contains("certificate") || err_str.contains("TLS") || err_str.contains("handshake") {
                        ConnectionErrorInfo {
                            category: "tls".into(),
                            message: format!(
                                "TLS/SSL 握手失败 — 可能是证书问题。请检查服务器证书配置。原始错误: {e}"
                            ),
                        }
                    } else {
                        ConnectionErrorInfo {
                            category: "other".into(),
                            message: format!("连接失败: {e}"),
                        }
                    };

                    if let Some(ref cb) = self.error_callback {
                        cb(error_info);
                    }
                }
            }

            if !self.running.load(Ordering::SeqCst) {
                break;
            }

            state_callback(ConnectionState::Reconnecting);
            tokio::time::sleep(Duration::from_secs(reconnect_delay_secs)).await;

            reconnect_delay_secs =
                (reconnect_delay_secs * 2).min(self.config.max_reconnect_interval_secs);
        }

        // 还回 outgoing_rx 以便后续 run() 能再次启动
        self.outgoing_rx = Some(outgoing_rx);
        state_callback(ConnectionState::Disconnected);
        Ok(())
    }

    /// 停止客户端
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// 一次完整会话：直到出错或 running == false 为止
///
/// `health_cb` 接收心跳往返延迟（毫秒），用于向前端推送服务器健康状态。
async fn run_session(
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    outgoing_rx: &mut mpsc::UnboundedReceiver<OutgoingMessage>,
    task_tx: &mpsc::UnboundedSender<TaskPayload>,
    p2p_offer_tx: Option<&mpsc::UnboundedSender<crate::share::protocol::P2POffer>>,
    settlement_receipt_tx: Option<&mpsc::UnboundedSender<SettlementReceipt>>,
    heartbeat_interval_secs: u64,
    running: Arc<AtomicBool>,
    health_cb: Option<Arc<dyn Fn(u64) + Send + Sync>>,
) -> Result<(), ShareError> {
    let (mut sink, mut stream) = ws_stream.split();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(heartbeat_interval_secs));
    // 第一次 tick 立即触发；跳过它免得连上就发心跳
    heartbeat.tick().await;
    // 记录最近一次心跳发送时间，用于计算往返延迟
    let mut last_heartbeat_sent: Option<std::time::Instant> = None;

    loop {
        if !running.load(Ordering::SeqCst) {
            let _ = sink.close().await;
            return Ok(());
        }

        tokio::select! {
            // 读云端帧
            frame = stream.next() => {
                match frame {
                    Some(Ok(tungstenite::Message::Text(txt))) => {
                        handle_incoming_text(&txt, task_tx, p2p_offer_tx, settlement_receipt_tx, &last_heartbeat_sent, &health_cb);
                    }
                    Some(Ok(tungstenite::Message::Binary(b))) => {
                        if let Ok(txt) = std::str::from_utf8(&b) {
                            handle_incoming_text(txt, task_tx, p2p_offer_tx, settlement_receipt_tx, &last_heartbeat_sent, &health_cb);
                        }
                    }
                    Some(Ok(tungstenite::Message::Ping(p))) => {
                        if let Err(e) = sink.send(tungstenite::Message::Pong(p)).await {
                            return Err(ShareError::Connection(format!("write pong: {e}")));
                        }
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        return Err(ShareError::Connection("远端关闭".into()));
                    }
                    Some(Ok(_)) => { /* Pong / frame: ignore */ }
                    Some(Err(e)) => {
                        return Err(ShareError::Connection(format!("read: {e}")));
                    }
                }
            }

            // 写出站消息
            msg = outgoing_rx.recv() => {
                match msg {
                    Some(out) => write_outgoing(&mut sink, &out).await?,
                    None => {
                        // 通道已关闭：关 socket 退出
                        let _ = sink.close().await;
                        return Ok(());
                    }
                }
            }

            // 心跳
            _ = heartbeat.tick() => {
                last_heartbeat_sent = Some(std::time::Instant::now());
                write_outgoing(&mut sink, &OutgoingMessage::Heartbeat).await?;
            }
        }
    }
}

fn handle_incoming_text(
    txt: &str,
    task_tx: &mpsc::UnboundedSender<TaskPayload>,
    p2p_offer_tx: Option<&mpsc::UnboundedSender<crate::share::protocol::P2POffer>>,
    settlement_receipt_tx: Option<&mpsc::UnboundedSender<SettlementReceipt>>,
    last_heartbeat_sent: &Option<std::time::Instant>,
    health_cb: &Option<Arc<dyn Fn(u64) + Send + Sync>>,
) {
    // 云端的 task 消息是扁平的 {type:"task", task_id, model, ...}
    // 我们用一个 helper 结构来识别 type 后再把剩余字段反序列化为 TaskPayload。
    let probe: serde_json::Value = match serde_json::from_str(txt) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("CC-Share: 解析云端帧失败 - {e}: {txt}");
            return;
        }
    };
    let typ = probe.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match typ {
        "task" => match serde_json::from_value::<TaskPayload>(probe) {
            Ok(t) => {
                if task_tx.send(t).is_err() {
                    log::warn!("CC-Share: task_tx 已关闭，丢弃任务");
                }
            }
            Err(e) => log::warn!("CC-Share: 解析 task 失败 - {e}"),
        },
        "p2p_offer" => match serde_json::from_value::<crate::share::protocol::P2POffer>(probe) {
            Ok(offer) => {
                log::info!("CC-Share: 收到 P2P offer session_id={}", offer.session_id);
                if let Some(tx) = p2p_offer_tx {
                    if tx.send(offer).is_err() {
                        log::warn!("CC-Share: p2p_offer_tx 已关闭，丢弃 offer");
                    }
                } else {
                    log::warn!("CC-Share: 无 P2P offer 通道，丢弃 offer");
                }
            }
            Err(e) => log::warn!("CC-Share: 解析 p2p_offer 失败 - {e}"),
        },
        "settlement_receipt" => match serde_json::from_value::<SettlementReceipt>(probe) {
            Ok(receipt) => {
                log::info!(
                    "CC-Share: 收到结算回执 task_id={} direction={} credits={}",
                    receipt.task_id, receipt.direction, receipt.credits
                );
                if let Some(tx) = settlement_receipt_tx {
                    if tx.send(receipt).is_err() {
                        log::warn!("CC-Share: settlement_receipt_tx 已关闭，丢弃回执");
                    }
                } else {
                    log::warn!("CC-Share: 无结算回执通道，丢弃回执");
                }
            }
            Err(e) => log::warn!("CC-Share: 解析 settlement_receipt 失败 - {e}"),
        },
        "pong" => {
            // 计算心跳往返延迟
            let server_latency = probe.get("latency_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let client_rtt = last_heartbeat_sent
                .map(|sent| sent.elapsed().as_millis() as u64)
                .unwrap_or(0);
            // 总延迟 = 客户端 RTT + 服务端处理延迟
            let total_latency = client_rtt.saturating_add(server_latency);
            log::debug!(
                "CC-Share: heartbeat pong (rtt={}ms, server={}ms, total={}ms)",
                client_rtt, server_latency, total_latency
            );
            if let Some(cb) = health_cb.as_ref() {
                cb(total_latency);
            }
        }
        other => log::warn!("CC-Share: 未知消息类型 {other}"),
    }
}

async fn write_outgoing(sink: &mut WsSink, msg: &OutgoingMessage) -> Result<(), ShareError> {
    let txt = serde_json::to_string(msg)
        .map_err(|e| ShareError::Connection(format!("serialize outgoing: {e}")))?;
    sink.send(tungstenite::Message::Text(txt.into()))
        .await
        .map_err(|e| ShareError::Connection(format!("write: {e}")))
}

/// 构建带认证头的 WebSocket 请求
///
/// `host` 为用户填写的「域名 或 域名:端口」，由 [`build_server_url`] 推导完整 URL。
fn build_connect_request(
    host: &str,
    auth_token: &str,
    node_id: &str,
    use_https: bool,
) -> Result<tungstenite::http::Request<()>, ShareError> {
    use tungstenite::client::IntoClientRequest;

    let url = build_server_url(host, use_https);
    if url.is_empty() {
        return Err(ShareError::Connection("服务器地址未配置".into()));
    }

    // 在 URL 末尾追加 ?node_id=...&fingerprint=... 让云端能在 upgrade 前做绑定校验
    let mut full_url = url.clone();
    if !node_id.is_empty() {
        let sep = if url.contains('?') { '&' } else { '?' };
        let fp = crate::share::fingerprint::compute_fingerprint();
        full_url.push(sep);
        full_url.push_str(&format!(
            "node_id={}&fingerprint={}",
            urlencoding(node_id),
            urlencoding(&fp)
        ));
    }

    let mut request = full_url
        .as_str()
        .into_client_request()
        .map_err(|e| ShareError::Connection(format!("构建请求失败: {e}")))?;

    if !auth_token.is_empty() {
        let header_value = format!("Bearer {auth_token}")
            .parse()
            .map_err(|e| ShareError::Connection(format!("无效的认证头: {e}")))?;
        request
            .headers_mut()
            .insert("Authorization", header_value);
    }

    Ok(request)
}

/// 极小的 percent-encode；只 escape 我们用得到的几个保留字符
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert_eq!(config.heartbeat_interval_secs, 30);
        assert_eq!(config.max_reconnect_interval_secs, 60);
        assert!(config.server_host.is_empty());
    }

    #[test]
    fn test_connection_state_serde() {
        let states = vec![
            ConnectionState::Disconnected,
            ConnectionState::Connecting,
            ConnectionState::Connected,
            ConnectionState::Reconnecting,
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let decoded: ConnectionState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, decoded);
        }
    }

    #[test]
    fn test_build_server_url_pure_domain_no_tls() {
        assert_eq!(
            build_server_url("api.cc-share.com", false),
            "ws://api.cc-share.com/api/v1/agent/connect"
        );
    }

    #[test]
    fn test_build_server_url_pure_domain_with_tls() {
        assert_eq!(
            build_server_url("api.cc-share.com", true),
            "wss://api.cc-share.com/api/v1/agent/connect"
        );
    }

    #[test]
    fn test_build_server_url_with_port() {
        assert_eq!(
            build_server_url("192.168.101.60:8080", false),
            "ws://192.168.101.60:8080/api/v1/agent/connect"
        );
        assert_eq!(
            build_server_url("api.cc-share.com:8443", true),
            "wss://api.cc-share.com:8443/api/v1/agent/connect"
        );
    }

    #[test]
    fn test_build_server_url_strips_protocol_prefix() {
        assert_eq!(
            build_server_url("wss://api.cc-share.com", false),
            "wss://api.cc-share.com/api/v1/agent/connect"
        );
        assert_eq!(
            build_server_url("ws://192.168.1.60:8080", true),
            "ws://192.168.1.60:8080/api/v1/agent/connect"
        );
    }

    #[test]
    fn test_build_server_url_strips_extra_path() {
        assert_eq!(
            build_server_url("api.cc-share.com/some/path", false),
            "ws://api.cc-share.com/api/v1/agent/connect"
        );
    }

    #[test]
    fn test_build_server_url_empty() {
        assert_eq!(build_server_url("", false), "");
        assert_eq!(build_server_url("   ", false), "");
    }

    #[test]
    fn test_build_connect_request_uses_host() {
        let req = build_connect_request("example.com", "test-token", "", false).unwrap();
        let uri = req.uri().to_string();
        assert_eq!(uri, "ws://example.com/api/v1/agent/connect");
        assert_eq!(
            req.headers().get("Authorization").unwrap(),
            "Bearer test-token"
        );
    }

    #[test]
    fn test_build_connect_request_empty_host_errors() {
        let result = build_connect_request("", "tok", "", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_connect_request_empty_token() {
        let req = build_connect_request("example.com", "", "", false).unwrap();
        assert!(req.headers().get("Authorization").is_none());
    }

    #[test]
    fn test_build_connect_request_appends_node_id_and_fingerprint() {
        let req = build_connect_request("example.com", "tok", "node-A", false).unwrap();
        let uri = req.uri().to_string();
        assert!(uri.contains("node_id=node-A"), "uri: {uri}");
        assert!(uri.contains("fingerprint="), "uri: {uri}");
    }

    #[test]
    fn test_reconnect_backoff() {
        let max_secs = 60u64;
        let mut delay = 1u64;
        let delays: Vec<u64> = (0..10)
            .map(|_| {
                let d = delay;
                delay = (delay * 2).min(max_secs);
                d
            })
            .collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 16, 32, 60, 60, 60, 60]);
    }

    #[test]
    fn test_p2p_client_new() {
        let config = ClientConfig::default();
        let client = P2PClient::new(config);
        assert!(!client.running.load(Ordering::SeqCst));
        assert!(client.outgoing_rx.is_some());
    }

    #[test]
    fn test_p2p_client_sender() {
        let config = ClientConfig::default();
        let client = P2PClient::new(config);
        let sender = client.sender();
        let result = sender.send(OutgoingMessage::Heartbeat);
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_incoming_task() {
        let raw = r#"{"type":"task","task_id":"t1","model":"claude-sonnet-4-6","messages":[{"role":"user","content":"hi"}],"stream":false,"params":null}"#;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let no_cb: Option<Arc<dyn Fn(u64) + Send + Sync>> = None;
        handle_incoming_text(raw, &tx, None, None, &None, &no_cb);
        let task = rx.try_recv().expect("应收到 task");
        assert_eq!(task.task_id, "t1");
        assert_eq!(task.model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_handle_incoming_pong_ignored() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let no_cb: Option<Arc<dyn Fn(u64) + Send + Sync>> = None;
        handle_incoming_text(r#"{"type":"pong","latency_ms":5}"#, &tx, None, None, &None, &no_cb);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_handle_incoming_unknown_type_ignored() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let no_cb: Option<Arc<dyn Fn(u64) + Send + Sync>> = None;
        handle_incoming_text(r#"{"type":"nope"}"#, &tx, None, None, &None, &no_cb);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_outgoing_message_serializes_with_type_tag() {
        let m = OutgoingMessage::Heartbeat;
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(s, r#"{"type":"heartbeat"}"#);
    }

    #[test]
    fn test_outgoing_node_status_round_trip() {
        use crate::share::protocol::NodeState;
        let mut upstream = HashMap::new();
        upstream.insert("claude-sonnet-4".to_string(), "glm-5.1:cloud".to_string());
        let m = OutgoingMessage::NodeStatus(NodeStatus {
            node_id: "n1".into(),
            state: NodeState::Idle,
            available_models: vec!["claude-sonnet-4".into()],
            upstream_models: upstream,
            current_concurrency: 0,
            max_concurrency: 1,
            p2p_public_key: None,
        });
        let s = serde_json::to_string(&m).unwrap();
        // 验证带 type 标签且字段被扁平展开（serde tag 模式）
        assert!(s.contains(r#""type":"node_status""#));
        assert!(s.contains(r#""node_id":"n1""#));
        assert!(s.contains(r#""upstream_models""#));
        assert!(s.contains(r#""glm-5.1:cloud""#));
    }

    #[test]
    fn test_connection_error_info_serde() {
        let info = ConnectionErrorInfo {
            category: "path".into(),
            message: "404 Not Found".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: ConnectionErrorInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, decoded);
    }
}
