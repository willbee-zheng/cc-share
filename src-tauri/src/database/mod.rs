//! CC-Share 独立数据库模块
//!
//! 使用独立的 share.db 文件，与 cc-switch.db 完全解耦。
//! 这确保了上游 CC-Switch 更新时不会产生 schema 冲突。

pub mod dao_config;
pub mod dao_credits;
pub mod dao_share;
pub mod dao_sync;
pub mod schema;

use schema::SCHEMA_VERSION;
use std::path::Path;
use std::sync::Mutex;

/// CC-Share 独立数据库
///
/// 内部 `conn` 字段对同 crate 的 DAO 模块可见（通过 `pub(crate)`）。
pub struct ShareDb {
    pub(crate) conn: Mutex<rusqlite::Connection>,
}

impl ShareDb {
    /// 初始化数据库，创建表结构并执行迁移
    pub fn init(config_dir: &Path) -> Result<Self, String> {
        let db_path = config_dir.join("share.db");
        log::info!("初始化 CC-Share 数据库: {}", db_path.display());

        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| format!("打开 share.db 失败: {e}"))?;

        // 启用 WAL 模式和外键约束
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| format!("设置 PRAGMA 失败: {e}"))?;

        // 创建表
        schema::create_tables(&conn)?;

        // 执行迁移
        let current_version = Self::get_user_version(&conn)?;
        if current_version > SCHEMA_VERSION {
            return Err(format!(
                "share.db 版本过新（{current_version}），当前插件仅支持 {SCHEMA_VERSION}"
            ));
        }
        schema::apply_migrations(&conn, current_version)?;

        log::info!("✓ CC-Share 数据库初始化完成（版本 {SCHEMA_VERSION}）");
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 创建内存数据库（用于测试）
    #[cfg(test)]
    pub fn memory() -> Result<Self, String> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| format!("创建内存数据库失败: {e}"))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| format!("设置 PRAGMA 失败: {e}"))?;
        schema::create_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn get_user_version(conn: &rusqlite::Connection) -> Result<i32, String> {
        conn.query_row("PRAGMA user_version;", [], |row| row.get(0))
            .map_err(|e| format!("读取 user_version 失败: {e}"))
    }
}