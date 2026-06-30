import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Play, Square, Copy, LogOut, LogIn, Loader2, User } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  getClientConfig,
  setClientConfig,
  startLocalServer,
  stopLocalServer,
  getLocalServerAddr,
  authLogout,
  getAuthState,
  authBrowserLogin,
  type ClientConfig,
  type AuthState,
  type AuthError,
} from "@/lib/api";
import { subscribeAuthStateChanged } from "@/lib/events";
import { ApiKeyManager } from "@/auth/ApiKeyManager";

interface SettingsPanelProps {
  onServerHostChanged?: (host: string) => void;
  onSignInNeeded?: () => void;
}

export function SettingsPanel({ onServerHostChanged, onSignInNeeded }: SettingsPanelProps) {
  const { t } = useTranslation();
  const [config, setConfig] = useState<ClientConfig>(DEFAULT_CONFIG);
  const [saving, setSaving] = useState(false);
  const [serverAddr, setServerAddr] = useState("");
  const [busy, setBusy] = useState(false);
  const [authState, setAuthState] = useState<AuthState | null>(null);
  const [loggingOut, setLoggingOut] = useState(false);
  const [signingIn, setSigningIn] = useState(false);

  useEffect(() => {
    getClientConfig()
      .then((c) => {
        setConfig(c);
        onServerHostChanged?.(c.server_host);
      })
      .catch(() => undefined);
    getLocalServerAddr().then(setServerAddr).catch(() => undefined);
    getAuthState().then(setAuthState).catch(() => undefined);
  }, [onServerHostChanged]);

  // Subscribe to auth state changes (login, logout, token refresh)
  // so the account card stays in sync even when login happens elsewhere.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    subscribeAuthStateChanged((state) => {
      setAuthState(state);
    }).then((fn) => {
      unlisten = fn;
    }).catch(() => undefined);
    return () => {
      unlisten?.();
    };
  }, []);

  function update<K extends keyof ClientConfig>(k: K, v: ClientConfig[K]) {
    setConfig((prev) => ({ ...prev, [k]: v }));
  }

  async function save() {
    setSaving(true);
    try {
      await setClientConfig(config);
      onServerHostChanged?.(config.server_host);
    } finally {
      setSaving(false);
    }
  }

  async function toggleServer() {
    setBusy(true);
    try {
      if (serverAddr) {
        await stopLocalServer();
        setServerAddr("");
      } else {
        const addr = await startLocalServer();
        setServerAddr(addr);
      }
    } finally {
      setBusy(false);
    }
  }

  async function handleLogout() {
    setLoggingOut(true);
    try {
      await authLogout(config.server_host);
      setAuthState(null);
    } catch {
      // Clear local state even if server call fails
      setAuthState(null);
    } finally {
      setLoggingOut(false);
    }
  }

  const openaiBase = serverAddr ? `http://${serverAddr}/v1` : "";

  return (
    <div className="space-y-4">
      <h2 className="text-base font-semibold">{t("share.tabs.settings")}</h2>

      {/* Account Card — replaces manual auth_token / hmac_secret */}
      <Card>
        <CardHeader>
          <CardTitle>{t("auth:profile.title")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          {authState ? (
            <div className="space-y-3">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-full bg-black/5 dark:bg-white/10">
                  <User className="h-5 w-5 text-muted-foreground" />
                </div>
                <div>
                  <p className="font-medium">{authState.display_name || authState.email}</p>
                  <p className="text-xs text-muted-foreground">{authState.email}</p>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                <div>
                  <span className="font-medium text-foreground">{t("auth:profile.role")}:</span> {authState.role}
                </div>
              </div>
              <Button
                size="sm"
                variant="ghost"
                className="text-red-600 hover:text-red-700"
                onClick={() => void handleLogout()}
                disabled={loggingOut}
              >
                {loggingOut ? (
                  <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                ) : (
                  <LogOut className="mr-1 h-3.5 w-3.5" />
                )}
                {loggingOut ? t("auth:profile.loggingOut") : t("auth:profile.logout")}
              </Button>
            </div>
          ) : (
            <div className="flex items-center gap-3">
              <p className="text-muted-foreground">{t("auth:profile.notLoggedIn")}</p>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  if (onSignInNeeded) {
                    onSignInNeeded();
                  } else {
                    setSigningIn(true);
                    getClientConfig()
                      .then((cfg) => {
                        if (!cfg.server_host) return;
                        return authBrowserLogin(cfg.server_host);
                      })
                      .then((state) => { if (state) setAuthState(state); })
                      .catch(() => undefined)
                      .finally(() => setSigningIn(false));
                  }
                }}
                disabled={signingIn}
              >
                {signingIn ? (
                  <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                ) : (
                  <LogIn className="mr-1 h-3.5 w-3.5" />
                )}
                {signingIn ? t("auth:browserLogin.waiting") : t("auth:browserLogin.button")}
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Cloud server config (without auth_token / hmac_secret) */}
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.cloud", { defaultValue: "Cloud server" })}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          <Field
            label={t("settings.serverHost", { defaultValue: "Server Address" })}
            value={config.server_host}
            onChange={(v) => update("server_host", v)}
            placeholder="api.cc-share.com 或 192.168.1.60:8080"
          />
          <div className="flex items-center gap-2">
            <input
              type="checkbox"
              id="use-https"
              checked={config.use_https}
              onChange={(e) => update("use_https", e.target.checked)}
              className="h-4 w-4 rounded border-black/15 dark:border-white/15"
            />
            <label htmlFor="use-https" className="text-xs text-muted-foreground">
              {t("settings.useHttps", { defaultValue: "Use HTTPS (enable for production)" })}
            </label>
          </div>
          <Field
            label={t("settings.nodeId", { defaultValue: "Node ID" })}
            value={config.node_id}
            onChange={(v) => update("node_id", v)}
            placeholder="node-alice-01"
          />
          <div className="grid grid-cols-2 gap-3">
            <Field
              label={t("settings.heartbeat", { defaultValue: "Heartbeat (sec)" })}
              value={String(config.heartbeat_interval_secs)}
              onChange={(v) => update("heartbeat_interval_secs", Number(v) || 30)}
            />
            <Field
              label={t("settings.reconnect", { defaultValue: "Reconnect (sec)" })}
              value={String(config.max_reconnect_interval_secs)}
              onChange={(v) => update("max_reconnect_interval_secs", Number(v) || 60)}
            />
          </div>
          <Button size="sm" onClick={() => void save()} disabled={saving}>
            {saving ? "…" : t("settings.save", { defaultValue: "Save" })}
          </Button>
        </CardContent>
      </Card>

      {/* API Key management */}
      {config.server_host && (
        <ApiKeyManager serverHost={config.server_host} />
      )}

      {/* Local OpenAI server */}
      <Card>
        <CardHeader>
          <CardTitle>
            {t("settings.localServer", { defaultValue: "Local OpenAI server" })}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          <p className="text-xs text-muted-foreground">
            {t("settings.localServerHint", {
              defaultValue:
                "Starts a local OpenAI-compatible server. Add it as a custom provider in cc-switch (or any OpenAI client) to consume the SharePlan pool.",
            })}
          </p>
          <div className="flex items-center gap-2">
            <Button size="sm" variant={serverAddr ? "destructive" : "default"} onClick={() => void toggleServer()} disabled={busy}>
              {serverAddr ? <Square className="mr-1 h-3.5 w-3.5" /> : <Play className="mr-1 h-3.5 w-3.5" />}
              {serverAddr
                ? t("settings.stop", { defaultValue: "Stop" })
                : t("settings.start", { defaultValue: "Start" })}
            </Button>
            {serverAddr && (
              <Badge variant="success">listening on {serverAddr}</Badge>
            )}
          </div>
          {openaiBase && (
            <div className="flex items-center gap-2 rounded-md border border-black/10 p-2 dark:border-white/10">
              <code className="flex-1 text-xs">{openaiBase}</code>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void navigator.clipboard.writeText(openaiBase)}
              >
                <Copy className="h-3.5 w-3.5" />
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function Field({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
}) {
  return (
    <div>
      <label className="mb-1 block text-xs font-medium text-muted-foreground">{label}</label>
      <input
        type={type}
        className="flex h-9 w-full rounded-md border border-black/15 bg-transparent px-3 py-1 text-sm dark:border-white/15"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

const DEFAULT_CONFIG: ClientConfig = {
  server_host: "",
  heartbeat_interval_secs: 30,
  max_reconnect_interval_secs: 60,
  auth_token: "",
  node_id: "",
  hmac_secret: "",
  use_https: false,
};