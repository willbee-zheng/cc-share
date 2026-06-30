//! 计费清算
//!
//! 请求结束后根据 Token 用量计算积分，
//! 扣除消费者积分，增加供应者积分，平台抽成。

use crate::database::ShareDb;
use crate::error::ShareError;
use crate::share::protocol::{TaskResult, TaskStatus, TokenUsage};
use std::sync::Arc;

/// 清算结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SettlementResult {
    /// 供应者获得的积分
    pub supplier_earned: f64,
    /// 消费者扣除的积分
    pub consumer_spent: f64,
    /// 平台抽成
    pub platform_fee: f64,
    /// 使用的模型
    pub model: String,
    /// 输入 Token 数
    pub prompt_tokens: u32,
    /// 输出 Token 数
    pub completion_tokens: u32,
}

/// 平台抽成比例
///
/// 供应者获得 90%，平台抽成 10%
const SUPPLIER_RATE: f64 = 0.90;
const PLATFORM_RATE: f64 = 0.10;

/// 清算引擎
///
/// 负责在任务完成后：
/// 1. 根据模型定价和 Token 用量计算积分
/// 2. 从消费者钱包扣除积分
/// 3. 向供应者钱包增加积分
/// 4. 记录平台抽成
pub struct SettlementEngine {
    db: Arc<ShareDb>,
    pricing: crate::credits::pricing::PricingTable,
}

impl SettlementEngine {
    /// 创建清算引擎
    pub fn new(db: Arc<ShareDb>) -> Self {
        Self {
            db,
            pricing: crate::credits::pricing::PricingTable::new(),
        }
    }

    /// 结算一次任务
    ///
    /// 流程：
    /// 1. 从 TaskResult 中提取 Token 用量
    /// 2. 计算消费者应扣积分
    /// 3. 检查消费者余额是否充足
    /// 4. 执行扣除和增加操作
    /// 5. 返回清算结果
    pub fn settle(
        &self,
        consumer_id: &str,
        supplier_id: &str,
        model: &str,
        result: &TaskResult,
    ) -> Result<SettlementResult, ShareError> {
        // 只结算成功的任务
        if result.status != TaskStatus::Completed {
            return Err(ShareError::Dispatch(format!(
                "任务未完成，状态: {:?}",
                result.status
            )));
        }

        let usage = result.usage.as_ref().ok_or_else(|| {
            ShareError::Dispatch("任务完成但无 Token 用量信息".to_string())
        })?;

        // 计算消费者应扣积分
        let consumer_spent = self.pricing.calculate_cost(
            model,
            usage.prompt_tokens,
            usage.completion_tokens,
        );

        // 供应者获得 90%
        let supplier_earned = consumer_spent * SUPPLIER_RATE;
        // 平台抽成 10%
        let platform_fee = consumer_spent * PLATFORM_RATE;

        // 检查消费者余额
        let wallet = self.db.get_or_create_wallet(consumer_id)?;
        if wallet.balance_credits < consumer_spent {
            return Err(ShareError::InsufficientBalance);
        }

        // 扣除消费者积分，增加供应者积分
        self.db
            .update_wallet_balance(consumer_id, 0.0, consumer_spent)?;
        // 确保 supplier wallet 行存在再 UPDATE
        self.db.get_or_create_wallet(supplier_id)?;
        self.db
            .update_wallet_balance(supplier_id, supplier_earned, 0.0)?;

        Ok(SettlementResult {
            supplier_earned,
            consumer_spent,
            platform_fee,
            model: model.to_string(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        })
    }

    /// 预估任务费用（不执行扣款）
    pub fn estimate_cost(
        &self,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> f64 {
        self.pricing
            .calculate_cost(model, prompt_tokens, completion_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> Arc<ShareDb> {
        Arc::new(ShareDb::memory().expect("创建内存数据库失败"))
    }

    #[test]
    fn test_settlement_rates() {
        assert!((SUPPLIER_RATE - 0.90).abs() < f64::EPSILON);
        assert!((PLATFORM_RATE - 0.10).abs() < f64::EPSILON);
        assert!((SUPPLIER_RATE + PLATFORM_RATE - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_estimate_cost() {
        let db = create_test_db();
        let engine = SettlementEngine::new(db);

        // Claude Sonnet: 输入 0.03/1k, 输出 0.15/1k
        let cost = engine.estimate_cost("claude-3-5-sonnet", 1000, 500);
        let expected = 0.03 + 0.075; // 0.105
        assert!((cost - expected).abs() < 0.001);
    }

    #[test]
    fn test_settle_success() {
        let db = create_test_db();

        // 给消费者充值
        db.get_or_create_wallet("consumer-1").unwrap();
        db.update_wallet_balance("consumer-1", 100.0, 0.0).unwrap();

        let engine = SettlementEngine::new(db.clone());
        let result = TaskResult {
            task_id: "task-1".to_string(),
            status: TaskStatus::Completed,
            content: "Hello".to_string(),
            usage: Some(TokenUsage {
                prompt_tokens: 1000,
                completion_tokens: 500,
                total_tokens: 1500,
            }),
            error: None,
            sequence: None,
            r#final: None,
        };

        let settlement = engine
            .settle("consumer-1", "supplier-1", "claude-3-5-sonnet", &result)
            .unwrap();

        assert!(settlement.supplier_earned > 0.0);
        assert!(settlement.consumer_spent > 0.0);
        assert!(settlement.platform_fee > 0.0);

        // 验证余额变化
        let consumer_wallet = db.get_or_create_wallet("consumer-1").unwrap();
        assert!(consumer_wallet.balance_credits < 100.0);

        let supplier_wallet = db.get_or_create_wallet("supplier-1").unwrap();
        assert!(supplier_wallet.balance_credits > 0.0);
    }

    #[test]
    fn test_settle_insufficient_balance() {
        let db = create_test_db();
        let engine = SettlementEngine::new(db);

        let result = TaskResult {
            task_id: "task-2".to_string(),
            status: TaskStatus::Completed,
            content: "Hello".to_string(),
            usage: Some(TokenUsage {
                prompt_tokens: 10000,
                completion_tokens: 10000,
                total_tokens: 20000,
            }),
            error: None,
            sequence: None,
            r#final: None,
        };

        let outcome = engine.settle("poor-consumer", "supplier-1", "claude-3-5-sonnet", &result);
        assert!(outcome.is_err());
    }

    #[test]
    fn test_settle_failed_task() {
        let db = create_test_db();
        let engine = SettlementEngine::new(db);

        let result = TaskResult {
            task_id: "task-3".to_string(),
            status: TaskStatus::Failed,
            content: String::new(),
            usage: None,
            error: Some("执行失败".to_string()),
            sequence: None,
            r#final: None,
        };

        let outcome = engine.settle("consumer-1", "supplier-1", "claude-3-5-sonnet", &result);
        assert!(outcome.is_err());
    }
}