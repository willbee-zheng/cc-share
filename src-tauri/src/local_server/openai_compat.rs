//! OpenAI-compatible request/response translation.
//!
//! Translates between OpenAI Chat Completions wire format and the SharePlan
//! cloud-server dispatch protocol.

use std::sync::Arc;
use std::time::Duration;

use async_stream::stream;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::LocalServerState;
use crate::share::signing::{build_headers, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP};

/// OpenAI `GET /v1/models` response shape.
#[derive(Serialize)]
pub struct ModelsResponse {
    pub object: &'static str,
    pub data: Vec<ModelInfo>,
}

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: &'static str,
    pub owned_by: &'static str,
}

/// OpenAI `POST /v1/chat/completions` request shape (subset).
#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Value,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// `GET /v1/models` — return a small static catalog. Real availability is
/// whatever the cloud has; clients typically just probe here.
pub async fn list_models(State(_state): State<Arc<LocalServerState>>) -> Json<ModelsResponse> {
    let data = [
        "claude-sonnet-4",
        "claude-opus-4",
        "claude-haiku-4",
        "gpt-4o",
        "gpt-4o-mini",
        "gemini-1.5-pro",
        "gemini-1.5-flash",
    ]
    .iter()
    .map(|id| ModelInfo {
        id: id.to_string(),
        object: "model",
        owned_by: "shareplan",
    })
    .collect();
    Json(ModelsResponse {
        object: "list",
        data,
    })
}

/// `POST /v1/chat/completions` — forward to cloud `/dispatch`.
pub async fn chat_completions(
    State(state): State<Arc<LocalServerState>>,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    let cloud_base = state.cloud_base_url.read().await.clone();
    let config_auth_token = state.auth_token.read().await.clone();
    let hmac_secret = state.hmac_secret.read().await.clone();

    if cloud_base.is_empty() {
        log::warn!("local_server: chat_completions rejected — cloud server URL not configured");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": {"message": "cloud server URL not configured", "type": "server_error"}})),
        )
            .into_response();
    }

    // When config auth_token is empty, fall back to the logged-in user's
    // access_token from AuthState. This ensures the local server works
    // correctly after browser/email login without requiring a separate API key.
    let auth_token = if config_auth_token.is_empty() {
        crate::auth::token::load_auth_state(&state.db)
            .ok()
            .flatten()
            .map(|s| s.access_token)
            .unwrap_or_default()
    } else {
        config_auth_token
    };

    let dispatch_url = format!("{}/api/v1/dispatch", cloud_base.trim_end_matches('/'));
    let est_prompt = estimate_tokens(&req.messages);
    let body = json!({
        "model": req.model,
        "messages": req.messages,
        "stream": req.stream,
        "params": build_params(&req),
        "est_prompt_tokens": est_prompt,
        "max_output_tokens": req.max_tokens.unwrap_or(1024),
    });
    let body_bytes = match serde_json::to_vec(&body) {
        Ok(b) => b,
        Err(e) => {
            log::error!("local_server: failed to encode dispatch body for model={}: {}", req.model, e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"message": format!("encode: {e}"), "type": "server_error"}})),
            )
                .into_response();
        }
    };

    log::info!(
        "local_server: POST {} model={} stream={} est_prompt={} body_size={}B",
        dispatch_url, req.model, req.stream, est_prompt, body_bytes.len()
    );

    let mut request = state
        .http
        .post(&dispatch_url)
        .header(header::CONTENT_TYPE, "application/json")
        .timeout(Duration::from_secs(300))
        .body(body_bytes.clone());

    if !auth_token.is_empty() {
        request = request.header(header::AUTHORIZATION, format!("Bearer {auth_token}"));
    }
    if !hmac_secret.is_empty() {
        let now = chrono::Utc::now().timestamp();
        let nonce = uuid::Uuid::new_v4().to_string();
        let (ts, nonce, sig) = build_headers(
            hmac_secret.as_bytes(),
            "POST",
            "/api/v1/dispatch",
            &body_bytes,
            now,
            &nonce,
        );
        request = request.header(HEADER_TIMESTAMP, ts.to_string());
        request = request.header(HEADER_NONCE, nonce);
        request = request.header(HEADER_SIGNATURE, sig);
    }

    let resp = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            log::error!("local_server: cloud request failed for model={}: {}", req.model, e);
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": format!("cloud unreachable: {e}"), "type": "server_error"}})),
            )
                .into_response();
        }
    };

    let status = resp.status();
    log::info!("local_server: cloud response status={} model={}", status.as_u16(), req.model);
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        log::warn!(
            "local_server: error response {} model={} body={}",
            status.as_u16(),
            req.model,
            truncate_str(&body_text, 500)
        );
        let user_message = if body_text.trim().starts_with('<') {
            format!(
                "cloud server returned {} — the upstream service may be down or misconfigured",
                status
            )
        } else {
            body_text.clone()
        };
        return (
            map_status(status),
            Json(json!({"error": {"message": user_message, "type": "upstream_error"}})),
        )
            .into_response();
    }

    if req.stream {
        stream_openai_sse(resp, req.model).await
    } else {
        non_stream_openai_json(resp, req.model).await
    }
}

/// Normalize `content` field from cloud dispatch response to a flat string.
///
/// Cloud may return `content` as a plain string or an array of blocks like:
/// - `{"type":"text","text":"hello"}`
/// - `{"type":"thinking","thinking":"..."}`
pub(super) fn extract_content_string(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let mut out = String::new();
        for block in arr {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let text = match block_type {
                "text" => block.get("text").and_then(|v| v.as_str()),
                "thinking" => block.get("thinking").and_then(|v| v.as_str()),
                _ => block
                    .get("text")
                    .or_else(|| block.get("thinking"))
                    .and_then(|v| v.as_str()),
            };
            if let Some(t) = text {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
        return out;
    }
    String::new()
}

/// Translate cloud non-streaming dispatch response → OpenAI Chat Completion.
async fn non_stream_openai_json(resp: reqwest::Response, model: String) -> Response {
    let cloud: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("local_server: failed to parse cloud response JSON for model={}: {}", model, e);
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": format!("parse: {e}"), "type": "server_error"}})),
            )
                .into_response();
        }
    };

    let content = cloud
        .get("content")
        .map(|c| extract_content_string(c))
        .unwrap_or_default();
    let usage = cloud.get("usage").cloned();

    let completion = json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
        "usage": usage.unwrap_or_else(|| json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0})),
    });
    log::info!(
        "local_server: non-stream response model={} content_len={}",
        model, content.len()
    );
    Json(completion).into_response()
}

/// Translate cloud SSE (task_result frames) → OpenAI SSE (chat.completion.chunk).
///
/// Uses an async buffer to correctly handle SSE events that span multiple
/// TCP chunks. Cloud sends `event: task_result\ndata: {...}\n\n`; we
/// accumulate until we see `\n\n` then parse each complete event.
async fn stream_openai_sse(resp: reqwest::Response, model: String) -> Response {
    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());

    let stream = async_stream::stream! {
        let mut buffer = String::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    log::error!("local_server: stream read error for model={}: {}", model, e);
                    break;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Process complete SSE events (delimited by \n\n).
            while let Some(pos) = buffer.find("\n\n") {
                let event_block = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                for line in event_block.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with(':') {
                        continue;
                    }
                    let payload = match trimmed.strip_prefix("data: ").or_else(|| trimmed.strip_prefix("data:")) {
                        Some(s) => s.trim(),
                        None => continue,
                    };
                    if payload == "[DONE]" {
                        yield Ok::<_, std::io::Error>(format!("data: [DONE]\n\n"));
                        continue;
                    }
                    let v: Value = match serde_json::from_str(payload) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                    let content = v.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    let final_flag = v.get("final").and_then(|f| f.as_bool()).unwrap_or(false);
                    let usage = v.get("usage").cloned();

                    // Map failed/rejected/busy to an error chunk before the final stop.
                    if status == "failed" || status == "rejected" || status == "busy" {
                        let err_msg = v.get("error").and_then(|e| e.as_str()).unwrap_or(status);
                        yield Ok(format!("data: {}\n\n", json!({
                            "id": completion_id,
                            "object": "chat.completion.chunk",
                            "created": 0,
                            "model": model,
                            "choices": [{
                                "index": 0,
                                "delta": {"content": ""},
                                "finish_reason": "stop",
                            }],
                            "error": {"message": err_msg, "type": "upstream_error"},
                        })));
                        continue;
                    }

                    let delta = if status == "completed" {
                        json!({})
                    } else {
                        json!({"content": content})
                    };

                    let mut chunk = json!({
                        "id": completion_id,
                        "object": "chat.completion.chunk",
                        "created": 0,
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "delta": delta,
                            "finish_reason": if status == "completed" || final_flag { json!("stop") } else { json!(null) },
                        }],
                    });
                    if let Some(u) = usage {
                        chunk["usage"] = u;
                    }
                    yield Ok(format!("data: {}\n\n", chunk));
                }
            }
        }

        // If buffer has remaining data, try to parse it (incomplete event at stream end).
        if !buffer.trim().is_empty() {
            for line in buffer.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with(':') {
                    continue;
                }
                let payload = match trimmed.strip_prefix("data: ").or_else(|| trimmed.strip_prefix("data:")) {
                    Some(s) => s.trim(),
                    None => continue,
                };
                if payload == "[DONE]" {
                    yield Ok(format!("data: [DONE]\n\n"));
                    continue;
                }
                let v: Value = match serde_json::from_str(payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let content = v.get("content").and_then(|c| c.as_str()).unwrap_or("");
                let final_flag = v.get("final").and_then(|f| f.as_bool()).unwrap_or(false);
                let usage = v.get("usage").cloned();

                let delta = if status == "completed" { json!({}) } else { json!({"content": content}) };
                let mut chunk = json!({
                    "id": completion_id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": delta,
                        "finish_reason": if status == "completed" || final_flag { json!("stop") } else { json!(null) },
                    }],
                });
                if let Some(u) = usage {
                    chunk["usage"] = u;
                }
                yield Ok(format!("data: {}\n\n", chunk));
            }
        }
    };

    let body = Body::from_stream(stream);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::CONNECTION, "keep-alive"),
        ],
        body,
    )
        .into_response()
}

pub(super) fn build_params(req: &ChatCompletionRequest) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(t) = req.temperature {
        params.insert("temperature".into(), json!(t));
    }
    if let Some(m) = req.max_tokens {
        params.insert("max_tokens".into(), json!(m));
    }
    Value::Object(params)
}

pub(super) fn estimate_tokens(messages: &Value) -> u32 {
    // 4 chars/token heuristic, matching cloud-server's estimate.
    let s = messages.to_string();
    (s.len() as u32) / 4
}

pub(super) fn map_status(s: reqwest::StatusCode) -> StatusCode {
    match s.as_u16() {
        503 => StatusCode::SERVICE_UNAVAILABLE,
        504 => StatusCode::GATEWAY_TIMEOUT,
        402 => StatusCode::PAYMENT_REQUIRED,
        _ => StatusCode::BAD_GATEWAY,
    }
}

/// Truncate a string for logging, avoiding giant log lines.
pub(super) fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}… ({}B total)", &s[..max_len], s.len())
    }
}
