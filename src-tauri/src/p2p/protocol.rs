//! P2P wire protocol for QUIC stream framing.
//!
//! Messages on QUIC streams use a simple length-prefixed binary format:
//!   [4 bytes: message type (u32 big-endian)]
//!   [4 bytes: payload length (u32 big-endian)]
//!   [N bytes: payload (MessagePack encoded)]
//!
//! For encrypted payloads, the payload field contains:
//!   [12 bytes: nonce | ciphertext + tag] (ChaCha20-Poly1305 output from crypto.rs)

use serde::{Deserialize, Serialize};

/// P2P wire message type discriminator.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P2pMessageType {
    /// Consumer → Supplier: task request with encrypted payload
    TaskRequest = 1,
    /// Supplier → Consumer: task result (single or streaming chunk)
    TaskResult = 2,
    /// Bidirectional: keep-alive ping
    Heartbeat = 3,
    /// Bidirectional: session close notification
    SessionClose = 4,
}

impl TryFrom<u32> for P2pMessageType {
    type Error = String;

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(P2pMessageType::TaskRequest),
            2 => Ok(P2pMessageType::TaskResult),
            3 => Ok(P2pMessageType::Heartbeat),
            4 => Ok(P2pMessageType::SessionClose),
            _ => Err(format!("unknown P2P message type: {v}")),
        }
    }
}

/// P2P task request sent from consumer to supplier over QUIC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2pTaskRequest {
    /// Session ID from cloud signaling
    pub session_id: String,
    /// Consumer-generated task ID
    pub task_id: String,
    /// Target model
    pub model: String,
    /// Encrypted messages (ChaCha20-Poly1305 ciphertext)
    pub messages: Vec<u8>,
    /// Whether streaming is requested
    pub stream: bool,
    /// Encrypted params (ChaCha20-Poly1305 ciphertext)
    pub params: Vec<u8>,
    /// Estimated prompt tokens
    pub est_prompt_tokens: u32,
    /// Max output tokens
    pub max_output_tokens: u32,
}

/// P2P task result sent from supplier to consumer over QUIC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2pTaskResult {
    /// Session ID from cloud signaling
    pub session_id: String,
    /// Task ID
    pub task_id: String,
    /// Task status
    pub status: String,
    /// Encrypted content (ChaCha20-Poly1305 ciphertext)
    pub content: Vec<u8>,
    /// Token usage (encrypted within content for terminal frames)
    pub usage: Option<TokenUsageEncrypted>,
    /// Error message (plain text, not encrypted — errors are non-sensitive)
    pub error: Option<String>,
    /// Stream sequence number (for streaming responses)
    pub sequence: Option<u64>,
    /// Whether this is the terminal frame
    pub r#final: Option<bool>,
}

/// Encrypted token usage (embedded inside content for terminal frames).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageEncrypted {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// P2P heartbeat frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2pHeartbeat {
    pub timestamp_ms: u64,
}

/// P2P session close frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2pSessionClose {
    pub session_id: String,
    pub reason: String,
}

/// A framed P2P message on the wire.
#[derive(Debug, Clone)]
pub struct P2pFrame {
    pub msg_type: P2pMessageType,
    pub payload: Vec<u8>,
}

impl P2pFrame {
    /// Encode a frame to bytes: [type:u32be][length:u32be][payload].
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + self.payload.len());
        buf.extend_from_slice(&(self.msg_type as u32).to_be_bytes());
        buf.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Decode a frame from bytes. Returns the frame and the number of bytes consumed.
    /// Returns None if the buffer doesn't contain a complete frame.
    pub fn decode(data: &[u8]) -> Result<Option<(P2pFrame, usize)>, String> {
        if data.len() < 8 {
            return Ok(None); // Need more data
        }

        let msg_type_u32 = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let msg_type = P2pMessageType::try_from(msg_type_u32)?;
        let payload_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;

        if data.len() < 8 + payload_len {
            return Ok(None); // Need more data
        }

        let payload = data[8..8 + payload_len].to_vec();
        let consumed = 8 + payload_len;

        Ok(Some((P2pFrame { msg_type, payload }, consumed)))
    }
}

/// Encode a typed P2P message into a frame using MessagePack.
pub fn encode_message<T: Serialize>(msg_type: P2pMessageType, msg: &T) -> Result<P2pFrame, String> {
    let payload = rmp_serde::to_vec(msg).map_err(|e| format!("msgpack encode: {e}"))?;
    Ok(P2pFrame { msg_type, payload })
}

/// Decode a frame payload as a typed P2P message.
pub fn decode_message<T: for<'de> Deserialize<'de>>(frame: &P2pFrame) -> Result<T, String> {
    rmp_serde::from_slice(&frame.payload).map_err(|e| format!("msgpack decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_encode_decode_roundtrip() {
        let frame = P2pFrame {
            msg_type: P2pMessageType::Heartbeat,
            payload: vec![1, 2, 3, 4, 5],
        };
        let encoded = frame.encode();
        let (decoded, consumed) = P2pFrame::decode(&encoded).unwrap().unwrap();
        assert_eq!(decoded.msg_type, P2pMessageType::Heartbeat);
        assert_eq!(decoded.payload, vec![1, 2, 3, 4, 5]);
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn test_frame_decode_partial() {
        let frame = P2pFrame {
            msg_type: P2pMessageType::TaskRequest,
            payload: vec![0; 100],
        };
        let encoded = frame.encode();

        // Not enough data
        assert!(P2pFrame::decode(&encoded[..4]).unwrap().is_none());
        assert!(P2pFrame::decode(&encoded[..12]).unwrap().is_none());

        // Full data works
        let (decoded, _) = P2pFrame::decode(&encoded).unwrap().unwrap();
        assert_eq!(decoded.msg_type, P2pMessageType::TaskRequest);
    }

    #[test]
    fn test_task_request_msgpack_roundtrip() {
        let req = P2pTaskRequest {
            session_id: "sess-123".into(),
            task_id: "task-456".into(),
            model: "claude-sonnet-4".into(),
            messages: vec![1, 2, 3], // encrypted payload
            stream: true,
            params: vec![4, 5, 6],
            est_prompt_tokens: 100,
            max_output_tokens: 2048,
        };

        let frame = encode_message(P2pMessageType::TaskRequest, &req).unwrap();
        let decoded: P2pTaskRequest = decode_message(&frame).unwrap();

        assert_eq!(decoded.session_id, "sess-123");
        assert_eq!(decoded.model, "claude-sonnet-4");
        assert!(decoded.stream);
        assert_eq!(decoded.messages, vec![1, 2, 3]);
    }
}