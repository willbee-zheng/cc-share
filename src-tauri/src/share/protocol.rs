//! P2P 通信协议
//!
//! 定义客户端与云端调度服务器之间的消息格式，
//! 包括任务分发、状态上报、心跳等。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 云端下发的任务 Payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPayload {
    /// 任务唯一 ID
    pub task_id: String,
    /// 目标模型
    pub model: String,
    /// 对话消息列表
    pub messages: serde_json::Value,
    /// 是否流式返回
    pub stream: bool,
    /// 可选参数（temperature, max_tokens 等）
    #[serde(default)]
    pub params: serde_json::Value,
}

/// 任务执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// 任务 ID
    pub task_id: String,
    /// 执行状态
    pub status: TaskStatus,
    /// 返回内容（流式为 chunk，非流式为完整响应）
    pub content: String,
    /// Token 用量统计
    pub usage: Option<TokenUsage>,
    /// 错误信息
    pub error: Option<String>,
    /// 流式 chunk 序号（非流式时为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    /// 是否为最终帧（流式时 terminal chunk 设为 Some(true)）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#final: Option<bool>,
}

impl TaskResult {
    /// Create a non-streaming terminal result (completed or failed).
    pub fn terminal(task_id: &str, status: TaskStatus, content: String, usage: Option<TokenUsage>, error: Option<String>) -> Self {
        Self {
            task_id: task_id.to_string(),
            status,
            content,
            usage,
            error,
            sequence: None,
            r#final: None,
        }
    }

    /// Create a streaming delta chunk (status=Running).
    pub fn running_chunk(task_id: &str, content: String, sequence: u64) -> Self {
        Self {
            task_id: task_id.to_string(),
            status: TaskStatus::Running,
            content,
            usage: None,
            error: None,
            sequence: Some(sequence),
            r#final: Some(false),
        }
    }

    /// Create a streaming terminal chunk (status=Completed, with usage).
    pub fn completed_chunk(task_id: &str, usage: TokenUsage, sequence: u64) -> Self {
        Self {
            task_id: task_id.to_string(),
            status: TaskStatus::Completed,
            content: String::new(),
            usage: Some(usage),
            error: None,
            sequence: Some(sequence),
            r#final: Some(true),
        }
    }

    /// Create a streaming terminal chunk (status=Failed, with error).
    pub fn failed_chunk(task_id: &str, error: String, sequence: u64) -> Self {
        Self {
            task_id: task_id.to_string(),
            status: TaskStatus::Failed,
            content: String::new(),
            usage: None,
            error: Some(error),
            sequence: Some(sequence),
            r#final: Some(true),
        }
    }
}

/// 任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Rejected,
    Busy,
}

/// Token 用量
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// 节点状态上报
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    /// 节点 ID
    pub node_id: String,
    /// 在线状态
    pub state: NodeState,
    /// 可用模型列表（representative names, e.g. "claude-sonnet-4"）
    pub available_models: Vec<String>,
    /// Mapping from representative model name to real upstream model name.
    /// E.g., {"claude-sonnet-4": "glm-5.1:cloud"}
    #[serde(default)]
    pub upstream_models: HashMap<String, String>,
    /// 当前并发数
    pub current_concurrency: u32,
    /// 最大并发数
    pub max_concurrency: u32,
}

/// 节点在线状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Idle,
    Busy,
    Offline,
}