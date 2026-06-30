//! 系统托盘状态绑定
//!
//! cc-share 不直接修改 cc-switch 的 tray.rs（"最小改动"原则）。
//! 这个模块在前端订阅 daemon 事件 + 钱包变动，并在文档标题里反映状态，
//! 同时通过 `app://shareplan/tray` 事件名向上层广播托盘建议项（status 文本、
//! 颜色），由集成方在 tray.rs 端订阅渲染（接缝点见下方注释）。

import { useEffect } from "react";
import { emit } from "@tauri-apps/api/event";
import {
  subscribeConnectionState,
  subscribeTaskFinished,
  type ConnectionState,
} from "./events";

const TRAY_EVENT = "shareplan:tray-suggest";

interface TraySuggestion {
  /** 建议的托盘 status（cc-switch tray.rs 决定如何映射成图标） */
  status: "online" | "idle" | "offline" | "error";
  /** Tooltip 文本（会拼到 cc-switch 默认 tooltip 后面） */
  tooltip: string;
  /** 最近一次任务完成的简要信息（可选） */
  last_task?: {
    completed: boolean;
    latency_ms: number;
  };
}

const STATE_TO_TRAY: Record<ConnectionState, TraySuggestion["status"]> = {
  connected: "online",
  reconnecting: "error",
  connecting: "idle",
  disconnected: "offline",
};

const STATE_TO_TOOLTIP: Record<ConnectionState, string> = {
  connected: "CC-Share: 挂机中",
  reconnecting: "CC-Share: 重连中",
  connecting: "CC-Share: 连接中",
  disconnected: "CC-Share: 未连接",
};

/**
 * 在 React 树根节点调用一次，把 cc-share 的连接状态/任务事件
 * 桥接到 tray suggestion 事件流。
 *
 * cc-switch 集成方（tray.rs）在接收端：
 *
 *     app.listen("shareplan:tray-suggest", |evt| { ... });
 */
export function useTrayStatusBridge(): void {
  useEffect(() => {
    let cancelled = false;
    const cleanups: Array<() => void> = [];

    void subscribeConnectionState((state) => {
      if (cancelled) return;
      const suggestion: TraySuggestion = {
        status: STATE_TO_TRAY[state],
        tooltip: STATE_TO_TOOLTIP[state],
      };
      void emit(TRAY_EVENT, suggestion);
      // window title fallback：在 tray.rs 接入前先反映在标题栏
      document.title = `${suggestion.tooltip} — CC-Switch`;
    }).then((un) => cleanups.push(un));

    void subscribeTaskFinished((evt) => {
      if (cancelled) return;
      const suggestion: TraySuggestion = {
        status: evt.status === "completed" ? "online" : "error",
        tooltip:
          evt.status === "completed"
            ? `CC-Share: 任务完成 (${evt.latency_ms}ms)`
            : `CC-Share: 任务失败 (${evt.status})`,
        last_task: { completed: evt.status === "completed", latency_ms: evt.latency_ms },
      };
      void emit(TRAY_EVENT, suggestion);
    }).then((un) => cleanups.push(un));

    return () => {
      cancelled = true;
      cleanups.forEach((c) => c());
    };
  }, []);
}
