//! HMAC-SHA256 请求签名（与 cloud-server `internal/signing` 对齐）
//!
//! 在 `Consumer` 调用 `/api/v1/dispatch` 前，注入 HMAC 头：
//!   X-Shareplan-Timestamp / X-Shareplan-Nonce / X-Shareplan-Signature
//!
//! Canonical message: `METHOD\nPATH\nTIMESTAMP\nNONCE\nSHA256(body-hex)`

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub const HEADER_TIMESTAMP: &str = "X-Shareplan-Timestamp";
pub const HEADER_NONCE: &str = "X-Shareplan-Nonce";
pub const HEADER_SIGNATURE: &str = "X-Shareplan-Signature";

type HmacSha256 = Hmac<Sha256>;

/// 构造规范字符串。返回 owned `Vec<u8>` 方便给 hmac/sha256 输入。
pub fn canonical_message(
    method: &str,
    path: &str,
    timestamp: i64,
    nonce: &str,
    body_hash_hex: &str,
) -> Vec<u8> {
    format!("{method}\n{path}\n{timestamp}\n{nonce}\n{body_hash_hex}").into_bytes()
}

/// 计算 sha256(body) 的 hex 字符串
pub fn body_hash_hex(body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(body);
    hex::encode(h.finalize())
}

/// 计算 hex(HMAC-SHA256(secret, message))
pub fn sign(secret: &[u8], message: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

/// 一次性生成签名所需的全部头：(timestamp, nonce, signature_hex)
///
/// `now_unix` 让单测可注入固定时间；生产代码传 `chrono::Utc::now().timestamp()`。
pub fn build_headers(
    secret: &[u8],
    method: &str,
    path: &str,
    body: &[u8],
    now_unix: i64,
    nonce: &str,
) -> (i64, String, String) {
    let body_hash = body_hash_hex(body);
    let msg = canonical_message(method, path, now_unix, nonce, &body_hash);
    let sig = sign(secret, &msg);
    (now_unix, nonce.to_string(), sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_message_format() {
        let m = canonical_message("POST", "/api/v1/dispatch", 1700000000, "n1", "abcd");
        assert_eq!(
            std::str::from_utf8(&m).unwrap(),
            "POST\n/api/v1/dispatch\n1700000000\nn1\nabcd"
        );
    }

    #[test]
    fn test_body_hash_known_vector() {
        // sha256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = body_hash_hex(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sign_known_vector() {
        // RFC 4231 test case 1: key=0x0b*20, data="Hi There"
        let key = vec![0x0b; 20];
        let sig = sign(&key, b"Hi There");
        assert_eq!(
            sig,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn test_build_headers_round_trip() {
        let secret = b"super-secret";
        let body = br#"{"hello":"world"}"#;
        let (ts, nonce, sig) = build_headers(secret, "POST", "/api/v1/dispatch", body, 1700000000, "n1");
        assert_eq!(ts, 1700000000);
        assert_eq!(nonce, "n1");
        // Recompute on the verifying side to check parity
        let msg = canonical_message("POST", "/api/v1/dispatch", ts, &nonce, &body_hash_hex(body));
        assert_eq!(sign(secret, &msg), sig);
    }
}
