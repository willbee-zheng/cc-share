//! 系统日志模块
//!
//! 把 `log::*` 宏产生的日志同步持久化到 `share.db` 的 `system_log` 表，
//! 供前端「系统日志」面板查询、过滤、清空。
//!
//! 架构：
//! 1. `tauri_plugin_log` 配置一个 `TargetKind::Dispatch` 回调，每次 `log::*` 调用
//!    都会把 `log::Record` 推到全局 `LOG_TX` 的无界 mpsc channel。
//! 2. 后台 `spawn_batch_writer` task 批量取出（最多 500 条或每秒一次），
//!    通过 [`dao::insert_logs_batch`] 写入 SQLite。
//! 3. 每次写入后通过 `on_flush` 回调通知上层（`lib.rs` 用来 emit `LOG_APPENDED`
//!    事件给前端实时刷新）。
//!
//! 初始化分两步：
//! - `set_log_sender(tx)` — 在 Tauri builder 阶段调用，确保 tauri_plugin_log 回调
//!   一注册就能转发日志到 channel（但 batch writer 还没启动，日志暂存 buffer）。
//! - `start_batch_writer(db, rx, on_flush)` — 在 setup 中拿到 db 后调用，启动
//!   批量写入 task。

pub mod dao;
pub mod pipeline;

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

/// 全局日志通道，由 [`set_log_sender`] 设置；`tauri_plugin_log` 回调写入这里。
static LOG_TX: OnceLock<UnboundedSender<LogEntry>> = OnceLock::new();

/// 一条待写入的日志（未分配 id）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// 毫秒级 Unix 时间戳
    pub timestamp_ms: u64,
    /// `debug` / `info` / `warn` / `error`
    pub level: String,
    /// 模块路径（`log::Record::target`）
    pub target: String,
    /// 已格式化的消息
    pub message: String,
}

impl LogEntry {
    /// 从 `log::Record` 构造一条待持久化的日志
    pub fn from_record(record: &log::Record) -> Self {
        Self {
            timestamp_ms: now_ms(),
            level: record.level().as_str().to_lowercase(),
            target: record.target().to_string(),
            message: format!("{}", record.args()),
        }
    }
}

/// 已持久化的日志（带数据库 id），返回给前端
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemLogEntry {
    pub id: i64,
    pub timestamp_ms: u64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// 日志查询过滤条件
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogFilter {
    /// 最低级别（含）过滤：`debug` / `info` / `warn` / `error`
    pub level: Option<String>,
    /// target 子串匹配（大小写敏感）
    pub target: Option<String>,
    /// message 子串匹配
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// 按级别统计的日志数量
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogStats {
    pub total: u64,
    pub debug: u64,
    pub info: u64,
    pub warn: u64,
    pub error: u64,
}

/// 在 Tauri builder 阶段注册全局日志发送端。
///
/// 必须在 `tauri_plugin_log` 初始化前调用，确保 callback 触发时 LOG_TX 已就绪。
pub fn set_log_sender(tx: UnboundedSender<LogEntry>) -> Result<(), UnboundedSender<LogEntry>> {
    LOG_TX.set(tx)
}

/// 在 setup 阶段启动 batch writer task。
///
/// `rx` 来自 [`set_log_sender`] 时创建的 `mpsc::unbounded_channel` 的接收端。
/// `on_flush` 每次 batch 写入后被调用，参数为新增条目数。
pub fn start_batch_writer(
    db: std::sync::Arc<crate::database::ShareDb>,
    rx: tokio::sync::mpsc::UnboundedReceiver<LogEntry>,
    on_flush: Box<dyn Fn(usize) + Send + Sync>,
) {
    // 启动时清理 30 天前的日志
    if let Err(e) = db.prune_old_logs(30) {
        log::warn!("启动清理旧日志失败: {e}");
    }
    pipeline::spawn_batch_writer(db, rx, on_flush);
}

/// 由 `tauri_plugin_log` 回调调用：把一条 `log::Record` 推到管道
pub fn dispatch_record(record: &log::Record) {
    let Some(tx) = LOG_TX.get() else {
        return;
    };
    let entry = LogEntry::from_record(record);
    // 无界 channel：send 仅在 channel 关闭时失败
    let _ = tx.send(entry);
}

/// 当前毫秒级 Unix 时间戳
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_entry_from_record() {
        let record = log::Record::builder()
            .level(log::Level::Warn)
            .target("cc_share::share::client")
            .args(format_args!("connection failed: {}", "timeout"))
            .build();
        let entry = LogEntry::from_record(&record);
        assert_eq!(entry.level, "warn");
        assert_eq!(entry.target, "cc_share::share::client");
        assert_eq!(entry.message, "connection failed: timeout");
        assert!(entry.timestamp_ms > 0);
    }

    #[test]
    fn test_log_entry_level_lowercased() {
        for (lvl, expected) in [
            (log::Level::Debug, "debug"),
            (log::Level::Info, "info"),
            (log::Level::Warn, "warn"),
            (log::Level::Error, "error"),
        ] {
            let record = log::Record::builder().level(lvl).build();
            assert_eq!(LogEntry::from_record(&record).level, expected);
        }
    }
}