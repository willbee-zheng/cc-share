//! E2E encryption for P2P task data.
//!
//! Uses ChaCha20-Poly1305 AEAD with keys derived from X25519 Diffie-Hellman.
//! Each P2P session derives a unique task key via HKDF-SHA256 from the
//! shared secret and session ID.

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// HKDF-SHA256 derivation: produces a 32-byte key from a shared secret
/// and info string.
pub fn derive_task_key(shared_secret: &[u8], session_id: &str) -> [u8; 32] {
    // Simplified HKDF: extract-then-expand.
    let salt = [0u8; 32];
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&salt)
        .expect("HMAC key length is valid");
    mac.update(shared_secret);
    let prk = mac.finalize().into_bytes();

    let info = format!("shareplan-task-key-{}", session_id);
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&prk)
        .expect("HMAC key length is valid");
    mac.update(info.as_bytes());
    let okm = mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    key.copy_from_slice(&okm[..32]);
    key
}

/// Encrypt a plaintext payload with ChaCha20-Poly1305.
///
/// Returns `nonce (12 bytes) || ciphertext + tag`.
/// The nonce is randomly generated for each message.
pub fn encrypt_payload(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| format!("invalid key: {e}"))?;

    let nonce_bytes = generate_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("encryption failed: {e}"))?;

    // Prepend nonce to ciphertext.
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypt a ChaCha20-Poly1305 encrypted payload.
///
/// Input format: `nonce (12 bytes) || ciphertext + tag`.
pub fn decrypt_payload(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 12 {
        return Err("payload too short: missing nonce".into());
    }

    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| format!("invalid key: {e}"))?;

    let (nonce_bytes, ciphertext) = data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decryption failed: {e}"))
}

/// Generate a random 12-byte nonce for ChaCha20-Poly1305.
fn generate_nonce() -> [u8; 12] {
    use rand::RngCore;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let shared_secret = [0xAB; 32];
        let session_id = "test-session-123";
        let key = derive_task_key(&shared_secret, session_id);

        let plaintext = b"Hello, P2P world!";
        let encrypted = encrypt_payload(&key, plaintext).unwrap();
        let decrypted = decrypt_payload(&key, &encrypted).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_different_sessions_produce_different_keys() {
        let shared_secret = [0xAB; 32];
        let key1 = derive_task_key(&shared_secret, "session-1");
        let key2 = derive_task_key(&shared_secret, "session-2");

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_decrypt_with_wrong_key_fails() {
        let shared_secret = [0xAB; 32];
        let key = derive_task_key(&shared_secret, "session-1");
        let wrong_key = derive_task_key(&shared_secret, "session-2");

        let plaintext = b"secret data";
        let encrypted = encrypt_payload(&key, plaintext).unwrap();

        assert!(decrypt_payload(&wrong_key, &encrypted).is_err());
    }
}