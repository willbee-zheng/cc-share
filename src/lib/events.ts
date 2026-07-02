//! Tauri 事件订阅辅助
//!
//! 提供与 cc-share Rust 后端事件的类型安全订阅。

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Role } from "./api";
import type { AuthState } from "./api";

/** 与 Rust 端 ConnectionState 枚举对齐 */
export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "reconnecting";

/** 与 Rust 端 TaskStatus 枚举对齐 */
export type TaskStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "rejected"
  | "busy";

/** P2P session state — mirrors Rust P2PSessionState */
export type P2PSessionState =
  | "awaiting_answer"
  | "connecting"
  | "connected"
  | "executing"
  | "completed"
  | "failed";

/** P2P connection status — mirrors Rust P2PConnStatus */
export type P2PConnStatus =
  | "started"
  | "stopped"
  | "peer_connected"
  | "peer_disconnected";

/** P2P session state event payload */
export interface P2PSessionEvent {
  session_id: string;
  state: P2PSessionState;
  peer_address?: string;
  model?: string;
  error?: string;
}

/** P2P connection status event payload */
export interface P2PConnectionEvent {
  status: P2PConnStatus;
  port?: number;
  peer_address?: string;
  active_connections: number;
}

/** 连接错误详情 */
export interface ConnectionErrorEvent {
  category: "auth" | "path" | "network" | "tls" | "other";
  message: string;
}

export interface TaskFinishedEvent {
  task_id: string;
  status: TaskStatus;
  latency_ms: number;
}

/** 服务器健康状态 */
export interface ServerHealth {
  healthy: boolean;
  latency_ms: number;
  error: string | null;
}

const EVENTS = {
  CONNECTION_STATE: "share:connection-state",
  CONNECTION_ERROR: "share:connection-error",
  TASK_FINISHED: "share:task-finished",
  ROLE_CHANGED: "share:role-changed",
  LOG_APPENDED: "share:log-appended",
  AUTH_STATE_CHANGED: "share:auth-state-changed",
  P2P_SESSION_STATE: "share:p2p-session-state",
  P2P_CONNECTION_STATUS: "share:p2p-connection-status",
} as const;

/** 订阅连接状态变化。返回的函数取消订阅。 */
export function subscribeConnectionState(
  handler: (state: ConnectionState) => void,
): Promise<UnlistenFn> {
  return listen<ConnectionState>(EVENTS.CONNECTION_STATE, (e) => handler(e.payload));
}

/** 订阅连接错误详情。 */
export function subscribeConnectionError(
  handler: (error: ConnectionErrorEvent) => void,
): Promise<UnlistenFn> {
  return listen<ConnectionErrorEvent>(EVENTS.CONNECTION_ERROR, (e) => handler(e.payload));
}

/** 订阅任务完成事件。 */
export function subscribeTaskFinished(
  handler: (evt: TaskFinishedEvent) => void,
): Promise<UnlistenFn> {
  return listen<TaskFinishedEvent>(EVENTS.TASK_FINISHED, (e) => handler(e.payload));
}

/** 订阅角色变化事件。返回的函数取消订阅。 */
export function subscribeRoleChanged(
  handler: (role: Role) => void,
): Promise<UnlistenFn> {
  return listen<string>(EVENTS.ROLE_CHANGED, (e) => {
    const role: Role = JSON.parse(e.payload);
    handler(role);
  });
}

/** 订阅日志追加事件（payload 为新增条目数）。 */
export function subscribeLogAppended(
  handler: (count: number) => void,
): Promise<UnlistenFn> {
  return listen<number>(EVENTS.LOG_APPENDED, (e) => handler(e.payload));
}

/** 订阅认证状态变化事件（登录/登出/token 刷新时触发）。 */
export function subscribeAuthStateChanged(
  handler: (state: AuthState | null) => void,
): Promise<UnlistenFn> {
  return listen<AuthState | null>(EVENTS.AUTH_STATE_CHANGED, (e) => handler(e.payload));
}

/** 订阅 P2P 会话状态变化事件。 */
export function subscribeP2PSessionState(
  handler: (event: P2PSessionEvent) => void,
): Promise<UnlistenFn> {
  return listen<P2PSessionEvent>(EVENTS.P2P_SESSION_STATE, (e) => handler(e.payload));
}

/** 订阅 P2P 连接状态变化事件。 */
export function subscribeP2PConnectionStatus(
  handler: (event: P2PConnectionEvent) => void,
): Promise<UnlistenFn> {
  return listen<P2PConnectionEvent>(EVENTS.P2P_CONNECTION_STATUS, (e) => handler(e.payload));
}