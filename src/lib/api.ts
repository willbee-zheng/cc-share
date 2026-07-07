//! CC-Share Tauri 命令的类型安全调用层
//!
//! 把 `invoke()` 包装成强类型函数，前端组件直接调用即可，不再各处重复
//! 写 plugin 命令名字符串和类型断言。

import { invoke } from "@tauri-apps/api/core";

// Standalone Tauri app: commands are registered directly on the builder,
// no plugin prefix. (Legacy plugin used `plugin:shareplan|<name>`.)
const cmd = (name: string) => name;

// ---------- ClientConfig ----------

export interface ClientConfig {
  server_host: string;
  heartbeat_interval_secs: number;
  max_reconnect_interval_secs: number;
  auth_token: string;
  node_id: string;
  hmac_secret: string;
  /** Whether to use HTTPS/WSS for cloud connections. Enable for production. */
  use_https: boolean;
}

export const DEFAULT_CLIENT_CONFIG: ClientConfig = {
  server_host: "",
  heartbeat_interval_secs: 30,
  max_reconnect_interval_secs: 60,
  auth_token: "",
  node_id: "",
  hmac_secret: "",
  use_https: false,
};

export function getClientConfig(): Promise<ClientConfig> {
  return invoke<ClientConfig>(cmd("get_client_config"));
}

export function setClientConfig(config: ClientConfig): Promise<void> {
  return invoke<void>(cmd("set_client_config"), { config });
}

// ---------- Share connect / disconnect ----------

export interface ShareConnectArgs {
  node_id?: string;
  available_models?: string[];
  max_concurrency?: number;
}

export function shareConnect(args: ShareConnectArgs): Promise<void> {
  return invoke<void>(cmd("share_connect"), { args });
}

export function shareDisconnect(): Promise<void> {
  return invoke<void>(cmd("share_disconnect"));
}

export function shareGetStatus(): Promise<string> {
  return invoke<string>(cmd("share_get_status"));
}

// ---------- Share settings (per-provider) ----------

export interface ShareSettings {
  provider_id: string;
  app_type: string;
  is_sharing: boolean;
  max_token_per_min: number;
  token_unit_price: number;
  concurrency_limit: number;
  cooldown_seconds: number;
}

export function getAllSharingProviders(): Promise<ShareSettings[]> {
  return invoke<ShareSettings[]>(cmd("get_all_sharing_providers"));
}

export function getShareSettings(providerId: string, appType: string): Promise<ShareSettings | null> {
  return invoke<ShareSettings | null>(cmd("get_share_settings"), {
    providerId,
    appType,
  });
}

export function upsertShareSettings(settings: ShareSettings): Promise<void> {
  return invoke<void>(cmd("upsert_share_settings"), { settings });
}

export function deleteShareSettings(providerId: string, appType: string): Promise<void> {
  return invoke<void>(cmd("delete_share_settings"), { providerId, appType });
}

// ---------- Wallet ----------

export interface UserWallet {
  user_id: string;
  balance_credits: number;
  total_earned: number;
  total_spent: number;
  last_sync_at: number | null;
}

export interface P2PTaskLog {
  task_id: string;
  direction: "consume" | "supply";
  model: string;
  upstream_model: string | null;
  tokens_prompt: number;
  tokens_completion: number;
  credits: number;
  latency_ms: number | null;
  status: "pending" | "running" | "completed" | "failed" | "rejected" | "busy";
  error_message: string | null;
  created_at: number;
}

export interface HourlyPoint {
  bucket_unix: number;
  supplied_tokens: number;
  consumed_tokens: number;
}

export interface WalletSummary {
  wallet: UserWallet;
  today_supplied_tokens: number;
  today_consumed_tokens: number;
  total_supplied_tokens: number;
  total_consumed_tokens: number;
  hourly_trend: HourlyPoint[];
  recent_logs: P2PTaskLog[];
}

export function getWallet(userId: string): Promise<UserWallet> {
  return invoke<UserWallet>(cmd("get_wallet"), { userId });
}

export function getWalletSummary(userId: string): Promise<WalletSummary> {
  return invoke<WalletSummary>(cmd("get_wallet_summary"), { userId });
}

export function syncWallet(): Promise<UserWallet> {
  return invoke<UserWallet>(cmd("sync_wallet"));
}

export function getRecentTaskLogs(
  direction: P2PTaskLog["direction"] | null,
  limit: number,
): Promise<P2PTaskLog[]> {
  return invoke<P2PTaskLog[]>(cmd("get_recent_task_logs"), { direction, limit });
}

export interface ModelTokenStat {
  model: string;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  task_count: number;
}

export function getSupplierTokenByModel(days: number): Promise<ModelTokenStat[]> {
  return invoke<ModelTokenStat[]>(cmd("get_supplier_token_by_model"), { days });
}

export function getConsumerTokenByModel(days: number): Promise<ModelTokenStat[]> {
  return invoke<ModelTokenStat[]>(cmd("get_consumer_token_by_model"), { days });
}

// ---------- Share Pool consume (consumer-side) ----------

export interface ConsumeArgs {
  model: string;
  messages: unknown;
  stream?: boolean;
  params?: unknown;
  est_prompt_tokens?: number;
  max_output_tokens?: number;
}

export interface TokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

export interface ConsumeResult {
  success: boolean;
  content: string;
  usage: TokenUsage | null;
  error: string | null;
  node_id: string | null;
}

export interface ShareNode {
  node_id: string;
  models: string;
  price: number;
  status: "idle" | "busy" | "offline";
  latency_ms: number | null;
  last_heartbeat: number | null;
}

export function shareConsume(args: ConsumeArgs): Promise<ConsumeResult> {
  return invoke<ConsumeResult>(cmd("share_consume"), { args });
}

export function listShareNodes(): Promise<ShareNode[]> {
  return invoke<ShareNode[]>(cmd("list_share_nodes"));
}

// ---------- cc-switch proxy discovery + whitelist (Phase 4/6) ----------

export interface ActiveTarget {
  app_type: string;
  provider_name: string;
  provider_id: string;
  api_format: "Anthropic" | "OpenAiChat" | "OpenAiResponses" | "GeminiNative";
  models: string[];
  /** Mapping from representative model name to real upstream model name for this provider. */
  upstream_models: Record<string, string>;
}

export interface DiscoverySnapshot {
  reachable: boolean;
  running: boolean;
  current_provider: string | null;
  providers: ActiveTarget[];
  /** Whether providers were read from cc-switch's database (not confirmed by traffic). */
  from_db: boolean;
  available_formats: string[];
  available_models: string[];
  /** Mapping from representative model name to real upstream model name.
   *  E.g., {"claude-sonnet-4": "glm-5.1:cloud"} */
  upstream_models: Record<string, string>;
  last_error: string | null;
}

export interface DiagnosticWarning {
  code: string;
  message: string;
  severity: "info" | "warn" | "error";
}

export function refreshProviders(): Promise<DiscoverySnapshot> {
  return invoke<DiscoverySnapshot>(cmd("refresh_providers"));
}

export function listProxyProviders(): Promise<DiscoverySnapshot> {
  return invoke<DiscoverySnapshot>(cmd("list_proxy_providers"));
}

export function getDiagnostics(): Promise<DiagnosticWarning[]> {
  return invoke<DiagnosticWarning[]>(cmd("get_diagnostics"));
}

export function getWhitelist(): Promise<string[]> {
  return invoke<string[]>(cmd("get_whitelist"));
}

export function setWhitelist(models: string[]): Promise<void> {
  return invoke<void>(cmd("set_whitelist"), { models });
}

export function getShareableModels(): Promise<string[]> {
  return invoke<string[]>(cmd("get_shareable_models"));
}

// ---------- Local OpenAI server (Phase 9) ----------

export function startLocalServer(bindAddr?: string): Promise<string> {
  return invoke<string>(cmd("start_local_server"), { bindAddr: bindAddr ?? null });
}

export function stopLocalServer(): Promise<void> {
  return invoke<void>(cmd("stop_local_server"));
}

export function getLocalServerAddr(): Promise<string> {
  return invoke<string>(cmd("get_local_server_addr"));
}

// ---------- Consumer role & config ----------

export type Role = "supplier" | "consumer" | "idle";

export function getRole(): Promise<string> {
  return invoke<string>(cmd("get_role"));
}

export function setRole(role: Role): Promise<void> {
  return invoke<void>(cmd("set_role"), { role: JSON.stringify(role) });
}

export interface ConsumerConfig {
  id: string;
  name: string;
  settingsConfig: Record<string, unknown>;
  category: string;
  icon?: string;
  iconColor?: string;
}

export function generateConsumerConfig(
  appType: string,
  model?: string,
  bindAddr?: string,
): Promise<ConsumerConfig> {
  return invoke<ConsumerConfig>(cmd("generate_consumer_config"), {
    appType,
    model: model ?? null,
    bindAddr: bindAddr ?? null,
  });
}

export function getConsumerProxyAddr(): Promise<string> {
  return invoke<string>(cmd("get_consumer_proxy_addr"));
}

// ---------- Auth ----------

export interface AuthState {
  user_id: string;
  email: string;
  display_name: string;
  role: string;
  access_token: string;
  refresh_token: string;
  access_expires_at: number;
}

export interface UserProfile {
  id: string;
  email: string;
  display_name: string;
  role: string;
  status: string;
  created_at: string;
  updated_at: string;
  last_login_at: string | null;
}

export interface ApiKeyInfo {
  id: string;
  name: string;
  key_prefix: string;
  permissions: string[];
  status: string;
  last_used_at: string | null;
  created_at: string;
}

export interface CreateKeyResponse {
  id: string;
  name: string;
  key: string;
  key_prefix: string;
  permissions: string[];
  created_at: string;
}

export type AuthErrorKind =
  | "network"
  | "unauthorized"
  | "email_exists"
  | "invalid_credentials"
  | "token_expired"
  | "validation"
  | "server";

export interface AuthError {
  kind: AuthErrorKind;
  message: string;
}

function parseAuthError(err: string): AuthError {
  // Tauri commands return `Result<T, String>`, so the error is a stringified AuthError or plain message.
  try {
    const parsed = JSON.parse(err);
    if (parsed.kind) {
      return { kind: parsed.kind as AuthErrorKind, message: parsed.message ?? err };
    }
  } catch { /* not JSON */ }
  return { kind: "server", message: err };
}

export function getAuthState(): Promise<AuthState | null> {
  return invoke<AuthState | null>(cmd("get_auth_state"));
}

export async function authBrowserLogin(serverHost: string): Promise<AuthState> {
  try {
    return await invoke<AuthState>(cmd("auth_browser_login"), { serverHost });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authRegister(
  serverHost: string,
  email: string,
  password: string,
  displayName?: string,
): Promise<AuthState> {
  try {
    return await invoke<AuthState>(cmd("auth_register"), {
      serverHost,
      email,
      password,
      displayName: displayName ?? null,
    });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authLogin(
  serverHost: string,
  email: string,
  password: string,
): Promise<AuthState> {
  try {
    return await invoke<AuthState>(cmd("auth_login"), {
      serverHost,
      email,
      password,
    });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authLogout(serverHost: string): Promise<void> {
  return invoke<void>(cmd("auth_logout"), { serverHost });
}

export async function authRefresh(serverHost: string): Promise<AuthState> {
  try {
    return await invoke<AuthState>(cmd("auth_refresh"), { serverHost });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authChangePassword(
  serverHost: string,
  currentPassword: string,
  newPassword: string,
): Promise<void> {
  try {
    return invoke<void>(cmd("auth_change_password"), {
      serverHost,
      currentPassword,
      newPassword,
    });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authGetProfile(serverHost: string): Promise<UserProfile> {
  try {
    return invoke<UserProfile>(cmd("auth_get_profile"), { serverHost });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authCreateApiKey(
  serverHost: string,
  name: string,
  permissions: string[],
): Promise<CreateKeyResponse> {
  try {
    return invoke<CreateKeyResponse>(cmd("auth_create_api_key"), {
      serverHost,
      name,
      permissions,
    });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authListApiKeys(serverHost: string): Promise<ApiKeyInfo[]> {
  try {
    return invoke<ApiKeyInfo[]>(cmd("auth_list_api_keys"), { serverHost });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

export async function authRevokeApiKey(
  serverHost: string,
  keyId: string,
): Promise<void> {
  try {
    return invoke<void>(cmd("auth_revoke_api_key"), { serverHost, keyId });
  } catch (e) {
    throw parseAuthError(String(e));
  }
}

// ---------- Stats sync ----------

export interface DailySyncRow {
  stat_date: string;
  direction: string;
  model: string;
  upstream_model: string;
  prompt_tokens: number;
  completion_tokens: number;
  task_count: number;
  credits: number;
}

export interface SyncResult {
  pushed: number;
  accepted: number;
  summary: CloudStatsSummary | null;
  error: string | null;
}

export interface CloudDailyStat {
  stat_date: string;
  direction: string;
  model: string;
  upstream_model: string;
  prompt_tokens: number;
  completion_tokens: number;
  task_count: number;
  credits: number;
}

export interface CloudStatsSummary {
  daily_stats: CloudDailyStat[];
  total_supplied_tokens: number;
  total_consumed_tokens: number;
}

export function syncDailyStats(): Promise<SyncResult> {
  return invoke<SyncResult>(cmd("sync_daily_stats"));
}

export function getCloudStatsSummary(): Promise<CloudStatsSummary> {
  return invoke<CloudStatsSummary>(cmd("get_cloud_stats_summary"));
}

// ---------- System logs ----------

export interface SystemLogEntry {
  id: number;
  timestamp_ms: number;
  level: string;
  target: string;
  message: string;
}

export interface LogFilter {
  level?: string;
  target?: string;
  search?: string;
  limit?: number;
  offset?: number;
}

export interface LogStats {
  total: number;
  debug: number;
  info: number;
  warn: number;
  error: number;
}

export function getSystemLogs(filter: LogFilter): Promise<SystemLogEntry[]> {
  return invoke<SystemLogEntry[]>(cmd("get_system_logs"), { filter });
}

export function clearSystemLogs(): Promise<void> {
  return invoke<void>(cmd("clear_system_logs"));
}

export function getSystemLogStats(): Promise<LogStats> {
  return invoke<LogStats>(cmd("get_system_log_stats"));
}

export function listSystemLogTargets(): Promise<string[]> {
  return invoke<string[]>(cmd("list_system_log_targets"));
}

export function setLogLevel(level: string): Promise<void> {
  return invoke<void>(cmd("set_log_level"), { level });
}

export function pruneSystemLogs(keepDays: number): Promise<number> {
  return invoke<number>(cmd("prune_system_logs"), { keepDays });
}

// ---------- P2P status ----------

export interface P2PStatus {
  enabled: boolean;
  running: boolean;
  port: number;
  public_key: string;
  local_addresses: string[];
  active_connections: number;
  hole_punch_retries: number;
  hole_punch_delay_ms: number;
  stun_server: string;
}

export interface P2PConfig {
  enabled: boolean;
  hole_punch_retries: number;
  hole_punch_delay_ms: number;
  stun_server: string;
  p2p_port: number;
}

export interface P2PPublicAddr {
  public_addr: string;
  stun_server: string;
}

export function p2pGetStatus(): Promise<P2PStatus> {
  return invoke<P2PStatus>(cmd("p2p_get_status"));
}

export function p2pGetPublicKey(): Promise<string> {
  return invoke<string>(cmd("p2p_get_public_key"));
}

export function p2pStart(): Promise<void> {
  return invoke<void>(cmd("p2p_start"));
}

export function p2pStop(): Promise<void> {
  return invoke<void>(cmd("p2p_stop"));
}

export function p2pGetConfig(): Promise<P2PConfig> {
  return invoke<P2PConfig>(cmd("p2p_get_config"));
}

export function p2pSetConfig(config: P2PConfig): Promise<void> {
  return invoke<void>(cmd("p2p_set_config"), { config });
}

export function p2pDiscoverPublicAddr(): Promise<P2PPublicAddr> {
  return invoke<P2PPublicAddr>(cmd("p2p_discover_public_addr"));
}

