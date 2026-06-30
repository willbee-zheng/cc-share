import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Key, Plus, Copy, Trash2, Loader2, Check } from "lucide-react";
import {
  authCreateApiKey,
  authListApiKeys,
  authRevokeApiKey,
  type ApiKeyInfo,
  type CreateKeyResponse,
  type AuthError,
} from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogTitle, DialogDescription, DialogFooter } from "@/components/ui/dialog";

interface ApiKeyManagerProps {
  serverHost: string;
  onKeyCreated?: (response: CreateKeyResponse) => void;
}

function formatAuthError(err: AuthError, t: (key: string, opts?: Record<string, string>) => string): string {
  switch (err.kind) {
    case "network":
      return t("auth:errors.network");
    case "unauthorized":
      return t("auth:errors.unauthorized");
    case "token_expired":
      return t("auth:errors.tokenExpired");
    case "validation":
      return t("auth:errors.validation", { message: err.message });
    case "server":
      return t("auth:errors.server", { message: err.message });
    default:
      return t("auth:errors.unknown");
  }
}

export function ApiKeyManager({ serverHost, onKeyCreated }: ApiKeyManagerProps) {
  const { t } = useTranslation();
  const [keys, setKeys] = useState<ApiKeyInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // Create key dialog state
  const [showCreate, setShowCreate] = useState(false);
  const [newKeyName, setNewKeyName] = useState("");
  const [creating, setCreating] = useState(false);

  // Created key dialog state
  const [createdKey, setCreatedKey] = useState<CreateKeyResponse | null>(null);
  const [copied, setCopied] = useState(false);

  // Error state
  const [error, setError] = useState<string | null>(null);

  async function loadKeys() {
    setLoading(true);
    try {
      const list = await authListApiKeys(serverHost);
      setKeys(list);
      setLoaded(true);
    } catch {
      setError(t("auth:errors.network"));
    } finally {
      setLoading(false);
    }
  }

  async function handleCreate() {
    if (!newKeyName.trim()) return;
    setCreating(true);
    setError(null);
    try {
      const resp = await authCreateApiKey(serverHost, newKeyName.trim(), ["dispatch", "agent_connect"]);
      setNewKeyName("");
      setShowCreate(false);
      setCreatedKey(resp);
      setCopied(false);
      onKeyCreated?.(resp);
      // Refresh key list
      await loadKeys();
    } catch (err) {
      setError(formatAuthError(err as AuthError, t));
    } finally {
      setCreating(false);
    }
  }

  async function handleRevoke(keyId: string) {
    try {
      await authRevokeApiKey(serverHost, keyId);
      setKeys((prev) => prev.map((k) => k.id === keyId ? { ...k, status: "revoked" } : k));
    } catch {
      setError(t("auth:errors.network"));
    }
  }

  function handleCopyKey() {
    if (!createdKey) return;
    void navigator.clipboard.writeText(createdKey.key);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  // Lazy-load keys on first render
  if (!loaded && !loading) {
    void loadKeys();
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="flex items-center gap-2">
            <Key className="h-5 w-5" />
            {t("auth:apiKey.title")}
          </CardTitle>
          <Button size="sm" onClick={() => setShowCreate(true)}>
            <Plus className="mr-1 h-3.5 w-3.5" />
            {t("auth:apiKey.create")}
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {error && (
          <p className="mb-3 text-sm text-red-600 dark:text-red-400">{error}</p>
        )}

        {loading && !loaded && (
          <div className="flex items-center justify-center py-8">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        )}

        {loaded && keys.length === 0 && (
          <p className="py-6 text-center text-sm text-muted-foreground">
            {t("auth:apiKey.noKeys")}
          </p>
        )}

        {loaded && keys.length > 0 && (
          <div className="space-y-2">
            {keys.map((key) => (
              <div
                key={key.id}
                className="flex items-center justify-between rounded-md border border-black/10 p-3 dark:border-white/10"
              >
                <div className="space-y-0.5">
                  <p className="text-sm font-medium">{key.name}</p>
                  <p className="text-xs text-muted-foreground">
                    {t("auth:apiKey.prefix")}: {key.key_prefix}… ·{" "}
                    {key.last_used_at
                      ? `${t("auth:apiKey.lastUsed")}: ${new Date(key.last_used_at).toLocaleDateString()}`
                      : t("auth:apiKey.never")}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <Badge variant={key.status === "active" ? "success" : "secondary"}>
                    {key.status === "active"
                      ? t("auth:apiKey.active")
                      : t("auth:apiKey.revoked")}
                  </Badge>
                  {key.status === "active" && (
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-red-600 hover:text-red-700"
                      onClick={() => {
                        if (window.confirm(t("auth:apiKey.confirmRevoke"))) {
                          void handleRevoke(key.id);
                        }
                      }}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Create Key Dialog */}
        <Dialog open={showCreate} onOpenChange={setShowCreate}>
          <div className="space-y-4">
            <DialogTitle>{t("auth:apiKey.create")}</DialogTitle>
            <DialogDescription className="text-sm text-muted-foreground">
              {t("auth:apiKey.useForKey")}
            </DialogDescription>
            <div className="space-y-1.5">
              <Label htmlFor="key-name">{t("auth:apiKey.name")}</Label>
              <Input
                id="key-name"
                placeholder={t("auth:apiKey.namePlaceholder")}
                value={newKeyName}
                onChange={(e) => setNewKeyName(e.target.value)}
                disabled={creating}
                autoFocus
              />
            </div>
            <DialogFooter>
              <Button variant="ghost" onClick={() => setShowCreate(false)} disabled={creating}>
                Cancel
              </Button>
              <Button onClick={() => void handleCreate()} disabled={creating || !newKeyName.trim()}>
                {creating ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                {creating ? t("auth:apiKey.creating") : t("auth:apiKey.create")}
              </Button>
            </DialogFooter>
          </div>
        </Dialog>

        {/* Created Key Dialog */}
        <Dialog open={createdKey !== null} onOpenChange={(open) => { if (!open) setCreatedKey(null); }}>
          <div className="space-y-4">
            <DialogTitle>{t("auth:apiKey.created")}</DialogTitle>
            <DialogDescription className="text-sm text-amber-600 dark:text-amber-400">
              {t("auth:apiKey.createdWarning")}
            </DialogDescription>
            <div className="flex items-center gap-2 rounded-md border border-black/10 p-3 dark:border-white/10">
              <code className="flex-1 break-all text-xs">{createdKey?.key}</code>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => handleCopyKey()}
              >
                {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                {copied ? t("auth:apiKey.copied") : t("auth:apiKey.copy")}
              </Button>
            </div>
            <DialogFooter>
              <Button onClick={() => setCreatedKey(null)}>OK</Button>
            </DialogFooter>
          </div>
        </Dialog>
      </CardContent>
    </Card>
  );
}