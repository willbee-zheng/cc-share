//! P2P 守护线程
//!
//! 后台常驻线程，整合：
//! - [`P2PClient`] WebSocket 长连接与重连
//! - [`Supplier`] 接收任务并执行
//! - 连接状态/任务事件回调（由 [`crate::lib`] 桥接到 Tauri events）

use crate::ccswitch::ProviderRegistry;
use crate::commands::providers;
use crate::database::ShareDb;
use crate::error::ShareError;
use crate::p2p::connection::{P2PConnectionManager, DEFAULT_P2P_PORT};
use crate::p2p::key::P2PKeyManager;
use crate::p2p::supplier::{self, P2PSessionStore};
use crate::share::client::{ClientConfig, ConnectionErrorInfo, ConnectionState, OutgoingMessage, P2PClient};
use crate::share::executor::SharedExecutor;
use crate::share::protocol::{NodeState, NodeStatus, P2PAnswer, P2POffer, TaskPayload, TaskStatus};
use crate::share::supplier::Supplier;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// How often (in seconds) the daemon refreshes model availability from cc-switch
/// and re-sends NodeStatus to the cloud.
const MODEL_REFRESH_INTERVAL_SECS: u64 = 150;

/// 由 daemon 推给上层（commands/Tauri events）的通知
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonEvent {
    /// 连接状态变化
    ConnectionState(ConnectionState),
    /// 连接错误详情（包含错误类别和消息）
    ConnectionError {
        category: String,
        message: String,
    },
    /// 一个任务完成（供前端展示进度/收益条）
    TaskFinished {
        task_id: String,
        status: TaskStatus,
        latency_ms: u64,
    },
}

/// 守护线程管理器
pub struct Daemon {
    db: Arc<ShareDb>,
    executor: SharedExecutor,
    /// 默认 provider id：当云端任务未指定 provider_id 时回落到此（Phase 4 启用按 model 路由）
    #[allow(dead_code)]
    default_provider_id: String,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    refresh_handle: Option<JoinHandle<()>>,
    outgoing_tx: Option<mpsc::UnboundedSender<OutgoingMessage>>,
    /// cc-switch provider registry for periodic model refresh.
    provider_registry: Arc<ProviderRegistry>,
    /// P2P key manager for advertising public key and deriving shared secrets.
    p2p_key_manager: Arc<P2PKeyManager>,
    /// P2P connection manager for accepting incoming QUIC connections.
    p2p_conn_manager: Arc<P2PConnectionManager>,
    /// P2P session store for mapping session_id → consumer_pubkey.
    p2p_session_store: Arc<P2PSessionStore>,
}

impl Daemon {
    pub fn new(
        db: Arc<ShareDb>,
        executor: SharedExecutor,
        default_provider_id: String,
        provider_registry: Arc<ProviderRegistry>,
        p2p_key_manager: Arc<P2PKeyManager>,
        p2p_conn_manager: Arc<P2PConnectionManager>,
    ) -> Self {
        Self {
            db,
            executor,
            default_provider_id,
            running: Arc::new(AtomicBool::new(false)),
            handle: None,
            refresh_handle: None,
            outgoing_tx: None,
            provider_registry,
            p2p_key_manager,
            p2p_conn_manager,
            p2p_session_store: Arc::new(P2PSessionStore::new()),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 上报当前节点状态（手动触发，例如 share toggle on/off 时刷新）
    pub fn report_status(&self, status: NodeStatus) -> Result<(), ShareError> {
        match &self.outgoing_tx {
            Some(tx) => tx
                .send(OutgoingMessage::NodeStatus(status))
                .map_err(|e| ShareError::Connection(format!("send node_status: {e}"))),
            None => Err(ShareError::Connection("daemon 未启动".into())),
        }
    }

    /// 启动守护线程。返回时立刻返回，I/O 在后台 tokio 任务中跑。
    ///
    /// 调用方提供 `event_callback` 接收连接状态变化和任务完成事件。
    pub fn start<F>(
        &mut self,
        client_config: ClientConfig,
        mut node_status: NodeStatus,
        event_callback: F,
    ) -> Result<(), ShareError>
    where
        F: Fn(DaemonEvent) + Send + Sync + 'static,
    {
        if self.running.load(Ordering::SeqCst) {
            return Err(ShareError::Connection("daemon 已在运行".into()));
        }
        self.running.store(true, Ordering::SeqCst);

        // Store node_id and max_concurrency for periodic model refresh.
        let refresh_node_id = node_status.node_id.clone();
        let refresh_max_concurrency = node_status.max_concurrency;

        let (task_tx, mut task_rx) = mpsc::unbounded_channel::<TaskPayload>();
        let (p2p_offer_tx, mut p2p_offer_rx) = mpsc::unbounded_channel::<P2POffer>();
        let mut client = P2PClient::new(client_config);
        client.set_p2p_offer_channel(p2p_offer_tx);
        let outgoing_tx = client.sender();
        self.outgoing_tx = Some(outgoing_tx.clone());

        // Advertise P2P public key in NodeStatus so cloud can match P2P-capable nodes.
        node_status.p2p_public_key = Some(self.p2p_key_manager.public_key_base64());

        // 启动后立刻发一次 NodeStatus（让云端把我们登记为可用）
        let _ = outgoing_tx.send(OutgoingMessage::NodeStatus(node_status));

        let supplier = Arc::new(Supplier::new(self.db.clone(), self.executor.clone()));
        let event_cb = Arc::new(event_callback);
        let provider_registry = self.provider_registry.clone();
        let running_flag = self.running.clone();
        let p2p_pubkey_for_offer = self.p2p_key_manager.public_key_base64();
        let p2p_conn_for_offer = self.p2p_conn_manager.clone();
        let p2p_key_for_accept = self.p2p_key_manager.clone();
        let p2p_conn_for_accept = self.p2p_conn_manager.clone();
        let executor_for_accept = self.executor.clone();
        let db_for_accept = self.db.clone();
        let db_for_offer = self.db.clone();
        let p2p_session_store = self.p2p_session_store.clone();

        // Clone outgoing_tx for refresh task before moving into session.
        let refresh_tx = outgoing_tx.clone();

        let handle = tokio::spawn(async move {
            // 子任务 A：处理云端下发的任务
            let supplier_clone = supplier.clone();
            let outgoing_for_results = outgoing_tx.clone();
            let event_cb_for_tasks = event_cb.clone();
            let registry_for_tasks = provider_registry.clone();
            let task_handler: JoinHandle<()> = tokio::spawn(async move {
                while let Some(task) = task_rx.recv().await {
                    let task_id = task.task_id.clone();
                    let started = std::time::Instant::now();
                    // MVP: 所有任务路由到 default provider；Phase 4 引入按 model 路由
                    let provider_id = "default";
                    // Resolve representative model name to real upstream model name.
                    let upstream_model = registry_for_tasks
                        .snapshot()
                        .await
                        .upstream_models
                        .get(&task.model)
                        .cloned();
                    let upstream_ref = upstream_model.as_deref();

                    if task.stream {
                        // 流式：逐 chunk 通过 WS 发回云端
                        let result = supplier_clone
                            .handle_task_stream(task, provider_id, upstream_ref, |r| {
                                let _ = outgoing_for_results.send(OutgoingMessage::TaskResult(r));
                            })
                            .await;
                        let elapsed = started.elapsed().as_millis() as u64;
                        event_cb_for_tasks(DaemonEvent::TaskFinished {
                            task_id,
                            status: result.status,
                            latency_ms: elapsed,
                        });
                    } else {
                        // 非流式：单次响应
                        let result = supplier_clone.handle_task(task, provider_id, upstream_ref).await;
                        let elapsed = started.elapsed().as_millis() as u64;

                        let _ = outgoing_for_results.send(OutgoingMessage::TaskResult(result.clone()));
                        event_cb_for_tasks(DaemonEvent::TaskFinished {
                            task_id,
                            status: result.status,
                            latency_ms: elapsed,
                        });
                    }
                }
            });

            // 子任务 A2：处理云端下发的 P2P Offer
            let outgoing_for_p2p = outgoing_tx.clone();
            let p2p_session_store_for_offer = p2p_session_store.clone();
            let p2p_port_for_offer = DEFAULT_P2P_PORT;
            let p2p_offer_handler: JoinHandle<()> = tokio::spawn(async move {
                while let Some(offer) = p2p_offer_rx.recv().await {
                    log::info!("P2P: received offer session_id={}", offer.session_id);
                    // Register the consumer's public key so the QUIC handler can
                    // derive the correct task key when a connection arrives.
                    p2p_session_store_for_offer.register(
                        offer.session_id.clone(),
                        offer.consumer_pubkey.clone(),
                    ).await;
                    // Accept the offer and provide candidate addresses.
                    // Start with STUN-discovered public address (highest priority).
                    let mut candidates: Vec<String> = Vec::new();

                    // Discover public address via STUN.
                    let p2p_config = db_for_offer.load_p2p_config().unwrap_or_default();
                    let stun_server = if p2p_config.stun_server.is_empty() {
                        format!("{}:7890", "stun.shareplan.cloud")
                    } else {
                        p2p_config.stun_server.clone()
                    };
                    match crate::p2p::stun_client::discover_public_addr(
                        &stun_server,
                        p2p_port_for_offer,
                        std::time::Duration::from_secs(3),
                    ).await {
                        Ok(public_addr) => {
                            log::info!("P2P: supplier STUN discovered public address: {}", public_addr);
                            candidates.push(public_addr.to_string());
                        }
                        Err(e) => {
                            log::warn!("P2P: supplier STUN discovery failed (non-fatal): {e}");
                        }
                    }

                    // Add local QUIC endpoint addresses (fallback).
                    match p2p_conn_for_offer.local_addr().await {
                        Ok(addrs) if !addrs.is_empty() => {
                            for addr in &addrs {
                                let s = addr.to_string();
                                if !candidates.contains(&s) {
                                    candidates.push(s);
                                }
                            }
                        }
                        _ => {
                            let fallback = format!("127.0.0.1:{}", p2p_port_for_offer);
                            if !candidates.contains(&fallback) {
                                candidates.push(fallback);
                            }
                        }
                    }

                    let local_pubkey = p2p_pubkey_for_offer.clone();
                    let answer = P2PAnswer {
                        session_id: offer.session_id.clone(),
                        accepted: true,
                        supplier_candidates: candidates,
                        supplier_pubkey: Some(local_pubkey),
                        reason: None,
                    };
                    let session_id = answer.session_id.clone();
                    let _ = outgoing_for_p2p.send(OutgoingMessage::P2pAnswer(answer));
                    log::info!("P2P: sent P2PAnswer for session_id={}", session_id);
                }
                log::info!("P2P: offer handler channel closed");
            });

            // 子任务 A3：P2P QUIC accept loop（接受消费者直连）
            let p2p_session_store_for_accept = p2p_session_store.clone();
            let p2p_accept_handle: JoinHandle<()> = tokio::spawn(async move {
                supplier::accept_loop(
                    p2p_conn_for_accept,
                    p2p_key_for_accept,
                    executor_for_accept,
                    db_for_accept,
                    p2p_session_store_for_accept,
                ).await;
            });

            // 子任务 B：WebSocket 连接循环
            let event_cb_for_state = event_cb.clone();
            let state_callback: Box<dyn Fn(ConnectionState) + Send + Sync> = Box::new(move |s| {
                event_cb_for_state(DaemonEvent::ConnectionState(s));
            });

            let event_cb_for_error = event_cb.clone();
            let error_callback: Box<dyn Fn(ConnectionErrorInfo) + Send + Sync> = Box::new(move |info| {
                event_cb_for_error(DaemonEvent::ConnectionError {
                    category: info.category.clone(),
                    message: info.message.clone(),
                });
            });
            client.set_error_callback(error_callback);

            if let Err(e) = client.run(task_tx, state_callback).await {
                log::warn!("CC-Share daemon: client 退出 - {e}");
            }

            // 客户端结束时停掉任务处理、P2P offer 处理和 P2P accept loop
            running_flag.store(false, Ordering::SeqCst);
            task_handler.abort();
            p2p_offer_handler.abort();
            p2p_accept_handle.abort();
        });

        self.handle = Some(handle);

        // Start periodic model refresh: re-discovers models from cc-switch and
        // re-sends NodeStatus so the cloud stays up-to-date.
        let refresh_running = self.running.clone();
        let refresh_registry = self.provider_registry.clone();
        let refresh_db = self.db.clone();
        let refresh_p2p_key = self.p2p_key_manager.public_key_base64();
        let refresh_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(MODEL_REFRESH_INTERVAL_SECS)).await;
                if !refresh_running.load(Ordering::SeqCst) {
                    break;
                }
                // Re-discover models from cc-switch
                refresh_registry.refresh().await;
                let snap = refresh_registry.snapshot().await;
                let whitelist = match refresh_db.load_whitelist() {
                    Ok(wl) => wl,
                    Err(e) => {
                        log::warn!("daemon refresh: failed to load whitelist: {e}");
                        Vec::new()
                    }
                };
                let models = providers::filter_models(snap.available_models, whitelist);
                if models.is_empty() {
                    log::warn!("daemon refresh: no models available after refresh, skipping NodeStatus update");
                    continue;
                }
                let status = NodeStatus {
                    node_id: refresh_node_id.clone(),
                    state: NodeState::Idle,
                    available_models: models,
                    upstream_models: snap.upstream_models,
                    current_concurrency: 0,
                    max_concurrency: refresh_max_concurrency,
                    p2p_public_key: Some(refresh_p2p_key.clone()),
                };
                log::info!("daemon refresh: sending updated NodeStatus with {} models", status.available_models.len());
                let _ = refresh_tx.send(OutgoingMessage::NodeStatus(status));
            }
        });
        self.refresh_handle = Some(refresh_handle);

        Ok(())
    }

    /// 停止守护线程
    pub async fn stop(&mut self) {
        if !self.running.load(Ordering::SeqCst) {
            log::info!("daemon stop: already stopped, skipping");
            return;
        }
        log::info!("◀ daemon stop: stopping...");
        self.running.store(false, Ordering::SeqCst);

        // 发送一个 offline 状态（best-effort）
        if let Some(tx) = &self.outgoing_tx {
            let _ = tx.send(OutgoingMessage::NodeStatus(NodeStatus {
                node_id: String::new(),
                state: NodeState::Offline,
                available_models: vec![],
                upstream_models: HashMap::new(),
                current_concurrency: 0,
                max_concurrency: 0,
                p2p_public_key: None,
            }));
        }

        if let Some(h) = self.handle.take() {
            h.abort();
            let _ = h.await;
        }
        if let Some(h) = self.refresh_handle.take() {
            h.abort();
            let _ = h.await;
        }
        self.outgoing_tx = None;
        log::info!("✓ daemon stop: stopped successfully");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccswitch::proxy_client::CcSwitchProxyClient;
    use crate::share::executor::NullExecutor;

    fn create_test_db() -> Arc<ShareDb> {
        Arc::new(ShareDb::memory().expect("创建内存数据库失败"))
    }

    fn create_test_registry() -> Arc<ProviderRegistry> {
        ProviderRegistry::new(CcSwitchProxyClient::new("http://127.0.0.1:1"))
    }

    fn create_test_daemon() -> Daemon {
        let db = create_test_db();
        let key_manager = Arc::new(P2PKeyManager::generate());
        let conn_manager = Arc::new(P2PConnectionManager::new(key_manager.clone(), 15731));
        Daemon::new(db, Arc::new(NullExecutor), "default".into(), create_test_registry(), key_manager, conn_manager)
    }

    #[test]
    fn test_daemon_new_idle() {
        let d = create_test_daemon();
        assert!(!d.is_running());
        assert!(d.outgoing_tx.is_none());
    }

    #[tokio::test]
    async fn test_daemon_double_start_errors() {
        let mut d = create_test_daemon();
        // 配置一个明显不可达的 URL — daemon 启动后立刻进入 Reconnecting 循环
        let cfg = ClientConfig {
            server_host: "127.0.0.1:1".into(),
            heartbeat_interval_secs: 30,
            max_reconnect_interval_secs: 60,
            auth_token: "".into(),
            node_id: "n1".into(),
            hmac_secret: String::new(),
            use_https: false,
        };
        let status = NodeStatus {
            node_id: "n1".into(),
            state: NodeState::Idle,
            available_models: vec!["claude-sonnet-4".into()],
            upstream_models: HashMap::new(),
            current_concurrency: 0,
            max_concurrency: 1,
            p2p_public_key: None,
        };
        d.start(cfg.clone(), status.clone(), |_| {}).unwrap();
        let err = d.start(cfg, status, |_| {}).unwrap_err();
        assert!(matches!(err, ShareError::Connection(_)));
        d.stop().await;
    }
}
