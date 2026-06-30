//! CC-Share 积分钱包与任务日志 DAO
//!
//! 管理本地积分余额和 P2P 任务审计日志

use crate::database::ShareDb;
use crate::error::ShareError;
use rusqlite::params;

/// 本地钱包信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserWallet {
    pub user_id: String,
    pub balance_credits: f64,
    pub total_earned: f64,
    pub total_spent: f64,
    pub last_sync_at: Option<i64>,
}

/// P2P 任务日志
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct P2PTaskLog {
    pub task_id: String,
    pub direction: String,
    pub model: String,
    pub upstream_model: Option<String>,
    pub tokens_prompt: i32,
    pub tokens_completion: i32,
    pub credits: f64,
    pub latency_ms: Option<i32>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: i64,
}

/// 云端节点缓存
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShareNode {
    pub node_id: String,
    pub models: String,
    pub price: f64,
    pub status: String,
    pub latency_ms: Option<i32>,
    pub last_heartbeat: Option<i64>,
}

/// 按模型聚合的 token 统计（单方向）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelTokenStat {
    pub model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub task_count: i64,
}

impl ShareDb {
    // ---- Wallet ----

    /// 获取本地钱包，不存在则创建默认
    pub fn get_or_create_wallet(&self, user_id: &str) -> Result<UserWallet, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let result = conn.query_row(
            "SELECT user_id, balance_credits, total_earned, total_spent, last_sync_at
             FROM user_wallet WHERE user_id = ?1",
            params![user_id],
            |row| {
                Ok(UserWallet {
                    user_id: row.get(0)?,
                    balance_credits: row.get(1)?,
                    total_earned: row.get(2)?,
                    total_spent: row.get(3)?,
                    last_sync_at: row.get(4)?,
                })
            },
        );

        match result {
            Ok(wallet) => Ok(wallet),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "INSERT INTO user_wallet (user_id) VALUES (?1)",
                    params![user_id],
                )?;
                Ok(UserWallet {
                    user_id: user_id.to_string(),
                    balance_credits: 0.0,
                    total_earned: 0.0,
                    total_spent: 0.0,
                    last_sync_at: None,
                })
            }
            Err(e) => Err(ShareError::Database(e.to_string())),
        }
    }

    /// 更新钱包余额（原子操作）
    pub fn update_wallet_balance(
        &self,
        user_id: &str,
        earned_delta: f64,
        spent_delta: f64,
    ) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE user_wallet SET
                balance_credits = balance_credits + ?1 - ?2,
                total_earned = total_earned + ?1,
                total_spent = total_spent + ?2,
                last_sync_at = unixepoch()
             WHERE user_id = ?3",
            params![earned_delta, spent_delta, user_id],
        )?;
        Ok(())
    }

    /// Set wallet balance from cloud data (absolute values, not deltas).
    /// Creates the row if it doesn't exist.
    pub fn set_wallet_from_cloud(
        &self,
        user_id: &str,
        balance: f64,
        total_earned: f64,
        total_spent: f64,
    ) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO user_wallet (user_id, balance_credits, total_earned, total_spent, last_sync_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())
             ON CONFLICT(user_id) DO UPDATE SET
                balance_credits = ?2,
                total_earned = ?3,
                total_spent = ?4,
                last_sync_at = unixepoch()",
            params![user_id, balance, total_earned, total_spent],
        )?;
        Ok(())
    }

    // ---- P2P Task Log ----

    /// 插入任务日志
    pub fn insert_p2p_task_log(&self, log: &P2PTaskLog) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO p2p_task_log (task_id, direction, model, upstream_model, tokens_prompt,
                    tokens_completion, credits, latency_ms, status, error_message, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                log.task_id,
                log.direction,
                log.model,
                log.upstream_model,
                log.tokens_prompt,
                log.tokens_completion,
                log.credits,
                log.latency_ms,
                log.status,
                log.error_message,
                log.created_at,
            ],
        )?;
        Ok(())
    }

    /// 更新任务状态
    pub fn update_p2p_task_status(
        &self,
        task_id: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE p2p_task_log SET status = ?1, error_message = ?2 WHERE task_id = ?3",
            params![status, error_message, task_id],
        )?;
        Ok(())
    }

    /// 查询最近的任务日志
    pub fn get_recent_p2p_task_logs(
        &self,
        direction: Option<&str>,
        limit: i32,
    ) -> Result<Vec<P2PTaskLog>, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let mut logs = Vec::new();

        let sql = match direction {
            Some(_) =>
                "SELECT task_id, direction, model, upstream_model, tokens_prompt, tokens_completion,
                        credits, latency_ms, status, error_message, created_at
                 FROM p2p_task_log WHERE direction = ?1
                 ORDER BY created_at DESC LIMIT ?2",
            None =>
                "SELECT task_id, direction, model, upstream_model, tokens_prompt, tokens_completion,
                        credits, latency_ms, status, error_message, created_at
                 FROM p2p_task_log
                 ORDER BY created_at DESC LIMIT ?1",
        };

        match direction {
            Some(dir) => {
                let mut stmt = conn.prepare(sql)?;
                let rows = stmt.query_map(params![dir, limit], |row| {
                    Ok(P2PTaskLog {
                        task_id: row.get(0)?,
                        direction: row.get(1)?,
                        model: row.get(2)?,
                        upstream_model: row.get(3)?,
                        tokens_prompt: row.get(4)?,
                        tokens_completion: row.get(5)?,
                        credits: row.get(6)?,
                        latency_ms: row.get(7)?,
                        status: row.get(8)?,
                        error_message: row.get(9)?,
                        created_at: row.get(10)?,
                    })
                })?;
                for row in rows {
                    logs.push(row?);
                }
            }
            None => {
                let mut stmt = conn.prepare(sql)?;
                let rows = stmt.query_map(params![limit], |row| {
                    Ok(P2PTaskLog {
                        task_id: row.get(0)?,
                        direction: row.get(1)?,
                        model: row.get(2)?,
                        upstream_model: row.get(3)?,
                        tokens_prompt: row.get(4)?,
                        tokens_completion: row.get(5)?,
                        credits: row.get(6)?,
                        latency_ms: row.get(7)?,
                        status: row.get(8)?,
                        error_message: row.get(9)?,
                        created_at: row.get(10)?,
                    })
                })?;
                for row in rows {
                    logs.push(row?);
                }
            }
        }

        Ok(logs)
    }

    // ---- Share Node Registry ----

    /// 更新或插入云端节点缓存
    pub fn upsert_share_node(&self, node: &ShareNode) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO share_node_registry (node_id, models, price, status, latency_ms, last_heartbeat)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(node_id) DO UPDATE SET
                models = excluded.models,
                price = excluded.price,
                status = excluded.status,
                latency_ms = excluded.latency_ms,
                last_heartbeat = excluded.last_heartbeat",
            params![
                node.node_id,
                node.models,
                node.price,
                node.status,
                node.latency_ms,
                node.last_heartbeat,
            ],
        )?;
        Ok(())
    }

    /// 获取所有在线节点
    pub fn get_online_share_nodes(&self) -> Result<Vec<ShareNode>, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT node_id, models, price, status, latency_ms, last_heartbeat
             FROM share_node_registry WHERE status != 'offline'
             ORDER BY price ASC, latency_ms ASC",
        )?;
        let nodes = stmt
            .query_map([], |row| {
                Ok(ShareNode {
                    node_id: row.get(0)?,
                    models: row.get(1)?,
                    price: row.get(2)?,
                    status: row.get(3)?,
                    latency_ms: row.get(4)?,
                    last_heartbeat: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(nodes)
    }

    /// 标记节点离线
    pub fn set_share_node_offline(&self, node_id: &str) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE share_node_registry SET status = 'offline' WHERE node_id = ?1",
            params![node_id],
        )?;
        Ok(())
    }

    /// 清除所有节点缓存
    pub fn clear_share_node_registry(&self) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute("DELETE FROM share_node_registry", [])?;
        Ok(())
    }

    // ---- Token 统计 ----

    /// 按 model 分组聚合某方向在指定时间窗口内的 token 用量。
    ///
    /// - `direction`: `"supply"` 或 `"consume"`
    /// - `since`: 起始 Unix 时间戳（秒）；传 `None` 表示全量
    /// - 只统计 `status = 'completed'` 的任务（失败/拒绝的任务 token 不可信）
    pub fn get_token_stats_by_direction(
        &self,
        direction: &str,
        since: Option<i64>,
    ) -> Result<Vec<ModelTokenStat>, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let sql = match since {
            Some(_) =>
                "SELECT model,
                        SUM(tokens_prompt)     AS prompt_sum,
                        SUM(tokens_completion) AS completion_sum,
                        SUM(tokens_prompt + tokens_completion) AS total_sum,
                        COUNT(*)               AS task_count
                 FROM p2p_task_log
                 WHERE direction = ?1 AND status = 'completed' AND created_at >= ?2
                 GROUP BY model
                 ORDER BY total_sum DESC",
            None =>
                "SELECT model,
                        SUM(tokens_prompt)     AS prompt_sum,
                        SUM(tokens_completion) AS completion_sum,
                        SUM(tokens_prompt + tokens_completion) AS total_sum,
                        COUNT(*)               AS task_count
                 FROM p2p_task_log
                 WHERE direction = ?1 AND status = 'completed'
                 GROUP BY model
                 ORDER BY total_sum DESC",
        };
        let map_row = |row: &rusqlite::Row| {
            Ok(ModelTokenStat {
                model: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                prompt_tokens: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                completion_tokens: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                total_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                task_count: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = match since {
            Some(ts) => stmt.query_map(params![direction, ts], map_row)?,
            None => stmt.query_map(params![direction], map_row)?,
        };
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// 某方向在时间窗口内的 token 总量（prompt + completion），只统计 completed 任务。
    pub fn get_token_totals_by_direction(
        &self,
        direction: &str,
        since: Option<i64>,
    ) -> Result<(i64, i64), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let sql = match since {
            Some(_) =>
                "SELECT COALESCE(SUM(tokens_prompt), 0), COALESCE(SUM(tokens_completion), 0)
                 FROM p2p_task_log
                 WHERE direction = ?1 AND status = 'completed' AND created_at >= ?2",
            None =>
                "SELECT COALESCE(SUM(tokens_prompt), 0), COALESCE(SUM(tokens_completion), 0)
                 FROM p2p_task_log
                 WHERE direction = ?1 AND status = 'completed'",
        };
        let mut stmt = conn.prepare(sql)?;
        let row = match since {
            Some(ts) => stmt.query_row(params![direction, ts], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            }),
            None => stmt.query_row(params![direction], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            }),
        };
        row.map_err(|e| ShareError::Database(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_db() -> ShareDb {
        ShareDb::memory().expect("memory db")
    }

    fn log(
        db: &ShareDb,
        task_id: &str,
        direction: &str,
        model: &str,
        prompt: i32,
        completion: i32,
        status: &str,
        created_at: i64,
    ) {
        db.insert_p2p_task_log(&P2PTaskLog {
            task_id: task_id.into(),
            direction: direction.into(),
            model: model.into(),
            upstream_model: None,
            tokens_prompt: prompt,
            tokens_completion: completion,
            credits: 0.0,
            latency_ms: Some(10),
            status: status.into(),
            error_message: None,
            created_at,
        })
        .expect("insert log");
    }

    #[test]
    fn token_stats_group_by_model_sums_completed_only() {
        let db = fresh_db();
        let now = 1_700_000_000;
        log(&db, "t1", "supply", "claude-sonnet-4", 100, 50, "completed", now);
        log(&db, "t2", "supply", "claude-sonnet-4", 200, 80, "completed", now + 10);
        log(&db, "t3", "supply", "claude-haiku-4", 30, 10, "completed", now + 20);
        log(&db, "t4", "supply", "claude-sonnet-4", 999, 999, "failed", now + 30);

        let stats = db.get_token_stats_by_direction("supply", None).unwrap();
        assert_eq!(stats.len(), 2);
        let sonnet = stats.iter().find(|s| s.model == "claude-sonnet-4").unwrap();
        assert_eq!(sonnet.prompt_tokens, 300);
        assert_eq!(sonnet.completion_tokens, 130);
        assert_eq!(sonnet.total_tokens, 430);
        assert_eq!(sonnet.task_count, 2);
        let haiku = stats.iter().find(|s| s.model == "claude-haiku-4").unwrap();
        assert_eq!(haiku.total_tokens, 40);
        assert_eq!(haiku.task_count, 1);
    }

    #[test]
    fn token_stats_respects_direction_and_since() {
        let db = fresh_db();
        let now = 1_700_000_000;
        log(&db, "c1", "consume", "gpt-4o", 50, 20, "completed", now);
        log(&db, "c2", "consume", "gpt-4o", 60, 30, "completed", now + 7200);
        log(&db, "s1", "supply", "gpt-4o", 1, 1, "completed", now);

        let consume_all = db.get_token_stats_by_direction("consume", None).unwrap();
        assert_eq!(consume_all.len(), 1);
        assert_eq!(consume_all[0].total_tokens, 160);

        let consume_recent = db
            .get_token_stats_by_direction("consume", Some(now + 3600))
            .unwrap();
        assert_eq!(consume_recent.len(), 1);
        assert_eq!(consume_recent[0].total_tokens, 90);

        let supply = db.get_token_stats_by_direction("supply", None).unwrap();
        assert_eq!(supply.len(), 1);
        assert_eq!(supply[0].total_tokens, 2);
    }

    #[test]
    fn token_totals_by_direction() {
        let db = fresh_db();
        let now = 1_700_000_000;
        log(&db, "t1", "supply", "a", 100, 50, "completed", now);
        log(&db, "t2", "supply", "b", 200, 80, "completed", now);
        log(&db, "t3", "supply", "a", 1, 1, "rejected", now);

        let (p, c) = db.get_token_totals_by_direction("supply", None).unwrap();
        assert_eq!(p, 300);
        assert_eq!(c, 130);
    }
}