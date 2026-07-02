//! P2P connection pool and reuse.
//!
//! Manages a pool of established QUIC connections keyed by peer node ID.
//! Connections are reused across tasks to reduce handshake overhead.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::connection::P2PConnectionManager;
use crate::error::ShareError;

/// A pool of QUIC connections keyed by peer node ID.
pub struct ConnectionPool {
    connections: RwLock<HashMap<String, quinn::Connection>>,
    conn_manager: Arc<P2PConnectionManager>,
}

impl ConnectionPool {
    /// Create a new empty connection pool.
    pub fn new(conn_manager: Arc<P2PConnectionManager>) -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            conn_manager,
        }
    }

    /// Get an existing connection for a peer, or establish a new one.
    pub async fn get_or_connect(
        &self,
        peer_node_id: &str,
        candidates: &[String],
        session_id: &str,
    ) -> Result<quinn::Connection, ShareError> {
        // Check pool first.
        {
            let connections = self.connections.read().await;
            if let Some(conn) = connections.get(peer_node_id) {
                // Verify the connection is still alive.
                if conn.close_reason().is_none() {
                    log::debug!("P2P: reusing existing connection to {}", peer_node_id);
                    return Ok(conn.clone());
                }
            }
        }

        // Connection not in pool or closed — establish a new one.
        log::info!("P2P: establishing new connection to {}", peer_node_id);
        let conn = self.conn_manager.connect_to_peer(candidates, session_id).await?;

        // Store in pool.
        {
            let mut connections = self.connections.write().await;
            connections.insert(peer_node_id.to_string(), conn.clone());
        }

        Ok(conn)
    }

    /// Remove a peer's connection from the pool.
    pub async fn remove(&self, peer_node_id: &str) {
        let mut connections = self.connections.write().await;
        connections.remove(peer_node_id);
    }

    /// Close all connections and clear the pool.
    pub async fn close_all(&self) {
        let mut connections = self.connections.write().await;
        for (_, conn) in connections.drain() {
            conn.close(quinn::VarInt::from_u32(0), b"pool shutdown");
        }
    }

    /// Get the number of active connections in the pool.
    pub async fn len(&self) -> usize {
        self.connections.read().await.len()
    }
}