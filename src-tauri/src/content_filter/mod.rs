//! 内容过滤模块
//!
//! 负责对云端下发的请求进行安全审查：
//! - 关键词黑名单过滤
//! - 防止恶意请求污染供应者账号
//! - 过滤日志记录

pub mod rules;

pub use rules::ContentFilter;