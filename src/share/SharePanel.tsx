import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Activity, Play, Settings2, Share2, Square, Wifi, WifiOff, AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  subscribeConnectionState,
  subscribeConnectionError,
  subscribeTaskFinished,
  type ConnectionState,
  type ConnectionErrorEvent,
  type TaskFinishedEvent,
} from "../lib/events";
import {
  getShareableModels,
  getSupplierTokenByModel,
  refreshProviders,
  shareConnect,
  shareDisconnect,
  shareGetStatus,
  type ModelTokenStat,
  type ActiveTarget,
} from "../lib/api";
import { friendlyError } from "../lib/errors";
import { ShareConfigForm } from "./ShareConfigForm";
import { FirstRunGuide } from "./FirstRunGuide";
import { EarningsCalculator } from "./EarningsCalculator";

interface Stats {
  shared: number;
  completed: number;
}

const isBusyState = (s: ConnectionState) => s === "connecting" || s === "reconnecting";

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function SharePanel({ onSignInNeeded }: { onSignInNeeded?: () => void }) {
  const { t } = useTranslation("share");
  const [connectionState, setConnectionState] = useState<ConnectionState>("disconnected");
  const [connectionError, setConnectionError] = useState<ConnectionErrorEvent | null>(null);
  const [stats, setStats] = useState<Stats>({ shared: 0, completed: 0 });
  const [modelStats, setModelStats] = useState<ModelTokenStat[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [configOpen, setConfigOpen] = useState(false);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [upstreamModels, setUpstreamModels] = useState<Record<string, string>>({});
  const [providers, setProviders] = useState<ActiveTarget[]>([]);

  async function refreshModelStats() {
    try {
      const data = await getSupplierTokenByModel(7);
      setModelStats(data);
    } catch (e) {
      // 静默：表格下方会显示空态
      setModelStats([]);
    }
  }

  async function fetchModels() {
    try {
      const snap = await refreshProviders();
      const models = await getShareableModels();
      setAvailableModels(models);
      setUpstreamModels(snap.upstream_models ?? {});
      setProviders(snap.providers ?? []);
      console.log("[SharePanel] fetchModels result:", {
        reachable: snap.reachable,
        running: snap.running,
        providers: snap.providers.length,
        available_models: snap.available_models,
        upstream_models: snap.upstream_models,
        shareable_models: models,
      });
    } catch (e) {
      console.error("[SharePanel] fetchModels error:", e);
    }
  }

  useEffect(() => {
    let cancelled = false;
    void shareGetStatus().then((s) => {
      if (!cancelled) setConnectionState(s as ConnectionState);
    });
    void refreshModelStats();
    void fetchModels();

    const cleanups: Array<() => void> = [];

    void subscribeConnectionState((s) => {
      if (cancelled) return;
      setConnectionState(s);
      // Clear error when successfully connected
      if (s === "connected") {
        setConnectionError(null);
      }
    }).then((un) => cleanups.push(un));

    void subscribeConnectionError((evt) => {
      if (cancelled) return;
      setConnectionError(evt);
    }).then((un) => cleanups.push(un));

    void subscribeTaskFinished((evt: TaskFinishedEvent) => {
      if (cancelled) return;
      setStats((prev) => ({
        shared: prev.shared + 1,
        completed: prev.completed + (evt.status === "completed" ? 1 : 0),
      }));
      // 任务完成后刷新按模型统计
      void refreshModelStats();
    }).then((un) => cleanups.push(un));

    return () => {
      cancelled = true;
      cleanups.forEach((c) => c());
    };
  }, []);

  async function handleStart() {
    if (onSignInNeeded) {
      onSignInNeeded();
      return;
    }
    setBusy(true);
    setError(null);
    setConnectionError(null);
    try {
      // Refresh model availability before connecting
      const snap = await refreshProviders();
      const models = await getShareableModels();
      setAvailableModels(models);
      setUpstreamModels(snap.upstream_models ?? {});
      setProviders(snap.providers ?? []);
      if (models.length === 0) {
        // Provide a more specific error based on the snapshot state
        if (!snap.reachable) {
          setError(t("sharing.noModelsCcswitchUnreachable"));
        } else if (!snap.running) {
          setError(t("sharing.noModelsCcswitchNotRunning"));
        } else if (snap.providers.length === 0) {
          setError(t("sharing.noModelsNoProvider"));
        } else {
          setError(t("sharing.noModels"));
        }
        setBusy(false);
        return;
      }
      await shareConnect({ available_models: models });
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    setError(null);
    try {
      await shareDisconnect();
      setConnectionState("disconnected");
      setConnectionError(null);
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  const isConnected = connectionState === "connected";
  const successRate = stats.shared > 0 ? Math.round((stats.completed / stats.shared) * 100) : 0;
  const totalSuppliedTokens = modelStats.reduce((sum, s) => sum + s.total_tokens, 0);
  const topModels = [...modelStats]
    .sort((a, b) => b.total_tokens - a.total_tokens)
    .slice(0, 5);

  const statusIcon = connectionState === "connected"
    ? <Wifi className="w-4 h-4 text-emerald-500" />
    : connectionState === "disconnected"
      ? <WifiOff className="w-4 h-4 text-zinc-400" />
      : <Wifi className="w-4 h-4 text-amber-500 animate-pulse" />;

  return (
    <div className="space-y-6">
      <FirstRunGuide />
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">{t("title")}</h2>
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          {statusIcon}
          <span>{t(`status.${connectionState}`)}</span>
        </div>
      </div>

      {/* Connection error detail */}
      {connectionError && connectionState !== "connected" && (
        <div className="flex items-start gap-2 rounded-md bg-red-50 dark:bg-red-950/30 p-3 text-sm text-red-700 dark:text-red-300">
          <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
          <div>
            <p className="font-medium">{t(`errors.category.${connectionError.category}`)}</p>
            <p className="mt-1 text-xs opacity-80">{connectionError.message}</p>
          </div>
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Share2 className="w-4 h-4" />
            {t("sharing.title")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground mb-4">{t("sharing.description")}</p>

          {/* Available models display */}
          {availableModels.length > 0 && (
            <div className="mb-4">
              <p className="text-xs text-muted-foreground mb-1">{t("sharing.availableModels")}</p>
              {providers.length > 1 ? (
                // Group by provider when multiple providers exist
                <div className="space-y-2">
                  {providers.map((p) => {
                    const providerModels = p.models.filter((m) => availableModels.includes(m));
                    if (providerModels.length === 0) return null;
                    return (
                      <div key={p.provider_id}>
                        <p className="text-xs font-medium text-muted-foreground">{p.provider_name}</p>
                        <div className="flex flex-wrap gap-1 mt-0.5">
                          {providerModels.map((m) => {
                            const upstream = p.upstream_models[m] || upstreamModels[m];
                            return (
                              <span key={m} className="inline-flex items-center rounded-md bg-primary/10 px-2 py-0.5 text-xs font-mono text-primary">
                                {upstream ? `${m} → ${upstream}` : m}
                              </span>
                            );
                          })}
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                // Single provider or no provider info — flat list
                <div className="flex flex-wrap gap-1">
                  {availableModels.map((m) => {
                    const upstream = upstreamModels[m];
                    return (
                      <span key={m} className="inline-flex items-center rounded-md bg-primary/10 px-2 py-0.5 text-xs font-mono text-primary">
                        {upstream ? `${m} → ${upstream}` : m}
                      </span>
                    );
                  })}
                </div>
              )}
            </div>
          )}
          {availableModels.length === 0 && connectionState === "disconnected" && (
            <p className="text-xs text-amber-600 dark:text-amber-400 mb-4">{t("sharing.noModelsHint")}</p>
          )}

          <div className="flex flex-wrap gap-2">
            {isConnected ? (
              <Button variant="destructive" onClick={handleStop} disabled={busy}>
                <Square className="w-4 h-4 mr-2" />
                {t("sharing.stop")}
              </Button>
            ) : isBusyState(connectionState) ? (
              <Button variant="destructive" onClick={handleStop} disabled={busy}>
                <Square className="w-4 h-4 mr-2" />
                {t("sharing.cancel")}
              </Button>
            ) : (
              <Button onClick={handleStart} disabled={busy}>
                <Play className="w-4 h-4 mr-2" />
                {t("sharing.start")}
              </Button>
            )}

            <Collapsible open={configOpen} onOpenChange={setConfigOpen}>
              <CollapsibleTrigger asChild>
                <Button variant="outline">
                  <Settings2 className="w-4 h-4 mr-2" />
                  {t("config.title")}
                </Button>
              </CollapsibleTrigger>
              <CollapsibleContent className="mt-4 w-full">
                <ShareConfigForm />
              </CollapsibleContent>
            </Collapsible>
          </div>

          {error && (
            <p className="text-sm text-destructive mt-3">{error}</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Activity className="w-4 h-4" />
            {t("stats.title")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-2 gap-4">
            <div className="text-center p-3 rounded-lg bg-muted/50">
              <p className="text-2xl font-bold">{stats.shared}</p>
              <p className="text-xs text-muted-foreground">{t("stats.sharedRequests")}</p>
            </div>
            <div className="text-center p-3 rounded-lg bg-muted/50">
              <p className="text-2xl font-bold">{stats.completed}</p>
              <p className="text-xs text-muted-foreground">{t("stats.completed")}</p>
            </div>
            <div className="text-center p-3 rounded-lg bg-muted/50">
              <p className="text-2xl font-bold">{successRate}%</p>
              <p className="text-xs text-muted-foreground">{t("stats.successRate")}</p>
            </div>
            <div className="text-center p-3 rounded-lg bg-muted/50">
              <p className="text-2xl font-bold text-emerald-600">{formatTokens(totalSuppliedTokens)}</p>
              <p className="text-xs text-muted-foreground">{t("stats.suppliedTokens")}</p>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("stats.byModelTitle")}</CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          {topModels.length > 0 ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("stats.model")}</TableHead>
                  <TableHead className="text-right">{t("stats.prompt")}</TableHead>
                  <TableHead className="text-right">{t("stats.completion")}</TableHead>
                  <TableHead className="text-right">{t("stats.total")}</TableHead>
                  <TableHead className="text-right">{t("stats.taskCount")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {topModels.map((s) => (
                  <TableRow key={s.model}>
                    <TableCell className="font-mono text-xs">{s.model}</TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground">
                      {formatTokens(s.prompt_tokens)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground">
                      {formatTokens(s.completion_tokens)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs text-emerald-600">
                      {formatTokens(s.total_tokens)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs">{s.task_count}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : (
            <p className="text-sm text-muted-foreground p-4">{t("stats.byModelEmpty")}</p>
          )}
        </CardContent>
      </Card>

      <EarningsCalculator />
    </div>
  );
}