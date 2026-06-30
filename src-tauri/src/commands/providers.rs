//! Tauri IPC commands for provider discovery + diagnostics + whitelist.

use std::sync::Arc;

use crate::ccswitch::{DiscoverySnapshot, ProviderRegistry};
use crate::database::ShareDb;
use crate::diagnostics::{DiagnosticWarning, Diagnostics};

/// Managed state: provider registry + diagnostics, injected in setup().
pub struct ProviderState {
    pub registry: Arc<ProviderRegistry>,
    pub diagnostics: Arc<Diagnostics>,
    pub db: Arc<ShareDb>,
}

/// Refresh the cc-switch proxy discovery snapshot. Returns the new snapshot.
#[tauri::command]
pub async fn refresh_providers(
    state: tauri::State<'_, ProviderState>,
) -> Result<DiscoverySnapshot, String> {
    log::info!("▶ refresh_providers: querying cc-switch proxy status");
    state.registry.refresh().await;
    let snap = state.registry.snapshot().await;
    log::info!(
        "refresh_providers: reachable={}, running={}, current_provider={:?}, providers_count={}, models_count={}",
        snap.reachable,
        snap.running,
        snap.current_provider,
        snap.providers.len(),
        snap.available_models.len(),
    );

    // Update diagnostics based on the fresh snapshot.
    let mut warnings: Vec<DiagnosticWarning> = Vec::new();
    if !snap.reachable {
        warnings.push(DiagnosticWarning {
            code: "ccswitch_unreachable".into(),
            message: snap
                .last_error
                .clone()
                .unwrap_or_else(|| "cc-switch proxy unreachable".into()),
            severity: "error".into(),
        });
    } else if !snap.running {
        warnings.push(DiagnosticWarning {
            code: "ccswitch_not_running".into(),
            message: "cc-switch proxy is not running. Start cc-switch and enable its proxy."
                .into(),
            severity: "warn".into(),
        });
    } else if snap.providers.is_empty() {
        if snap.from_db {
            // Providers were read from cc-switch's database (not yet confirmed by traffic).
            // This is normal — the provider is configured but hasn't handled any requests.
            let names: Vec<&str> = snap.providers.iter().map(|p| p.provider_name.as_str()).collect();
            warnings.push(DiagnosticWarning {
                code: "providers_from_db_not_active".into(),
                message: format!(
                    "cc-switch has {} provider(s) configured ({}) — not yet active via proxy traffic. \
                     The first proxy request will confirm the provider and update the model list.",
                    snap.providers.len(),
                    names.join(", ")
                ),
                severity: "info".into(),
            });
        } else if snap.current_provider.is_some() {
            warnings.push(DiagnosticWarning {
                code: "no_active_providers".into(),
                message: format!(
                    "cc-switch proxy is running with current provider \"{}\", but it is not listed as an active target. \
                     This usually means the provider is not fully configured in cc-switch — check that it is enabled.",
                    snap.current_provider.as_deref().unwrap_or_default()
                ),
                severity: "info".into(),
            });
        } else {
            warnings.push(DiagnosticWarning {
                code: "no_active_providers".into(),
                message: "cc-switch proxy is running but has no active provider targets. Configure a provider in cc-switch."
                    .into(),
                severity: "warn".into(),
            });
        }
    }

    if warnings.is_empty() {
        log::info!("✓ refresh_providers: no diagnostic warnings");
    } else {
        log::info!("refresh_providers: {} diagnostic warning(s)", warnings.len());
        for w in &warnings {
            log::info!("  [{}] {}: {}", w.severity, w.code, w.message);
        }
    }
    state.diagnostics.replace(warnings).await;

    Ok(snap)
}

/// Get the cached discovery snapshot without refreshing.
#[tauri::command]
pub async fn list_proxy_providers(
    state: tauri::State<'_, ProviderState>,
) -> Result<DiscoverySnapshot, String> {
    Ok(state.registry.snapshot().await)
}

/// Get current diagnostic warnings.
#[tauri::command]
pub async fn get_diagnostics(
    state: tauri::State<'_, ProviderState>,
) -> Result<Vec<DiagnosticWarning>, String> {
    Ok(state.diagnostics.all().await)
}

/// Get the share whitelist (model names). Empty = share everything discovered.
#[tauri::command]
pub async fn get_whitelist(
    state: tauri::State<'_, ProviderState>,
) -> Result<Vec<String>, String> {
    state.db.load_whitelist().map_err(|e| e.to_string())
}

/// Set the share whitelist (model names). Empty = share everything.
#[tauri::command]
pub async fn set_whitelist(
    state: tauri::State<'_, ProviderState>,
    models: Vec<String>,
) -> Result<(), String> {
    log::info!("set_whitelist: models={:?}", models);
    state
        .db
        .save_whitelist(&models)
        .map_err(|e| e.to_string())
}

/// Compute the models this node should advertise to the cloud:
/// discovered models from cc-switch, filtered by whitelist (if non-empty).
#[tauri::command]
pub async fn get_shareable_models(
    state: tauri::State<'_, ProviderState>,
) -> Result<Vec<String>, String> {
    let snap = state.registry.snapshot().await;
    let whitelist = state.db.load_whitelist().map_err(|e| e.to_string())?;
    let models = filter_models(snap.available_models.clone(), whitelist.clone());
    log::info!(
        "get_shareable_models: snap.reachable={}, snap.running={}, snap.providers={}, snap.available_models={:?}, whitelist={:?}, result={:?}",
        snap.reachable,
        snap.running,
        snap.providers.len(),
        snap.available_models,
        whitelist,
        models,
    );
    Ok(models)
}

/// Pure helper: filter discovered models by whitelist. Empty whitelist = all.
/// If whitelist is non-empty but filters out everything, falls back to returning
/// all discovered models (the whitelist likely contains stale model names).
pub fn filter_models(discovered: Vec<String>, whitelist: Vec<String>) -> Vec<String> {
    if whitelist.is_empty() {
        return discovered;
    }
    let wl_lower: Vec<String> = whitelist.iter().map(|w| w.to_ascii_lowercase()).collect();
    let filtered: Vec<String> = discovered
        .iter()
        .filter(|m| {
            let m_lower = m.to_ascii_lowercase();
            wl_lower
                .iter()
                .any(|w| m_lower == *w || m_lower.starts_with(&format!("{w}-")))
        })
        .cloned()
        .collect();
    // If whitelist filtered out everything but we have discovered models,
    // the whitelist likely contains stale names. Fall back to all discovered.
    if filtered.is_empty() && !discovered.is_empty() {
        log::warn!(
            "filter_models: whitelist {:?} filtered out all discovered models {:?}; \
             falling back to returning all discovered models",
            whitelist, discovered,
        );
        return discovered;
    }
    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_empty_whitelist_returns_all() {
        let d = vec!["a".into(), "b".into()];
        assert_eq!(filter_models(d.clone(), vec![]), d);
    }

    #[test]
    fn filter_exact_and_prefix_match() {
        let d = vec![
            "claude-sonnet-4".into(),
            "claude-opus-4".into(),
            "gpt-4o".into(),
            "gemini-1.5-pro".into(),
        ];
        let w = vec!["claude".into(), "gpt-4o".into()];
        let out = filter_models(d, w);
        assert!(out.contains(&"claude-sonnet-4".to_string()));
        assert!(out.contains(&"claude-opus-4".to_string()));
        assert!(out.contains(&"gpt-4o".to_string()));
        assert!(!out.contains(&"gemini-1.5-pro".to_string()));
    }

    #[test]
    fn filter_case_insensitive() {
        let d = vec!["Claude-Sonnet-4".into()];
        let w = vec!["claude".into()];
        assert_eq!(filter_models(d, w), vec!["Claude-Sonnet-4".to_string()]);
    }

    #[test]
    fn filter_stale_whitelist_falls_back_to_all() {
        // Whitelist has old model names that don't match discovered names.
        // filter_models should fall back to returning all discovered models.
        let d = vec!["claude-sonnet-4".into(), "gpt-4o".into()];
        let w = vec!["claude-3-5-sonnet".into()];
        assert_eq!(filter_models(d.clone(), w), d);
    }
}
