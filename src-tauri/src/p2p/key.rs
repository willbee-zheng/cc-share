//! X25519 key management for P2P E2E encryption.
//!
//! Each cc-share instance generates an X25519 key pair on first startup.
//! The public key is registered with the cloud server via NodeStatus.
//! During P2P signaling, both peers' public keys are exchanged,
//! and a shared secret is derived for task-level encryption.

use std::sync::RwLock;
use x25519_dalek::{PublicKey, StaticSecret};
pub struct P2PKeyManager {
    secret: RwLock<StaticSecret>,
    public: PublicKey,
}

impl P2PKeyManager {
    /// Generate a new X25519 key pair.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&secret);
        Self {
            secret: RwLock::new(secret),
            public,
        }
    }

    /// Create from existing key pair bytes (loaded from database).
    pub fn from_bytes(secret_bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(secret_bytes);
        let public = PublicKey::from(&secret);
        Self {
            secret: RwLock::new(secret),
            public,
        }
    }

    /// Get the base64-encoded public key for inclusion in NodeStatus.
    pub fn public_key_base64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(self.public.as_bytes())
    }

    /// Get the raw public key bytes (32 bytes).
    pub fn public_key_bytes(&self) -> &[u8; 32] {
        self.public.as_bytes()
    }

    /// Get the raw secret key bytes for database storage.
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.secret.read().unwrap().to_bytes()
    }

    /// Perform X25519 Diffie-Hellman with a peer's public key to derive
    /// a shared secret.
    pub fn diffie_hellman(&self, peer_public: &[u8; 32]) -> [u8; 32] {
        let peer_pk = PublicKey::from(*peer_public);
        let secret = self.secret.read().unwrap();
        *secret.diffie_hellman(&peer_pk).as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let km = P2PKeyManager::generate();
        let pubkey = km.public_key_base64();
        assert!(!pubkey.is_empty());
        assert_eq!(km.public_key_bytes().len(), 32);
    }

    #[test]
    fn test_diffie_hellman_symmetry() {
        let alice = P2PKeyManager::generate();
        let bob = P2PKeyManager::generate();

        let shared_ab = alice.diffie_hellman(bob.public_key_bytes());
        let shared_ba = bob.diffie_hellman(alice.public_key_bytes());

        assert_eq!(shared_ab, shared_ba, "X25519 DH must produce the same shared secret on both sides");
    }

    #[test]
    fn test_from_bytes_roundtrip() {
        let original = P2PKeyManager::generate();
        let secret_bytes = original.secret_key_bytes();
        let public_bytes = *original.public_key_bytes();

        let restored = P2PKeyManager::from_bytes(secret_bytes);
        assert_eq!(*restored.public_key_bytes(), public_bytes, "restored public key must match original");
    }
}