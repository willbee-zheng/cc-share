//! 系统日志查询/管理 Tauri 命令

use crate::system_log::{LogFilter, LogStats, SystemLogEntry};
use crate::ShareState;

/// 查询日志列表（按时间倒序），支持级别 / 模块 / 关键字过滤
#[tauri::command]
pub async fn get_system_logs(
    state: tauri::State<'_, ShareState>,
    filter: LogFilter,
) -> Result<Vec<SystemLogEntry>, String> {
    state.db.query_logs(&filter).map_err(|e| e.to_string())
}

/// 清空所有日志
#[tauri::command]
pub async fn clear_system_logs(state: tauri::State<'_, ShareState>) -> Result<(), String> {
    state.db.clear_logs().map_err(|e| e.to_string())
}

/// 按级别统计日志数量
#[tauri::command]
pub async fn get_system_log_stats(
    state: tauri::State<'_, ShareState>,
) -> Result<LogStats, String> {
    state.db.log_stats().map_err(|e| e.to_string())
}

/// 列出所有出现过的 target（前端模块过滤下拉用）
#[tauri::command]
pub async fn list_system_log_targets(
    state: tauri::State<'_, ShareState>,
) -> Result<Vec<String>, String> {
    state.db.list_log_targets().map_err(|e| e.to_string())
}

/// 运行时切换全局日志级别
///
/// `level`: `debug` / `info` / `warn` / `error`。会调用 `log::set_max_level`
/// 覆盖 `tauri_plugin_log` 的初始化级别，便于排查问题时临时打开 debug。
#[tauri::command]
pub async fn set_log_level(level: String) -> Result<(), String> {
    let filter = match level.to_lowercase().as_str() {
        "debug" => log::LevelFilter::Debug,
        "info" => log::LevelFilter::Info,
        "warn" => log::LevelFilter::Warn,
        "error" => log::LevelFilter::Error,
        "off" => log::LevelFilter::Off,
        other => return Err(format!("unknown log level: {other}")),
    };
    log::set_max_level(filter);
    log::info!("日志级别已切换为 {level}");
    Ok(())
}

/// 手动触发一次旧日志清理（保留最近 N 天）
#[tauri::command]
pub async fn prune_system_logs(
    state: tauri::State<'_, ShareState>,
    keep_days: u32,
) -> Result<usize, String> {
    state
        .db
        .prune_old_logs(keep_days)
        .map_err(|e| e.to_string())
}
