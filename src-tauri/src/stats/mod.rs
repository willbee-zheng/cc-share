//! CC-Share 统计同步模块
//!
//! 安全设计要点：
//! 1. 本地 p2p_task_log 是客户端自己的记录，仅用于本地展示
//! 2. 云端 daily_stats 由云服务在 Finalize 时写入（权威来源）
//! 3. 客户端推送本地聚合数据到云端，云端交叉验证后才采纳
//! 4. 客户端拉取云端权威摘要用于钱包/统计面板展示
//! 5. 防刷：本地任务记录只统计 status='completed' 的任务，
//!    云端会与自己的 billing 记录交叉验证，超额部分会被拒绝

pub mod sync;

pub use sync::StatsSyncer;