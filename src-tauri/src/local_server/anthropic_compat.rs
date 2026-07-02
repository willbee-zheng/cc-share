//! Anthropic Messages API-compatible request/response translation.
//!
//! Translates between Anthropic Messages API wire format and the SharePlan
//! cloud-server dispatch protocol. This enables clients that speak the
//! Anthropic Messages API (e.g., Claude Code CLI, Cursor in Anthropic mode)
//! to connect to the local consumer server.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::openai_compat::{estimate_tokens, extract_content_string, map_status, truncate_str};
use super::LocalServerState;
use crate::share::signing::{build_headers, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP};

/// Anthropic `POST /v1/messages` request shape (subset of full spec).
#[derive(Debug, Deserialize)]
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub messages: Value,
    #[serde(default)]
    pub system: Option<Value>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub top_k: Option<u32>,
}

/// `POST /v1/messages` — forward to cloud `/dispatch`, translate response
/// to Anthropic Messages API format.
pub async fn messages(
    State(state): State<Arc<LocalServerState>>,
    Json(req): Json<AnthropicMessagesRequest>,
) -> Response {
    let cloud_base = state.cloud_base_url.read().await.clone();
    let config_auth_token = state.auth_token.read().await.clone();
    let hmac_secret = state.hmac_secret.read().await.clone();

    if cloud_base.is_empty() {
        log::warn!("local_server: anthropic messages rejected — cloud server URL not configured");
        return anthropic_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "cloud server URL not configured",
        );
    }

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
    let params = build_anthropic_params(&req);

    let body = json!({
        "model": req.model,
        "messages": req.messages,
        "stream": req.stream,
        "params": params,
        "est_prompt_tokens": est_prompt,
        "max_output_tokens": req.max_tokens.unwrap_or(4096),
    });
    let body_bytes = match serde_json::to_vec(&body) {
        Ok(b) => b,
        Err(e) => {
            log::error!(
                "local_server: failed to encode dispatch body for model={}: {}",
                req.model,
                e
            );
            return anthropic_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &format!("encode: {e}"),
            );
        }
    };

    log::info!(
        "local_server: POST {} model={} stream={} est_prompt={} body_size={}B [anthropic]",
        dispatch_url,
        req.model,
        req.stream,
        est_prompt,
        body_bytes.len()
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
            log::error!(
                "local_server: cloud request failed for model={}: {}",
                req.model,
                e
            );
            return anthropic_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                &format!("cloud unreachable: {e}"),
            );
        }
    };

    let status = resp.status();
    log::info!(
        "local_server: cloud response status={} model={} [anthropic]",
        status.as_u16(),
        req.model
    );
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        log::warn!(
            "local_server: error response {} model={} body={} [anthropic]",
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
        return anthropic_error(map_status(status), "api_error", &user_message);
    }

    if req.stream {
        stream_anthropic_sse(resp, req.model).await
    } else {
        non_stream_anthropic_json(resp, req.model).await
    }
}

/// Build `params` JSON for Anthropic requests. Includes `system` (Anthropic
/// top-level param), `temperature`, `max_tokens`, `top_p`, `top_k`.
fn build_anthropic_params(req: &AnthropicMessagesRequest) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(ref system) = req.system {
        params.insert("system".into(), system.clone());
    }
    if let Some(t) = req.temperature {
        params.insert("temperature".into(), json!(t));
    }
    if let Some(m) = req.max_tokens {
        params.insert("max_tokens".into(), json!(m));
    }
    if let Some(p) = req.top_p {
        params.insert("top_p".into(), json!(p));
    }
    if let Some(k) = req.top_k {
        params.insert("top_k".into(), json!(k));
    }
    Value::Object(params)
}

/// Translate cloud non-streaming dispatch response → Anthropic Messages format.
async fn non_stream_anthropic_json(resp: reqwest::Response, model: String) -> Response {
    let cloud: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            log::error!(
                "local_server: failed to parse cloud response JSON for model={}: {} [anthropic]",
                model,
                e
            );
            return anthropic_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                &format!("parse: {e}"),
            );
        }
    };

    let content = cloud
        .get("content")
        .map(|c| extract_content_string(c))
        .unwrap_or_default();

    let usage = cloud.get("usage").cloned();

    let response = json!({
        "id": format!("msg_{}", uuid::Uuid::new_v4()),
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": content}],
        "model": model,
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": map_usage_to_anthropic(usage),
    });

    log::info!(
        "local_server: non-stream response model={} content_len={} [anthropic]",
        model,
        content.len()
    );
    Json(response).into_response()
}

/// Translate cloud SSE (task_result frames) → Anthropic SSE events.
///
/// Cloud sends `event: task_result\ndata: {...}\n\n` frames. We translate
/// them into Anthropic SSE events:
/// - First content delta → `message_start` + `content_block_start` + `content_block_delta`
/// - Subsequent deltas → `content_block_delta`
/// - Terminal frame → `content_block_stop` + `message_delta` (with usage) + `message_stop`
async fn stream_anthropic_sse(resp: reqwest::Response, model: String) -> Response {
    let msg_id = format!("msg_{}", uuid::Uuid::new_v4());

    let stream = async_stream::stream! {
        let mut buffer = String::new();
        let mut stream = resp.bytes_stream();
        let mut started = false;
        let block_index: u32 = 0;
        let mut input_tokens: u32 = 0;

        while let Some(chunk_result) = stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    log::error!("local_server: stream read error for model={}: {} [anthropic]", model, e);
                    // Emit error event and stop.
                    yield Ok::<_, std::io::Error>(format!("event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"api_error\",\"message\":\"stream read error\"}}}}}}\n\n"));
                    yield Ok(format!("event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"));
                    break;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

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
                    let v: Value = match serde_json::from_str(payload) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                    let content = v.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    let final_flag = v.get("final").and_then(|f| f.as_bool()).unwrap_or(false);
                    let usage = v.get("usage").cloned();

                    // Error statuses: emit an error event and terminate.
                    if status == "failed" || status == "rejected" || status == "busy" {
                        let _err_msg = v.get("error").and_then(|e| e.as_str()).unwrap_or(status);
                        if started {
                            yield Ok(format!("event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":{block_index}}}\n\n"));
                        }
                        let stop_reason = match status {
                            "busy" => "max_capacity",
                            "rejected" => "refusal",
                            _ => "error",
                        };
                        yield Ok(format!(
                            "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{stop_reason}\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":0}}}}\n\n"
                        ));
                        yield Ok(format!("event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"));
                        break;
                    }

                    if status == "completed" || final_flag {
                        // Terminal event.
                        if !started {
                            // No content was ever received; send minimal message_start.
                            yield Ok(format!("event: message_start\ndata: {}\n\n", json!({
                                "type": "message_start",
                                "message": {
                                    "id": msg_id,
                                    "type": "message",
                                    "role": "assistant",
                                    "content": [],
                                    "model": model,
                                    "stop_reason": null,
                                    "stop_sequence": null,
                                    "usage": {"input_tokens": input_tokens, "output_tokens": 0},
                                },
                            })));
                        }

                        // Close the content block.
                        yield Ok(format!("event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":{block_index}}}\n\n"));

                        // Extract output tokens from usage.
                        let output_tokens = usage.as_ref()
                            .and_then(|u| u.get("completion_tokens").and_then(|v| v.as_u64()))
                            .unwrap_or(0) as u32;
                        // Also try to get input_tokens from final usage if we don't have it.
                        if input_tokens == 0 {
                            input_tokens = usage.as_ref()
                                .and_then(|u| u.get("prompt_tokens").and_then(|v| v.as_u64()))
                                .unwrap_or(0) as u32;
                        }

                        yield Ok(format!(
                            "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":{output_tokens}}}}}\n\n"
                        ));
                        yield Ok(format!("event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"));
                        break;
                    }

                    // Regular content delta.
                    if !content.is_empty() {
                        if !started {
                            // First delta: emit message_start + content_block_start + content_block_delta.
                            started = true;
                            // Track input_tokens if present in this frame.
                            if let Some(ref u) = usage {
                                input_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            }

                            yield Ok(format!("event: message_start\ndata: {}\n\n", json!({
                                "type": "message_start",
                                "message": {
                                    "id": msg_id,
                                    "type": "message",
                                    "role": "assistant",
                                    "content": [],
                                    "model": model,
                                    "stop_reason": null,
                                    "stop_sequence": null,
                                    "usage": {"input_tokens": input_tokens, "output_tokens": 0},
                                },
                            })));
                            yield Ok(format!("event: content_block_start\ndata: {}\n\n", json!({
                                "type": "content_block_start",
                                "index": block_index,
                                "content_block": {"type": "text", "text": ""},
                            })));
                        }
                        yield Ok(format!("event: content_block_delta\ndata: {}\n\n", json!({
                            "type": "content_block_delta",
                            "index": block_index,
                            "delta": {"type": "text_delta", "text": content},
                        })));
                    }
                }
            }
        }

        // If the stream ended without a terminal event, close gracefully.
        if started {
            yield Ok(format!("event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":{block_index}}}\n\n"));
            yield Ok(format!(
                "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":0}}}}\n\n"
            ));
            yield Ok(format!("event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"));
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

/// Build an Anthropic-format error response.
fn anthropic_error(status: StatusCode, error_type: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "type": "error",
            "error": {
                "type": error_type,
                "message": message,
            },
        })),
    )
        .into_response()
}

/// Map OpenAI-style `usage` to Anthropic-style `usage`.
///
/// Input: `{"prompt_tokens": N, "completion_tokens": N, "total_tokens": N}`
/// Output: `{"input_tokens": N, "output_tokens": N}`
fn map_usage_to_anthropic(usage: Option<Value>) -> Value {
    match usage {
        Some(u) => json!({
            "input_tokens": u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            "output_tokens": u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        None => json!({"input_tokens": 0, "output_tokens": 0}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_anthropic_params_with_all_fields() {
        let req = AnthropicMessagesRequest {
            model: "claude-sonnet-4".into(),
            messages: json!([]),
            system: Some(json!("You are helpful.")),
            max_tokens: Some(2048),
            stream: false,
            temperature: Some(0.7),
            top_p: Some(0.9),
            top_k: Some(50),
        };
        let params = build_anthropic_params(&req);
        assert_eq!(params["system"], "You are helpful.");
        assert_eq!(params["temperature"], 0.7);
        assert_eq!(params["max_tokens"], 2048);
        assert_eq!(params["top_p"], 0.9);
        assert_eq!(params["top_k"], 50);
    }

    #[test]
    fn build_anthropic_params_with_minimal_fields() {
        let req = AnthropicMessagesRequest {
            model: "claude-sonnet-4".into(),
            messages: json!([]),
            system: None,
            max_tokens: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
        };
        let params = build_anthropic_params(&req);
        assert!(params.as_object().unwrap().is_empty());
    }

    #[test]
    fn map_usage_to_anthropic_converts_fields() {
        let usage = json!({"prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150});
        let mapped = map_usage_to_anthropic(Some(usage));
        assert_eq!(mapped["input_tokens"], 100);
        assert_eq!(mapped["output_tokens"], 50);
    }

    #[test]
    fn map_usage_to_anthropic_with_none() {
        let mapped = map_usage_to_anthropic(None);
        assert_eq!(mapped["input_tokens"], 0);
        assert_eq!(mapped["output_tokens"], 0);
    }

    #[test]
    fn anthropic_error_format() {
        let response = anthropic_error(StatusCode::BAD_GATEWAY, "api_error", "cloud unreachable");
        // We can't easily inspect the response body in a unit test, but we can
        // verify the function compiles and returns the correct status code.
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn system_as_array_content_blocks() {
        let req = AnthropicMessagesRequest {
            model: "claude-sonnet-4".into(),
            messages: json!([]),
            system: Some(json!([{"type": "text", "text": "Be concise."}])),
            max_tokens: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
        };
        let params = build_anthropic_params(&req);
        assert_eq!(
            params["system"],
            json!([{"type": "text", "text": "Be concise."}])
        );
    }
}