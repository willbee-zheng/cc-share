//! CC-Share 独立数据库 Schema
//!
//! 管理独立的 share.db，与 cc-switch.db 完全解耦。
//! 版本号独立维护，从 v1 开始。

use rusqlite::Connection;

/// 当前 Schema 版本
pub(crate) const SCHEMA_VERSION: i32 = 5;

/// 创建所有数据库表
pub fn create_tables(conn: &Connection) -> Result<(), String> {
    // share_settings — 供应者共享策略配置
    conn.execute(
        "CREATE TABLE IF NOT EXISTS share_settings (
            provider_id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            is_sharing INTEGER NOT NULL DEFAULT 0,
            max_token_per_min INTEGER NOT NULL DEFAULT 4000,
            token_unit_price REAL NOT NULL DEFAULT 0.05,
            concurrency_limit INTEGER NOT NULL DEFAULT 1,
            cooldown_seconds INTEGER NOT NULL DEFAULT 60,
            PRIMARY KEY (provider_id, app_type)
        )",
        [],
    )
    .map_err(|e| format!("创建 share_settings 表失败: {e}"))?;

    // user_wallet — 本地积分钱包镜像
    conn.execute(
        "CREATE TABLE IF NOT EXISTS user_wallet (
            user_id TEXT PRIMARY KEY,
            balance_credits REAL NOT NULL DEFAULT 0,
            total_earned REAL NOT NULL DEFAULT 0,
            total_spent REAL NOT NULL DEFAULT 0,
            last_sync_at INTEGER
        )",
        [],
    )
    .map_err(|e| format!("创建 user_wallet 表失败: {e}"))?;

    // p2p_task_log — P2P 任务审计日志
    conn.execute(
        "CREATE TABLE IF NOT EXISTS p2p_task_log (
            task_id TEXT PRIMARY KEY,
            direction TEXT NOT NULL CHECK (direction IN ('consume', 'supply')),
            model TEXT NOT NULL,
            upstream_model TEXT,
            tokens_prompt INTEGER NOT NULL DEFAULT 0,
            tokens_completion INTEGER NOT NULL DEFAULT 0,
            credits REAL NOT NULL DEFAULT 0,
            latency_ms INTEGER,
            status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'completed', 'failed', 'rejected', 'busy')),
            error_message TEXT,
            created_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("创建 p2p_task_log 表失败: {e}"))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_p2p_task_log_direction
         ON p2p_task_log(direction, created_at DESC)",
        [],
    )
    .map_err(|e| format!("创建 p2p_task_log 索引失败: {e}"))?;

    // share_node_registry — 云端节点缓存
    conn.execute(
        "CREATE TABLE IF NOT EXISTS share_node_registry (
            node_id TEXT PRIMARY KEY,
            models TEXT NOT NULL DEFAULT '[]',
            price REAL NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'idle' CHECK (status IN ('idle', 'busy', 'offline')),
            latency_ms INTEGER,
            last_heartbeat INTEGER
        )",
        [],
    )
    .map_err(|e| format!("创建 share_node_registry 表失败: {e}"))?;

    // client_config — 单行 KV 配置（云端 URL、token、node_id 等）
    conn.execute(
        "CREATE TABLE IF NOT EXISTS client_config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("创建 client_config 表失败: {e}"))?;

    // system_log — 运行日志（由 system_log 模块批量写入）
    conn.execute(
        "CREATE TABLE IF NOT EXISTS system_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            level TEXT NOT NULL,
            target TEXT NOT NULL,
            message TEXT NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("创建 system_log 表失败: {e}"))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_system_log_ts
         ON system_log(timestamp DESC)",
        [],
    )
    .map_err(|e| format!("创建 system_log 时间索引失败: {e}"))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_system_log_level
         ON system_log(level, timestamp DESC)",
        [],
    )
    .map_err(|e| format!("创建 system_log 级别索引失败: {e}"))?;

    // daily_sync_log — 每日统计聚合（Phase 7 统计同步）
    // 注：此表也在 v4/v5 迁移中创建，这里重复创建是为了让内存数据库测试也能用
    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_sync_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            stat_date TEXT NOT NULL,
            direction TEXT NOT NULL CHECK (direction IN ('supply', 'consume')),
            model TEXT NOT NULL,
            upstream_model TEXT NOT NULL DEFAULT '',
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            task_count INTEGER NOT NULL DEFAULT 0,
            credits REAL NOT NULL DEFAULT 0,
            synced INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("创建 daily_sync_log 表失败: {e}"))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_daily_sync_log_date
         ON daily_sync_log(stat_date, direction, synced)",
        [],
    )
    .map_err(|e| format!("创建 daily_sync_log 索引失败: {e}"))?;

    Ok(())
}

/// 应用 Schema 迁移
pub fn apply_migrations(conn: &Connection, from_version: i32) -> Result<(), String> {
    let mut version = from_version;

    while version < SCHEMA_VERSION {
        match version {
            0 => {
                log::info!("share.db: 从 v0 迁移到 v1（初始表创建）");
                // 表已在 create_tables 中创建
                set_user_version(conn, 1)?;
            }
            1 => {
                log::info!("share.db: 从 v1 迁移到 v2（添加 client_config 表）");
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS client_config (
                        key TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    )",
                    [],
                )
                .map_err(|e| format!("v2 迁移失败: {e}"))?;
                set_user_version(conn, 2)?;
            }
            2 => {
                log::info!("share.db: 从 v2 迁移到 v3（添加 system_log 表）");
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS system_log (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        timestamp INTEGER NOT NULL,
                        level TEXT NOT NULL,
                        target TEXT NOT NULL,
                        message TEXT NOT NULL
                    )",
                    [],
                )
                .map_err(|e| format!("v3 迁移建表失败: {e}"))?;
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_system_log_ts
                     ON system_log(timestamp DESC)",
                    [],
                )
                .map_err(|e| format!("v3 迁移时间索引失败: {e}"))?;
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_system_log_level
                     ON system_log(level, timestamp DESC)",
                    [],
                )
                .map_err(|e| format!("v3 迁移级别索引失败: {e}"))?;
                set_user_version(conn, 3)?;
            }
            3 => {
                log::info!("share.db: 从 v3 迁移到 v4（添加 daily_sync_log 表）");
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS daily_sync_log (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        stat_date TEXT NOT NULL,
                        direction TEXT NOT NULL CHECK (direction IN ('supply', 'consume')),
                        model TEXT NOT NULL,
                        prompt_tokens INTEGER NOT NULL DEFAULT 0,
                        completion_tokens INTEGER NOT NULL DEFAULT 0,
                        task_count INTEGER NOT NULL DEFAULT 0,
                        credits REAL NOT NULL DEFAULT 0,
                        synced INTEGER NOT NULL DEFAULT 0,
                        created_at INTEGER NOT NULL
                    )",
                    [],
                )
                .map_err(|e| format!("v4 迁移建表失败: {e}"))?;
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_daily_sync_log_date
                     ON daily_sync_log(stat_date, direction, synced)",
                    [],
                )
                .map_err(|e| format!("v4 迁移索引失败: {e}"))?;
                set_user_version(conn, 4)?;
            }
            4 => {
                log::info!("share.db: 从 v4 迁移到 v5（添加 upstream_model 列）");
                conn.execute(
                    "ALTER TABLE p2p_task_log ADD COLUMN upstream_model TEXT",
                    [],
                )
                .map_err(|e| format!("v5 迁移 p2p_task_log 添加列失败: {e}"))?;
                conn.execute(
                    "ALTER TABLE daily_sync_log ADD COLUMN upstream_model TEXT NOT NULL DEFAULT ''",
                    [],
                )
                .map_err(|e| format!("v5 迁移 daily_sync_log 添加列失败: {e}"))?;
                set_user_version(conn, 5)?;
            }
            _ => {
                return Err(format!(
                    "未知的 share.db 版本 {version}，无法迁移到 {SCHEMA_VERSION}"
                ));
            }
        }
        version = get_user_version(conn)?;
    }

    Ok(())
}

fn get_user_version(conn: &Connection) -> Result<i32, String> {
    conn.query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(|e| format!("读取 share.db user_version 失败: {e}"))
}

fn set_user_version(conn: &Connection, version: i32) -> Result<(), String> {
    let sql = format!("PRAGMA user_version = {version};");
    conn.execute(&sql, [])
        .map(|_| ())
        .map_err(|e| format!("写入 share.db user_version 失败: {e}"))
}