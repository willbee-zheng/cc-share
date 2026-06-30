//! CC-Share 共享设置 DAO
//!
//! 管理供应者的共享策略配置

use crate::database::ShareDb;
use crate::error::ShareError;
use rusqlite::params;

/// 共享策略配置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShareSettings {
    pub provider_id: String,
    pub app_type: String,
    pub is_sharing: bool,
    pub max_token_per_min: i32,
    pub token_unit_price: f64,
    pub concurrency_limit: i32,
    pub cooldown_seconds: i32,
}

impl ShareDb {
    /// 获取指定 Provider 的共享设置
    pub fn get_share_settings(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Result<Option<ShareSettings>, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT provider_id, app_type, is_sharing, max_token_per_min,
                    token_unit_price, concurrency_limit, cooldown_seconds
             FROM share_settings WHERE provider_id = ?1 AND app_type = ?2",
        )?;
        let result = stmt
            .query_row(params![provider_id, app_type], |row| {
                Ok(ShareSettings {
                    provider_id: row.get(0)?,
                    app_type: row.get(1)?,
                    is_sharing: row.get::<_, i32>(2)? != 0,
                    max_token_per_min: row.get(3)?,
                    token_unit_price: row.get(4)?,
                    concurrency_limit: row.get(5)?,
                    cooldown_seconds: row.get(6)?,
                })
            })
            .ok();
        Ok(result)
    }

    /// 保存或更新共享设置（upsert）
    pub fn upsert_share_settings(&self, settings: &ShareSettings) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO share_settings (provider_id, app_type, is_sharing, max_token_per_min,
                    token_unit_price, concurrency_limit, cooldown_seconds)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(provider_id, app_type) DO UPDATE SET
                is_sharing = excluded.is_sharing,
                max_token_per_min = excluded.max_token_per_min,
                token_unit_price = excluded.token_unit_price,
                concurrency_limit = excluded.concurrency_limit,
                cooldown_seconds = excluded.cooldown_seconds",
            params![
                settings.provider_id,
                settings.app_type,
                settings.is_sharing as i32,
                settings.max_token_per_min,
                settings.token_unit_price,
                settings.concurrency_limit,
                settings.cooldown_seconds,
            ],
        )?;
        Ok(())
    }

    /// 获取所有正在共享的 Provider
    pub fn get_all_sharing_providers(&self) -> Result<Vec<ShareSettings>, ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT provider_id, app_type, is_sharing, max_token_per_min,
                    token_unit_price, concurrency_limit, cooldown_seconds
             FROM share_settings WHERE is_sharing = 1",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ShareSettings {
                    provider_id: row.get(0)?,
                    app_type: row.get(1)?,
                    is_sharing: row.get::<_, i32>(2)? != 0,
                    max_token_per_min: row.get(3)?,
                    token_unit_price: row.get(4)?,
                    concurrency_limit: row.get(5)?,
                    cooldown_seconds: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 删除共享设置
    pub fn delete_share_settings(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Result<(), ShareError> {
        let conn = self.conn.lock().map_err(|e| ShareError::Database(e.to_string()))?;
        conn.execute(
            "DELETE FROM share_settings WHERE provider_id = ?1 AND app_type = ?2",
            params![provider_id, app_type],
        )?;
        Ok(())
    }
}