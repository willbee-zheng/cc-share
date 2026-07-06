//! STUN client for discovering public IP:port via the cloud STUN server.
//!
//! Implements a minimal RFC 5389 STUN Binding Request client that discovers
//! the client's public (reflexive) transport address. This is used for
//! P2P NAT traversal — the discovered address is advertised as a candidate
//! so peers behind different NATs can connect directly via UDP hole punching.

use std::net::SocketAddr;
use std::time::Duration;

use crate::error::ShareError;

/// STUN magic cookie (RFC 5389).
const MAGIC_COOKIE: u32 = 0x2112A442;

/// STUN Binding Request message type.
const BINDING_REQUEST: u16 = 0x0001;

/// STUN Binding Response message type.
const BINDING_RESPONSE: u16 = 0x0101;

/// XOR-MAPPED-ADDRESS attribute type.
const ATTR_XOR_MAPPED_ADDR: u16 = 0x0020;

/// MAPPED-ADDRESS attribute type (legacy, for older servers).
const ATTR_MAPPED_ADDR: u16 = 0x0001;

/// Discover the client's public (reflexive) IP:port by sending a STUN
/// Binding Request to the specified STUN server.
///
/// The STUN server address should be in the format "host:port"
/// (e.g., "shareplan.cloud:7890").
///
/// The UDP socket binds to the same port as the QUIC endpoint (using
/// SO_REUSEADDR/SO_REUSEPORT) so the STUN-discovered address matches
/// the NAT mapping that QUIC connections will use.
///
/// Returns the public SocketAddr that the STUN server observed.
pub async fn discover_public_addr(
    stun_server: &str,
    local_port: u16,
    timeout: Duration,
) -> Result<SocketAddr, ShareError> {
    // Resolve the STUN server address.
    let stun_addr = resolve_addr(stun_server)?;

    // Build a STUN Binding Request.
    let transaction_id = generate_transaction_id();
    let request = build_binding_request(&transaction_id);

    // Create a UDP socket with SO_REUSEADDR/SO_REUSEPORT set BEFORE binding,
    // so it can coexist with the QUIC endpoint on the same port.
    let local_addr: SocketAddr = format!("0.0.0.0:{}", local_port).parse()
        .map_err(|e| ShareError::Connection(format!("invalid local addr: {e}")))?;

    let socket = socket2::Socket::new(
        socket2::Domain::for_address(local_addr),
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(|e| ShareError::Connection(format!("STUN socket create: {e}")))?;

    socket.set_reuse_address(true)
        .map_err(|e| ShareError::Connection(format!("STUN set_reuse_address: {e}")))?;

    #[cfg(unix)]
    socket.set_reuse_port(true)
        .map_err(|e| ShareError::Connection(format!("STUN set_reuse_port: {e}")))?;

    socket.bind(&local_addr.into())
        .map_err(|e| ShareError::Connection(format!("STUN bind {}: {e}", local_addr)))?;

    socket.set_nonblocking(true)
        .map_err(|e| ShareError::Connection(format!("STUR set_nonblocking: {e}")))?;

    let std_socket: std::net::UdpSocket = socket.into();

    let socket = tokio::net::UdpSocket::from_std(std_socket)
        .map_err(|e| ShareError::Connection(format!("STUN socket: {e}")))?;

    // Send the Binding Request.
    socket
        .send_to(&request, stun_addr)
        .await
        .map_err(|e| ShareError::Connection(format!("STUN send: {e}")))?;

    // Receive the Binding Response with timeout.
    let mut buf = vec![0u8; 576]; // Minimum STUN MTU
    let recv_result = tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await;

    let (n, _from_addr) = recv_result
        .map_err(|_| ShareError::Connection("STUN response timeout".into()))?
        .map_err(|e| ShareError::Connection(format!("STUN recv: {e}")))?;

    // Parse the Binding Response.
    parse_binding_response(&buf[..n], &transaction_id)
}

/// Resolve a host:port string to a SocketAddr.
fn resolve_addr(addr: &str) -> Result<SocketAddr, ShareError> {
    // Try parsing directly first (for IP:port).
    if let Ok(parsed) = addr.parse::<SocketAddr>() {
        return Ok(parsed);
    }

    // For hostname:port, use tokio DNS resolution.
    let parts: Vec<&str> = addr.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(ShareError::Connection(format!("invalid STUN addr: {addr}")));
    }
    let port: u16 = parts[0].parse()
        .map_err(|e| ShareError::Connection(format!("invalid STUN port: {e}")))?;
    let host = parts[1];

    // Use blocking DNS resolution via std::net.
    let addr = format!("{host}:{port}");
    let socket_addrs: Vec<SocketAddr> = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .map_err(|e| ShareError::Connection(format!("DNS resolve {addr}: {e}")))?
        .collect();

    socket_addrs
        .into_iter()
        .next()
        .ok_or_else(|| ShareError::Connection(format!("no address for {addr}")))
}

/// Generate a random 12-byte STUN transaction ID.
fn generate_transaction_id() -> [u8; 12] {
    let mut id = [0u8; 12];
    use rand::Rng;
    let mut rng = rand::thread_rng();
    for i in 0..12 {
        id[i] = rng.gen();
    }
    id
}

/// Build a 20-byte STUN Binding Request.
fn build_binding_request(transaction_id: &[u8; 12]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(20);
    // Message type: Binding Request (0x0001)
    msg.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    // Message length: 0 (no attributes)
    msg.extend_from_slice(&0u16.to_be_bytes());
    // Magic cookie
    msg.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    // Transaction ID (12 bytes)
    msg.extend_from_slice(transaction_id);
    msg
}

/// Parse a STUN Binding Response and extract the mapped address.
fn parse_binding_response(data: &[u8], expected_txn_id: &[u8; 12]) -> Result<SocketAddr, ShareError> {
    if data.len() < 20 {
        return Err(ShareError::Connection("STUN response too short".into()));
    }

    // Parse header.
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    let _msg_len = u16::from_be_bytes([data[2], data[3]]);
    let magic = u16::from_be_bytes([data[4], data[5]]) as u32
        | ((data[6] as u32) << 24) | ((data[7] as u32) << 16);
    // Actually magic cookie is 4 bytes at offset 4
    let magic_cookie = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

    if magic_cookie != MAGIC_COOKIE {
        return Err(ShareError::Connection(format!(
            "STUN bad magic cookie: {:#010x}",
            magic_cookie
        )));
    }

    // Check message type.
    if msg_type != BINDING_RESPONSE {
        return Err(ShareError::Connection(format!(
            "STUN unexpected message type: {:#06x}",
            msg_type
        )));
    }

    // Verify transaction ID.
    let txn_id = &data[8..20];
    if txn_id != expected_txn_id {
        return Err(ShareError::Connection("STUN transaction ID mismatch".into()));
    }

    // Parse attributes.
    let mut offset = 20; // Start after the header
    while offset + 4 <= data.len() {
        let attr_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let attr_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        let attr_data_start = offset + 4;
        let attr_data_end = attr_data_start + attr_len;

        if attr_data_end > data.len() {
            break;
        }

        match attr_type {
            ATTR_XOR_MAPPED_ADDR => {
                return parse_xor_mapped_addr(
                    &data[attr_data_start..attr_data_end],
                    magic_cookie,
                    expected_txn_id,
                );
            }
            ATTR_MAPPED_ADDR => {
                // Legacy MAPPED-ADDRESS (non-XOR) — fallback for older servers.
                return parse_mapped_addr(&data[attr_data_start..attr_data_end]);
            }
            _ => {}
        }

        // Attributes are padded to 4-byte boundaries.
        let padded_len = (attr_len + 3) & !3;
        offset = attr_data_start + padded_len;
    }

    Err(ShareError::Connection("STUN no XOR-MAPPED-ADDRESS attribute found".into()))
}

/// Parse an XOR-MAPPED-ADDRESS attribute (RFC 5389 Section 15.2).
fn parse_xor_mapped_addr(
    attr_data: &[u8],
    magic_cookie: u32,
    txn_id: &[u8; 12],
) -> Result<SocketAddr, ShareError> {
    if attr_data.len() < 4 {
        return Err(ShareError::Connection("XOR-MAPPED-ADDRESS too short".into()));
    }

    let _reserved = attr_data[0];
    let family = attr_data[1];
    let xor_port = u16::from_be_bytes([attr_data[2], attr_data[3]]);

    // XOR port with upper 16 bits of magic cookie.
    let port = xor_port ^ ((magic_cookie >> 16) as u16);

    match family {
        0x01 => {
            // IPv4
            if attr_data.len() < 8 {
                return Err(ShareError::Connection("XOR-MAPPED-ADDRESS IPv4 too short".into()));
            }
            let xor_ip = u32::from_be_bytes([attr_data[4], attr_data[5], attr_data[6], attr_data[7]]);
            let ip = xor_ip ^ magic_cookie;
            let ip_bytes = ip.to_be_bytes();
            let addr = std::net::Ipv4Addr::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]);
            Ok(SocketAddr::new(std::net::IpAddr::V4(addr), port))
        }
        0x02 => {
            // IPv6
            if attr_data.len() < 20 {
                return Err(ShareError::Connection("XOR-MAPPED-ADDRESS IPv6 too short".into()));
            }
            let mut xor_ip = [0u8; 16];
            xor_ip.copy_from_slice(&attr_data[4..20]);

            // XOR with magic cookie (4 bytes) + transaction ID (12 bytes).
            let magic_bytes = magic_cookie.to_be_bytes();
            let mut ip = [0u8; 16];
            for i in 0..4 {
                ip[i] = xor_ip[i] ^ magic_bytes[i];
            }
            for i in 4..16 {
                ip[i] = xor_ip[i] ^ txn_id[i - 4];
            }

            let addr = std::net::Ipv6Addr::from(ip);
            Ok(SocketAddr::new(std::net::IpAddr::V6(addr), port))
        }
        _ => Err(ShareError::Connection(format!(
            "XOR-MAPPED-ADDRESS unknown family: {family}"
        ))),
    }
}

/// Parse a legacy MAPPED-ADDRESS attribute (non-XOR).
fn parse_mapped_addr(attr_data: &[u8]) -> Result<SocketAddr, ShareError> {
    if attr_data.len() < 8 {
        return Err(ShareError::Connection("MAPPED-ADDRESS too short".into()));
    }

    let _reserved = attr_data[0];
    let family = attr_data[1];
    let port = u16::from_be_bytes([attr_data[2], attr_data[3]]);

    match family {
        0x01 => {
            let ip = std::net::Ipv4Addr::new(attr_data[4], attr_data[5], attr_data[6], attr_data[7]);
            Ok(SocketAddr::new(std::net::IpAddr::V4(ip), port))
        }
        0x02 => {
            if attr_data.len() < 20 {
                return Err(ShareError::Connection("MAPPED-ADDRESS IPv6 too short".into()));
            }
            let mut ip_bytes = [0u8; 16];
            ip_bytes.copy_from_slice(&attr_data[4..20]);
            let ip = std::net::Ipv6Addr::from(ip_bytes);
            Ok(SocketAddr::new(std::net::IpAddr::V6(ip), port))
        }
        _ => Err(ShareError::Connection(format!(
            "MAPPED-ADDRESS unknown family: {family}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_binding_request() {
        let txn_id = [0u8; 12];
        let msg = build_binding_request(&txn_id);
        assert_eq!(msg.len(), 20);
        // Message type: Binding Request (0x0001)
        assert_eq!(u16::from_be_bytes([msg[0], msg[1]]), 0x0001);
        // Message length: 0
        assert_eq!(u16::from_be_bytes([msg[2], msg[3]]), 0);
        // Magic cookie
        assert_eq!(u32::from_be_bytes([msg[4], msg[5], msg[6], msg[7]]), MAGIC_COOKIE);
        // Transaction ID
        assert_eq!(&msg[8..20], &[0u8; 12]);
    }

    #[test]
    fn test_parse_xor_mapped_addr_ipv4() {
        // Simulate a STUN Binding Response with XOR-MAPPED-ADDRESS.
        let txn_id = [0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c];
        let magic_cookie: u32 = 0x2112A442;

        // Build a response with XOR-MAPPED-ADDRESS attribute.
        // Real IP: 192.168.1.100, Port: 48579
        // XOR IP: 192.168.1.100 ^ 0x2112A442 = 0xC0A80164 ^ 0x2112A442 = ...
        // Let's compute:
        let real_ip: u32 = (192 << 24) | (168 << 16) | (1 << 8) | 100;
        let xor_ip = real_ip ^ magic_cookie;
        let real_port: u16 = 48579;
        let xor_port = real_port ^ ((magic_cookie >> 16) as u16);

        // Attribute: XOR-MAPPED-ADDRESS
        let mut attr_data = vec![0u8; 8];
        attr_data[0] = 0x00; // Reserved
        attr_data[1] = 0x01; // IPv4
        attr_data[2..4].copy_from_slice(&xor_port.to_be_bytes());
        attr_data[4..8].copy_from_slice(&xor_ip.to_be_bytes());

        let result = parse_xor_mapped_addr(&attr_data, magic_cookie, &txn_id).unwrap();
        assert_eq!(result.port(), real_port);
        match result {
            SocketAddr::V4(addr) => {
                assert_eq!(addr.ip().to_string(), "192.168.1.100");
            }
            SocketAddr::V6(_) => panic!("expected IPv4"),
        }
    }
}