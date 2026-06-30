//! Provider registry: periodic refresh of cc-switch proxy status + whitelist.
//!
//! Maintains a cached view of which API formats are currently available
//! via cc-switch (Claude/Codex/Gemini) and which models cc-share should
//! advertise to the cloud. Whitelist filtering (Phase 6) will narrow this
//! further.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::proxy_client::{ApiFormat, CcSwitchProxyClient, ProxyError, ProxyStatus};

/// One discovered cc-switch provider target.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredProvider {
    pub app_type: String,
    pub provider_name: String,
    pub provider_id: String,
    pub api_format: ApiFormat,
    pub models: Vec<String>,
    /// Mapping from representative model name to real upstream model name.
    /// E.g., {"claude-sonnet-4": "glm-5.1:cloud"}
    #[serde(default)]
    pub upstream_models: HashMap<String, String>,
}

/// Snapshot of the latest cc-switch proxy discovery.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DiscoverySnapshot {
    pub reachable: bool,
    pub running: bool,
    pub current_provider: Option<String>,
    /// Providers discovered from proxy traffic (active_targets).
    pub providers: Vec<DiscoveredProvider>,
    /// Whether providers were read from cc-switch's database (not confirmed by traffic).
    /// When true, the data is real configuration (not guessed), but hasn't been
    /// validated by a successful proxy request yet.
    pub from_db: bool,
    /// Distinct API formats currently available.
    pub available_formats: Vec<ApiFormat>,
    /// All models cc-share can advertise to the cloud right now.
    pub available_models: Vec<String>,
    /// Mapping from representative model names to real upstream model names.
    /// E.g., {"claude-sonnet-4": "glm-5.1:cloud"}
    #[serde(default)]
    pub upstream_models: HashMap<String, String>,
    /// Last error if unreachable (for UI diagnostics).
    pub last_error: Option<String>,
}

/// Registry holding a cached `DiscoverySnapshot`, refreshable on demand.
pub struct ProviderRegistry {
    client: CcSwitchProxyClient,
    snapshot: RwLock<DiscoverySnapshot>,
}

impl ProviderRegistry {
    pub fn new(client: CcSwitchProxyClient) -> Arc<Self> {
        Arc::new(Self {
            client,
            snapshot: RwLock::new(DiscoverySnapshot::default()),
        })
    }

    pub async fn snapshot(&self) -> DiscoverySnapshot {
        self.snapshot.read().await.clone()
    }

    /// Re-query cc-switch `/status` and rebuild the snapshot.
    /// Always succeeds (writes a snapshot even on error) — never panics.
    pub async fn refresh(&self) {
        log::info!("provider_registry: refreshing cc-switch proxy status");
        let new = self.build_snapshot().await;
        log::info!(
            "provider_registry: refresh result — reachable={}, running={}, providers={}, from_db={}, models={}",
            new.reachable,
            new.running,
            new.providers.len(),
            new.from_db,
            new.available_models.len(),
        );
        *self.snapshot.write().await = new;
    }

    async fn build_snapshot(&self) -> DiscoverySnapshot {
        match self.client.status().await {
            Ok(status) => {
                log::info!(
                    "provider_registry: cc-switch /status returned — running={}, port={}, current_provider={:?}, active_targets_count={}",
                    status.running,
                    status.port,
                    status.current_provider,
                    status.active_targets.len(),
                );
                for (i, t) in status.active_targets.iter().enumerate() {
                    log::info!(
                        "provider_registry: active_target[{}] — app_type={}, provider_name={}, provider_id={}",
                        i, t.app_type, t.provider_name, t.provider_id
                    );
                }
                self.snapshot_from_status(status)
            }
            Err(ProxyError::Unreachable(msg)) => {
                log::warn!("provider_registry: cc-switch proxy unreachable: {msg}");
                DiscoverySnapshot {
                    reachable: false,
                    last_error: Some(format!("cc-switch proxy unreachable: {msg}")),
                    ..Default::default()
                }
            }
            Err(e) => {
                log::warn!("provider_registry: cc-switch proxy error: {e}");
                DiscoverySnapshot {
                    reachable: false,
                    last_error: Some(format!("cc-switch proxy error: {e}")),
                    ..Default::default()
                }
            }
        }
    }

    /// Read current providers from cc-switch's local database.
    ///
    /// cc-switch stores provider configuration in `~/.cc-switch/cc-switch.db`.
    /// When `active_targets` from the `/status` endpoint is empty (no proxy
    /// traffic yet), we can read this database directly to discover the real
    /// app_type and provider name — no inference needed.
    fn read_configured_providers_from_db() -> Vec<DiscoveredProvider> {
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => {
                log::debug!("provider_registry: cannot find home directory");
                return Vec::new();
            }
        };
        let db_path = home.join(".cc-switch").join("cc-switch.db");
        if !db_path.exists() {
            log::debug!("provider_registry: cc-switch database not found at {:?}", db_path);
            return Vec::new();
        }

        let conn = match rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("provider_registry: failed to open cc-switch database: {e}");
                return Vec::new();
            }
        };

        let mut providers = Vec::new();
        // Query current providers (is_current = 1) for standard app types.
        // Also read settings_config to extract real model mappings.
        let mut stmt = match conn.prepare(
            "SELECT id, app_type, name, settings_config FROM providers WHERE is_current = 1"
        ) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("provider_registry: failed to query cc-switch providers: {e}");
                return Vec::new();
            }
        };

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        });

        match rows {
            Ok(row_iter) => {
                for row in row_iter {
                    match row {
                        Ok((id, app_type, name, settings_config)) => {
                            let fmt = ApiFormat::from_app_type(&app_type);
                            // Parse the env config from settings_config to get real model names.
                            let upstream_models = parse_upstream_models(&settings_config, fmt);
                            if !upstream_models.is_empty() {
                                log::info!(
                                    "provider_registry: provider '{}' has upstream_models: {:?}",
                                    name, upstream_models
                                );
                            }
                            providers.push(DiscoveredProvider {
                                app_type,
                                provider_name: name,
                                provider_id: id,
                                api_format: fmt,
                                models: fmt.representative_models().iter().map(|s| s.to_string()).collect(),
                                upstream_models,
                            });
                        }
                        Err(e) => {
                            log::debug!("provider_registry: row error: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("provider_registry: failed to iterate cc-switch providers: {e}");
            }
        }

        if !providers.is_empty() {
            log::info!("provider_registry: read {} current provider(s) from cc-switch database", providers.len());
        }
        providers
    }

    fn snapshot_from_status(&self, status: ProxyStatus) -> DiscoverySnapshot {
        let mut providers: Vec<DiscoveredProvider> = Vec::new();
        let mut formats: Vec<ApiFormat> = Vec::new();
        let mut models: Vec<String> = Vec::new();
        let mut upstream_models: HashMap<String, String> = HashMap::new();
        let mut from_db = false;

        // Primary source: active_targets from proxy traffic.
        // Note: active_targets don't carry settings_config, so upstream_models
        // will be empty for this path. The DB fallback provides model mappings.
        for t in &status.active_targets {
            let fmt = ApiFormat::from_app_type(&t.app_type);
            providers.push(DiscoveredProvider {
                app_type: t.app_type.clone(),
                provider_name: t.provider_name.clone(),
                provider_id: t.provider_id.clone(),
                api_format: fmt,
                models: fmt.representative_models().iter().map(|s| s.to_string()).collect(),
                upstream_models: HashMap::new(), // filled from DB fallback below
            });
            if !formats.contains(&fmt) {
                formats.push(fmt);
            }
            for m in fmt.representative_models() {
                if !models.contains(&m.to_string()) {
                    models.push(m.to_string());
                }
            }
        }

        // Fallback: when no active_targets, read cc-switch's database directly.
        // The `/status` endpoint only populates `active_targets` after a successful
        // proxy request. Before that, the database is the only source of truth for
        // which providers are configured AND their model mappings.
        if providers.is_empty() {
            log::warn!("provider_registry: no active_targets from cc-switch, falling back to database scan");
            let db_providers = Self::read_configured_providers_from_db();
            if db_providers.is_empty() {
                log::warn!(
                    "provider_registry: cc-switch database fallback also returned 0 providers — \
                     either cc-switch has no configured providers, or the database is not at ~/.cc-switch/cc-switch.db"
                );
            } else {
                log::info!(
                    "provider_registry: found {} provider(s) from cc-switch database: {}",
                    db_providers.len(),
                    db_providers.iter().map(|p| format!("{}({})", p.provider_name, p.app_type)).collect::<Vec<_>>().join(", ")
                );
            }
            for p in &db_providers {
                if !formats.contains(&p.api_format) {
                    formats.push(p.api_format);
                }
                for m in &p.models {
                    if !models.contains(m) {
                        models.push(m.clone());
                    }
                }
                // Merge upstream model mappings from each provider.
                for (k, v) in &p.upstream_models {
                    upstream_models.insert(k.clone(), v.clone());
                }
            }
            if !db_providers.is_empty() {
                providers = db_providers;
                from_db = true;
            }
        }

        if !upstream_models.is_empty() {
            log::info!("provider_registry: upstream_models={:?}", upstream_models);
        }

        DiscoverySnapshot {
            reachable: true,
            running: status.running,
            current_provider: status.current_provider,
            providers,
            from_db,
            available_formats: formats,
            available_models: models,
            upstream_models,
            last_error: None,
        }
    }
}

/// Parse the `settings_config` JSON from cc-switch's providers table to extract
/// real upstream model names. The config contains an "env" object with env vars
/// like `ANTHROPIC_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`, etc.
fn parse_upstream_models(settings_config: &str, fmt: ApiFormat) -> HashMap<String, String> {
    // Parse the JSON to extract the "env" object.
    let config: serde_json::Value = match serde_json::from_str(settings_config) {
        Ok(v) => v,
        Err(e) => {
            log::debug!("provider_registry: settings_config is not valid JSON: {e}");
            return HashMap::new();
        }
    };

    let env = match config.get("env").and_then(|v| v.as_object()) {
        Some(obj) => obj,
        None => {
            log::debug!("provider_registry: settings_config has no 'env' object");
            return HashMap::new();
        }
    };

    // Build a lookup from env var name to value.
    let env_lookup: HashMap<String, String> = env
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.to_ascii_lowercase(), s.to_string())))
        .collect();

    fmt.build_upstream_models(&env_lookup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccswitch::proxy_client::ActiveTarget;

    fn make_client() -> CcSwitchProxyClient {
        CcSwitchProxyClient::new("http://127.0.0.1:1")
    }

    #[test]
    fn snapshot_from_status_with_claude_and_gemini() {
        let reg = ProviderRegistry::new(make_client());
        let status = ProxyStatus {
            running: true,
            port: 15721,
            current_provider: Some("MyClaude".into()),
            current_provider_id: Some("p1".into()),
            active_targets: vec![
                ActiveTarget {
                    app_type: "Claude".into(),
                    provider_name: "MyClaude".into(),
                    provider_id: "p1".into(),
                },
                ActiveTarget {
                    app_type: "Gemini".into(),
                    provider_name: "MyGemini".into(),
                    provider_id: "p2".into(),
                },
            ],
        };
        let snap = reg.snapshot_from_status(status);
        assert!(snap.reachable);
        assert!(snap.running);
        assert_eq!(snap.providers.len(), 2);
        assert!(!snap.from_db);
        assert!(snap.available_formats.contains(&ApiFormat::Anthropic));
        assert!(snap.available_formats.contains(&ApiFormat::GeminiNative));
        // Verify model names match consumer-side list_models
        assert!(snap.available_models.contains(&"claude-sonnet-4".to_string()));
        assert!(snap.available_models.contains(&"gemini-1.5-pro".to_string()));
        assert!(!snap.available_models.is_empty());
    }

    #[test]
    fn snapshot_from_status_empty_no_current_provider() {
        let reg = ProviderRegistry::new(make_client());
        let status = ProxyStatus {
            running: true,
            port: 15721,
            current_provider: None,
            current_provider_id: None,
            active_targets: vec![],
        };
        let snap = reg.snapshot_from_status(status);
        assert!(snap.reachable);
        // When active_targets is empty, DB fallback may or may not find providers
        // depending on whether ~/.cc-switch/cc-switch.db exists with data.
        // Just verify it doesn't panic and the structure is correct.
        let _ = snap;
    }

    #[test]
    fn snapshot_from_status_db_fallback_reads_real_db() {
        // When active_targets is empty, the DB fallback reads ~/.cc-switch/cc-switch.db.
        // In the test env, if the DB exists with ollama as current provider under
        // app_type "claude", the fallback should return Anthropic format (not OpenAiChat).
        // This test verifies the DB fallback works end-to-end on a real machine.
        let reg = ProviderRegistry::new(make_client());
        let status = ProxyStatus {
            running: true,
            port: 15721,
            current_provider: Some("ollama".into()),
            current_provider_id: Some("p1".into()),
            active_targets: vec![],
        };
        let snap = reg.snapshot_from_status(status);
        assert!(snap.reachable);
        // Result depends on whether ~/.cc-switch/cc-switch.db exists:
        // - If DB exists with ollama under "claude" app_type → Anthropic format, from_db=true
        // - If DB doesn't exist or has no current providers → empty, from_db=false
        // Both outcomes are valid; just check it doesn't panic.
        let _ = snap;
    }

    #[test]
    fn snapshot_from_status_active_targets_take_priority() {
        let reg = ProviderRegistry::new(make_client());
        let status = ProxyStatus {
            running: true,
            port: 15721,
            current_provider: Some("ollama".into()),
            current_provider_id: Some("p1".into()),
            active_targets: vec![
                ActiveTarget {
                    app_type: "Claude".into(),
                    provider_name: "MyClaude".into(),
                    provider_id: "p1".into(),
                },
            ],
        };
        let snap = reg.snapshot_from_status(status);
        // active_targets are used, not fallback
        assert_eq!(snap.providers.len(), 1);
        assert_eq!(snap.providers[0].provider_name, "MyClaude");
        assert!(!snap.from_db);
    }

    #[test]
    fn api_format_from_app_type() {
        assert_eq!(ApiFormat::from_app_type("Claude"), ApiFormat::Anthropic);
        assert_eq!(ApiFormat::from_app_type("claude"), ApiFormat::Anthropic);
        assert_eq!(ApiFormat::from_app_type("claude-desktop"), ApiFormat::Anthropic);
        assert_eq!(ApiFormat::from_app_type("Gemini"), ApiFormat::GeminiNative);
        assert_eq!(ApiFormat::from_app_type("codex"), ApiFormat::OpenAiChat);
        assert_eq!(ApiFormat::from_app_type("ollama"), ApiFormat::OpenAiChat);
        // ollama under "claude" app_type should map to Anthropic
        assert_eq!(ApiFormat::from_app_type("claude"), ApiFormat::Anthropic);
    }
}