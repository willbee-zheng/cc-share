//! 本地钱包管理
//!
//! 维护本地积分余额的镜像，定期与云端同步，
//! 提供余额查询、冻结/解冻预扣积分等操作。

use crate::database::ShareDb;
use crate::error::ShareError;
use std::sync::Arc;

/// 钱包管理器
///
/// 本地钱包是云端余额的镜像，通过定期同步保持一致。
/// 提供余额查询、预扣（冻结）、解冻等操作，
/// 用于 P2P 交易的实时计费。
pub struct WalletManager {
    db: Arc<ShareDb>,
}

impl WalletManager {
    /// 创建钱包管理器
    pub fn new(db: Arc<ShareDb>) -> Self {
        Self { db }
    }

    /// 获取钱包余额
    pub fn get_balance(&self, user_id: &str) -> Result<f64, ShareError> {
        let wallet = self.db.get_or_create_wallet(user_id)?;
        Ok(wallet.balance_credits)
    }

    /// 检查余额是否充足
    pub fn has_sufficient_balance(&self, user_id: &str, amount: f64) -> Result<bool, ShareError> {
        let balance = self.get_balance(user_id)?;
        Ok(balance >= amount)
    }

    /// 充值（增加余额）
    pub fn top_up(&self, user_id: &str, amount: f64) -> Result<(), ShareError> {
        if amount <= 0.0 {
            return Err(ShareError::Dispatch("充值金额必须大于 0".to_string()));
        }
        // 确保 wallet 行存在（update_wallet_balance 是 UPDATE，不会自动创建）
        self.db.get_or_create_wallet(user_id)?;
        self.db.update_wallet_balance(user_id, amount, 0.0)?;
        Ok(())
    }

    /// 消费（减少余额）
    pub fn spend(&self, user_id: &str, amount: f64) -> Result<(), ShareError> {
        if amount <= 0.0 {
            return Err(ShareError::Dispatch("消费金额必须大于 0".to_string()));
        }
        let balance = self.get_balance(user_id)?;
        if balance < amount {
            return Err(ShareError::InsufficientBalance);
        }
        self.db.update_wallet_balance(user_id, 0.0, amount)?;
        Ok(())
    }

    /// 获取钱包完整信息
    pub fn get_wallet_info(
        &self,
        user_id: &str,
    ) -> Result<crate::database::dao_credits::UserWallet, ShareError> {
        self.db.get_or_create_wallet(user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_wallet_manager() -> WalletManager {
        let db = Arc::new(ShareDb::memory().expect("创建内存数据库失败"));
        WalletManager::new(db)
    }

    #[test]
    fn test_new_wallet_has_zero_balance() {
        let mgr = create_wallet_manager();
        let balance = mgr.get_balance("user-1").unwrap();
        assert_eq!(balance, 0.0);
    }

    #[test]
    fn test_top_up() {
        let mgr = create_wallet_manager();
        mgr.top_up("user-1", 100.0).unwrap();
        let balance = mgr.get_balance("user-1").unwrap();
        assert!((balance - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_spend() {
        let mgr = create_wallet_manager();
        mgr.top_up("user-1", 100.0).unwrap();
        mgr.spend("user-1", 30.0).unwrap();
        let balance = mgr.get_balance("user-1").unwrap();
        assert!((balance - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_spend_insufficient_balance() {
        let mgr = create_wallet_manager();
        mgr.top_up("user-1", 10.0).unwrap();
        let result = mgr.spend("user-1", 20.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_has_sufficient_balance() {
        let mgr = create_wallet_manager();
        mgr.top_up("user-1", 100.0).unwrap();
        assert!(mgr.has_sufficient_balance("user-1", 50.0).unwrap());
        assert!(mgr.has_sufficient_balance("user-1", 100.0).unwrap());
        assert!(!mgr.has_sufficient_balance("user-1", 101.0).unwrap());
    }

    #[test]
    fn test_top_up_negative_amount() {
        let mgr = create_wallet_manager();
        let result = mgr.top_up("user-1", -10.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_wallet_info() {
        let mgr = create_wallet_manager();
        mgr.top_up("user-1", 50.0).unwrap();
        mgr.spend("user-1", 10.0).unwrap();

        let wallet = mgr.get_wallet_info("user-1").unwrap();
        assert!((wallet.balance_credits - 40.0).abs() < f64::EPSILON);
        assert!((wallet.total_earned - 50.0).abs() < f64::EPSILON);
        assert!((wallet.total_spent - 10.0).abs() < f64::EPSILON);
    }
}