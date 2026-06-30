//! 积分与钱包模块
//!
//! 负责 CC-Share 的积分体系：
//! - 本地钱包管理（余额、收入、支出）
//! - 计费清算（按 Token 用量计算积分）
//! - 定价策略（每模型每千 Token 的积分单价）

pub mod pricing;
pub mod settlement;
pub mod wallet;