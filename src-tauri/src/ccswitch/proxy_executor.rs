//! `ProxyExecutor` — forwards LLM tasks to cc-switch's local proxy.
//!
//! Supports both non-streaming (single JSON response) and streaming (SSE)
//! forwarding. For streaming, cc-switch returns SSE events which are parsed
//! and emitted as `StreamEvent::Delta` / `StreamEvent::Usage` / `StreamEvent::End`.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::ccswitch::{ApiFormat, CcSwitchProxyClient, ProxyError};
use crate::share::executor::{
    ExecuteError, ExecuteRequest, ExecuteResponse, StreamReader, StreamEvent, TaskExecutor,
};
use crate::share::protocol::TokenUsage;

/// TaskExecutor impl that forwards to the cc-switch local proxy.
pub struct ProxyExecutor {
    client: CcSwitchProxyClient,
}

impl ProxyExecutor {
    pub fn new(client: CcSwitchProxyClient) -> Arc<Self> {
        Arc::new(Self { client })
    }

    /// Pick the API format for a request. In v2 we don't pin a specific
    /// cc-switch provider — we infer the format from the model name, since
    /// the cloud dispatches by model. Claude models → Anthropic, Gemini →
    /// GeminiNative, everything else → OpenAI Chat.
    fn infer_format(model: &str) -> ApiFormat {
        let m = model.to_ascii_lowercase();
        if m.starts_with("claude") {
            ApiFormat::Anthropic
        } else if m.starts_with("gemini") {
            ApiFormat::GeminiNative
        } else {
            ApiFormat::OpenAiChat
        }
    }

    /// Build the upstream path + request body for a given format.
    fn build_request(format: ApiFormat, req: &ExecuteRequest) -> (String, Value) {
        match format {
            ApiFormat::Anthropic => {
                let mut body = json!({
                    "model": req.model,
                    "messages": req.messages,
                    "max_tokens": 1024,
                    "stream": req.stream,
                });
                if let Value::Object(map) = &mut body {
                    if let Value::Object(p) = &req.params {
                        for (k, v) in p {
                            map.insert(k.clone(), v.clone());
                        }
                    }
                }
                ("/v1/messages".to_string(), body)
            }
            ApiFormat::OpenAiChat => {
                let mut body = json!({
                    "model": req.model,
                    "messages": req.messages,
                    "stream": req.stream,
                });
                if req.stream {
                    body["stream_options"] = json!({"include_usage": true});
                }
                if let Value::Object(map) = &mut body {
                    if let Value::Object(p) = &req.params {
                        for (k, v) in p {
                            map.insert(k.clone(), v.clone());
                        }
                    }
                }
                ("/v1/chat/completions".to_string(), body)
            }
            ApiFormat::OpenAiResponses => {
                let body = json!({
                    "model": req.model,
                    "input": req.messages,
                    "stream": req.stream,
                });
                ("/v1/responses".to_string(), body)
            }
            ApiFormat::GeminiNative => {
                let body = json!({
                    "contents": req.messages,
                });
                let path = format!("/v1beta/{}:generateContent", req.model);
                (path, body)
            }
        }
    }

    /// Extract textual content + usage from a non-streaming upstream response.
    fn parse_response(format: ApiFormat, body: &Value) -> (String, Option<TokenUsage>) {
        let content = match format {
            ApiFormat::Anthropic => Self::extract_anthropic_content(body),
            ApiFormat::OpenAiChat | ApiFormat::OpenAiResponses => body
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string(),
            ApiFormat::GeminiNative => body
                .get("candidates")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.get(0))
                .and_then(|p| p.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string(),
        };

        let usage = Self::parse_usage(format, body);
        (content, usage)
    }

    /// Extract text from an Anthropic-format response.
    ///
    /// Handles all content block types:
    /// - `text`: `{"type":"text","text":"..."}`
    /// - `thinking`: `{"type":"thinking","thinking":"..."}`
    /// - `tool_use`/`tool_result` and others: extract whatever string field exists
    fn extract_anthropic_content(body: &Value) -> String {
        if let Some(c) = body.get("content") {
            if let Some(s) = c.as_str() {
                return s.to_string();
            }
            if let Some(arr) = c.as_array() {
                let mut out = String::new();
                for block in arr {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    // Each block type uses a different field name for its text content.
                    let text = match block_type {
                        "text" => block.get("text").and_then(|v| v.as_str()),
                        "thinking" => block.get("thinking").and_then(|v| v.as_str()),
                        // Fallback: try common text-carrying fields regardless of type.
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
                if !out.is_empty() {
                    return out;
                }
            }
        }
        // Fallback: some proxies return OpenAI-shaped responses.
        body.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn parse_usage(format: ApiFormat, body: &Value) -> Option<TokenUsage> {
        match format {
            ApiFormat::Anthropic => {
                let u = body.get("usage")?;
                Some(TokenUsage {
                    prompt_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    completion_tokens: u
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    total_tokens: 0,
                })
            }
            ApiFormat::OpenAiChat | ApiFormat::OpenAiResponses => {
                let u = body.get("usage")?;
                let prompt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let completion = u
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                Some(TokenUsage {
                    prompt_tokens: prompt,
                    completion_tokens: completion,
                    total_tokens: prompt + completion,
                })
            }
            ApiFormat::GeminiNative => {
                let m = body.get("usageMetadata")?;
                Some(TokenUsage {
                    prompt_tokens: m
                        .get("promptTokenCount")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    completion_tokens: m
                        .get("candidatesTokenCount")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    total_tokens: m
                        .get("totalTokenCount")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                })
            }
        }
    }

    /// Parse SSE lines from a byte chunk and emit StreamEvents.
    /// Handles both Anthropic and OpenAI SSE formats.
    /// Returns true if a terminal event was seen (message_stop or [DONE]).
    fn parse_sse_chunk(
        format: ApiFormat,
        line: &str,
        tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
        usage_buffer: &mut Option<TokenUsage>,
    ) -> bool {
        let data = match line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
            Some(d) => d.trim(),
            None => return false,
        };
        if data == "[DONE]" {
            let _ = tx.send(StreamEvent::End);
            return true;
        }
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return false,
        };

        match format {
            ApiFormat::Anthropic => Self::parse_anthropic_sse_event(&v, tx, usage_buffer),
            ApiFormat::OpenAiChat | ApiFormat::OpenAiResponses => {
                Self::parse_openai_sse_event(&v, tx, usage_buffer)
            }
            ApiFormat::GeminiNative => {
                // Gemini streaming is not yet implemented; treat as non-streaming.
                false
            }
        }
    }

    /// Parse an Anthropic SSE event (content_block_delta, message_delta, message_stop).
    /// Returns true if terminal.
    fn parse_anthropic_sse_event(
        v: &Value,
        tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
        usage_buffer: &mut Option<TokenUsage>,
    ) -> bool {
        let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "content_block_delta" => {
                // Extract delta text from any delta type.
                if let Some(delta) = v.get("delta") {
                    let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    let text = match delta_type {
                        "text_delta" => delta.get("text").and_then(|t| t.as_str()),
                        "thinking_delta" => delta.get("thinking").and_then(|t| t.as_str()),
                        // Fallback: try common text fields.
                        _ => delta
                            .get("text")
                            .or_else(|| delta.get("thinking"))
                            .and_then(|v| v.as_str()),
                    };
                    if let Some(t) = text {
                        if !t.is_empty() {
                            let _ = tx.send(StreamEvent::Delta(t.to_string()));
                        }
                    }
                }
                false
            }
            "message_delta" => {
                // Final metadata: stop_reason and output token usage.
                if let Some(u) = v.get("usage") {
                    *usage_buffer = Some(TokenUsage {
                        prompt_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        completion_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        total_tokens: 0,
                    });
                }
                // If we have input_tokens from message_start, merge them in.
                false
            }
            "message_start" => {
                // Collect input_tokens from initial message.
                if let Some(msg) = v.get("message") {
                    if let Some(u) = msg.get("usage") {
                        let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        // Merge into usage_buffer if we already have output_tokens, or store for later.
                        if let Some(ref mut buf) = usage_buffer {
                            buf.prompt_tokens = input;
                        } else {
                            *usage_buffer = Some(TokenUsage {
                                prompt_tokens: input,
                                completion_tokens: 0,
                                total_tokens: 0,
                            });
                        }
                    }
                }
                false
            }
            "message_stop" => {
                // Terminal event. Emit buffered usage then End.
                if let Some(u) = usage_buffer.take() {
                    let _ = tx.send(StreamEvent::Usage(u));
                }
                let _ = tx.send(StreamEvent::End);
                true
            }
            // content_block_start, content_block_stop, ping — ignore.
            _ => false,
        }
    }

    /// Parse an OpenAI SSE event (chat.completion.chunk).
    /// Returns true if terminal.
    fn parse_openai_sse_event(
        v: &Value,
        tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
        usage_buffer: &mut Option<TokenUsage>,
    ) -> bool {
        let choices = match v.get("choices") {
            Some(c) => c,
            None => {
                // Possibly a usage-only chunk (when stream_options.include_usage is true).
                if let Some(u) = v.get("usage") {
                    let prompt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let completion = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    *usage_buffer = Some(TokenUsage {
                        prompt_tokens: prompt,
                        completion_tokens: completion,
                        total_tokens: prompt + completion,
                    });
                }
                return false;
            }
        };

        if let Some(choice) = choices.get(0) {
            // Extract delta content.
            if let Some(delta) = choice.get("delta") {
                if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                    if !content.is_empty() {
                        let _ = tx.send(StreamEvent::Delta(content.to_string()));
                    }
                }
            }
            // Check for finish_reason.
            if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                if reason != "null" && !reason.is_empty() {
                    // Terminal chunk.
                    if let Some(u) = usage_buffer.take() {
                        let _ = tx.send(StreamEvent::Usage(u));
                    }
                    // OpenAI may send usage in the final chunk or a separate usage chunk.
                    // Check if usage is present in this chunk.
                    if let Some(u) = v.get("usage") {
                        let prompt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let completion = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        if prompt > 0 || completion > 0 {
                            let _ = tx.send(StreamEvent::Usage(TokenUsage {
                                prompt_tokens: prompt,
                                completion_tokens: completion,
                                total_tokens: prompt + completion,
                            }));
                        }
                    }
                    let _ = tx.send(StreamEvent::End);
                    return true;
                }
            }
        }

        // Check for usage-only chunk (empty choices array with usage).
        if let Some(u) = v.get("usage") {
            if u.get("prompt_tokens").is_some() || u.get("completion_tokens").is_some() {
                let prompt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let completion = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if prompt > 0 || completion > 0 {
                    *usage_buffer = Some(TokenUsage {
                        prompt_tokens: prompt,
                        completion_tokens: completion,
                        total_tokens: prompt + completion,
                    });
                }
            }
        }

        false
    }
}

#[async_trait]
impl TaskExecutor for ProxyExecutor {
    async fn execute(&self, req: ExecuteRequest) -> Result<ExecuteResponse, ExecuteError> {
        let format = Self::infer_format(&req.model);
        let (path, body) = Self::build_request(format, &req);
        let url = format!("{}{}", self.client.base_url(), path);
        log::info!(
            "proxy_exec: POST {} model={} format={:?} stream={}",
            url, req.model, format, req.stream
        );

        let resp = self
            .client
            .http()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                log::error!("proxy_exec: POST {} network error: {}", url, e);
                ExecuteError::Network(format!("cc-switch proxy: {e}"))
            })?;

        let status = resp.status();
        log::info!("proxy_exec: POST {} response status={}", url, status.as_u16());
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            log::warn!(
                "proxy_exec: error response {} body={}",
                status.as_u16(),
                truncate_str(&body_text, 500)
            );
            return Err(ExecuteError::Upstream {
                status: status.as_u16(),
                body: body_text,
            });
        }

        // If the request was for streaming, but we're in the non-streaming execute path,
        // we still get here (e.g., from the default execute_stream impl). Parse the
        // full response as JSON regardless — cc-switch may or may not return SSE.
        let json: Value = resp
            .json()
            .await
            .map_err(|e| {
                log::error!("proxy_exec: failed to parse response JSON for model={}: {}", req.model, e);
                ExecuteError::Other(format!("parse response: {e}"))
            })?;

        let (content, usage) = Self::parse_response(format, &json);
        if content.is_empty() {
            log::warn!(
                "proxy_exec: empty content from cc-switch model={} format={:?} body={}",
                req.model,
                format,
                truncate_str(&json.to_string(), 1000)
            );
        }
        log::info!(
            "proxy_exec: success model={} content_len={} usage={:?}",
            req.model,
            content.len(),
            usage
        );
        Ok(ExecuteResponse { content, usage })
    }

    async fn execute_stream(&self, req: ExecuteRequest) -> Result<StreamReader, ExecuteError> {
        let format = Self::infer_format(&req.model);
        let (path, body) = Self::build_request(format, &req);
        let url = format!("{}{}", self.client.base_url(), path);
        log::info!(
            "proxy_exec_stream: POST {} model={} format={:?}",
            url, req.model, format
        );

        // Use the streaming-capable HTTP client (no total timeout).
        let resp = self
            .client
            .http_streaming()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                log::error!("proxy_exec_stream: POST {} network error: {}", url, e);
                ExecuteError::Network(format!("cc-switch proxy: {e}"))
            })?;

        let status = resp.status();
        log::info!("proxy_exec_stream: POST {} response status={}", url, status.as_u16());
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            log::warn!(
                "proxy_exec_stream: error response {} body={}",
                status.as_u16(),
                truncate_str(&body_text, 500)
            );
            return Err(ExecuteError::Upstream {
                status: status.as_u16(),
                body: body_text,
            });
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let format_clone = format;

        // Spawn a task to read SSE stream and send events through the channel.
        tokio::spawn(async move {
            let mut sse_buffer = String::new();
            let mut usage_buffer: Option<TokenUsage> = None;
            let mut stream = resp.bytes_stream();

            while let Some(chunk_result) = stream.next().await {
                let bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        log::error!("proxy_exec_stream: error reading stream: {}", e);
                        let _ = tx.send(StreamEvent::End);
                        return;
                    }
                };
                sse_buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Process complete SSE events (delimited by \n\n).
                while let Some(pos) = sse_buffer.find("\n\n") {
                    let event_block = sse_buffer[..pos].to_string();
                    sse_buffer = sse_buffer[pos + 2..].to_string();

                    for line in event_block.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with(':') {
                            continue;
                        }
                        let terminal = Self::parse_sse_chunk(format_clone, trimmed, &tx, &mut usage_buffer);
                        if terminal {
                            return;
                        }
                    }
                }
            }

            // Stream ended without a terminal event — send End.
            if let Some(u) = usage_buffer.take() {
                let _ = tx.send(StreamEvent::Usage(u));
            }
            let _ = tx.send(StreamEvent::End);
        });

        Ok(rx)
    }
}

impl From<ProxyError> for ExecuteError {
    fn from(e: ProxyError) -> Self {
        match e {
            ProxyError::Unreachable(m) => ExecuteError::Network(format!("cc-switch proxy unreachable: {m}")),
            ProxyError::HttpStatus(s, b) => ExecuteError::Upstream { status: s, body: b },
            ProxyError::Parse(m) => ExecuteError::Other(format!("parse: {m}")),
        }
    }
}

/// Truncate a string for logging, avoiding giant log lines.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}… ({}B total)", &s[..max_len], s.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_format_by_model() {
        assert_eq!(ProxyExecutor::infer_format("claude-3-5-sonnet"), ApiFormat::Anthropic);
        assert_eq!(ProxyExecutor::infer_format("Claude-Opus"), ApiFormat::Anthropic);
        assert_eq!(ProxyExecutor::infer_format("gemini-1.5-pro"), ApiFormat::GeminiNative);
        assert_eq!(ProxyExecutor::infer_format("gpt-4o"), ApiFormat::OpenAiChat);
        assert_eq!(ProxyExecutor::infer_format("deepseek-chat"), ApiFormat::OpenAiChat);
    }

    #[test]
    fn build_request_anthropic() {
        let req = ExecuteRequest {
            provider_id: String::new(),
            model: "claude-3-5-sonnet".into(),
            messages: json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: json!({"temperature": 0.7}),
        };
        let (path, body) = ProxyExecutor::build_request(ApiFormat::Anthropic, &req);
        assert_eq!(path, "/v1/messages");
        assert_eq!(body["model"], "claude-3-5-sonnet");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["temperature"], 0.7);
    }

    #[test]
    fn build_request_openai_chat() {
        let req = ExecuteRequest {
            provider_id: String::new(),
            model: "gpt-4o".into(),
            messages: json!([]),
            stream: true,
            params: Value::Null,
        };
        let (path, body) = ProxyExecutor::build_request(ApiFormat::OpenAiChat, &req);
        assert_eq!(path, "/v1/chat/completions");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn build_request_gemini() {
        let req = ExecuteRequest {
            provider_id: String::new(),
            model: "gemini-1.5-pro".into(),
            messages: json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: Value::Null,
        };
        let (path, body) = ProxyExecutor::build_request(ApiFormat::GeminiNative, &req);
        assert_eq!(path, "/v1beta/gemini-1.5-pro:generateContent");
        assert!(body.get("contents").is_some());
    }

    #[test]
    fn parse_response_openai_chat() {
        let body = json!({
            "choices": [{"message": {"content": "hello world"}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        });
        let (content, usage) = ProxyExecutor::parse_response(ApiFormat::OpenAiChat, &body);
        assert_eq!(content, "hello world");
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 5);
        assert_eq!(u.completion_tokens, 2);
        assert_eq!(u.total_tokens, 7);
    }

    #[test]
    fn parse_response_anthropic() {
        let body = json!({
            "content": [{"type": "text", "text": "hi there"}],
            "usage": {"input_tokens": 3, "output_tokens": 2}
        });
        let (content, usage) = ProxyExecutor::parse_response(ApiFormat::Anthropic, &body);
        assert_eq!(content, "hi there");
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 3);
        assert_eq!(u.completion_tokens, 2);
    }

    #[test]
    fn parse_response_gemini() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "gem hi"}]}
            }],
            "usageMetadata": {"promptTokenCount": 4, "candidatesTokenCount": 1, "totalTokenCount": 5}
        });
        let (content, usage) = ProxyExecutor::parse_response(ApiFormat::GeminiNative, &body);
        assert_eq!(content, "gem hi");
        let u = usage.unwrap();
        assert_eq!(u.total_tokens, 5);
    }

    #[tokio::test]
    async fn proxy_executor_unreachable_returns_network_error() {
        let client = CcSwitchProxyClient::new("http://127.0.0.1:1");
        let exec = ProxyExecutor::new(client);
        let req = ExecuteRequest {
            provider_id: String::new(),
            model: "claude-3-5-sonnet".into(),
            messages: json!([{"role":"user","content":"hi"}]),
            stream: false,
            params: Value::Null,
        };
        let err = exec.execute(req).await.unwrap_err();
        assert!(matches!(err, ExecuteError::Network(_)), "got {:?}", err);
    }

    #[test]
    fn parse_anthropic_sse_text_delta() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
        let mut usage = None;
        let v: Value = serde_json::from_str(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#).unwrap();
        let terminal = ProxyExecutor::parse_anthropic_sse_event(&v, &tx, &mut usage);
        assert!(!terminal);
        assert!(usage.is_none());
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, StreamEvent::Delta(ref s) if s == "Hello"));
    }

    #[test]
    fn parse_anthropic_sse_thinking_delta() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
        let mut usage = None;
        let v: Value = serde_json::from_str(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#).unwrap();
        let terminal = ProxyExecutor::parse_anthropic_sse_event(&v, &tx, &mut usage);
        assert!(!terminal);
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, StreamEvent::Delta(ref s) if s == "Let me think..."));
    }

    #[test]
    fn parse_response_anthropic_thinking_block() {
        // Response with only a thinking block (e.g. GLM models via cc-switch)
        let body = json!({
            "content": [{"type": "thinking", "thinking": "Let me analyze this"}],
            "usage": {"input_tokens": 6, "output_tokens": 10}
        });
        let (content, usage) = ProxyExecutor::parse_response(ApiFormat::Anthropic, &body);
        assert_eq!(content, "Let me analyze this");
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 6);
        assert_eq!(u.completion_tokens, 10);
    }

    #[test]
    fn parse_response_anthropic_mixed_blocks() {
        // Response with both thinking and text blocks
        let body = json!({
            "content": [
                {"type": "thinking", "thinking": "Hmm..."},
                {"type": "text", "text": "Hello world"}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });
        let (content, usage) = ProxyExecutor::parse_response(ApiFormat::Anthropic, &body);
        assert_eq!(content, "Hmm...\nHello world");
        assert!(usage.is_some());
    }

    #[test]
    fn parse_response_anthropic_string_content() {
        // Fallback: content is a plain string
        let body = json!({
            "content": "plain text response",
            "usage": {"input_tokens": 5, "output_tokens": 3}
        });
        let (content, _) = ProxyExecutor::parse_response(ApiFormat::Anthropic, &body);
        assert_eq!(content, "plain text response");
    }

    #[test]
    fn parse_openai_sse_delta() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
        let mut usage = None;
        let v: Value = serde_json::from_str(r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":0,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#).unwrap();
        let terminal = ProxyExecutor::parse_openai_sse_event(&v, &tx, &mut usage);
        assert!(!terminal);
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, StreamEvent::Delta(ref s) if s == "Hi"));
    }
}