import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Check, Play, Square, AlertTriangle, Radio } from "lucide-react";
import {
  type Role,
  getRole,
  setRole,
  generateConsumerConfig,
  startLocalServer,
  stopLocalServer,
  getLocalServerAddr,
  getConsumerTokenByModel,
  p2pGetStatus,
  type ModelTokenStat,
  type P2PStatus,
} from "@/lib/api";
import {
  subscribeConnectionState,
  subscribeRoleChanged,
  subscribeTaskFinished,
  subscribeP2PSessionState,
  subscribeP2PConnectionStatus,
  type P2PSessionState,
  type P2PConnectionEvent,
} from "@/lib/events";
import { cn } from "@/components/ui/cn";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const APP_TYPES = ["claude", "codex", "gemini"] as const;
type AppType = (typeof APP_TYPES)[number];

const DEFAULT_MODELS: Record<AppType, string> = {
  claude: "claude-sonnet-4",
  codex: "claude-sonnet-4",
  gemini: "gemini-2.5-pro",
};

const MODEL_OPTIONS: Record<AppType, { value: string; labelKey: string }[]> = {
  claude: [
    { value: "claude-sonnet-4", labelKey: "consume.models.claude-sonnet-4" },
    { value: "claude-opus-4", labelKey: "consume.models.claude-opus-4" },
    { value: "claude-haiku-4", labelKey: "consume.models.claude-haiku-4" },
  ],
  codex: [
    { value: "claude-sonnet-4", labelKey: "consume.models.claude-sonnet-4" },
    { value: "gpt-4o", labelKey: "consume.models.gpt-4o" },
    { value: "gpt-4o-mini", labelKey: "consume.models.gpt-4o-mini" },
  ],
  gemini: [
    { value: "gemini-2.5-pro", labelKey: "consume.models.gemini-2.5-pro" },
    { value: "gemini-2.5-flash", labelKey: "consume.models.gemini-2.5-flash" },
  ],
};

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function ConsumePanel() {
  const { t } = useTranslation("share");
  const [role, setRoleState] = useState<Role>("idle");
  const [proxyAddr, setProxyAddr] = useState("");
  const [isConnecting, setIsConnecting] = useState(false);
  const [appType, setAppType] = useState<AppType>("claude");
  const [model, setModel] = useState(DEFAULT_MODELS.claude);
  const [configJson, setConfigJson] = useState("");
  const [copied, setCopied] = useState(false);
  const [isSupplierActive, setIsSupplierActive] = useState(false);
  const [confirmSwitch, setConfirmSwitch] = useState(false);
  const [modelStats, setModelStats] = useState<ModelTokenStat[]>([]);
  // P2P state for consumer
  const [p2pStatus, setP2PStatus] = useState<P2PStatus | null>(null);
  const [p2pSessionState, setP2PSessionState] = useState<P2PSessionState | null>(null);
  const [p2pActiveConns, setP2PActiveConns] = useState(0);

  async function refreshModelStats() {
    try {
      const data = await getConsumerTokenByModel(7);
      setModelStats(data);
    } catch {
      setModelStats([]);
    }
  }

  useEffect(() => {
    let cancelled = false;
    const cleanups: Array<() => void> = [];

    void getRole().then((r) => {
      if (!cancelled) {
        const parsed: Role = JSON.parse(r);
        setRoleState(parsed);
      }
    });
    void getLocalServerAddr().then((addr) => {
      if (!cancelled) setProxyAddr(addr);
    });
    void refreshModelStats();
    // Fetch initial P2P status
    p2pGetStatus().then((s) => { if (!cancelled) setP2PStatus(s); }).catch(() => {});

    void subscribeConnectionState((state) => {
      if (cancelled) return;
      setIsSupplierActive(
        state === "connected" || state === "connecting" || state === "reconnecting",
      );
    }).then((un) => cleanups.push(un));

    void subscribeRoleChanged((newRole) => {
      if (cancelled) return;
      setRoleState(newRole);
      if (newRole === "consumer") {
        void getLocalServerAddr().then((addr) => {
          if (!cancelled) setProxyAddr(addr);
        });
      }
    }).then((un) => cleanups.push(un));

    void subscribeTaskFinished(() => {
      if (cancelled) return;
      void refreshModelStats();
    }).then((un) => cleanups.push(un));

    void subscribeP2PSessionState((evt) => {
      if (cancelled) return;
      setP2PSessionState(evt.state);
      p2pGetStatus().then((s) => { if (!cancelled) setP2PStatus(s); }).catch(() => {});
    }).then((un) => cleanups.push(un));

    void subscribeP2PConnectionStatus((evt: P2PConnectionEvent) => {
      if (cancelled) return;
      setP2PActiveConns(evt.active_connections);
      p2pGetStatus().then((s) => { if (!cancelled) setP2PStatus(s); }).catch(() => {});
    }).then((un) => cleanups.push(un));

    return () => {
      cancelled = true;
      cleanups.forEach((c) => c());
    };
  }, []);

  const handleStartProxy = async () => {
    if (isSupplierActive) {
      setConfirmSwitch(true);
      return;
    }
    setIsConnecting(true);
    try {
      await setRole("consumer");
      const addr = await startLocalServer();
      setProxyAddr(addr);
      setRoleState("consumer");
    } catch (e) {
      console.error("Failed to start proxy:", e);
    } finally {
      setIsConnecting(false);
      setConfirmSwitch(false);
    }
  };

  const handleStopProxy = async () => {
    try {
      await stopLocalServer();
      setProxyAddr("");
      if (role === "consumer") {
        await setRole("idle");
        setRoleState("idle");
      }
    } catch (e) {
      console.error("Failed to stop proxy:", e);
    }
  };

  const handleConfirmSwitch = async () => {
    setIsConnecting(true);
    try {
      await setRole("consumer");
      const addr = await startLocalServer();
      setProxyAddr(addr);
      setRoleState("consumer");
      setIsSupplierActive(false);
    } catch (e) {
      console.error("Failed to switch to consumer:", e);
    } finally {
      setIsConnecting(false);
      setConfirmSwitch(false);
    }
  };

  const handleGenerateConfig = async () => {
    try {
      const config = await generateConsumerConfig(appType, model);
      setConfigJson(JSON.stringify(config, null, 2));
    } catch (e) {
      console.error("Failed to generate config:", e);
    }
  };

  const handleCopy = async () => {
    await navigator.clipboard.writeText(configJson);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleAppTypeChange = (newType: AppType) => {
    setAppType(newType);
    setModel(DEFAULT_MODELS[newType]);
    setConfigJson("");
  };

  const isProxyRunning = proxyAddr.length > 0;
  const totalConsumedTokens = modelStats.reduce((sum, s) => sum + s.total_tokens, 0);
  const topModels = [...modelStats]
    .sort((a, b) => b.total_tokens - a.total_tokens)
    .slice(0, 5);

  return (
    <div className="flex flex-col gap-4 max-w-xl">
      {/* Role warning */}
      {isSupplierActive && !confirmSwitch && (
        <div className="flex items-center gap-2 rounded-md bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 p-3 text-sm text-amber-800 dark:text-amber-200">
          <AlertTriangle className="h-4 w-4 shrink-0" />
          <span>{t("consume.roleWarning")}</span>
        </div>
      )}

      {confirmSwitch && (
        <div className="flex flex-col gap-2 rounded-md bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 p-3">
          <p className="text-sm text-amber-800 dark:text-amber-200">{t("consume.roleWarningShort")}</p>
          <div className="flex gap-2">
            <button
              onClick={handleConfirmSwitch}
              className="rounded-md bg-amber-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-amber-700"
            >
              {t("consume.startProxy")}
            </button>
            <button
              onClick={() => setConfirmSwitch(false)}
              className="rounded-md bg-zinc-200 dark:bg-zinc-700 px-3 py-1.5 text-sm font-medium hover:bg-zinc-300 dark:hover:bg-zinc-600"
            >
              {t("guide.back")}
            </button>
          </div>
        </div>
      )}

      {/* Proxy control */}
      <div className="rounded-lg border border-black/10 dark:border-white/10 p-4">
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-semibold">{t("consume.title")}</h3>
          <div className="flex items-center gap-2 text-xs">
            <div
              className={cn(
                "w-2 h-2 rounded-full",
                isProxyRunning
                  ? "bg-emerald-500 animate-pulse"
                  : "bg-zinc-300 dark:bg-zinc-600"
              )}
            />
            <span className="text-muted-foreground">
              {isProxyRunning
                ? `${t("consume.proxyRunning")} ${proxyAddr}`
                : t("consume.proxyStopped")}
            </span>
          </div>
        </div>

        {/* P2P status indicator */}
        {p2pStatus && p2pStatus.running && isProxyRunning && (
          <div className="flex items-center gap-2 mb-3 px-2 py-1.5 rounded bg-emerald-50 dark:bg-emerald-950/30 text-xs">
            <Radio className="w-3.5 h-3.5 text-emerald-500" />
            <span className="text-emerald-700 dark:text-emerald-300 font-medium">
              {t("p2p.directConnected", "P2P Direct")}
            </span>
            {p2pSessionState && (
              <span className={cn(
                "text-xs",
                p2pSessionState === "connected" ? "text-emerald-600 dark:text-emerald-400" :
                p2pSessionState === "executing" ? "text-blue-600 dark:text-blue-400" :
                p2pSessionState === "failed" ? "text-red-600 dark:text-red-400" :
                "text-amber-600 dark:text-amber-400 animate-pulse"
              )}>
                {t(`p2p.${p2pSessionState}`, p2pSessionState)}
              </span>
            )}
            {p2pActiveConns > 0 && (
              <span className="text-muted-foreground">
                {t("p2p.activePeers", "{{count}} peer(s)", { count: p2pActiveConns })}
              </span>
            )}
          </div>
        )}

        <div className="flex gap-2">
          {!isProxyRunning ? (
            <button
              onClick={handleStartProxy}
              disabled={isConnecting}
              className="inline-flex items-center gap-1.5 rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
            >
              <Play className="h-3.5 w-3.5" />
              {isConnecting ? t("status.connecting") : t("consume.startProxy")}
            </button>
          ) : (
            <button
              onClick={handleStopProxy}
              className="inline-flex items-center gap-1.5 rounded-md bg-red-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-700"
            >
              <Square className="h-3.5 w-3.5" />
              {t("consume.stopProxy")}
            </button>
          )}
        </div>
      </div>

      {/* Consumed tokens summary */}
      <div className="rounded-lg border border-black/10 dark:border-white/10 p-4">
        <h3 className="text-sm font-semibold mb-3">{t("consume.statsTitle")}</h3>
        <div className="grid grid-cols-2 gap-3 mb-4">
          <div className="text-center p-3 rounded-md bg-muted/50">
            <p className="text-2xl font-bold text-rose-600">{formatTokens(totalConsumedTokens)}</p>
            <p className="text-xs text-muted-foreground">{t("consume.consumedTokens")}</p>
          </div>
          <div className="text-center p-3 rounded-md bg-muted/50">
            <p className="text-2xl font-bold">{modelStats.length}</p>
            <p className="text-xs text-muted-foreground">{t("consume.modelCount")}</p>
          </div>
        </div>
        {topModels.length > 0 ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("consume.model")}</TableHead>
                <TableHead className="text-right">{t("consume.prompt")}</TableHead>
                <TableHead className="text-right">{t("consume.completion")}</TableHead>
                <TableHead className="text-right">{t("consume.total")}</TableHead>
                <TableHead className="text-right">{t("consume.taskCount")}</TableHead>
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
                  <TableCell className="text-right font-mono text-xs text-rose-600">
                    {formatTokens(s.total_tokens)}
                  </TableCell>
                  <TableCell className="text-right font-mono text-xs">{s.task_count}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <p className="text-sm text-muted-foreground">{t("consume.byModelEmpty")}</p>
        )}
      </div>

      {/* Config generator */}
      <div className="rounded-lg border border-black/10 dark:border-white/10 p-4">
        <h3 className="text-sm font-semibold mb-3">{t("consume.configPreview")}</h3>

        <div className="flex gap-3 mb-3">
          <div className="flex-1">
            <label className="block text-xs text-muted-foreground mb-1">
              {t("consume.selectAppType")}
            </label>
            <select
              value={appType}
              onChange={(e) => handleAppTypeChange(e.target.value as AppType)}
              className="w-full rounded-md border border-black/10 dark:border-white/10 bg-white dark:bg-zinc-800 px-2 py-1.5 text-sm"
            >
              {APP_TYPES.map((type) => (
                <option key={type} value={type}>
                  {t(`consume.appTypes.${type}`)}
                </option>
              ))}
            </select>
          </div>

          <div className="flex-1">
            <label className="block text-xs text-muted-foreground mb-1">
              {t("consume.selectModel")}
            </label>
            <select
              value={model}
              onChange={(e) => {
                setModel(e.target.value);
                setConfigJson("");
              }}
              className="w-full rounded-md border border-black/10 dark:border-white/10 bg-white dark:bg-zinc-800 px-2 py-1.5 text-sm"
            >
              {MODEL_OPTIONS[appType].map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {t(opt.labelKey)}
                </option>
              ))}
            </select>
          </div>
        </div>

        <button
          onClick={handleGenerateConfig}
          className="w-full rounded-md bg-zinc-900 dark:bg-zinc-100 px-3 py-2 text-sm font-medium text-white dark:text-zinc-900 hover:bg-zinc-800 dark:hover:bg-zinc-200"
        >
          {t("consume.generateConfig")}
        </button>

        {configJson && (
          <div className="mt-3 relative">
            <pre className="rounded-md bg-zinc-100 dark:bg-zinc-800 p-3 text-xs overflow-x-auto max-h-80 border border-black/5 dark:border-white/5">
              {configJson}
            </pre>
            <button
              onClick={handleCopy}
              className="absolute top-2 right-2 inline-flex items-center gap-1 rounded-md bg-white dark:bg-zinc-700 border border-black/10 dark:border-white/10 px-2 py-1 text-xs hover:bg-zinc-50 dark:hover:bg-zinc-600"
            >
              {copied ? (
                <Check className="h-3 w-3 text-emerald-500" />
              ) : (
                <Copy className="h-3 w-3" />
              )}
              {copied ? t("consume.copied") : t("consume.copyConfig")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}