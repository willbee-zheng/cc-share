//! 钱包相关 Tauri 命令
//!
//! 前端通过这些命令查询积分余额、任务日志、按模型聚合的 token 统计等。
//!
//! 注意：本系统 token 是统计主单位，积分（credits）由云端清算，本地仅做镜像。
//! 因此 today_* 与 hourly_trend 都以 token 数为口径。

use crate::database::dao_credits::{ModelTokenStat, P2PTaskLog, UserWallet};
use crate::credits::pricing::PricingEntry;
use crate::ShareState;

use serde::Deserialize;

#[tauri::command]
pub async fn get_wallet(
    state: tauri::State<'_, ShareState>,
    user_id: String,
) -> Result<UserWallet, String> {
    log::debug!("get_wallet: user_id={}", user_id);
    state
        .db
        .get_or_create_wallet(&user_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_wallet_balance(
    state: tauri::State<'_, ShareState>,
    user_id: String,
    earned_delta: f64,
    spent_delta: f64,
) -> Result<(), String> {
    log::info!("update_wallet_balance: user_id={}, earned_delta={}, spent_delta={}", user_id, earned_delta, spent_delta);
    state
        .db
        .update_wallet_balance(&user_id, earned_delta, spent_delta)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_recent_task_logs(
    state: tauri::State<'_, ShareState>,
    direction: Option<String>,
    limit: i32,
) -> Result<Vec<P2PTaskLog>, String> {
    let limit = limit.clamp(1, 100);
    state
        .db
        .get_recent_p2p_task_logs(direction.as_deref(), limit)
        .map_err(|e| e.to_string())
}

/// Cloud wallet data returned by GET /api/v1/user/profile.
#[derive(Debug, Deserialize)]
struct CloudWallet {
    balance: String,
    total_earned: String,
    total_spent: String,
}

#[derive(Debug, Deserialize)]
struct CloudProfileResponse {
    wallet: CloudWallet,
}

/// Sync wallet balance from the cloud server.
///
/// Reads the auth token from the stored auth state, calls the cloud
/// `/api/v1/user/profile` endpoint to update the local wallet, and then
/// pulls recent settlement receipts from `/api/v1/settlements/recent`
/// to update task credits for any receipts missed during WS disconnection.
///
/// Automatically retries with HTTPS if the initial HTTP request returns 403,
/// which indicates a production server that requires TLS.
#[tauri::command]
pub async fn sync_wallet(state: tauri::State<'_, ShareState>) -> Result<UserWallet, String> {
    // Load auth state for the access token and user ID.
    let auth = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| format!("load auth state: {e}"))?
        .ok_or("Not logged in — please sign in first")?;

    log::info!("sync_wallet: syncing wallet for user={}", auth.user_id);

    // Build the cloud base URL from config, respecting the use_https flag.
    let cfg = state.client_config.read().await.clone();
    let cloud_base = crate::url_utils::build_http_base_with_tls(&cfg.server_host, cfg.use_https);
    if cloud_base.is_empty() {
        return Err("Server host not configured".to_string());
    }

    // Step 1: Sync wallet balance from profile endpoint.
    let profile_url = format!("{}/api/v1/user/profile", cloud_base.trim_end_matches('/'));
    let client = crate::http_client::shareplan_client();
    let resp = fetch_profile(&client, &profile_url, &auth.access_token).await?;

    let profile: CloudProfileResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse profile response: {e}"))?;

    // Parse decimal strings to f64.
    let balance: f64 = profile.wallet.balance.parse().unwrap_or(0.0);
    let total_earned: f64 = profile.wallet.total_earned.parse().unwrap_or(0.0);
    let total_spent: f64 = profile.wallet.total_spent.parse().unwrap_or(0.0);

    log::info!(
        "sync_wallet: cloud balance={balance}, earned={total_earned}, spent={total_spent} for user={}",
        auth.user_id
    );

    // Update the local wallet with cloud data.
    state
        .db
        .set_wallet_from_cloud(&auth.user_id, balance, total_earned, total_spent)
        .map_err(|e| e.to_string())?;

    // Step 2: Pull recent settlement receipts to catch up on missed WS pushes.
    let last_sync = state
        .db
        .get_config("last_settlement_sync")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    let receipts_url = format!(
        "{}/api/v1/settlements/recent?after={}&limit=100",
        cloud_base.trim_end_matches('/'),
        last_sync
    );

    match fetch_settlements(&client, &receipts_url, &auth.access_token).await {
        Ok(receipts) => {
            let mut max_ts = last_sync;
            for r in &receipts {
                if let Err(e) = state.db.update_task_credits(&r.task_id, r.credits) {
                    log::warn!("sync_wallet: update_task_credits({}): {e}", r.task_id);
                }
                if r.timestamp > max_ts {
                    max_ts = r.timestamp;
                }
            }
            // Save the latest receipt timestamp for incremental sync next time.
            if max_ts > last_sync {
                if let Err(e) = state.db.set_config("last_settlement_sync", &max_ts.to_string()) {
                    log::warn!("sync_wallet: save last_settlement_sync: {e}");
                }
            }
            log::info!("sync_wallet: processed {} settlement receipts", receipts.len());
        }
        Err(e) => {
            log::warn!("sync_wallet: failed to fetch settlements (non-fatal): {e}");
        }
    }

    // Return the updated wallet.
    state
        .db
        .get_or_create_wallet(&auth.user_id)
        .map_err(|e| e.to_string())
}

/// Send a GET request with Authorization header.
async fn fetch_profile(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<reqwest::Response, String> {
    log::info!("sync_wallet: GET {}", url);

    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            log::error!("sync_wallet: request to {} failed: {e}", url);
            format!("cloud request failed: {e}")
        })?;

    if resp.status().is_success() {
        return Ok(resp);
    }

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    log::error!(
        "sync_wallet: {} returned {}: {}",
        url,
        status,
        &body[..body.len().min(200)]
    );
    Err(format!(
        "cloud returned {}: {}",
        status,
        &body[..body.len().min(200)]
    ))
}

/// A settlement receipt from the cloud `/settlements/recent` endpoint.
#[derive(Debug, serde::Deserialize)]
struct CloudSettlementReceipt {
    task_id: String,
    direction: String,
    credits: f64,
    timestamp: i64,
}

/// Response from the settlements/recent endpoint.
#[derive(Debug, serde::Deserialize)]
struct SettlementsResponse {
    receipts: Vec<CloudSettlementReceipt>,
}

/// Fetch settlement receipts from cloud.
async fn fetch_settlements(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<Vec<CloudSettlementReceipt>, String> {
    log::info!("sync_wallet: GET {}", url);

    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("fetch settlements: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("settlements returned {}: {}", status, &body[..body.len().min(200)]));
    }

    let settlements: SettlementsResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse settlements response: {e}"))?;

    Ok(settlements.receipts)
}

/// 钱包摘要：余额 + 24h 供应/消费 token + 趋势点 + 最近流水
#[derive(Debug, serde::Serialize)]
pub struct WalletSummary {
    pub wallet: UserWallet,
    /// 今日（近 24h）供应 token 总量
    pub today_supplied_tokens: i64,
    /// 今日（近 24h）消费 token 总量
    pub today_consumed_tokens: i64,
    /// 累计供应 token 总量（不限时间窗口）
    pub total_supplied_tokens: i64,
    /// 累计消费 token 总量
    pub total_consumed_tokens: i64,
    /// 24 小时按小时聚合的趋势点（24 个，从 23 小时前到现在）
    pub hourly_trend: Vec<HourlyPoint>,
    /// 最近 50 条流水（混合方向）
    pub recent_logs: Vec<P2PTaskLog>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct HourlyPoint {
    /// Unix 时间戳（小时桶起点，秒）
    pub bucket_unix: i64,
    pub supplied_tokens: i64,
    pub consumed_tokens: i64,
}

#[tauri::command]
pub async fn get_wallet_summary(
    state: tauri::State<'_, ShareState>,
    user_id: String,
) -> Result<WalletSummary, String> {
    let wallet = state
        .db
        .get_or_create_wallet(&user_id)
        .map_err(|e| e.to_string())?;
    let recent = state
        .db
        .get_recent_p2p_task_logs(None, 50)
        .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp();
    let one_day_ago = now - 24 * 3600;

    // 初始化 24 个小时桶
    let mut buckets: Vec<HourlyPoint> = (0..24)
        .map(|i| HourlyPoint {
            bucket_unix: one_day_ago + (i as i64) * 3600,
            supplied_tokens: 0,
            consumed_tokens: 0,
        })
        .collect();

    // 全量取近 24h 内的 supply/consume 日志做汇总（避免 50 条上限）
    let logs_24h = state
        .db
        .get_recent_p2p_task_logs(None, 1000)
        .map_err(|e| e.to_string())?;

    let mut today_supplied: i64 = 0;
    let mut today_consumed: i64 = 0;

    for log in &logs_24h {
        if log.created_at < one_day_ago {
            continue;
        }
        // 只统计 completed 任务，避免失败/拒绝任务的 token 噪声
        if log.status != "completed" {
            continue;
        }
        let tokens = (log.tokens_prompt as i64) + (log.tokens_completion as i64);
        let idx = ((log.created_at - one_day_ago) / 3600).clamp(0, 23) as usize;
        match log.direction.as_str() {
            "supply" => {
                buckets[idx].supplied_tokens += tokens;
                today_supplied += tokens;
            }
            "consume" => {
                buckets[idx].consumed_tokens += tokens;
                today_consumed += tokens;
            }
            _ => {}
        }
    }

    let (sup_prompt, sup_comp) = state
        .db
        .get_token_totals_by_direction("supply", None)
        .map_err(|e| e.to_string())?;
    let (con_prompt, con_comp) = state
        .db
        .get_token_totals_by_direction("consume", None)
        .map_err(|e| e.to_string())?;

    Ok(WalletSummary {
        wallet,
        today_supplied_tokens: today_supplied,
        today_consumed_tokens: today_consumed,
        total_supplied_tokens: sup_prompt + sup_comp,
        total_consumed_tokens: con_prompt + con_comp,
        hourly_trend: buckets,
        recent_logs: recent,
    })
}

/// 供应商按模型聚合的 token 统计。
///
/// `days` = 0 表示全量；否则统计最近 N 天（按 created_at >= now - days*86400 过滤）。
#[tauri::command]
pub async fn get_supplier_token_by_model(
    state: tauri::State<'_, ShareState>,
    days: i32,
) -> Result<Vec<ModelTokenStat>, String> {
    let since = since_from_days(days);
    state
        .db
        .get_token_stats_by_direction("supply", since)
        .map_err(|e| e.to_string())
}

/// 消费者按模型聚合的 token 统计。
#[tauri::command]
pub async fn get_consumer_token_by_model(
    state: tauri::State<'_, ShareState>,
    days: i32,
) -> Result<Vec<ModelTokenStat>, String> {
    let since = since_from_days(days);
    state
        .db
        .get_token_stats_by_direction("consume", since)
        .map_err(|e| e.to_string())
}

fn since_from_days(days: i32) -> Option<i64> {
    if days <= 0 {
        None
    } else {
        let now = chrono::Utc::now().timestamp();
        Some(now - (days as i64) * 86400)
    }
}

/// Fetch pricing rules from the cloud server and update the local cache.
///
/// Returns the list of pricing entries after refresh.
#[tauri::command]
pub async fn fetch_pricing(state: tauri::State<'_, ShareState>) -> Result<Vec<PricingEntry>, String> {
    let auth = crate::auth::token::load_auth_state(&state.db)
        .map_err(|e| format!("load auth state: {e}"))?
        .ok_or("Not logged in")?;

    let cfg = state.client_config.read().await.clone();
    let cloud_base = crate::url_utils::build_http_base_with_tls(&cfg.server_host, cfg.use_https);
    if cloud_base.is_empty() {
        return Err("Server host not configured".to_string());
    }

    state
        .pricing
        .refresh_from_cloud(&state.db, &cfg.server_host, &auth.access_token, cfg.use_https)
        .await?;

    Ok(state.pricing.all_pricings())
}

/// Return the current in-memory pricing entries (no network call).
#[tauri::command]
pub async fn get_pricing(state: tauri::State<'_, ShareState>) -> Result<Vec<PricingEntry>, String> {
    Ok(state.pricing.all_pricings())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_from_days_zero_is_none() {
        assert!(since_from_days(0).is_none());
        assert!(since_from_days(-1).is_none());
    }

    #[test]
    fn since_from_days_positive_is_some() {
        let s = since_from_days(7).unwrap();
        // 不验证具体值（依赖当前时间），只验证是过去的时间戳
        assert!(s < chrono::Utc::now().timestamp());
    }
}
