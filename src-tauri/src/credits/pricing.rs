//! 定价策略
//!
//! 定义每模型的积分单价，用于本地估算和展示。
//! 云端是计费权威来源，本地定价仅用于：
//! - 消费前估算费用
//! - 收益计算器展示
//!
//! 定价数据优先从云端获取 (GET /api/v1/pricing)，
//! 缓存到 client_config 表，fallback 到硬编码默认值。

use crate::database::ShareDb;
use crate::error::ShareError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// 模型定价条目（与云端 PricingEntry 对齐）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingEntry {
    /// 模型前缀（最长前缀匹配）
    pub model_prefix: String,
    /// 输入价格（积分/千 Token）
    pub prompt_per_1k: f64,
    /// 输出价格（积分/千 Token）
    pub completion_per_1k: f64,
}

/// 云端定价 API 响应
#[derive(Debug, Deserialize)]
struct PricingResponse {
    rules: Vec<PricingEntry>,
}

/// 定价表
///
/// 管理所有模型的积分定价，支持按模型查询价格。
/// 优先使用云端获取的定价，fallback 到硬编码默认值。
pub struct PricingTable {
    prices: RwLock<HashMap<String, PricingEntry>>,
}

impl PricingTable {
    /// 创建定价表，先尝试从数据库缓存加载，再 fallback 到默认值
    pub fn new(db: &ShareDb) -> Self {
        let prices = match Self::load_from_db(db) {
            Ok(cached) => cached,
            Err(_) => Self::default_pricing()
                .into_iter()
                .map(|p| (p.model_prefix.clone(), p))
                .collect(),
        };
        Self {
            prices: RwLock::new(prices),
        }
    }

    /// 从云端刷新定价表，并缓存到数据库
    pub async fn refresh_from_cloud(
        &self,
        db: &ShareDb,
        server_host: &str,
        access_token: &str,
        use_https: bool,
    ) -> Result<usize, String> {
        let base_url = crate::url_utils::build_http_base_with_tls(server_host, use_https);
        let url = format!("{base_url}/api/v1/pricing");

        let client = crate::http_client::shareplan_client_builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| format!("build client: {e}"))?;

        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("fetch pricing: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("fetch pricing failed ({status}): {body}"));
        }

        let pricing_resp: PricingResponse = resp
            .json()
            .await
            .map_err(|e| format!("parse pricing response: {e}"))?;

        let count = pricing_resp.rules.len();
        let prices: HashMap<String, PricingEntry> = pricing_resp
            .rules
            .into_iter()
            .map(|p| (p.model_prefix.clone(), p))
            .collect();

        // Update in-memory table
        {
            let mut w = self.prices.write().map_err(|e| format!("lock: {e}"))?;
            *w = prices;
        }

        // Save to database cache
        {
            let prices = self.prices.read().map_err(|e| format!("lock: {e}"))?;
            let entries: Vec<PricingEntry> = prices.values().cloned().collect();
            Self::save_to_db(db, &entries).map_err(|e| e.to_string())?;
        }

        Ok(count)
    }

    /// 获取指定模型的定价
    ///
    /// 精确匹配模型 ID，如果找不到则尝试前缀匹配。
    pub fn get_pricing(&self, model: &str) -> Option<PricingEntry> {
        let prices = self.prices.read().ok()?;
        // 精确匹配
        if let Some(pricing) = prices.get(model) {
            return Some(pricing.clone());
        }
        // 前缀匹配（处理带日期后缀的模型名）
        for (key, pricing) in prices.iter() {
            if model.starts_with(key) {
                return Some(pricing.clone());
            }
        }
        None
    }

    /// 计算请求的积分费用（本地估算，非权威）
    ///
    /// 用于消费前估算和收益计算器展示。
    /// 云端是计费权威来源。
    pub fn calculate_cost(
        &self,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> f64 {
        let pricing = match self.get_pricing(model) {
            Some(p) => p,
            None => {
                // 默认定价
                return (prompt_tokens as f64 / 1000.0) * 0.05
                    + (completion_tokens as f64 / 1000.0) * 0.15;
            }
        };

        let input_cost = (prompt_tokens as f64 / 1000.0) * pricing.prompt_per_1k;
        let output_cost = (completion_tokens as f64 / 1000.0) * pricing.completion_per_1k;
        input_cost + output_cost
    }

    /// 获取所有定价条目
    pub fn all_pricings(&self) -> Vec<PricingEntry> {
        self.prices
            .read()
            .map(|g| g.values().cloned().collect())
            .unwrap_or_default()
    }

    /// 从数据库缓存加载定价表
    fn load_from_db(db: &ShareDb) -> Result<HashMap<String, PricingEntry>, ShareError> {
        let json_str = match db.get_config("pricing_cache_v1")? {
            Some(s) if !s.is_empty() => s,
            _ => return Err(ShareError::Database("no pricing cache".into())),
        };
        let entries: Vec<PricingEntry> =
            serde_json::from_str(&json_str).map_err(|e| ShareError::Database(e.to_string()))?;
        Ok(entries
            .into_iter()
            .map(|p| (p.model_prefix.clone(), p))
            .collect())
    }

    /// 保存定价表到数据库缓存
    fn save_to_db(db: &ShareDb, entries: &[PricingEntry]) -> Result<(), ShareError> {
        let json_str =
            serde_json::to_string(entries).map_err(|e| ShareError::Database(e.to_string()))?;
        db.set_config("pricing_cache_v1", &json_str)
    }

    /// 默认定价表（与云端 pricing_rules 种子数据对齐）
    ///
    /// 价格单位：积分/千 Token（整数积分体系）
    fn default_pricing() -> Vec<PricingEntry> {
        vec![
            PricingEntry {
                model_prefix: "claude-opus-4".to_string(),
                prompt_per_1k: 15.0,
                completion_per_1k: 75.0,
            },
            PricingEntry {
                model_prefix: "claude-sonnet-4".to_string(),
                prompt_per_1k: 3.0,
                completion_per_1k: 15.0,
            },
            PricingEntry {
                model_prefix: "claude-haiku-4".to_string(),
                prompt_per_1k: 0.8,
                completion_per_1k: 4.0,
            },
            PricingEntry {
                model_prefix: "claude-3-5-sonnet".to_string(),
                prompt_per_1k: 3.0,
                completion_per_1k: 15.0,
            },
            PricingEntry {
                model_prefix: "claude-3-haiku".to_string(),
                prompt_per_1k: 0.8,
                completion_per_1k: 4.0,
            },
            PricingEntry {
                model_prefix: "gpt-4o".to_string(),
                prompt_per_1k: 2.5,
                completion_per_1k: 10.0,
            },
            PricingEntry {
                model_prefix: "gpt-4o-mini".to_string(),
                prompt_per_1k: 0.15,
                completion_per_1k: 0.6,
            },
            PricingEntry {
                model_prefix: "gpt-4".to_string(),
                prompt_per_1k: 10.0,
                completion_per_1k: 30.0,
            },
            PricingEntry {
                model_prefix: "gpt-4-turbo".to_string(),
                prompt_per_1k: 10.0,
                completion_per_1k: 30.0,
            },
            PricingEntry {
                model_prefix: "gemini-1.5-pro".to_string(),
                prompt_per_1k: 1.25,
                completion_per_1k: 5.0,
            },
            PricingEntry {
                model_prefix: "gemini-1.5-flash".to_string(),
                prompt_per_1k: 0.075,
                completion_per_1k: 0.3,
            },
            PricingEntry {
                model_prefix: "deepseek".to_string(),
                prompt_per_1k: 0.14,
                completion_per_1k: 0.28,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_db() -> Arc<ShareDb> {
        Arc::new(ShareDb::memory().expect("创建内存数据库失败"))
    }

    #[test]
    fn test_pricing_table_default() {
        let db = create_test_db();
        let table = PricingTable::new(&db);
        assert!(table.get_pricing("claude-sonnet-4").is_some());
        assert!(table.get_pricing("gpt-4o").is_some());
        assert!(table.get_pricing("nonexistent").is_none());
    }

    #[test]
    fn test_pricing_prefix_match() {
        let db = create_test_db();
        let table = PricingTable::new(&db);
        assert!(table.get_pricing("claude-sonnet-4-20250514").is_some());
    }

    #[test]
    fn test_calculate_cost() {
        let db = create_test_db();
        let table = PricingTable::new(&db);

        // Claude Sonnet: 输入 3/1k, 输出 15/1k
        let cost = table.calculate_cost("claude-sonnet-4", 1000, 500);
        let expected = 3.0 + 7.5; // 10.5
        assert!((cost - expected).abs() < 0.01);
    }

    #[test]
    fn test_calculate_cost_unknown_model() {
        let db = create_test_db();
        let table = PricingTable::new(&db);

        // 未知模型使用默认定价: 输入 0.05/1k, 输出 0.15/1k
        let cost = table.calculate_cost("unknown-model", 1000, 1000);
        let expected = 0.05 + 0.15;
        assert!((cost - expected).abs() < 0.01);
    }

    #[test]
    fn test_all_pricings() {
        let db = create_test_db();
        let table = PricingTable::new(&db);
        let all = table.all_pricings();
        assert!(!all.is_empty());
    }

    #[test]
    fn test_pricing_aligned_with_cloud() {
        let db = create_test_db();
        let table = PricingTable::new(&db);

        // 验证默认定价与云端对齐（整数积分体系）
        let sonnet = table.get_pricing("claude-sonnet-4").unwrap();
        assert!((sonnet.prompt_per_1k - 3.0).abs() < 0.01);
        assert!((sonnet.completion_per_1k - 15.0).abs() < 0.01);

        let opus = table.get_pricing("claude-opus-4").unwrap();
        assert!((opus.prompt_per_1k - 15.0).abs() < 0.01);
        assert!((opus.completion_per_1k - 75.0).abs() < 0.01);
    }
}