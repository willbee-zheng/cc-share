//! 系统日志的数据库访问层
//!
//! 所有写入/查询/清理都在 `ShareDb` 上执行。批量插入使用事务，
//! 避免 1000 条日志 = 1000 次 fsync 的性能陷阱。

use crate::database::ShareDb;
use crate::error::ShareError;
use crate::system_log::{LogEntry, LogFilter, LogStats, SystemLogEntry};
use rusqlite::params;

impl ShareDb {
    /// 批量插入日志条目（单事务）
    pub fn insert_logs_batch(&self, entries: &[LogEntry]) -> Result<usize, ShareError> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let tx = conn
            .transaction()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO system_log (timestamp, level, target, message)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| ShareError::Database(e.to_string()))?;
            for entry in entries {
                stmt.execute(params![
                    entry.timestamp_ms as i64,
                    entry.level,
                    entry.target,
                    entry.message,
                ])
                .map_err(|e| ShareError::Database(e.to_string()))?;
            }
        }
        tx.commit()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        Ok(entries.len())
    }

    /// 按过滤条件查询日志，按时间倒序
    pub fn query_logs(&self, filter: &LogFilter) -> Result<Vec<SystemLogEntry>, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;

        let limit = filter.limit.unwrap_or(500).min(5000) as i64;
        let offset = filter.offset.unwrap_or(0) as i64;

        // 构造 SQL：level 过滤按级别优先级，target/search 用 LIKE
        let mut where_clauses: Vec<String> = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(level) = &filter.level {
            let min_level = level_priority(level);
            if let Some(priority) = min_level {
                where_clauses.push(format!(
                    "CASE level
                        WHEN 'error' THEN 4
                        WHEN 'warn'  THEN 3
                        WHEN 'info'  THEN 2
                        WHEN 'debug' THEN 1
                        ELSE 0
                     END >= {priority}"
                ));
            }
        }
        if let Some(target) = &filter.target {
            if !target.is_empty() {
                where_clauses.push("target LIKE ?".to_string());
                param_values.push(Box::new(format!("%{target}%")));
            }
        }
        if let Some(search) = &filter.search {
            if !search.is_empty() {
                where_clauses.push("message LIKE ?".to_string());
                param_values.push(Box::new(format!("%{search}%")));
            }
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let sql = format!(
            "SELECT id, timestamp, level, target, message
             FROM system_log {where_sql}
             ORDER BY timestamp DESC, id DESC
             LIMIT ? OFFSET ?"
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| ShareError::Database(e.to_string()))?;

        // rusqlite params_from_iter 需要 `&dyn ToSql`
        let param_refs: Vec<&dyn rusqlite::ToSql> = param_values
            .iter()
            .map(|p| p.as_ref())
            .chain([(&limit as &dyn rusqlite::ToSql), (&offset as &dyn rusqlite::ToSql)])
            .collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(SystemLogEntry {
                    id: row.get(0)?,
                    timestamp_ms: row.get::<_, i64>(1)? as u64,
                    level: row.get(2)?,
                    target: row.get(3)?,
                    message: row.get(4)?,
                })
            })
            .map_err(|e| ShareError::Database(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| ShareError::Database(e.to_string()))?);
        }
        Ok(out)
    }

    /// 清空所有日志
    pub fn clear_logs(&self) -> Result<(), ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute("DELETE FROM system_log", [])
            .map_err(|e| ShareError::Database(e.to_string()))?;
        Ok(())
    }

    /// 按级别统计
    pub fn log_stats(&self) -> Result<LogStats, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT level, COUNT(*) FROM system_log GROUP BY level")
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))
            .map_err(|e| ShareError::Database(e.to_string()))?;

        let mut stats = LogStats::default();
        for row in rows {
            let (level, count) = row.map_err(|e| ShareError::Database(e.to_string()))?;
            stats.total += count;
            match level.as_str() {
                "debug" => stats.debug = count,
                "info" => stats.info = count,
                "warn" => stats.warn = count,
                "error" => stats.error = count,
                _ => {}
            }
        }
        Ok(stats)
    }

    /// 列出所有出现过的 target（用于前端模块过滤下拉），按字母序
    pub fn list_log_targets(&self) -> Result<Vec<String>, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT DISTINCT target FROM system_log ORDER BY target ASC")
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| ShareError::Database(e.to_string()))?);
        }
        Ok(out)
    }

    /// 删除 `keep_days` 天之前的日志，返回删除条数
    pub fn prune_old_logs(&self, keep_days: u32) -> Result<usize, ShareError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ShareError::Database(e.to_string()))?;
        let cutoff_ms = {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            now - (keep_days as i64) * 86_400_000
        };
        let affected = conn
            .execute(
                "DELETE FROM system_log WHERE timestamp < ?1",
                params![cutoff_ms],
            )
            .map_err(|e| ShareError::Database(e.to_string()))?;
        Ok(affected)
    }
}

/// 级别 → 优先级数字（debug=1, info=2, warn=3, error=4）；未知返回 None
fn level_priority(level: &str) -> Option<i64> {
    match level.to_lowercase().as_str() {
        "debug" => Some(1),
        "info" => Some(2),
        "warn" => Some(3),
        "error" => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(level: &str, target: &str, message: &str) -> LogEntry {
        LogEntry {
            timestamp_ms: 1_700_000_000_000,
            level: level.into(),
            target: target.into(),
            message: message.into(),
        }
    }

    #[test]
    fn test_insert_and_query_round_trip() {
        let db = ShareDb::memory().unwrap();
        let entries = vec![
            make_entry("info", "cc_share::share", "started"),
            make_entry("warn", "cc_share::share", "retry"),
            make_entry("error", "cc_share::wallet", "boom"),
        ];
        let n = db.insert_logs_batch(&entries).unwrap();
        assert_eq!(n, 3);

        let all = db.query_logs(&LogFilter::default()).unwrap();
        assert_eq!(all.len(), 3);
        // 倒序：最新（最后插入的）在最前
        assert_eq!(all[0].level, "error");
        assert_eq!(all[0].target, "cc_share::wallet");
    }

    #[test]
    fn test_query_with_level_filter() {
        let db = ShareDb::memory().unwrap();
        db.insert_logs_batch(&[
            make_entry("debug", "a", "d"),
            make_entry("info", "a", "i"),
            make_entry("warn", "a", "w"),
            make_entry("error", "a", "e"),
        ])
        .unwrap();

        let filter = LogFilter {
            level: Some("warn".into()),
            ..Default::default()
        };
        let result = db.query_logs(&filter).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|e| e.level == "warn" || e.level == "error"));
    }

    #[test]
    fn test_query_with_target_and_search() {
        let db = ShareDb::memory().unwrap();
        db.insert_logs_batch(&[
            make_entry("info", "cc_share::share::client", "connect failed"),
            make_entry("info", "cc_share::wallet", "connect ok"),
        ])
        .unwrap();

        let filter = LogFilter {
            target: Some("client".into()),
            ..Default::default()
        };
        assert_eq!(db.query_logs(&filter).unwrap().len(), 1);

        let filter = LogFilter {
            search: Some("connect".into()),
            ..Default::default()
        };
        assert_eq!(db.query_logs(&filter).unwrap().len(), 2);
    }

    #[test]
    fn test_log_stats() {
        let db = ShareDb::memory().unwrap();
        db.insert_logs_batch(&[
            make_entry("info", "a", "i1"),
            make_entry("info", "a", "i2"),
            make_entry("warn", "a", "w"),
            make_entry("error", "a", "e"),
        ])
        .unwrap();

        let stats = db.log_stats().unwrap();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.info, 2);
        assert_eq!(stats.warn, 1);
        assert_eq!(stats.error, 1);
        assert_eq!(stats.debug, 0);
    }

    #[test]
    fn test_clear_logs() {
        let db = ShareDb::memory().unwrap();
        db.insert_logs_batch(&[make_entry("info", "a", "b")]).unwrap();
        assert_eq!(db.log_stats().unwrap().total, 1);
        db.clear_logs().unwrap();
        assert_eq!(db.log_stats().unwrap().total, 0);
    }

    #[test]
    fn test_list_log_targets() {
        let db = ShareDb::memory().unwrap();
        db.insert_logs_batch(&[
            make_entry("info", "cc_share::wallet", "x"),
            make_entry("info", "cc_share::share::client", "y"),
            make_entry("info", "cc_share::wallet", "z"),
        ])
        .unwrap();
        let targets = db.list_log_targets().unwrap();
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&"cc_share::wallet".to_string()));
        assert!(targets.contains(&"cc_share::share::client".to_string()));
    }

    #[test]
    fn test_prune_old_logs() {
        let db = ShareDb::memory().unwrap();
        // 一条旧日志（2000 年）+ 一条新日志（当前时间）
        let old = LogEntry {
            timestamp_ms: 946_684_800_000, // 2000-01-01
            level: "info".into(),
            target: "old".into(),
            message: "old".into(),
        };
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let new = LogEntry {
            timestamp_ms: now_ms,
            level: "info".into(),
            target: "new".into(),
            message: "new".into(),
        };
        db.insert_logs_batch(&[old, new]).unwrap();
        let pruned = db.prune_old_logs(30).unwrap();
        assert_eq!(pruned, 1);
        let remaining = db.query_logs(&LogFilter::default()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].target, "new");
    }
}
