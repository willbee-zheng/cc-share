import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  DEFAULT_CLIENT_CONFIG,
  getClientConfig,
  setClientConfig,
  type ClientConfig,
} from "../lib/api";

interface Props {
  onSaved?: (config: ClientConfig) => void;
}

export function ShareConfigForm({ onSaved }: Props) {
  const { t } = useTranslation("share");
  const [config, setConfig] = useState<ClientConfig>(DEFAULT_CLIENT_CONFIG);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    void getClientConfig()
      .then((c) => {
        if (!cancelled) setConfig(c);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      await setClientConfig(config);
      setSavedAt(Date.now());
      onSaved?.(config);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  function update<K extends keyof ClientConfig>(key: K, value: ClientConfig[K]) {
    setConfig((prev) => ({ ...prev, [key]: value }));
  }

  if (loading) {
    return <p className="text-sm text-muted-foreground">{t("config.loading")}</p>;
  }

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="cc-share-server-host">{t("config.serverHost")}</Label>
        <Input
          id="cc-share-server-host"
          type="text"
          value={config.server_host}
          onChange={(e) => update("server_host", e.target.value)}
          placeholder="api.cc-share.com 或 192.168.1.60:8080"
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="cc-share-auth-token">{t("config.authToken")}</Label>
        <Input
          id="cc-share-auth-token"
          type="password"
          value={config.auth_token}
          onChange={(e) => update("auth_token", e.target.value)}
          placeholder={t("config.authTokenPlaceholder")}
          autoComplete="off"
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="cc-share-hmac-secret">{t("config.hmacSecret")}</Label>
        <Input
          id="cc-share-hmac-secret"
          type="password"
          value={config.hmac_secret}
          onChange={(e) => update("hmac_secret", e.target.value)}
          placeholder={t("config.hmacSecretPlaceholder")}
          autoComplete="off"
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="cc-share-node-id">{t("config.nodeId")}</Label>
        <Input
          id="cc-share-node-id"
          type="text"
          value={config.node_id}
          onChange={(e) => update("node_id", e.target.value)}
          placeholder="my-laptop-01"
        />
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-2">
          <Label htmlFor="cc-share-heartbeat">{t("config.heartbeatInterval")}</Label>
          <Input
            id="cc-share-heartbeat"
            type="number"
            min={5}
            max={300}
            value={config.heartbeat_interval_secs}
            onChange={(e) => update("heartbeat_interval_secs", Number(e.target.value) || 30)}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="cc-share-reconnect">{t("config.maxReconnect")}</Label>
          <Input
            id="cc-share-reconnect"
            type="number"
            min={1}
            max={600}
            value={config.max_reconnect_interval_secs}
            onChange={(e) => update("max_reconnect_interval_secs", Number(e.target.value) || 60)}
          />
        </div>
      </div>

      {error && (
        <p className="text-sm text-destructive">{error}</p>
      )}

      <div className="flex items-center justify-between">
        <span className="text-xs text-muted-foreground">
          {savedAt ? t("config.saved") : t("config.unsaved")}
        </span>
        <Button onClick={handleSave} disabled={saving}>
          {saving ? t("config.saving") : t("config.save")}
        </Button>
      </div>
    </div>
  );
}
