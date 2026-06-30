//! CC-Share 错误类型

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShareError {
    #[error("数据库错误: {0}")]
    Database(String),

    #[error("WebSocket 连接错误: {0}")]
    Connection(String),

    #[error("内容过滤拦截: {0}")]
    ContentFiltered(String),

    #[error("节点忙碌")]
    NodeBusy,

    #[error("余额不足")]
    InsufficientBalance,

    #[error("云端调度错误: {0}")]
    Dispatch(String),
}

impl From<rusqlite::Error> for ShareError {
    fn from(err: rusqlite::Error) -> Self {
        ShareError::Database(err.to_string())
    }
}