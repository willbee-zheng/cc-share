//! 集成测试：通过真实的本地 WebSocket 桥接（[`WebBridge`]）验证
//! `dispatch → 扩展执行 → 结果回传` 的完整链路。
//!
//! 不需要外部 cloud-server — 我们用 tokio-tungstenite 起一个 mock 浏览器扩展，
//! 配对 → 上报 idle 状态 → 接收 task → 回 task_result，整条 happy path。

#![cfg(test)]

use cc_share::share::protocol::{TaskPayload, TaskStatus, TokenUsage};
use cc_share::share::web_bridge::{BridgeConfig, WebBridge};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// 用一个随机端口启动 bridge，返回它和它绑定的 ws://127.0.0.1:<port>
async fn start_bridge() -> (Arc<WebBridge>, String) {
    // 随机端口：bind 到 :0 让 OS 分配
    let bind_addr = "127.0.0.1:0".to_string();
    let listener = tokio::net::TcpListener::bind(&bind_addr).await.unwrap();
    let actual = listener.local_addr().unwrap();
    drop(listener);

    let mut bridge = WebBridge::new(BridgeConfig {
        bind_addr: actual.to_string(),
        pairing_token: "test-token".into(),
        task_timeout_secs: 5,
    });
    bridge.start("test-node".into()).await.unwrap();
    // 等待 listen socket 真的 ready
    tokio::time::sleep(Duration::from_millis(50)).await;
    (Arc::new(bridge), format!("ws://{}", actual))
}

async fn connect_mock_extension(
    url: &str,
    token: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>> {
    let (mut ws, _) = connect_async(url).await.expect("ws connect");
    let pair = json!({"type":"pair","token":token,"agent":"mock-ext"});
    ws.send(Message::Text(pair.to_string().into())).await.unwrap();
    // 接收 paired 帧
    match ws.next().await {
        Some(Ok(Message::Text(txt))) => {
            let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
            assert_eq!(v["type"], "paired");
        }
        other => panic!("expected paired frame, got {:?}", other),
    }
    ws
}

#[tokio::test]
async fn test_bridge_dispatch_full_roundtrip() {
    let (bridge, url) = start_bridge().await;
    let mut ws = connect_mock_extension(&url, "test-token").await;

    // 1. mock 扩展上报 idle 状态
    let status = json!({
        "type": "web_status",
        "provider_id": "web:chatgpt",
        "state": "idle"
    });
    ws.send(Message::Text(status.to_string().into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        bridge.provider_state("web:chatgpt").await,
        cc_share::share::web_bridge::WebProviderState::Idle
    );

    // 2. 启动响应循环：从 ws 读一帧 → 用同一 task_id 回 task_result
    let ws_handle = tokio::spawn(async move {
        while let Some(frame) = ws.next().await {
            let txt = match frame {
                Ok(Message::Text(t)) => t.to_string(),
                _ => continue,
            };
            let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
            if v["type"] == "task" {
                let task_id = v["task_id"].as_str().unwrap().to_string();
                let result = json!({
                    "type": "task_result",
                    "task_id": task_id,
                    "status": "completed",
                    "content": "mocked answer",
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
                });
                ws.send(Message::Text(result.to_string().into())).await.unwrap();
                return; // one task is enough for the test
            }
        }
    });

    // 3. 主线程派发任务并等待结果
    let task = TaskPayload {
        task_id: "ignored-overwritten".into(),
        model: "gpt-4o".into(),
        messages: json!([{"role": "user", "content": "hi"}]),
        stream: false,
        params: serde_json::Value::Null,
    };
    let result = bridge.dispatch(task, "web:chatgpt").await.unwrap();
    assert_eq!(result.status, TaskStatus::Completed);
    assert_eq!(result.content, "mocked answer");
    assert_eq!(result.usage, Some(TokenUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
    }));

    let _ = tokio::time::timeout(Duration::from_secs(2), ws_handle).await;
}

#[tokio::test]
async fn test_bridge_rejects_bad_pairing_token() {
    let (_bridge, url) = start_bridge().await;
    let (mut ws, _) = connect_async(&url).await.expect("connect");
    let pair = json!({"type":"pair","token":"WRONG","agent":"mock-ext"});
    ws.send(Message::Text(pair.to_string().into())).await.unwrap();
    // 服务端应该立即关闭：下一帧要么 Close 要么读到 None
    let next = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timeout waiting for close");
    match next {
        None => {} // 正常关闭
        Some(Ok(Message::Close(_))) => {}
        other => panic!("expected close, got {:?}", other),
    }
}

#[tokio::test]
async fn test_bridge_dispatch_busy_provider_short_circuits() {
    let (bridge, url) = start_bridge().await;
    let mut ws = connect_mock_extension(&url, "test-token").await;

    // 上报 busy 状态
    let status = json!({
        "type": "web_status",
        "provider_id": "web:chatgpt",
        "state": "busy"
    });
    ws.send(Message::Text(status.to_string().into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let task = TaskPayload {
        task_id: "t-busy".into(),
        model: "gpt-4o".into(),
        messages: json!([]),
        stream: false,
        params: serde_json::Value::Null,
    };
    let result = bridge.dispatch(task, "web:chatgpt").await.unwrap();
    assert_eq!(result.status, TaskStatus::Busy);
    let _ = ws; // keep alive until end
}
