//! 消费者配置生成：为 cc-switch 自动生成 Provider 配置 JSON
//!
//! 根据本地 OpenAI 兼容代理的地址，为 Claude / Codex / Gemini 三种 app_type
//! 生成可直接导入 cc-switch 的 Provider 配置对象。同时管理角色互斥：
//! 切换到消费者角色时停止供应者 Daemon，反之亦然。

use crate::commands::local_server::DEFAULT_BIND_ADDR;
use crate::database::dao_config::Role;
use crate::share::client::ClientConfig;
use crate::ShareState;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Emitter;

/// 获取当前角色
#[tauri::command]
pub async fn get_role(state: tauri::State<'_, ShareState>) -> Result<String, String> {
    let role = state.db.load_role().map_err(|e| e.to_string())?;
    Ok(serde_json::to_string(&role).map_err(|e| e.to_string())?)
}

/// 设置角色（带互斥逻辑）
///
/// - 设为 `consumer` 时：停止供应者 Daemon（如果正在运行）
/// - 设为 `supplier` 时：停止本地消费者代理（如果正在运行）
/// - 设为 `idle` 时：同时停止两者
#[tauri::command]
pub async fn set_role<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, ShareState>,
    local_handle: tauri::State<'_, Arc<crate::commands::local_server::LocalServerHandle>>,
    role: String,
) -> Result<(), String> {
    let new_role: Role = serde_json::from_str(&role)
        .map_err(|e| format!("invalid role: {e}"))?;

    match new_role {
        Role::Consumer => {
            // 停止供应者 Daemon
            let mut daemon = state.daemon.lock().await;
            daemon.stop().await;
            drop(daemon);
            {
                let mut conn_state = state.connection_state.write().await;
                *conn_state = crate::share::client::ConnectionState::Disconnected;
            }
            let _ = app.emit(crate::events::CONNECTION_STATE, "disconnected");
            log::info!("set_role(consumer): stopped supplier daemon");
        }
        Role::Supplier => {
            // 停止本地消费者代理
            let join = {
                let mut shutdown = local_handle.shutdown.lock().await;
                shutdown.take()
            };
            if let Some(j) = join {
                j.abort();
            }
            *local_handle.addr.lock().await = None;
            log::info!("set_role(supplier): stopped local consumer server");
        }
        Role::Idle => {
            // 停止两者
            {
                let mut daemon = state.daemon.lock().await;
                daemon.stop().await;
            }
            {
                let mut conn_state = state.connection_state.write().await;
                *conn_state = crate::share::client::ConnectionState::Disconnected;
            }
            let _ = app.emit(crate::events::CONNECTION_STATE, "disconnected");

            let join = {
                let mut shutdown = local_handle.shutdown.lock().await;
                shutdown.take()
            };
            if let Some(j) = join {
                j.abort();
            }
            *local_handle.addr.lock().await = None;
            log::info!("set_role(idle): stopped both supplier and consumer");
        }
    }

    state.db.save_role(&new_role).map_err(|e| e.to_string())?;
    let _ = app.emit(
        crate::events::ROLE_CHANGED,
        serde_json::to_string(&new_role).unwrap_or_default(),
    );
    Ok(())
}

/// 生成 cc-switch Provider 配置 JSON
///
/// 根据指定的 app_type 生成可以直接导入 cc-switch 的 Provider 配置对象。
/// 本地代理默认监听 `127.0.0.1:8081`。
#[tauri::command]
pub async fn generate_consumer_config(
    state: tauri::State<'_, ShareState>,
    _local_handle: tauri::State<'_, Arc<crate::commands::local_server::LocalServerHandle>>,
    app_type: String,
    model: Option<String>,
    bind_addr: Option<String>,
) -> Result<Value, String> {
    let addr = bind_addr.unwrap_or_else(|| DEFAULT_BIND_ADDR.to_string());
    let base_url = format!("http://{}/v1", addr);

    let cfg: ClientConfig = state.client_config.read().await.clone();

    match app_type.as_str() {
        "claude" => generate_claude_config(&base_url, &cfg.auth_token, model.as_deref()),
        "codex" => generate_codex_config(&base_url, &cfg.auth_token, model.as_deref()),
        "gemini" => generate_gemini_config(
            &format!("http://{}", addr),
            &cfg.auth_token,
            model.as_deref(),
        ),
        _ => Err(format!("unsupported app_type: {app_type}")),
    }
}

/// 获取本地代理地址（正在运行则返回实际地址，否则返回默认值）
#[tauri::command]
pub async fn get_consumer_proxy_addr(
    local_handle: tauri::State<'_, Arc<crate::commands::local_server::LocalServerHandle>>,
) -> Result<String, String> {
    let addr = local_handle.addr.lock().await;
    Ok(addr.map(|a| a.to_string()).unwrap_or_default())
}

// ---- Config generators ----

fn generate_claude_config(
    base_url: &str,
    auth_token: &str,
    model: Option<&str>,
) -> Result<Value, String> {
    let model = model.unwrap_or("claude-sonnet-4");
    Ok(json!({
        "id": "shareplan-consumer",
        "name": "SharePlan",
        "settingsConfig": {
            "env": {
                "ANTHROPIC_BASE_URL": base_url,
                "ANTHROPIC_AUTH_TOKEN": auth_token,
                "ANTHROPIC_MODEL": model,
            }
        },
        "category": "custom",
        "icon": "shareplan",
        "iconColor": "#6366f1",
    }))
}

fn generate_codex_config(
    base_url: &str,
    auth_token: &str,
    model: Option<&str>,
) -> Result<Value, String> {
    let model = model.unwrap_or("claude-sonnet-4");
    let base_url_no_v1 = base_url.trim_end_matches("/v1");
    let toml_config = format!(
        "model_provider = \"custom\"\nmodel = \"{model}\"\n\n[model_providers.custom]\nname = \"SharePlan\"\nbase_url = \"{base_url_no_v1}/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true"
    );
    Ok(json!({
        "id": "shareplan-consumer",
        "name": "SharePlan",
        "settingsConfig": {
            "auth": {
                "OPENAI_API_KEY": auth_token,
            },
            "config": toml_config,
        },
        "category": "custom",
        "icon": "shareplan",
        "iconColor": "#6366f1",
    }))
}

fn generate_gemini_config(
    base_url: &str,
    auth_token: &str,
    model: Option<&str>,
) -> Result<Value, String> {
    let model = model.unwrap_or("gemini-2.5-pro");
    Ok(json!({
        "id": "shareplan-consumer",
        "name": "SharePlan",
        "settingsConfig": {
            "env": {
                "GOOGLE_GEMINI_BASE_URL": base_url,
                "GEMINI_API_KEY": auth_token,
                "GEMINI_MODEL": model,
            },
            "config": {}
        },
        "category": "custom",
        "icon": "shareplan",
        "iconColor": "#6366f1",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_claude_config() {
        let config = generate_claude_config(
            "http://127.0.0.1:8081/v1",
            "my-token",
            Some("claude-opus-4"),
        )
        .unwrap();
        assert_eq!(config["id"], "shareplan-consumer");
        assert_eq!(
            config["settingsConfig"]["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:8081/v1"
        );
        assert_eq!(
            config["settingsConfig"]["env"]["ANTHROPIC_AUTH_TOKEN"],
            "my-token"
        );
        assert_eq!(
            config["settingsConfig"]["env"]["ANTHROPIC_MODEL"],
            "claude-opus-4"
        );
    }

    #[test]
    fn test_generate_claude_config_default_model() {
        let config = generate_claude_config("http://127.0.0.1:8081/v1", "tok", None).unwrap();
        assert_eq!(
            config["settingsConfig"]["env"]["ANTHROPIC_MODEL"],
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_generate_codex_config() {
        let config =
            generate_codex_config("http://127.0.0.1:8081/v1", "my-token", Some("gpt-4o")).unwrap();
        let toml_cfg = config["settingsConfig"]["config"].as_str().unwrap();
        assert!(toml_cfg.contains("model = \"gpt-4o\""));
        assert!(toml_cfg.contains("wire_api = \"responses\""));
        assert!(toml_cfg.contains("SharePlan"));
        assert_eq!(
            config["settingsConfig"]["auth"]["OPENAI_API_KEY"],
            "my-token"
        );
    }

    #[test]
    fn test_generate_gemini_config() {
        let config = generate_gemini_config(
            "http://127.0.0.1:8081",
            "my-token",
            Some("gemini-2.5-flash"),
        )
        .unwrap();
        assert_eq!(
            config["settingsConfig"]["env"]["GOOGLE_GEMINI_BASE_URL"],
            "http://127.0.0.1:8081"
        );
        assert_eq!(
            config["settingsConfig"]["env"]["GEMINI_MODEL"],
            "gemini-2.5-flash"
        );
    }

    #[test]
    fn test_role_round_trip() {
        let db = crate::database::ShareDb::memory().unwrap();
        assert!(matches!(db.load_role().unwrap(), Role::Idle));

        db.save_role(&Role::Consumer).unwrap();
        assert!(matches!(db.load_role().unwrap(), Role::Consumer));

        db.save_role(&Role::Supplier).unwrap();
        assert!(matches!(db.load_role().unwrap(), Role::Supplier));

        db.save_role(&Role::Idle).unwrap();
        assert!(matches!(db.load_role().unwrap(), Role::Idle));
    }
}
