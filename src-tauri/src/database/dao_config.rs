//! CC-Share 客户端配置持久化（单行 KV）
//!
//! 把 [`crate::share::client::ClientConfig`] 序列化到 SQLite，跨重启保留
//! 服务器地址、token、node_id 等连接参数。
//!
//! Schema：
//! ```sql
//! CREATE TABLE client_config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
//! ```
//! 当前只用一个 key `client_config_v1`，整体 JSON 序列化。
//! 未来字段增加只需 v2/v3 共存读取，不动 schema。

use crate::database::ShareDb;
use crate::error::ShareError;
use crate::share::client::ClientConfig;
use rusqlite::params;

const KEY: &str = "client_config_v1";
const ROLE_KEY: &str = "role_v1";
const P2P_CONFIG_KEY: &str = "p2p_config_v1";

/// P2P configuration persisted across restarts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct P2PConfig {
    /// Whether P2P direct connections are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Number of hole punch retries before falling back to cloud relay.
    #[serde(default = "default_hole_punch_retries")]
    pub hole_punch_retries: u32,
    /// Base delay between hole punch retries (ms), increases with each round.
    #[serde(default = "default_hole_punch_delay_ms")]
    pub hole_punch_delay_ms: u32,
    /// STUN server address (host:port). Empty = derive from cloud server host + port 7890.
    #[serde(default)]
    pub stun_server: String,
    /// Local P2P QUIC port.
    #[serde(default = "default_p2p_port")]
    pub p2p_port: u16,
}

fn default_true() -> bool { true }
fn default_hole_punch_retries() -> u32 { 10 }
fn default_hole_punch_delay_ms() -> u32 { 200 }
fn default_p2p_port() -> u16 { 15731 }

impl Default for P2PConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hole_punch_retries: default_hole_punch_retries(),
            hole_punch_delay_ms: default_hole_punch_delay_ms(),
            stun_server: String::new(),
            p2p_port: default_p2p_port(),
        }
    }
}

/// 角色枚举：供应者、消费者、空闲
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Supplier,
    Consumer,
    Idle,
}

impl ShareDb {
    /// 读取已保存的客户端配置；不存在或格式不兼容（如老版本 `server_url` 字段）返回 `None`
    ///
    /// 老版本配置 JSON 缺 `server_host` 字段会反序列化失败，这里把它视作「无配置」，
    /// 强制走 [`ClientConfig::default`] 并要求用户重填，避免半截配置被使用。
    pub fn load_client_config(&self) -> Result<Option<ClientConfig>, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let result = conn.query_row(
            "SELECT value FROM client_config WHERE key = ?1",
            params![KEY],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(raw) => match serde_json::from_str::<ClientConfig>(&raw) {
                Ok(cfg) => Ok(Some(cfg)),
                Err(e) => {
                    log::warn!(
                        "client_config 反序列化失败，已忽略旧配置并回退到默认值: {e}"
                    );
                    Ok(None)
                }
            },
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(ShareError::Database(e.to_string())),
        }
    }

    /// 保存客户端配置（覆盖）
    pub fn save_client_config(&self, cfg: &ClientConfig) -> Result<(), ShareError> {
        let raw = serde_json::to_string(cfg)
            .map_err(|e| ShareError::Database(format!("encode client_config: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO client_config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY, raw],
        )?;
        Ok(())
    }

    /// Share whitelist: which model names (or apiFormat prefixes) this node
    /// is willing to share. Empty = share everything discovered.
    const WHITELIST_KEY: &'static str = "share_whitelist_v1";

    pub fn load_whitelist(&self) -> Result<Vec<String>, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let result = conn.query_row(
            "SELECT value FROM client_config WHERE key = ?1",
            params![Self::WHITELIST_KEY],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(raw) => {
                let list: Vec<String> = serde_json::from_str(&raw)
                    .map_err(|e| ShareError::Database(format!("decode whitelist: {e}")))?;
                Ok(list)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Vec::new()),
            Err(e) => Err(ShareError::Database(e.to_string())),
        }
    }

    pub fn save_whitelist(&self, models: &[String]) -> Result<(), ShareError> {
        let raw = serde_json::to_string(models)
            .map_err(|e| ShareError::Database(format!("encode whitelist: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO client_config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![Self::WHITELIST_KEY, raw],
        )?;
        Ok(())
    }

    /// 读取当前角色；不存在时返回 Idle
    pub fn load_role(&self) -> Result<Role, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let result = conn.query_row(
            "SELECT value FROM client_config WHERE key = ?1",
            params![ROLE_KEY],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(raw) => {
                let role: Role = serde_json::from_str(&raw)
                    .map_err(|e| ShareError::Database(format!("decode role: {e}")))?;
                Ok(role)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Role::Idle),
            Err(e) => Err(ShareError::Database(e.to_string())),
        }
    }

    /// 保存角色
    pub fn save_role(&self, role: &Role) -> Result<(), ShareError> {
        let raw = serde_json::to_string(role)
            .map_err(|e| ShareError::Database(format!("encode role: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO client_config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![ROLE_KEY, raw],
        )?;
        Ok(())
    }

    /// Load P2P configuration; returns default if not stored.
    pub fn load_p2p_config(&self) -> Result<P2PConfig, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let result = conn.query_row(
            "SELECT value FROM client_config WHERE key = ?1",
            params![P2P_CONFIG_KEY],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(raw) => serde_json::from_str::<P2PConfig>(&raw)
                .map_err(|e| {
                    log::warn!("p2p_config 反序列化失败，使用默认值: {e}");
                    Ok::<P2PConfig, ShareError>(P2PConfig::default())
                })
                .or_else(|_| Ok(P2PConfig::default())),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(P2PConfig::default()),
            Err(e) => Err(ShareError::Database(e.to_string())),
        }
    }

    /// Save P2P configuration.
    pub fn save_p2p_config(&self, cfg: &P2PConfig) -> Result<(), ShareError> {
        let raw = serde_json::to_string(cfg)
            .map_err(|e| ShareError::Database(format!("encode p2p_config: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO client_config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![P2P_CONFIG_KEY, raw],
        )?;
        Ok(())
    }

    // --- Generic KV helpers for auth state and other dynamic config ---

    /// Get a config value by key. Returns None if not found.
    pub fn get_config(&self, key: &str) -> Result<Option<String>, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let result = conn.query_row(
            "SELECT value FROM client_config WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(ShareError::Database(e.to_string())),
        }
    }

    /// Set a config value by key (upsert).
    pub fn set_config(&self, key: &str, value: &str) -> Result<(), ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO client_config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Delete a config value by key.
    pub fn delete_config(&self, key: &str) -> Result<(), ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute("DELETE FROM client_config WHERE key = ?1", params![key])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_when_missing_returns_none() {
        let db = ShareDb::memory().unwrap();
        assert!(db.load_client_config().unwrap().is_none());
    }

    #[test]
    fn test_save_then_load_round_trip() {
        let db = ShareDb::memory().unwrap();
        let cfg = ClientConfig {
            server_host: "api.cc-share.com".into(),
            heartbeat_interval_secs: 25,
            max_reconnect_interval_secs: 90,
            auth_token: "tok".into(),
            node_id: "node-42".into(),
            hmac_secret: String::new(),
            use_https: false,
        };
        db.save_client_config(&cfg).unwrap();
        let back = db.load_client_config().unwrap().unwrap();
        assert_eq!(back.server_host, cfg.server_host);
        assert_eq!(back.heartbeat_interval_secs, 25);
        assert_eq!(back.node_id, "node-42");
    }

    #[test]
    fn test_whitelist_round_trip() {
        let db = ShareDb::memory().unwrap();
        assert!(db.load_whitelist().unwrap().is_empty());
        let models = vec!["claude-3-5-sonnet".into(), "gpt-4o".into()];
        db.save_whitelist(&models).unwrap();
        let back = db.load_whitelist().unwrap();
        assert_eq!(back, models);
    }

    #[test]
    fn test_whitelist_overwrites() {
        let db = ShareDb::memory().unwrap();
        db.save_whitelist(&["a".into()]).unwrap();
        db.save_whitelist(&["b".into(), "c".into()]).unwrap();
        let back = db.load_whitelist().unwrap();
        assert_eq!(back, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn test_whitelist_does_not_clobber_client_config() {
        let db = ShareDb::memory().unwrap();
        let cfg = ClientConfig::default();
        db.save_client_config(&cfg).unwrap();
        db.save_whitelist(&["m".into()]).unwrap();
        // client_config must still be readable after whitelist write (separate keys)
        assert!(db.load_client_config().unwrap().is_some());
        assert_eq!(db.load_whitelist().unwrap(), vec!["m".to_string()]);
    }

    #[test]
    fn test_save_overwrites() {
        let db = ShareDb::memory().unwrap();
        let mut cfg = ClientConfig::default();
        cfg.server_host = "a.example.com".into();
        db.save_client_config(&cfg).unwrap();
        cfg.server_host = "b.example.com".into();
        db.save_client_config(&cfg).unwrap();
        let back = db.load_client_config().unwrap().unwrap();
        assert_eq!(back.server_host, "b.example.com");
    }

    #[test]
    fn test_load_legacy_server_url_format_returns_none() {
        // 老版本写入的 JSON 含 server_url 字段而无 server_host，反序列化应失败 → 返回 None
        let db = ShareDb::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO client_config (key, value) VALUES (?1, ?2)",
            params![
                KEY,
                r#"{"server_url":"wss://api.cc-share.com/api/v1/agent/connect","heartbeat_interval_secs":30,"max_reconnect_interval_secs":60,"auth_token":"tok","node_id":"n1","hmac_secret":""}"#
            ],
        ).unwrap();
        drop(conn);
        let loaded = db.load_client_config().unwrap();
        assert!(loaded.is_none(), "legacy config without server_host should be treated as missing");
    }
}
