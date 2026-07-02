//! P2P direct connection module
//!
//! Manages peer-to-peer QUIC connections between consumer and supplier nodes,
//! bypassing the cloud relay for data transfer. The cloud server is still used
//! for signaling (matchmaking) and billing reconciliation.

pub mod connection;
pub mod consumer;
pub mod crypto;
pub mod events;
pub mod key;
pub mod pool;
pub mod protocol;
pub mod report;
pub mod session;

use crate::share::protocol::{P2PAnswer, P2POffer};

/// Handle a P2P offer received from the cloud server.
///
/// Validates the offer (content filter, mutex), generates local candidates,
/// and returns a P2PAnswer to send back via the WebSocket channel.
pub fn handle_p2p_offer(offer: &P2POffer, local_candidates: Vec<String>, local_pubkey: &str, accepted: bool, reason: Option<String>) -> P2PAnswer {
    P2PAnswer {
        session_id: offer.session_id.clone(),
        accepted,
        supplier_candidates: if accepted { local_candidates } else { Vec::new() },
        supplier_pubkey: if accepted { Some(local_pubkey.to_string()) } else { None },
        reason,
    }
}