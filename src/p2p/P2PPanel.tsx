import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Radio, Copy, Check, Wifi, WifiOff, Users, ScrollText } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  p2pGetStatus,
  p2pGetPublicKey,
  type P2PStatus,
} from "@/lib/api";
import {
  subscribeP2PSessionState,
  subscribeP2PConnectionStatus,
  type P2PSessionEvent,
  type P2PConnectionEvent,
  type P2PSessionState,
} from "@/lib/events";

const SESSION_COLORS: Record<P2PSessionState, string> = {
  awaiting_answer: "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300",
  connecting: "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300 animate-pulse",
  connected: "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300",
  executing: "bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300",
  completed: "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300",
  failed: "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300",
};

interface SessionEntry {
  id: string;
  state: P2PSessionState;
  peer?: string;
  model?: string;
  error?: string;
  timestamp: number;
}

export function P2PPanel() {
  const { t } = useTranslation("share");
  const [status, setStatus] = useState<P2PStatus | null>(null);
  const [publicKey, setPublicKey] = useState("");
  const [showFullKey, setShowFullKey] = useState(false);
  const [copied, setCopied] = useState(false);
  const [sessions, setSessions] = useState<SessionEntry[]>([]);
  const [activeConns, setActiveConns] = useState(0);

  const refreshStatus = useCallback(async () => {
    try {
      const s = await p2pGetStatus();
      setStatus(s);
      setActiveConns(s.active_connections);
    } catch {
      // silent
    }
  }, []);

  // Initial load
  useEffect(() => {
    void refreshStatus();
    p2pGetPublicKey().then(setPublicKey).catch(() => {});
  }, [refreshStatus]);

  // Real-time subscriptions
  useEffect(() => {
    const cleanups: Array<() => void> = [];
    let cancelled = false;

    void subscribeP2PSessionState((evt: P2PSessionEvent) => {
      if (cancelled) return;
      setSessions((prev) => [
        {
          id: evt.session_id,
          state: evt.state,
          peer: evt.peer_address,
          model: evt.model,
          error: evt.error,
          timestamp: Date.now(),
        },
        ...prev,
      ].slice(0, 50));
      void refreshStatus();
    }).then((un) => { if (!cancelled) cleanups.push(un); });

    void subscribeP2PConnectionStatus((evt: P2PConnectionEvent) => {
      if (cancelled) return;
      setActiveConns(evt.active_connections);
      void refreshStatus();
    }).then((un) => { if (!cancelled) cleanups.push(un); });

    return () => {
      cancelled = true;
      cleanups.forEach((u) => u());
    };
  }, [refreshStatus]);

  async function handleCopyKey() {
    try {
      await navigator.clipboard.writeText(publicKey);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // clipboard not available
    }
  }

  const running = status?.running ?? false;

  // Derive a human-readable connection state
  const connectionState: "idle" | "connected" | "connecting" | "failed" = (() => {
    if (!running) return "idle";
    if (sessions.length === 0) return "idle";
    const latest = sessions[0];
    if (latest.state === "failed") return "failed";
    if (latest.state === "connected" || latest.state === "executing") return "connected";
    if (latest.state === "connecting" || latest.state === "awaiting_answer") return "connecting";
    return "idle";
  })();

  return (
    <div className="space-y-4">
      <h2 className="text-base font-semibold">{t("p2p.panelTitle")}</h2>

      {/* Connection Status */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="flex items-center gap-2 text-sm">
            {connectionState === "connected" ? (
              <Wifi className="h-4 w-4 text-emerald-500" />
            ) : connectionState === "connecting" ? (
              <Radio className="h-4 w-4 text-amber-500 animate-pulse" />
            ) : connectionState === "failed" ? (
              <WifiOff className="h-4 w-4 text-red-500" />
            ) : (
              <WifiOff className="h-4 w-4 text-muted-foreground" />
            )}
            {t("p2p.connectionStatus")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          {/* Status indicator */}
          <div className="flex items-center gap-2">
            <Badge
              variant={
                connectionState === "connected" ? "success" :
                connectionState === "failed" ? "destructive" :
                connectionState === "connecting" ? "warning" : "secondary"
              }
            >
              {t(`p2p.state_${connectionState}`)}
            </Badge>
            {activeConns > 0 && (
              <span className="text-muted-foreground">
                {t("p2p.activePeers", { count: activeConns })}
              </span>
            )}
          </div>

          {connectionState === "idle" && running && (
            <p className="text-xs text-muted-foreground">{t("p2p.waitingForPeers")}</p>
          )}
          {connectionState === "connecting" && (
            <p className="text-xs text-amber-600 dark:text-amber-400">{t("p2p.connectingHint")}</p>
          )}
          {connectionState === "failed" && (
            <p className="text-xs text-red-600 dark:text-red-400">{t("p2p.autoReconnect")}</p>
          )}

          {/* Public Key */}
          {publicKey && (
            <div className="space-y-1">
              <span className="text-muted-foreground">{t("p2p.publicKey")}:</span>
              <div className="flex items-center gap-1">
                <code className="flex-1 rounded bg-black/5 px-2 py-1 text-xs dark:bg-white/10 break-all">
                  {showFullKey ? publicKey : `${publicKey.slice(0, 12)}…${publicKey.slice(-8)}`}
                </code>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 w-7 p-0"
                  onClick={() => setShowFullKey((v) => !v)}
                  title={showFullKey ? t("p2p.collapse") : t("p2p.expand")}
                >
                  {showFullKey ? "«" : "»"}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 w-7 p-0"
                  onClick={() => void handleCopyKey()}
                  title={t("p2p.copyKey")}
                >
                  {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                </Button>
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Session Log */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="flex items-center gap-2 text-sm">
            <ScrollText className="h-4 w-4" />
            {t("p2p.sessionLog")}
          </CardTitle>
        </CardHeader>
        <CardContent className="text-sm">
          {sessions.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t("p2p.noSessions")}</p>
          ) : (
            <div className="max-h-[50vh] overflow-auto">
              <table className="w-full text-xs">
                <thead className="sticky top-0 bg-background">
                  <tr className="border-b border-black/5 dark:border-white/5">
                    <th className="px-2 py-1.5 text-left font-medium">{t("p2p.time")}</th>
                    <th className="px-2 py-1.5 text-left font-medium">{t("p2p.sessionId")}</th>
                    <th className="px-2 py-1.5 text-left font-medium">{t("p2p.state")}</th>
                    <th className="px-2 py-1.5 text-left font-medium">{t("p2p.peer")}</th>
                    <th className="px-2 py-1.5 text-left font-medium">{t("p2p.model")}</th>
                    <th className="px-2 py-1.5 text-left font-medium">{t("p2p.error")}</th>
                  </tr>
                </thead>
                <tbody>
                  {sessions.map((s) => (
                    <tr key={`${s.id}-${s.timestamp}`} className="border-b border-black/5 dark:border-white/5 hover:bg-muted/30">
                      <td className="whitespace-nowrap px-2 py-1 font-mono text-muted-foreground">
                        {formatTime(s.timestamp)}
                      </td>
                      <td className="max-w-24 truncate px-2 py-1 font-mono" title={s.id}>
                        {s.id.slice(0, 8)}…
                      </td>
                      <td className="px-2 py-1">
                        <Badge
                          className={SESSION_COLORS[s.state]}
                          variant={s.state === "failed" ? "destructive" : s.state === "completed" || s.state === "connected" ? "success" : "warning"}
                        >
                          {t(`p2p.${s.state}`, s.state)}
                        </Badge>
                      </td>
                      <td className="max-w-32 truncate px-2 py-1 font-mono text-muted-foreground" title={s.peer}>
                        {s.peer ?? "—"}
                      </td>
                      <td className="max-w-28 truncate px-2 py-1 text-muted-foreground" title={s.model}>
                        {s.model ?? "—"}
                      </td>
                      <td className="max-w-40 truncate px-2 py-1 text-red-600 dark:text-red-400" title={s.error}>
                        {s.error ?? "—"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function formatTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}