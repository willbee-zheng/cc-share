//! 定价策略
//!
//! 定义每模型的积分单价，用于计费清算。
//! 定价基于模型能力和成本，单位：积分/千 Token。

use std::collections::HashMap;

/// 模型定价条目
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelPricing {
    /// 模型标识符（如 "claude-3-5-sonnet"）
    pub model_id: String,
    /// 输入价格（积分/千 Token）
    pub input_price_per_1k: f64,
    /// 输出价格（积分/千 Token）
    pub output_price_per_1k: f64,
    /// 模型类别（用于分组）
    pub category: PricingCategory,
}

/// 定价类别
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PricingCategory {
    /// 高端模型（GPT-4, Claude Opus 等）
    Premium,
    /// 中端模型（GPT-4o-mini, Claude Sonnet 等）
    Standard,
    /// 低端模型（GPT-3.5, Claude Haiku 等）
    Economy,
}

/// 定价表
///
/// 管理所有模型的积分定价，支持按模型查询价格
/// 和按类别批量获取。
pub struct PricingTable {
    prices: HashMap<String, ModelPricing>,
}

impl PricingTable {
    /// 创建定价表，加载默认定价
    pub fn new() -> Self {
        Self {
            prices: Self::default_pricing()
                .into_iter()
                .map(|p| (p.model_id.clone(), p))
                .collect(),
        }
    }

    /// 获取指定模型的定价
    ///
    /// 精确匹配模型 ID，如果找不到则尝试前缀匹配。
    /// 例如 "claude-3-5-sonnet-20241022" 会匹配 "claude-3-5-sonnet"。
    pub fn get_pricing(&self, model: &str) -> Option<&ModelPricing> {
        // 精确匹配
        if let Some(pricing) = self.prices.get(model) {
            return Some(pricing);
        }

        // 前缀匹配（处理带日期后缀的模型名）
        for (key, pricing) in &self.prices {
            if model.starts_with(key) {
                return Some(pricing);
            }
        }

        // 默认返回 Standard 类别的价格
        None
    }

    /// 计算请求的积分费用
    ///
    /// 根据 Token 用量和模型定价计算总积分。
    /// 输出 Token 价格通常是输入的 3 倍。
    pub fn calculate_cost(
        &self,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> f64 {
        let pricing = match self.get_pricing(model) {
            Some(p) => p,
            None => {
                // 默认定价：Standard 类别
                return (prompt_tokens as f64 / 1000.0) * 0.05
                    + (completion_tokens as f64 / 1000.0) * 0.15;
            }
        };

        let input_cost = (prompt_tokens as f64 / 1000.0) * pricing.input_price_per_1k;
        let output_cost = (completion_tokens as f64 / 1000.0) * pricing.output_price_per_1k;
        input_cost + output_cost
    }

    /// 获取所有定价
    pub fn all_pricings(&self) -> Vec<&ModelPricing> {
        self.prices.values().collect()
    }

    /// 默认定价表
    ///
    /// 价格单位：积分/千 Token
    /// 参考官方定价，换算比例 1 积分 ≈ 0.001 USD
    fn default_pricing() -> Vec<ModelPricing> {
        vec![
            // Claude 系列
            ModelPricing {
                model_id: "claude-opus-4".to_string(),
                input_price_per_1k: 0.15,
                output_price_per_1k: 0.75,
                category: PricingCategory::Premium,
            },
            ModelPricing {
                model_id: "claude-sonnet-4".to_string(),
                input_price_per_1k: 0.03,
                output_price_per_1k: 0.15,
                category: PricingCategory::Standard,
            },
            ModelPricing {
                model_id: "claude-haiku-4".to_string(),
                input_price_per_1k: 0.01,
                output_price_per_1k: 0.05,
                category: PricingCategory::Economy,
            },
            ModelPricing {
                model_id: "claude-3-5-sonnet".to_string(),
                input_price_per_1k: 0.03,
                output_price_per_1k: 0.15,
                category: PricingCategory::Standard,
            },
            ModelPricing {
                model_id: "claude-3-haiku".to_string(),
                input_price_per_1k: 0.01,
                output_price_per_1k: 0.05,
                category: PricingCategory::Economy,
            },
            // GPT 系列
            ModelPricing {
                model_id: "gpt-4o".to_string(),
                input_price_per_1k: 0.025,
                output_price_per_1k: 0.10,
                category: PricingCategory::Standard,
            },
            ModelPricing {
                model_id: "gpt-4o-mini".to_string(),
                input_price_per_1k: 0.0015,
                output_price_per_1k: 0.006,
                category: PricingCategory::Economy,
            },
            ModelPricing {
                model_id: "gpt-4-turbo".to_string(),
                input_price_per_1k: 0.10,
                output_price_per_1k: 0.30,
                category: PricingCategory::Premium,
            },
            // Gemini 系列
            ModelPricing {
                model_id: "gemini-2-pro".to_string(),
                input_price_per_1k: 0.0125,
                output_price_per_1k: 0.05,
                category: PricingCategory::Standard,
            },
            ModelPricing {
                model_id: "gemini-2-flash".to_string(),
                input_price_per_1k: 0.001,
                output_price_per_1k: 0.004,
                category: PricingCategory::Economy,
            },
        ]
    }
}

impl Default for PricingTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pricing_table_default() {
        let table = PricingTable::new();
        assert!(table.get_pricing("claude-3-5-sonnet").is_some());
        assert!(table.get_pricing("gpt-4o").is_some());
        assert!(table.get_pricing("nonexistent").is_none());
    }

    #[test]
    fn test_pricing_prefix_match() {
        let table = PricingTable::new();
        // 带日期后缀的模型名应匹配前缀
        assert!(table.get_pricing("claude-3-5-sonnet-20241022").is_some());
    }

    #[test]
    fn test_calculate_cost() {
        let table = PricingTable::new();

        // Claude Sonnet: 输入 0.03/1k, 输出 0.15/1k
        // 1000 prompt + 500 completion = 0.03 + 0.075 = 0.105
        let cost = table.calculate_cost("claude-3-5-sonnet", 1000, 500);
        assert!(cost > 0.0);
        let expected = 0.03 + 0.075; // 0.105
        assert!((cost - expected).abs() < 0.001);
    }

    #[test]
    fn test_calculate_cost_unknown_model() {
        let table = PricingTable::new();

        // 未知模型使用默认定价: 输入 0.05/1k, 输出 0.15/1k
        let cost = table.calculate_cost("unknown-model", 1000, 1000);
        let expected = 0.05 + 0.15; // 0.20
        assert!((cost - expected).abs() < 0.001);
    }

    #[test]
    fn test_all_pricings() {
        let table = PricingTable::new();
        let all = table.all_pricings();
        assert!(!all.is_empty());
    }
}