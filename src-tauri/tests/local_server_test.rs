//! Integration test: start the local OpenAI-compatible server, verify
//! /v1/models and a non-streaming /v1/chat/completions round-trip against
//! a mock cloud dispatch endpoint.

use shareplan_lib::local_server::{router, LocalServerState};
use axum::response::IntoResponse;
use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn start_mock_cloud() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/api/v1/dispatch",
        post(|Json(req): Json<Value>| async move {
            if req["stream"].as_bool() == Some(true) {
                // SSE: 2 deltas + final
                let body = concat!(
                    "event: task_result\n",
                    "data: {\"task_id\":\"t1\",\"status\":\"running\",\"content\":\"Hel\",\"sequence\":1,\"final\":false}\n\n",
                    "event: task_result\n",
                    "data: {\"task_id\":\"t1\",\"status\":\"running\",\"content\":\"lo\",\"sequence\":2,\"final\":false}\n\n",
                    "event: task_result\n",
                    "data: {\"task_id\":\"t1\",\"status\":\"completed\",\"content\":\"\",\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5},\"sequence\":3,\"final\":true}\n\n",
                    "data: [DONE]\n\n",
                );
                (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    body,
                ).into_response()
            } else {
                Json(json!({
                    "node_id": "n1",
                    "task_id": "t1",
                    "content": "hello world",
                    "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7},
                })).into_response()
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn list_models_returns_catalog() {
    let state = LocalServerState::new(String::new(), String::new(), String::new());
    let app = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let resp: Value = reqwest::get(format!("http://{addr}/v1/models")).await.unwrap().json().await.unwrap();
    assert_eq!(resp["object"], "list");
    assert!(resp["data"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn non_stream_completion_translates_openai_shape() {
    let cloud = start_mock_cloud().await;
    let state = LocalServerState::new(cloud, "tok".into(), String::new());
    let app = router(Arc::clone(&state));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp: Value = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&json!({
            "model": "claude-sonnet-4",
            "messages": [{"role":"user","content":"hi"}],
            "stream": false,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["object"], "chat.completion");
    assert_eq!(resp["choices"][0]["message"]["content"], "hello world");
    assert_eq!(resp["choices"][0]["finish_reason"], "stop");
    assert_eq!(resp["usage"]["total_tokens"], 7);
}

#[tokio::test]
async fn stream_completion_translates_sse_chunks() {
    let cloud = start_mock_cloud().await;
    let state = LocalServerState::new(cloud, "tok".into(), String::new());
    let app = router(Arc::clone(&state));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&json!({
            "model": "claude-sonnet-4",
            "messages": [{"role":"user","content":"hi"}],
            "stream": true,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Expect OpenAI-shaped chunks + [DONE]
    assert!(body.contains("\"object\":\"chat.completion.chunk\""), "body: {body}");
    assert!(body.contains("\"content\":\"Hel\""), "body: {body}");
    assert!(body.contains("\"content\":\"lo\""), "body: {body}");
    assert!(body.contains("\"finish_reason\":\"stop\""), "body: {body}");
    assert!(body.contains("data: [DONE]"), "body: {body}");
}
