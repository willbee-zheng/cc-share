//! QUIC hole punching for NAT traversal.
//!
//! Implements simultaneous open / UDP hole punching to establish direct P2P
//! QUIC connections between peers behind NATs. Both peers send "punch" UDP
//! packets to each other's public addresses to open NAT mappings, then
//! attempt QUIC connections.

use std::time::Duration;

use crate::error::ShareError;
use crate::p2p::connection::P2PConnectionManager;

/// Default number of hole punch retries before falling back to cloud relay.
pub const DEFAULT_HOLE_PUNCH_RETRIES: u32 = 10;

/// Default delay between retries (base, increases with each round).
pub const DEFAULT_HOLE_PUNCH_DELAY_MS: u32 = 200;

/// Attempt QUIC hole punching to establish a direct P2P connection.
///
/// Strategy:
/// 1. Sort peer candidates (public addresses first, then local).
/// 2. For each retry round:
///    a. Send a "punch" UDP packet to each peer candidate to open NAT mappings.
///    b. Try QUIC connect_to_peer with all candidates.
///    c. If connection succeeds, return it.
///    d. If connection fails, wait with increasing delay and retry.
/// 3. After all retries exhausted, return an error (triggers cloud relay fallback).
pub async fn punch_and_connect(
    conn_manager: &P2PConnectionManager,
    peer_candidates: &[String],
    session_id: &str,
    max_retries: u32,
    base_delay_ms: u32,
) -> Result<quinn::Connection, ShareError> {
    if peer_candidates.is_empty() {
        return Err(ShareError::Connection("no peer candidates for hole punching".into()));
    }

    // Sort candidates: prefer public (non-127.0.0.1, non-0.0.0.0) addresses first.
    let sorted_candidates = sort_candidates(peer_candidates);

    log::info!(
        "P2P hole punch: starting with {} candidates, max {} retries",
        sorted_candidates.len(),
        max_retries,
    );

    for attempt in 0..max_retries {
        // Increasing delay between retries: base * (attempt + 1).
        let delay = Duration::from_millis(base_delay_ms as u64 * (attempt + 1) as u64);

        if attempt > 0 {
            log::debug!(
                "P2P hole punch: attempt {}/{}, waiting {}ms",
                attempt + 1,
                max_retries,
                delay.as_millis(),
            );
            tokio::time::sleep(delay).await;
        }

        // Send punch UDP packets to each candidate to open NAT mappings.
        punch_udp(&sorted_candidates, conn_manager).await;

        // Try QUIC connection.
        match conn_manager.connect_to_peer(&sorted_candidates, session_id).await {
            Ok(conn) => {
                log::info!(
                    "P2P hole punch: connected on attempt {}/{} to {:?}",
                    attempt + 1,
                    max_retries,
                    conn.remote_address(),
                );
                return Ok(conn);
            }
            Err(e) => {
                log::debug!(
                    "P2P hole punch: attempt {}/{} failed: {}",
                    attempt + 1,
                    max_retries,
                    e,
                );
            }
        }
    }

    Err(ShareError::Connection(format!(
        "hole punch failed after {} retries with {} candidates",
        max_retries,
        sorted_candidates.len(),
    )))
}

/// Sort candidate addresses: public (non-private) addresses first.
///
/// Private/local addresses (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 0.0.0.0)
/// are deprioritized because they are not reachable from the public internet.
fn sort_candidates(candidates: &[String]) -> Vec<String> {
    let mut public = Vec::new();
    let mut private = Vec::new();

    for c in candidates {
        if is_private_address(c) {
            private.push(c.clone());
        } else {
            public.push(c.clone());
        }
    }

    // Public addresses first, then private.
    public.extend(private);
    public
}

/// Check if an address string is a private/local address.
fn is_private_address(addr: &str) -> bool {
    // Parse just the IP part (before the colon for port).
    let ip_str = addr.split(':').next().unwrap_or(addr);

    // Check for common private ranges.
    ip_str.starts_with("127.")
        || ip_str.starts_with("10.")
        || ip_str.starts_with("192.168.")
        || ip_str.starts_with("0.0.0.0")
        || ip_str == "::1"
        || ip_str.starts_with("172.")
            && ip_str[5..8].parse::<u8>().map_or(true, |b| b >= 16 && b <= 31)
}

/// Send UDP "punch" packets to each peer candidate address.
///
/// The punch packet is sent from the QUIC endpoint's port (with SO_REUSEADDR)
/// so the NAT mapping matches the QUIC connection.
async fn punch_udp(candidates: &[String], conn_manager: &P2PConnectionManager) {
    let local_addr = match conn_manager.local_addr().await {
        Ok(addrs) if !addrs.is_empty() => addrs[0],
        _ => return, // Can't punch if endpoint isn't started.
    };

    let bind_addr: std::net::SocketAddr = format!("0.0.0.0:{}", local_addr.port())
        .parse()
        .unwrap_or_else(|_| format!("0.0.0.0:{}", local_addr.port()).parse().unwrap());

    // Create a socket with SO_REUSEADDR/SO_REUSEPORT set BEFORE binding,
    // so it can coexist with the QUIC endpoint on the same port.
    let socket = match socket2::Socket::new(
        socket2::Domain::for_address(bind_addr),
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    ) {
        Ok(s) => s,
        Err(e) => {
            log::debug!("P2P hole punch: failed to create socket: {e}");
            return;
        }
    };

    if let Err(e) = socket.set_reuse_address(true) {
        log::debug!("P2P hole punch: set_reuse_address failed: {e}");
    }
    #[cfg(unix)]
    if let Err(e) = socket.set_reuse_port(true) {
        log::debug!("P2P hole punch: set_reuse_port failed: {e}");
    }

    if let Err(e) = socket.bind(&bind_addr.into()) {
        log::debug!("P2P hole punch: failed to bind UDP socket: {e}");
        return;
    }

    if let Err(e) = socket.set_nonblocking(true) {
        log::debug!("P2P hole punch: set_nonblocking failed: {e}");
        return;
    }

    let std_socket: std::net::UdpSocket = socket.into();
    let socket = match tokio::net::UdpSocket::from_std(std_socket) {
        Ok(s) => s,
        Err(e) => {
            log::debug!("P2P hole punch: failed to convert socket: {e}");
            return;
        }
    };

    // Small payload — just needs to create a NAT mapping.
    let punch_payload = b"SHAREPLAN_PUNCH";

    for candidate in candidates {
        let addr: std::net::SocketAddr = match candidate.parse() {
            Ok(a) => a,
            Err(_) => continue,
        };

        // We don't care if this actually reaches the peer —
        // the goal is just to create an outbound NAT mapping.
        match socket.send_to(punch_payload, addr).await {
            Ok(_) => {
                log::debug!("P2P hole punch: sent punch to {}", addr);
            }
            Err(e) => {
                log::debug!("P2P hole punch: punch to {} failed: {e}", addr);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_candidates_public_first() {
        let candidates = vec![
            "127.0.0.1:15731".to_string(),
            "203.0.113.5:15731".to_string(),
            "192.168.1.100:15731".to_string(),
            "198.51.100.10:15731".to_string(),
        ];
        let sorted = sort_candidates(&candidates);
        assert_eq!(sorted[0], "203.0.113.5:15731");
        assert_eq!(sorted[1], "198.51.100.10:15731");
        // Private addresses come after public ones.
        assert!(sorted[2].starts_with("127.") || sorted[2].starts_with("192.168."));
        assert!(sorted[3].starts_with("127.") || sorted[3].starts_with("192.168."));
    }

    #[test]
    fn test_is_private_address() {
        assert!(is_private_address("127.0.0.1:15731"));
        assert!(is_private_address("10.0.0.1:15731"));
        assert!(is_private_address("192.168.1.100:15731"));
        assert!(is_private_address("0.0.0.0:15731"));
        assert!(!is_private_address("203.0.113.5:15731"));
        assert!(!is_private_address("198.51.100.10:15731"));
    }
}