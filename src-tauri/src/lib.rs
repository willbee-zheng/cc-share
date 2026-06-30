//! SharePlan — standalone Tauri app.
//!
//! cc-share is an independent desktop application. It relays LLM tasks between
//! the SharePlan cloud-server and the cc-switch local proxy (127.0.0.1:15721).
//! cc-share never touches provider API keys — cc-switch holds them and makes
//! the upstream call. See `INTEGRATION.md` / `INSTALL.md` for the architecture.
//!
//! Module layout mirrors the legacy plugin crate (`cc-share/src/`) which has
//! been physically migrated here in Phase 2.

pub mod auth;
pub mod commands;
pub mod ccswitch;
pub mod content_filter;
pub mod credits;
pub mod database;
pub mod diagnostics;
pub mod error;
pub mod local_server;
pub mod share;
pub mod stats;
pub mod system_log;
pub mod http_client;
pub mod url_utils;

use commands::providers::ProviderState;
use ccswitch::{CcSwitchProxyClient, ProxyExecutor, ProviderRegistry};
use database::ShareDb;
use diagnostics::Diagnostics;
use share::client::{ClientConfig, ConnectionState};
use share::daemon::Daemon;
use share::executor::SharedExecutor;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

/// SharePlan app shared state.
///
/// Injected via `app.manage()`; all IPC commands access it through
/// `tauri::State<'_, ShareState>`.
pub struct ShareState {
    pub db: Arc<ShareDb>,
    /// Current WebSocket connection state. `Arc<RwLock<…>>` so the daemon
    /// callback closure can hold a copy.
    pub connection_state: Arc<RwLock<ConnectionState>>,
    pub client_config: RwLock<ClientConfig>,
    /// Daemon started on `share_connect`, stopped on `share_disconnect`.
    pub daemon: Mutex<Daemon>,
}

/// Tauri event names exposed to the frontend.
pub mod events {
    pub const CONNECTION_STATE: &str = "share:connection-state";
    pub const CONNECTION_ERROR: &str = "share:connection-error";
    pub const TASK_FINISHED: &str = "share:task-finished";
    pub const ROLE_CHANGED: &str = "share:role-changed";
    /// 后台 batch writer 完成一次写入时 emit；payload 是新增条目数
    pub const LOG_APPENDED: &str = "share:log-appended";
    /// Emitted when auth state changes (browser login, token refresh, etc.)
    pub const AUTH_STATE_CHANGED: &str = "share:auth-state-changed";
}

/// Boot the SharePlan desktop app.
///
/// Phase 2 wires up the full IPC command set (migrated from the legacy
/// plugin). The injected executor is `NullExecutor` for now — Phase 5
/// replaces it with a `ProxyExecutor` that forwards to the cc-switch
/// local proxy.
pub fn run() {
    // 日志管道：在 Tauri 初始化前先注册全局 LOG_TX，确保 tauri_plugin_log 的
    // callback 一注册就能转发。batch writer 等 setup 拿到 db 后再启动。
    let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel::<system_log::LogEntry>();
    if system_log::set_log_sender(log_tx).is_err() {
        eprintln!("LOG_TX 已被初始化过");
    }
    let log_rx = std::sync::Arc::new(std::sync::Mutex::new(Some(log_rx)));

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: None }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Dispatch(
                        fern::Dispatch::new().chain(fern::Output::call(|record: &log::Record| {
                            system_log::dispatch_record(record);
                        })),
                    )),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::auth::get_auth_state,
            commands::auth::auth_register,
            commands::auth::auth_login,
            commands::auth::auth_logout,
            commands::auth::auth_refresh,
            commands::auth::auth_change_password,
            commands::auth::auth_get_profile,
            commands::auth::auth_create_api_key,
            commands::auth::auth_list_api_keys,
            commands::auth::auth_revoke_api_key,
            commands::auth::auth_browser_login,
            commands::share::get_share_settings,
            commands::share::upsert_share_settings,
            commands::share::get_all_sharing_providers,
            commands::share::delete_share_settings,
            commands::share::get_client_config,
            commands::share::set_client_config,
            commands::share::share_connect,
            commands::share::share_disconnect,
            commands::share::share_get_status,
            commands::share::check_server_health,
            commands::wallet::get_wallet,
            commands::wallet::update_wallet_balance,
            commands::wallet::get_recent_task_logs,
            commands::wallet::get_wallet_summary,
            commands::wallet::get_supplier_token_by_model,
            commands::wallet::get_consumer_token_by_model,
            commands::wallet::sync_wallet,
            commands::consume::share_consume,
            commands::consume::list_share_nodes,
            commands::consume_config::get_role,
            commands::consume_config::set_role,
            commands::consume_config::generate_consumer_config,
            commands::consume_config::get_consumer_proxy_addr,
            commands::providers::refresh_providers,
            commands::providers::list_proxy_providers,
            commands::providers::get_diagnostics,
            commands::providers::get_whitelist,
            commands::providers::set_whitelist,
            commands::providers::get_shareable_models,
            commands::local_server::start_local_server,
            commands::local_server::stop_local_server,
            commands::local_server::get_local_server_addr,
            commands::system_log::get_system_logs,
            commands::system_log::clear_system_logs,
            commands::system_log::get_system_log_stats,
            commands::system_log::list_system_log_targets,
            commands::system_log::set_log_level,
            commands::system_log::prune_system_logs,
            commands::stats::sync_daily_stats,
            commands::stats::get_cloud_stats_summary,
        ])
        .setup(move |app| {
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            let db = ShareDb::init(&config_dir)
                .map_err(|e| tauri::Error::Setup(Box::<dyn std::error::Error>::from(e).into()))?;
            let db = Arc::new(db);
            // cc-switch proxy discovery + diagnostics + forwarding executor (Phase 4/5).
            let proxy_client = CcSwitchProxyClient::default();
            let registry = ProviderRegistry::new(proxy_client.clone());
            let diagnostics = Diagnostics::new();
            let executor: SharedExecutor =
                ProxyExecutor::new(proxy_client) as SharedExecutor;
            let daemon = Daemon::new(db.clone(), executor, "default".into(), registry.clone());
            let db_for_providers = db.clone();

            // 启动 system_log batch writer：从 log_rx 取出待写日志批量入库，
            // 每次 flush 后通过 LOG_APPENDED 事件通知前端实时刷新。
            let rx_option = log_rx.lock().unwrap().take();
            if let Some(rx) = rx_option {
                let app_for_log = app.handle().clone();
                system_log::start_batch_writer(
                    db.clone(),
                    rx,
                    Box::new(move |n: usize| {
                        if n > 0 {
                            let _ = app_for_log.emit(events::LOG_APPENDED, n);
                        }
                    }),
                );
            }

            // Start background auto-refresh for access tokens.
            // Use tauri::async_runtime::spawn because the setup closure may run
            // on a non-Tokio thread; async_runtime guarantees the correct handle.
            let db_for_refresh = db.clone();
            let app_for_refresh = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                auth::refresh::start_auto_refresh(app_for_refresh, db_for_refresh).await;
            });

            // Local OpenAI-compatible consumer server (Phase 9). Not started
            // yet — user invokes `start_local_server` from the UI.
            let local_state = local_server::LocalServerState::new(
                String::new(),
                String::new(),
                String::new(),
                db.clone(),
            );

            app.manage(ShareState {
                db,
                connection_state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
                client_config: RwLock::new(ClientConfig::default()),
                daemon: Mutex::new(daemon),
            });

            app.manage(ProviderState {
                registry,
                diagnostics,
                db: db_for_providers,
            });

            app.manage(commands::local_server::LocalServerHandle::new(local_state));

            log::info!("✓ SharePlan standalone app initialized (share.db + cc-switch proxy executor)");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running SharePlan application");
}
