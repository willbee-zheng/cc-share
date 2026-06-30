import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2, RefreshCw, AlertCircle, Info, AlertTriangle, Bug } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  type SystemLogEntry,
  type LogFilter,
  type LogStats,
  getSystemLogs,
  clearSystemLogs,
  getSystemLogStats,
  listSystemLogTargets,
  setLogLevel,
} from "@/lib/api";
import { subscribeLogAppended } from "@/lib/events";
import { cn } from "@/components/ui/cn";

const LEVEL_OPTIONS = ["debug", "info", "warn", "error"] as const;
const LEVEL_ICONS: Record<string, typeof Info> = {
  debug: Bug,
  info: Info,
  warn: AlertTriangle,
  error: AlertCircle,
};
const LEVEL_COLORS: Record<string, string> = {
  debug: "text-zinc-400 bg-zinc-100 dark:bg-zinc-800",
  info: "text-blue-600 bg-blue-50 dark:bg-blue-900/30",
  warn: "text-amber-600 bg-amber-50 dark:bg-amber-900/30",
  error: "text-red-600 bg-red-50 dark:bg-red-900/30",
};

function formatTimestamp(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function SystemLogPanel() {
  const { t } = useTranslation("share");
  const [logs, setLogs] = useState<SystemLogEntry[]>([]);
  const [stats, setStats] = useState<LogStats | null>(null);
  const [targets, setTargets] = useState<string[]>([]);
  const [level, setLevel] = useState<string>("info");
  const [target, setTarget] = useState<string>("");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const searchTimer = useRef<ReturnType<typeof setTimeout>>();

  async function refresh() {
    setLoading(true);
    try {
      const filter: LogFilter = {
        level: level || undefined,
        target: target || undefined,
        search: search || undefined,
        limit: 500,
        offset: 0,
      };
      const [entries, s, tg] = await Promise.all([
        getSystemLogs(filter),
        getSystemLogStats(),
        listSystemLogTargets(),
      ]);
      setLogs(entries);
      setStats(s);
      setTargets(tg);
    } catch {
      // silent
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
    return () => {
      if (searchTimer.current) clearTimeout(searchTimer.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Debounced search
  const debouncedRefresh = useCallback(() => {
    if (searchTimer.current) clearTimeout(searchTimer.current);
    searchTimer.current = setTimeout(() => void refresh(), 300);
  }, [level, target, search]);

  useEffect(() => {
    debouncedRefresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [level, target, search]);

  // Real-time log updates
  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    void subscribeLogAppended(() => {
      if (cancelled) return;
      void refresh();
    }).then((un) => unsubs.push(un));
    return () => {
      cancelled = true;
      unsubs.forEach((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function handleClear() {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    try {
      await clearSystemLogs();
      setConfirmClear(false);
      void refresh();
    } catch {
      // silent
    }
  }

  async function handleToggleDebug() {
    const newLevel = level === "debug" ? "info" : "debug";
    try {
      await setLogLevel(newLevel);
      setLevel(newLevel);
      void refresh();
    } catch {
      // silent
    }
  }

  const Icon = Info;

  return (
    <div className="space-y-4">
      <h2 className="text-base font-semibold">{t("logs.title")}</h2>

      {/* Stats */}
      {stats && (
        <div className="grid grid-cols-5 gap-2 text-center text-xs">
          {LEVEL_OPTIONS.map((l) => {
            const LevelIcon = LEVEL_ICONS[l] || Info;
            return (
              <div key={l} className={cn("rounded-md p-2", LEVEL_COLORS[l])}>
                <LevelIcon className="mx-auto mb-1 h-4 w-4" />
                <p className="font-bold">{stats[l as keyof LogStats]}</p>
                <p className="text-[10px] uppercase">{l}</p>
              </div>
            );
          })}
        </div>
      )}

      {/* Filters */}
      <div className="flex flex-wrap items-center gap-2">
        <select
          value={level}
          onChange={(e) => setLevel(e.target.value)}
          className="rounded-md border border-black/10 bg-white px-2 py-1 text-sm dark:border-white/10 dark:bg-zinc-800"
        >
          {LEVEL_OPTIONS.map((l) => (
            <option key={l} value={l}>
              {t(`logs.level.${l}`)}
            </option>
          ))}
        </select>

        {targets.length > 0 && (
          <select
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            className="max-w-48 rounded-md border border-black/10 bg-white px-2 py-1 text-sm dark:border-white/10 dark:bg-zinc-800"
          >
            <option value="">{t("logs.allModules")}</option>
            {targets.map((tg) => (
              <option key={tg} value={tg}>
                {tg}
              </option>
            ))}
          </select>
        )}

        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("logs.searchPlaceholder")}
          className="max-w-48 flex-1 rounded-md border border-black/10 bg-white px-2 py-1 text-sm dark:border-white/10 dark:bg-zinc-800"
        />

        <Button
          variant="outline"
          size="sm"
          onClick={() => void refresh()}
          disabled={loading}
        >
          <RefreshCw className={cn("mr-1 h-3.5 w-3.5", loading && "animate-spin")} />
          {t("logs.refresh")}
        </Button>

        <Button
          variant="outline"
          size="sm"
          onClick={() => void handleToggleDebug()}
        >
          {level === "debug" ? t("logs.hideDebug") : t("logs.showDebug")}
        </Button>

        <Button
          variant={confirmClear ? "destructive" : "outline"}
          size="sm"
          onClick={() => void handleClear()}
          onBlur={() => setConfirmClear(false)}
        >
          <Trash2 className="mr-1 h-3.5 w-3.5" />
          {confirmClear ? t("logs.clearConfirm") : t("logs.clear")}
        </Button>
      </div>

      {/* Log list */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">{t("logs.title")}</CardTitle>
        </CardHeader>
        <CardContent className="max-h-[60vh] overflow-auto p-0">
          {logs.length === 0 ? (
            <p className="p-4 text-sm text-muted-foreground">{t("logs.empty")}</p>
          ) : (
            <table className="w-full text-xs">
              <thead className="sticky top-0 bg-background">
                <tr className="border-b border-black/5 dark:border-white/5">
                  <th className="px-2 py-1.5 text-left font-medium">{t("logs.time")}</th>
                  <th className="px-2 py-1.5 text-left font-medium">{t("logs.levelLabel")}</th>
                  <th className="px-2 py-1.5 text-left font-medium">{t("logs.module")}</th>
                  <th className="px-2 py-1.5 text-left font-medium">{t("logs.message")}</th>
                </tr>
              </thead>
              <tbody>
                {logs.map((entry) => {
                  const LevelIcon = LEVEL_ICONS[entry.level] || Info;
                  return (
                    <tr
                      key={entry.id}
                      className="border-b border-black/5 dark:border-white/5 hover:bg-muted/30"
                    >
                      <td className="whitespace-nowrap px-2 py-1 font-mono text-muted-foreground">
                        {formatTimestamp(entry.timestamp_ms)}
                      </td>
                      <td className="px-2 py-1">
                        <span
                          className={cn(
                            "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase",
                            LEVEL_COLORS[entry.level] || LEVEL_COLORS.info,
                          )}
                        >
                          <LevelIcon className="h-3 w-3" />
                          {entry.level}
                        </span>
                      </td>
                      <td className="max-w-32 truncate px-2 py-1 font-mono text-muted-foreground">
                        {entry.target}
                      </td>
                      <td className="px-2 py-1 font-mono">{entry.message}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}