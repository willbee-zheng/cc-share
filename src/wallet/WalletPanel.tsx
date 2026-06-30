import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowDownCircle,
  ArrowUpCircle,
  RefreshCw,
  TrendingUp,
  Wallet,
} from "lucide-react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { getWalletSummary, syncWallet as syncWalletApi, getAuthState, type WalletSummary, type P2PTaskLog, type AuthState } from "../lib/api";
import { subscribeTaskFinished } from "../lib/events";

const DEFAULT_USER_ID = "local";

const STATUS_VARIANT: Record<P2PTaskLog["status"], "default" | "secondary" | "destructive" | "outline"> = {
  pending: "secondary",
  running: "secondary",
  completed: "default",
  failed: "destructive",
  rejected: "destructive",
  busy: "outline",
};

function formatHourLabel(unix: number): string {
  const d = new Date(unix * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:00`;
}

function formatTime(unix: number): string {
  return new Date(unix * 1000).toLocaleString();
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function formatCredits(n: number): string {
  return n.toFixed(n < 1 ? 4 : 2);
}

export function WalletPanel() {
  const { t } = useTranslation("wallet");
  const [summary, setSummary] = useState<WalletSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [userId, setUserId] = useState<string>(DEFAULT_USER_ID);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      // Sync wallet balance from cloud first, then fetch local summary.
      try {
        await syncWalletApi();
      } catch (syncErr) {
        // Cloud sync failure is non-fatal — show a warning but still load local data.
        console.warn("Wallet cloud sync failed:", syncErr);
      }
      const s = await getWalletSummary(userId);
      setSummary(s);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    // Resolve the actual user ID from auth state.
    getAuthState()
      .then((auth: AuthState | null) => {
        if (auth) setUserId(auth.user_id);
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    void refresh();
    let cleanup: (() => void) | null = null;
    void subscribeTaskFinished(() => {
      void refresh();
    }).then((un) => {
      cleanup = un;
    });
    return () => {
      if (cleanup) cleanup();
    };
  }, [userId]);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">{t("title")}</h2>
        <Button variant="ghost" size="sm" onClick={refresh} disabled={loading}>
          <RefreshCw className={`w-4 h-4 mr-2 ${loading ? "animate-spin" : ""}`} />
          {t("refresh")}
        </Button>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      <div className="grid grid-cols-3 gap-4">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="flex items-center gap-2 text-sm font-normal text-muted-foreground">
              <Wallet className="w-4 h-4" />
              {t("balance.available")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-2xl font-bold">
              {summary ? formatCredits(summary.wallet.balance_credits) : "—"}
            </p>
            <p className="text-xs text-muted-foreground mt-1">
              {t("balance.totalEarned")}: {summary ? formatCredits(summary.wallet.total_earned) : "—"}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="flex items-center gap-2 text-sm font-normal text-muted-foreground">
              <ArrowUpCircle className="w-4 h-4 text-emerald-500" />
              {t("balance.todaySupplied")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-2xl font-bold text-emerald-600">
              {summary ? `+${formatTokens(summary.today_supplied_tokens)}` : "—"}
            </p>
            <p className="text-xs text-muted-foreground mt-1">
              {t("balance.totalSupplied")}:{" "}
              {summary ? formatTokens(summary.total_supplied_tokens) : "—"}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="flex items-center gap-2 text-sm font-normal text-muted-foreground">
              <ArrowDownCircle className="w-4 h-4 text-rose-500" />
              {t("balance.todayConsumed")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-2xl font-bold text-rose-600">
              {summary ? `-${formatTokens(summary.today_consumed_tokens)}` : "—"}
            </p>
            <p className="text-xs text-muted-foreground mt-1">
              {t("balance.totalConsumed")}:{" "}
              {summary ? formatTokens(summary.total_consumed_tokens) : "—"}
            </p>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <TrendingUp className="w-4 h-4" />
            {t("chart.title")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {summary && summary.hourly_trend.length > 0 ? (
            <div style={{ width: "100%", height: 220 }}>
              <ResponsiveContainer>
                <AreaChart data={summary.hourly_trend.map((p) => ({
                  hour: formatHourLabel(p.bucket_unix),
                  supplied: p.supplied_tokens,
                  consumed: p.consumed_tokens,
                }))}>
                  <defs>
                    <linearGradient id="cc-share-supplied" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="0%" stopColor="#10b981" stopOpacity={0.4} />
                      <stop offset="100%" stopColor="#10b981" stopOpacity={0} />
                    </linearGradient>
                    <linearGradient id="cc-share-consumed" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="0%" stopColor="#f43f5e" stopOpacity={0.4} />
                      <stop offset="100%" stopColor="#f43f5e" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="hour" tick={{ fontSize: 10 }} />
                  <YAxis tick={{ fontSize: 10 }} width={48} tickFormatter={formatTokens} />
                  <Tooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: 6,
                      fontSize: 12,
                    }}
                    formatter={(value: number) => formatTokens(value)}
                  />
                  <Area
                    type="monotone"
                    dataKey="supplied"
                    stroke="#10b981"
                    fill="url(#cc-share-supplied)"
                    strokeWidth={2}
                  />
                  <Area
                    type="monotone"
                    dataKey="consumed"
                    stroke="#f43f5e"
                    fill="url(#cc-share-consumed)"
                    strokeWidth={2}
                  />
                </AreaChart>
              </ResponsiveContainer>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{t("chart.empty")}</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("transactions.title")}</CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          {summary && summary.recent_logs.length > 0 ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("transactions.time")}</TableHead>
                  <TableHead>{t("transactions.direction")}</TableHead>
                  <TableHead>{t("transactions.model")}</TableHead>
                  <TableHead className="text-right">{t("transactions.prompt")}</TableHead>
                  <TableHead className="text-right">{t("transactions.completion")}</TableHead>
                  <TableHead className="text-right">{t("transactions.total")}</TableHead>
                  <TableHead>{t("transactions.status")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {summary.recent_logs.map((log) => (
                  <TableRow key={log.task_id}>
                    <TableCell className="text-xs">{formatTime(log.created_at)}</TableCell>
                    <TableCell>
                      <Badge variant={log.direction === "supply" ? "default" : "secondary"}>
                        {t(`transactions.dir.${log.direction}`)}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-mono text-xs">{log.model}</TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground">
                      {log.tokens_prompt}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground">
                      {log.tokens_completion}
                    </TableCell>
                    <TableCell
                      className={`text-right font-mono text-xs ${
                        log.direction === "supply" ? "text-emerald-600" : "text-rose-600"
                      }`}
                    >
                      {log.tokens_prompt + log.tokens_completion}
                    </TableCell>
                    <TableCell>
                      <Badge variant={STATUS_VARIANT[log.status]}>{log.status}</Badge>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : (
            <p className="text-sm text-muted-foreground p-4">{t("transactions.empty")}</p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
