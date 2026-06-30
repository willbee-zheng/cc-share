//! 统计同步 Tauri 命令
//!
//! 前端通过这些命令触发本地聚合→云端同步，和拉取云端权威摘要。

use crate::auth::token;
use crate::database::dao_sync::SyncResult;
use crate::stats::sync::StatsSyncer;
use crate::ShareState;

/// 触发本地 P2P 任务日志聚合到 daily_sync_log，然后推送到云端。
///
/// 返回同步结果：推送条目数、云端接受数、权威摘要。
#[tauri::command]
pub async fn sync_daily_stats(
    state: tauri::State<'_, ShareState>,
) -> Result<SyncResult, String> {
    let config = state.client_config.read().await;
    let server_host = config.server_host.clone();
    let use_https = config.use_https;
    drop(config);

    // Need auth state for access token
    let auth_state = token::load_auth_state(&state.db).map_err(|e| e.to_string())?;
    let auth = auth_state.ok_or("Not logged in. Please sign in to sync stats.")?;

    if server_host.is_empty() {
        return Err("Server host not configured. Please set it in Settings.".into());
    }

    let syncer = StatsSyncer::new(state.db.clone());
    let result = syncer.sync_to_cloud(&server_host, &auth.access_token, use_https).await?;

    Ok(result)
}

/// 从云端拉取权威统计摘要。
///
/// 返回的 CloudStatsSummary 包含云端根据 billing 记录计算的权威数据，
/// 用于钱包/统计面板展示。本地数据仅供参考，不作为积分依据。
#[tauri::command]
pub async fn get_cloud_stats_summary(
    state: tauri::State<'_, ShareState>,
) -> Result<crate::database::dao_sync::CloudStatsSummary, String> {
    let config = state.client_config.read().await;
    let server_host = config.server_host.clone();
    let use_https = config.use_https;
    drop(config);

    let auth_state = token::load_auth_state(&state.db).map_err(|e| e.to_string())?;
    let auth = auth_state.ok_or("Not logged in")?;

    if server_host.is_empty() {
        return Err("Server host not configured".into());
    }

    let syncer = StatsSyncer::new(state.db.clone());
    syncer.fetch_cloud_summary(&server_host, &auth.access_token, use_https).await
}