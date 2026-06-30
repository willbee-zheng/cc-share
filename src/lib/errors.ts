//! 把后端 / 网络层抛出的错误字符串映射成友好中文提示
//!
//! 优先匹配错误码（如 `insufficient balance`、`no node available`），
//! 没有匹配时回落到原始字符串 — 这样既给最常见错误一个好看的提示，
//! 又不掩盖未识别的底层错误。

import i18next from "i18next";

interface ErrorMapping {
  /** 把错误字符串里出现这些 substring 中的任意一个就算命中 */
  match: string[];
  /** i18n key in the `share` namespace under `errors.<key>` */
  key: string;
}

const MAPPINGS: ErrorMapping[] = [
  { match: ["no node available", "no node"], key: "noNode" },
  { match: ["insufficient balance"], key: "insufficientBalance" },
  { match: ["task timeout"], key: "taskTimeout" },
  { match: ["fingerprint mismatch"], key: "fingerprintMismatch" },
  { match: ["invalid token", "missing bearer"], key: "invalidToken" },
  { match: ["nonce replayed"], key: "nonceReplay" },
  { match: ["timestamp outside allowed skew"], key: "clockSkew" },
  { match: ["invalid signature"], key: "invalidSignature" },
  { match: ["rate limit exceeded"], key: "rateLimited" },
  { match: ["no paired browser extension"], key: "noExtension" },
  { match: ["web provider busy"], key: "webBusy" },
  { match: ["web provider", "offline"], key: "webOffline" },
  { match: ["bad pairing token"], key: "badPairingToken" },
  { match: ["服务器地址未配置"], key: "serverUrlMissing" },
  { match: ["认证令牌未配置"], key: "authTokenMissing" },
  { match: ["node_id 未配置"], key: "nodeIdMissing" },
];

/**
 * 把任意错误（可能是 Tauri command 字符串、reqwest 错误、Error 实例）
 * 转成对终端用户友好的中文提示。
 */
export function friendlyError(err: unknown): string {
  const raw = errToString(err).toLowerCase();
  for (const m of MAPPINGS) {
    if (m.match.some((s) => raw.includes(s.toLowerCase()))) {
      return i18next.t(`share:errors.${m.key}`);
    }
  }
  // 兜底：原文截断展示，避免又长又乱
  const orig = errToString(err);
  return orig.length > 200 ? orig.slice(0, 200) + "…" : orig;
}

function errToString(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
