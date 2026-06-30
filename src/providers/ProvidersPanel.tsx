import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, AlertTriangle, CheckCircle2, XCircle, Info } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  refreshProviders,
  getWhitelist,
  setWhitelist,
  getDiagnostics,
  type DiscoverySnapshot,
  type DiagnosticWarning,
} from "@/lib/api";

export function ProvidersPanel() {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<DiscoverySnapshot | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticWarning[]>([]);
  const [whitelist, setWhitelistState] = useState<string[]>([]);
  const [whitelistInput, setWhitelistInput] = useState("");
  const [loading, setLoading] = useState(false);

  async function load() {
    setLoading(true);
    try {
      const [snap, diags, wl] = await Promise.all([
        refreshProviders(),
        getDiagnostics(),
        getWhitelist(),
      ]);
      setSnapshot(snap);
      setDiagnostics(diags);
      setWhitelistState(wl);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    const id = setInterval(load, 30000);
    return () => clearInterval(id);
  }, []);

  async function addWhitelist() {
    const m = whitelistInput.trim();
    if (!m) return;
    const next = [...whitelist, m];
    await setWhitelist(next);
    setWhitelistState(next);
    setWhitelistInput("");
  }

  async function removeWhitelist(m: string) {
    const next = whitelist.filter((x) => x !== m);
    await setWhitelist(next);
    setWhitelistState(next);
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <h2 className="text-base font-semibold">{t("share.tabs.providers")}</h2>
        <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading}>
          <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
        </Button>
      </div>

      {/* Diagnostics */}
      {diagnostics.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>{t("providers.diagnostics", { defaultValue: "Diagnostics" })}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            {diagnostics.map((d) => (
              <div key={d.code} className="flex items-start gap-2 text-sm">
                {d.severity === "error" ? (
                  <XCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-500" />
                ) : d.severity === "warn" ? (
                  <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
                ) : d.severity === "info" ? (
                  <Info className="mt-0.5 h-4 w-4 shrink-0 text-blue-500" />
                ) : (
                  <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-green-500" />
                )}
                <div>
                  <div className="font-medium">{d.code}</div>
                  <div className="text-muted-foreground">{d.message}</div>
                </div>
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      {/* Discovery snapshot */}
      <Card>
        <CardHeader>
          <CardTitle>
            {t("providers.ccswitchStatus", { defaultValue: "cc-switch proxy status" })}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {!snapshot ? (
            <div className="text-sm text-muted-foreground">Loading…</div>
          ) : (
            <div className="space-y-3 text-sm">
              <div className="flex flex-wrap gap-2">
                <Badge variant={snapshot.reachable ? "success" : "destructive"}>
                  {snapshot.reachable ? "reachable" : "unreachable"}
                </Badge>
                <Badge variant={snapshot.running ? "success" : "warning"}>
                  {snapshot.running ? "running" : "not running"}
                </Badge>
                {snapshot.current_provider && (
                  <Badge variant="secondary">current: {snapshot.current_provider}</Badge>
                )}
              </div>
              {snapshot.last_error && (
                <div className="text-xs text-red-500">{snapshot.last_error}</div>
              )}
              {snapshot.providers.length > 0 ? (
                <div className="space-y-2">
                  {snapshot.providers.map((p) => (
                    <div
                      key={p.provider_id}
                      className={`rounded-md border p-2 ${
                        snapshot.from_db
                          ? "border-dashed border-amber-400/50 bg-amber-50/50 dark:border-amber-400/30 dark:bg-amber-950/20"
                          : "border-black/10 dark:border-white/10"
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <span className="font-medium">{p.provider_name}</span>
                        <Badge
                          variant="outline"
                          className={snapshot.from_db ? "text-amber-600 border-amber-400" : ""}
                        >
                          {p.api_format}
                          {snapshot.from_db && ` · ${t("providers.configured", { defaultValue: "已配置" })}`}
                        </Badge>
                      </div>
                      <div className="mt-1 text-xs text-muted-foreground">
                        {snapshot.from_db
                          ? `app_type: ${p.app_type} · models: ${p.models.join(", ")}`
                          : `app_type: ${p.app_type} · models: ${p.models.join(", ")}`
                        }
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="text-xs text-muted-foreground">
                  No active provider targets. Configure one in cc-switch.
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Whitelist */}
      <Card>
        <CardHeader>
          <CardTitle>{t("providers.whitelist", { defaultValue: "Share whitelist" })}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-xs text-muted-foreground">
            {t("providers.whitelistHint", {
              defaultValue:
                "Restrict which models this node advertises to the cloud. Empty = share everything discovered.",
            })}
          </p>
          <div className="flex gap-2">
            <input
              className="flex-1 rounded-md border border-black/15 px-3 py-1 text-sm dark:border-white/15"
              placeholder="e.g. claude or gpt-4o"
              value={whitelistInput}
              onChange={(e) => setWhitelistInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void addWhitelist();
              }}
            />
            <Button size="sm" onClick={() => void addWhitelist()}>
              {t("providers.add", { defaultValue: "Add" })}
            </Button>
          </div>
          {whitelist.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {whitelist.map((m) => (
                <Badge key={m} variant="secondary" className="gap-1">
                  {m}
                  <button
                    className="ml-1 text-xs opacity-60 hover:opacity-100"
                    onClick={() => void removeWhitelist(m)}
                  >
                    ×
                  </button>
                </Badge>
              ))}
            </div>
          ) : (
            <div className="text-xs text-muted-foreground">
              {t("providers.whitelistEmpty", { defaultValue: "Empty — sharing all discovered models." })}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
