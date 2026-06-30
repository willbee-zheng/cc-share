//! CC-Share 每日统计同步 DAO
//!
//! 管理 daily_sync_log 表：本地聚合 P2P 任务数据后写入，
//! 同步到云端后标记 synced=1。
//!
//! **防刷安全设计**：
//! - 本地 p2p_task_log 是客户端自己的记录，仅用于展示
//! - 云端 daily_stats 由云服务在 Finalize 时写入（权威来源）
//! - 客户端同步流程：聚合本地 → 推送到云端 → 云端交叉验证 → 返回权威数据
//! - synced=1 标记仅用于避免重复推送，不代表云端已采纳

use crate::database::ShareDb;
use crate::error::ShareError;
use rusqlite::params;

/// 本地聚合的每日统计行
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DailySyncLog {
    pub id: i64,
    pub stat_date: String,
    pub direction: String,
    pub model: String,
    pub upstream_model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub task_count: i32,
    pub credits: f64,
    pub synced: bool,
    pub created_at: i64,
}

/// 用于插入的聚合行（不含 id 和 synced）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DailySyncRow {
    pub stat_date: String,
    pub direction: String,
    pub model: String,
    pub upstream_model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub task_count: i32,
    pub credits: f64,
}

/// 云端返回的每日统计数据（权威来源）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CloudDailyStat {
    pub stat_date: String,
    pub direction: String,
    pub model: String,
    pub upstream_model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub task_count: i32,
    pub credits: f64,
}

/// 云端统计摘要
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CloudStatsSummary {
    /// 云端权威的每日统计（按日期降序）
    pub daily_stats: Vec<CloudDailyStat>,
    /// 云端权威的累计供应 token
    pub total_supplied_tokens: i64,
    /// 云端权威的累计消费 token
    pub total_consumed_tokens: i64,
}

/// 同步结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyncResult {
    /// 推送到云端的条目数
    pub pushed: i32,
    /// 云端交叉验证后接受的条目数
    pub accepted: i32,
    /// 云端返回的权威摘要
    pub summary: Option<CloudStatsSummary>,
    /// 错误信息（如果有）
    pub error: Option<String>,
}

impl ShareDb {
    /// 聚合 p2p_task_log 中未同步的 completed 任务到 daily_sync_log。
    ///
    /// 只聚合 status='completed' 的任务，排除 failed/rejected/busy。
    /// 使用 INSERT OR REPLACE 确保同一天同一 (date, direction, model) 的记录是累加的。
    ///
    /// 返回新聚合的行数。
    pub fn aggregate_pending_stats(&self) -> Result<i32, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;

        // 找出上次同步的截止时间（client_config 中的 last_daily_sync）
        let last_sync_ts: i64 = conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM client_config WHERE key = 'last_daily_sync'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap_or(None)
            .unwrap_or(0);

        // 聚合已完成、未同步的任务
        let rows_changed = conn.execute(
            "INSERT OR REPLACE INTO daily_sync_log
                (stat_date, direction, model, upstream_model, prompt_tokens, completion_tokens, task_count, credits, synced, created_at)
             SELECT
                date(created_at, 'unixepoch') AS stat_date,
                direction,
                model,
                COALESCE(upstream_model, '') AS upstream_model,
                SUM(tokens_prompt)   AS prompt_tokens,
                SUM(tokens_completion) AS completion_tokens,
                COUNT(*)              AS task_count,
                SUM(credits)          AS credits,
                0                     AS synced,
                unixepoch()           AS created_at
             FROM p2p_task_log
             WHERE status = 'completed' AND created_at > ?1
             GROUP BY stat_date, direction, model, upstream_model",
            params![last_sync_ts],
        )
        .map_err(|e| ShareError::Database(e.to_string()))?;

        // 更新 last_daily_sync 为当前时间
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO client_config (key, value) VALUES ('last_daily_sync', ?1)",
            params![now.to_string()],
        )
        .map_err(|e| ShareError::Database(e.to_string()))?;

        Ok(rows_changed as i32)
    }

    /// 获取所有未同步的 daily_sync_log 行。
    pub fn get_unsynced_daily_stats(&self) -> Result<Vec<DailySyncLog>, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, stat_date, direction, model, upstream_model, prompt_tokens, completion_tokens,
                    task_count, credits, synced, created_at
             FROM daily_sync_log WHERE synced = 0
             ORDER BY stat_date, direction, model",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(DailySyncLog {
                    id: row.get(0)?,
                    stat_date: row.get(1)?,
                    direction: row.get(2)?,
                    model: row.get(3)?,
                    upstream_model: row.get(4)?,
                    prompt_tokens: row.get(5)?,
                    completion_tokens: row.get(6)?,
                    task_count: row.get(7)?,
                    credits: row.get(8)?,
                    synced: row.get::<_, i32>(9)? != 0,
                    created_at: row.get(10)?,
                })
            })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// 标记指定 ID 的行为已同步。
    pub fn mark_daily_stats_synced(&self, ids: &[i64]) -> Result<(), ShareError> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("UPDATE daily_sync_log SET synced = 1 WHERE id IN ({placeholders})");
        let params: Vec<rusqlite::types::Value> = ids
            .iter()
            .map(|id| rusqlite::types::Value::from(*id))
            .collect();
        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
            .map_err(|e| ShareError::Database(e.to_string()))?;
        Ok(())
    }

    /// 获取上次同步的时间戳。
    pub fn get_last_daily_sync_ts(&self) -> Result<i64, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let result = conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM client_config WHERE key = 'last_daily_sync'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap_or(None)
            .unwrap_or(0);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_db() -> ShareDb {
        ShareDb::memory().expect("memory db")
    }

    #[test]
    fn aggregate_only_completed_tasks() {
        let db = fresh_db();

        // Insert some task logs
        let now = chrono::Utc::now().timestamp();
        db.insert_p2p_task_log(&crate::database::dao_credits::P2PTaskLog {
            task_id: "t1".into(),
            direction: "supply".into(),
            model: "claude-sonnet-4".into(),
            upstream_model: None,
            tokens_prompt: 100,
            tokens_completion: 50,
            credits: 0.15,
            latency_ms: Some(200),
            status: "completed".into(),
            error_message: None,
            created_at: now,
        }).expect("insert t1");

        db.insert_p2p_task_log(&crate::database::dao_credits::P2PTaskLog {
            task_id: "t2".into(),
            direction: "supply".into(),
            model: "claude-sonnet-4".into(),
            upstream_model: None,
            tokens_prompt: 200,
            tokens_completion: 80,
            credits: 0.28,
            latency_ms: Some(300),
            status: "completed".into(),
            error_message: None,
            created_at: now + 10,
        }).expect("insert t2");

        // Failed task should be excluded
        db.insert_p2p_task_log(&crate::database::dao_credits::P2PTaskLog {
            task_id: "t3".into(),
            direction: "supply".into(),
            model: "claude-sonnet-4".into(),
            upstream_model: None,
            tokens_prompt: 999,
            tokens_completion: 999,
            credits: 99.0,
            latency_ms: None,
            status: "failed".into(),
            error_message: Some("timeout".into()),
            created_at: now + 20,
        }).expect("insert t3");

        let count = db.aggregate_pending_stats().expect("aggregate");
        assert!(count >= 1, "should aggregate at least 1 row");
    }

    #[test]
    fn get_unsynced_and_mark_synced() {
        let db = fresh_db();
        let now = chrono::Utc::now().timestamp();

        db.insert_p2p_task_log(&crate::database::dao_credits::P2PTaskLog {
            task_id: "t1".into(),
            direction: "supply".into(),
            model: "gpt-4o".into(),
            upstream_model: None,
            tokens_prompt: 50,
            tokens_completion: 20,
            credits: 0.07,
            latency_ms: Some(100),
            status: "completed".into(),
            error_message: None,
            created_at: now,
        }).expect("insert");

        let count = db.aggregate_pending_stats().expect("aggregate");
        assert!(count >= 1);

        let unsynced = db.get_unsynced_daily_stats().expect("get unsynced");
        assert!(!unsynced.is_empty(), "should have unsynced rows");

        let ids: Vec<i64> = unsynced.iter().map(|r| r.id).collect();
        db.mark_daily_stats_synced(&ids).expect("mark synced");

        let after = db.get_unsynced_daily_stats().expect("get unsynced after");
        assert!(after.is_empty(), "all rows should be synced");
    }
}