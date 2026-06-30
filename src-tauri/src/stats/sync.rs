//! CC-Share 统计同步器
//!
//! 负责将本地 P2P 任务统计聚合后推送到云端，并拉取云端权威摘要。
//!
//! **防刷机制**：
//! - 只聚合 status='completed' 的任务（排除 failed/rejected/busy）
//! - 云端交叉验证：提交的 stats 会与云端 billing 记录比对，超出部分被拒绝
//! - 本地 synced=1 标记仅用于避免重复推送，不代表云端已采纳
//! - 钱包/统计面板展示的是云端拉回的权威数据，而非本地数据

use crate::database::dao_sync::{CloudStatsSummary, DailySyncRow, SyncResult};
use crate::database::ShareDb;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

/// 云端统计同步 API 响应
#[derive(Debug, Deserialize)]
struct SyncResponse {
    synced: i32,
    skipped: i32,
    /// 云端交叉验证后拒绝的条目数
    rejected: Option<i32>,
    /// 云端返回的权威摘要
    summary: Option<CloudStatsSummary>,
}

#[derive(Debug, Deserialize)]
struct StatsSummaryResponse {
    summary: CloudStatsSummary,
}

/// 统计同步器
pub struct StatsSyncer {
    db: Arc<ShareDb>,
    http: Client,
}

impl StatsSyncer {
    pub fn new(db: Arc<ShareDb>) -> Self {
        Self {
            db,
            http: crate::http_client::shareplan_client_builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// 聚合本地未同步的 completed 任务到 daily_sync_log。
    ///
    /// 只聚合 status='completed' 的任务，排除 failed/rejected/busy。
    /// 聚合后更新 last_daily_sync 时间戳。
    pub fn aggregate(&self) -> Result<i32, String> {
        self.db.aggregate_pending_stats().map_err(|e| e.to_string())
    }

    /// 将未同步的本地聚合数据推送到云端，并拉取云端权威摘要。
    ///
    /// 流程：
    /// 1. 从 daily_sync_log 取出所有 synced=0 的行
    /// 2. 推送到 POST /api/v1/user/stats/sync
    /// 3. 云端交叉验证后会返回 accepted/rejected 数量
    /// 4. 标记推送的本地行为 synced=1（避免重复推送）
    /// 5. 返回同步结果（含云端权威摘要）
    pub async fn sync_to_cloud(
        &self,
        server_host: &str,
        access_token: &str,
        use_https: bool,
    ) -> Result<SyncResult, String> {
        // Step 1: Aggregate any new tasks
        let aggregated = self.aggregate()?;
        log::info!("Stats sync: aggregated {aggregated} new daily stat rows");

        // Step 2: Get unsynced rows
        let unsynced = self
            .db
            .get_unsynced_daily_stats()
            .map_err(|e| e.to_string())?;

        if unsynced.is_empty() {
            log::info!("Stats sync: nothing to push");
            return Ok(SyncResult {
                pushed: 0,
                accepted: 0,
                summary: None,
                error: None,
            });
        }

        // Step 3: Convert to sync request format
        let stats: Vec<DailySyncRow> = unsynced
            .iter()
            .map(|row| DailySyncRow {
                stat_date: row.stat_date.clone(),
                direction: row.direction.clone(),
                model: row.model.clone(),
                upstream_model: row.upstream_model.clone(),
                prompt_tokens: row.prompt_tokens,
                completion_tokens: row.completion_tokens,
                task_count: row.task_count,
                credits: row.credits,
            })
            .collect();

        let body = serde_json::json!({ "stats": stats });

        let base_url = build_base_url(server_host, use_https);
        let url = format!("{base_url}/api/v1/user/stats/sync");

        // Step 4: POST to cloud
        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Stats sync request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Stats sync failed ({status}): {body}"));
        }

        let sync_resp: SyncResponse = response
            .json()
            .await
            .map_err(|e| format!("Stats sync response parse failed: {e}"))?;

        // Step 5: Mark local rows as synced
        let ids: Vec<i64> = unsynced.iter().map(|r| r.id).collect();
        self.db.mark_daily_stats_synced(&ids).map_err(|e| e.to_string())?;

        log::info!(
            "Stats sync: pushed {}, accepted {}, rejected {:?}",
            unsynced.len(),
            sync_resp.synced,
            sync_resp.rejected,
        );

        Ok(SyncResult {
            pushed: unsynced.len() as i32,
            accepted: sync_resp.synced,
            summary: sync_resp.summary,
            error: None,
        })
    }

    /// 从云端拉取权威统计摘要。
    ///
    /// 返回的 CloudStatsSummary 包含云端根据 billing 记录计算的权威数据，
    /// 用于钱包/统计面板展示。
    pub async fn fetch_cloud_summary(
        &self,
        server_host: &str,
        access_token: &str,
        use_https: bool,
    ) -> Result<CloudStatsSummary, String> {
        let base_url = build_base_url(server_host, use_https);
        let url = format!("{base_url}/api/v1/user/stats/summary");

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Stats summary request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Stats summary failed ({status}): {body}"));
        }

        let result: StatsSummaryResponse = response
            .json()
            .await
            .map_err(|e| format!("Stats summary parse failed: {e}"))?;

        Ok(result.summary)
    }
}

/// Build the HTTP base URL from a server host string.
/// Respects explicit http:// or https:// prefixes.
fn build_base_url(host: &str, use_https: bool) -> String {
    crate::url_utils::build_http_base_with_tls(host, use_https)
}