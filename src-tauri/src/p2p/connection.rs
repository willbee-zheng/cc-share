//! QUIC connection manager for P2P direct connections.
//!
//! Manages QUIC endpoints, hole punching, and connection lifecycle.
//! Uses quinn for QUIC transport. TLS certificates are self-signed
//! since authentication is handled at the application layer via
//! X25519 key exchange and ChaCha20-Poly1305 E2E encryption.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::{ClientConfig, Endpoint, ServerConfig, VarInt};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::sync::RwLock;
use tauri::AppHandle;

use crate::error::ShareError;

use super::events::{P2PConnStatus, P2PConnectionEvent, emit_connection_status};
use super::key::P2PKeyManager;

/// Default QUIC port for P2P connections.
pub const DEFAULT_P2P_PORT: u16 = 15731;

/// Connection timeout for QUIC handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Manages QUIC endpoints and connections for P2P.
pub struct P2PConnectionManager {
    /// Local QUIC endpoint for accepting incoming connections.
    endpoint: RwLock<Option<Endpoint>>,
    /// P2P key manager for E2E encryption.
    key_manager: Arc<P2PKeyManager>,
    /// Port to listen on for incoming P2P connections.
    port: u16,
    /// Tauri app handle for emitting events to the frontend.
    app_handle: Option<AppHandle>,
    /// Number of active peer connections.
    active_connections: Arc<std::sync::atomic::AtomicUsize>,
}

impl P2PConnectionManager {
    /// Create a new P2P connection manager.
    pub fn new(key_manager: Arc<P2PKeyManager>, port: u16) -> Self {
        Self {
            endpoint: RwLock::new(None),
            key_manager,
            port,
            app_handle: None,
            active_connections: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Set the Tauri app handle for emitting P2P events.
    pub fn set_app_handle(&mut self, handle: AppHandle) {
        self.app_handle = Some(handle);
    }

    /// Get the current number of active peer connections.
    pub fn active_connection_count(&self) -> usize {
        self.active_connections.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Start the QUIC endpoint. Call this once at startup.
    ///
    /// Creates a UDP socket with SO_REUSEADDR/SO_REUSEPORT so that the
    /// STUN client and hole-punch packets can bind the same port.
    pub async fn start(&self) -> Result<(), ShareError> {
        let (server_config, client_config) = generate_configs()?;

        let bind_addr: SocketAddr = format!("0.0.0.0:{}", self.port)
            .parse()
            .map_err(|e| ShareError::Connection(format!("invalid bind address: {e}")))?;

        // Create a socket with SO_REUSEADDR (and SO_REUSEPORT on Unix) so
        // the STUN client and hole-punch UDP packets can share the same port.
        let socket = Socket::new(Domain::for_address(bind_addr), Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| ShareError::Connection(format!("socket create: {e}")))?;

        socket.set_reuse_address(true)
            .map_err(|e| ShareError::Connection(format!("set_reuse_address: {e}")))?;

        #[cfg(unix)]
        socket.set_reuse_port(true)
            .map_err(|e| ShareError::Connection(format!("set_reuse_port: {e}")))?;

        socket.bind(&bind_addr.into())
            .map_err(|e| ShareError::Connection(format!("socket bind {}: {e}", bind_addr)))?;

        socket.set_nonblocking(true)
            .map_err(|e| ShareError::Connection(format!("set_nonblocking: {e}")))?;

        let std_socket: std::net::UdpSocket = socket.into();

        let mut endpoint = Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(server_config),
            std_socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|e| ShareError::Connection(format!("QUIC endpoint create: {e}")))?;

        endpoint.set_default_client_config(client_config);

        log::info!("P2P: QUIC endpoint listening on {}", bind_addr);
        let mut guard = self.endpoint.write().await;
        *guard = Some(endpoint);

        if let Some(app) = &self.app_handle {
            emit_connection_status(app, P2PConnectionEvent {
                status: P2PConnStatus::Started,
                port: Some(self.port),
                peer_address: None,
                active_connections: 0,
            });
        }

        Ok(())
    }

    /// Connect to a peer at the given addresses with hole-punching.
    /// Tries each candidate address in order, returns the first successful connection.
    pub async fn connect_to_peer(
        &self,
        candidates: &[String],
        _session_id: &str,
    ) -> Result<quinn::Connection, ShareError> {
        let endpoint = {
            let guard = self.endpoint.read().await;
            guard.clone().ok_or_else(|| ShareError::Connection("P2P endpoint not started".into()))?
        };

        let mut last_error = None;

        for candidate in candidates {
            let addr: SocketAddr = match candidate.parse() {
                Ok(a) => a,
                Err(e) => {
                    log::debug!("P2P: skipping invalid candidate '{}': {}", candidate, e);
                    last_error = Some(ShareError::Connection(format!("invalid address: {candidate}")));
                    continue;
                }
            };

            log::info!("P2P: attempting QUIC connection to {}", addr);
            let connecting = match endpoint.connect(addr, "shareplan-p2p") {
                Ok(c) => c,
                Err(e) => {
                    log::debug!("P2P: connect() to {} failed: {}", addr, e);
                    last_error = Some(ShareError::Connection(format!("QUIC connect: {e}")));
                    continue;
                }
            };

            match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
                Ok(Ok(conn)) => {
                    log::info!("P2P: connected to {}", addr);
                    let count = self.active_connections.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

                    if let Some(app) = &self.app_handle {
                        emit_connection_status(app, P2PConnectionEvent {
                            status: P2PConnStatus::PeerConnected,
                            port: Some(self.port),
                            peer_address: Some(addr.to_string()),
                            active_connections: count,
                        });
                    }

                    return Ok(conn);
                }
                Ok(Err(e)) => {
                    log::debug!("P2P: handshake to {} failed: {}", addr, e);
                    last_error = Some(ShareError::Connection(format!("QUIC handshake: {e}")));
                }
                Err(_) => {
                    log::debug!("P2P: connection to {} timed out", addr);
                    last_error = Some(ShareError::Connection(format!("timeout connecting to {addr}")));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ShareError::Connection("no candidates provided".into())))
    }

    /// Get the local listening addresses.
    pub async fn local_addr(&self) -> Result<Vec<SocketAddr>, ShareError> {
        let guard = self.endpoint.read().await;
        let endpoint = guard.as_ref().ok_or_else(|| ShareError::Connection("P2P endpoint not started".into()))?;

        Ok(endpoint.local_addr()
            .map(|a| vec![a])
            .unwrap_or_default())
    }

    /// Check whether the QUIC endpoint is currently running.
    pub async fn is_running(&self) -> bool {
        self.endpoint.read().await.is_some()
    }

    /// Accept an incoming QUIC connection.
    pub async fn accept_incoming(&self) -> Result<quinn::Incoming, ShareError> {
        let guard = self.endpoint.read().await;
        let endpoint = guard.as_ref().ok_or_else(|| ShareError::Connection("P2P endpoint not started".into()))?;

        let incoming = endpoint.accept()
            .await
            .ok_or_else(|| ShareError::Connection("QUIC endpoint closed".into()))?;

        let count = self.active_connections.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if let Some(app) = &self.app_handle {
            emit_connection_status(app, P2PConnectionEvent {
                status: P2PConnStatus::PeerConnected,
                port: Some(self.port),
                peer_address: None,
                active_connections: count,
            });
        }

        Ok(incoming)
    }

    /// Decrement the active connection count (call when a peer disconnects).
    pub fn peer_disconnected(&self, peer_address: Option<String>) {
        let count = self.active_connections.fetch_sub(1, std::sync::atomic::Ordering::Relaxed).saturating_sub(1);
        if let Some(app) = &self.app_handle {
            emit_connection_status(app, P2PConnectionEvent {
                status: P2PConnStatus::PeerDisconnected,
                port: Some(self.port),
                peer_address,
                active_connections: count,
            });
        }
    }

    /// Shut down the QUIC endpoint.
    pub async fn shutdown(&self) {
        let mut guard = self.endpoint.write().await;
        if let Some(endpoint) = guard.take() {
            endpoint.close(VarInt::from_u32(0), b"shutdown");
            log::info!("P2P: QUIC endpoint shut down");
        }

        if let Some(app) = &self.app_handle {
            emit_connection_status(app, P2PConnectionEvent {
                status: P2PConnStatus::Stopped,
                port: Some(self.port),
                peer_address: None,
                active_connections: 0,
            });
        }
    }
}

/// Generate self-signed TLS certificate and quinn configs.
fn generate_configs() -> Result<(ServerConfig, ClientConfig), ShareError> {
    let key_pair = rcgen::KeyPair::generate()
        .map_err(|e| ShareError::Connection(format!("key gen: {e}")))?;
    let params = rcgen::CertificateParams::default();
    let cert = params.self_signed(&key_pair)
        .map_err(|e| ShareError::Connection(format!("cert gen: {e}")))?;

    let cert_der = cert.der().to_vec();
    let key_der = key_pair.serialize_der();

    // Server config
    let server_tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(cert_der)],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key_der.into()),
        )
        .map_err(|e| ShareError::Connection(format!("server TLS: {e}")))?;

    let server_config = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_tls)
            .map_err(|e| ShareError::Connection(format!("QUIC server config: {e}")))?,
    ));

    // Client config — accept any self-signed certificate since we use
    // application-level E2E encryption (ChaCha20-Poly1305) for authentication.
    let client_tls = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();

    let client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_tls)
            .map_err(|e| ShareError::Connection(format!("QUIC client config: {e}")))?,
    ));

    Ok((server_config, client_config))
}

/// A certificate verifier that accepts any certificate.
/// Safe because P2P uses application-level E2E encryption (ChaCha20-Poly1305)
/// on top of QUIC TLS. The TLS layer provides transport encryption;
/// real authentication is via X25519 key exchange.
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
        ]
    }
}

impl std::fmt::Display for NoVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NoVerifier")
    }
}