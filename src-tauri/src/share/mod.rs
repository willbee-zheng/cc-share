//! P2P 共享模块
//!
//! 负责 CC-Share 的核心 P2P 功能：
//! - 供应者模式：接收云端任务，本地执行并返回结果
//! - 消费者模式：将请求代理到共享池
//! - WebSocket 长连接客户端：与云端调度服务器通信
//! - 互斥与安全：检测本地使用状态，管理并发

pub mod client;
pub mod consumer;
pub mod daemon;
pub mod executor;
pub mod fingerprint;
pub mod humanizer;
pub mod mutex;
pub mod protocol;
pub mod signing;
pub mod supplier;
pub mod web_bridge;
pub mod web_executor;