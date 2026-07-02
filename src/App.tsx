import { useEffect, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Share2, Wallet, Settings, Server, ArrowDownToLine, ScrollText, Radio, LogOut, User, LogIn } from "lucide-react";
import { SharePanel } from "./share/SharePanel";
import { WalletPanel } from "./wallet/WalletPanel";
import { ProvidersPanel } from "./providers/ProvidersPanel";
import { SettingsPanel } from "./settings/SettingsPanel";
import { ConsumePanel } from "./consume/ConsumePanel";
import { P2PPanel } from "./p2p/P2PPanel";
import { SystemLogPanel } from "./system_log/SystemLogPanel";
import { useAuthState } from "./auth/AuthPanel";
import { AuthDialog } from "./auth/AuthDialog";
import { authLogout, getClientConfig, checkServerHealth, type AuthState, type ClientConfig, type ServerHealthResult } from "@/lib/api";
import { subscribeAuthStateChanged } from "@/lib/events";
import { cn } from "@/components/ui/cn";
import { Button } from "@/components/ui/button";

type Tab = "share" | "consume" | "wallet" | "providers" | "p2p" | "logs" | "settings";

const TABS: { id: Tab; icon: typeof Share2; titleKey: string }[] = [
  { id: "share", icon: Share2, titleKey: "tabs.share" },
  { id: "consume", icon: ArrowDownToLine, titleKey: "tabs.consume" },
  { id: "wallet", icon: Wallet, titleKey: "tabs.wallet" },
  { id: "providers", icon: Server, titleKey: "tabs.providers" },
  { id: "p2p", icon: Radio, titleKey: "tabs.p2p" },
  { id: "logs", icon: ScrollText, titleKey: "tabs.logs" },
  { id: "settings", icon: Settings, titleKey: "tabs.settings" },
];

function ServerHealthIndicator() {
  const { t } = useTranslation("share");
  const [health, setHealth] = useState<ServerHealthResult | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function check() {
      try {
        const result = await checkServerHealth();
        if (!cancelled) setHealth(result);
      } catch {
        if (!cancelled) {
          setHealth({ healthy: false, latency_ms: 0, error: "unreachable" });
        }
      }
    }

    check();
    const interval = setInterval(check, 30_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  if (health === null) {
    return (
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <div className="w-2 h-2 rounded-full bg-zinc-300 dark:bg-zinc-600" />
        {t("health.checking")}
      </div>
    );
  }

  if (health.healthy) {
    return (
      <div className="flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
        <div className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse" />
        {t("health.healthy")} ({health.latency_ms}ms)
      </div>
    );
  }

  return (
    <div className="flex items-center gap-1.5 text-xs text-red-500 dark:text-red-400" title={health.error ?? undefined}>
      <div className="w-2 h-2 rounded-full bg-red-500" />
      {t("health.unhealthy")}
    </div>
  );
}

function AccountBadge({ authState, serverHost, onLogout }: {
  authState: AuthState;
  serverHost: string;
  onLogout: () => void;
}) {
  const { t } = useTranslation("auth");
  const [loggingOut, setLoggingOut] = useState(false);

  async function handleLogout() {
    setLoggingOut(true);
    try {
      await authLogout(serverHost);
      onLogout();
    } catch {
      // Even if server logout fails, clear local state
      onLogout();
    } finally {
      setLoggingOut(false);
    }
  }

  return (
    <div className="flex items-center gap-2 text-xs">
      <User className="h-3.5 w-3.5 text-muted-foreground" />
      <span className="max-w-[120px] truncate text-muted-foreground" title={authState.email}>
        {authState.display_name || authState.email}
      </span>
      <Button
        size="sm"
        variant="ghost"
        className="h-6 px-1.5 text-xs"
        onClick={() => void handleLogout()}
        disabled={loggingOut}
      >
        <LogOut className="h-3 w-3" />
      </Button>
    </div>
  );
}

export default function App() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("share");
  const { authState, setAuthState, loading } = useAuthState("");
  const [serverHost, setServerHost] = useState("");

  // Load server host from config
  useEffect(() => {
    getClientConfig()
      .then((config: ClientConfig) => setServerHost(config.server_host))
      .catch(() => undefined);
  }, []);

  // Subscribe to auth state changes from browser login and token refresh.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    subscribeAuthStateChanged((state) => {
      setAuthState(state);
    }).then((fn) => {
      unlisten = fn;
    }).catch(() => {
      // Event subscription can fail if the Tauri backend isn't ready yet.
      // This is non-critical — the auth state is also loaded via getAuthState.
    });
    return () => {
      unlisten?.();
    };
  }, [setAuthState]);

  const handleLogout = useCallback(() => {
    setAuthState(null);
  }, [setAuthState]);

  const [showAuthDialog, setShowAuthDialog] = useState(false);

  const handleAuthDialogAuth = useCallback((state: AuthState) => {
    setAuthState(state);
    setShowAuthDialog(false);
  }, [setAuthState]);

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center bg-white dark:bg-zinc-950">
        <div className="text-sm text-muted-foreground">Loading…</div>
      </div>
    );
  }

  return (
    <div className="flex h-screen flex-col bg-white text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
      <header className="flex h-12 shrink-0 items-center gap-1 border-b border-black/10 dark:border-white/10 px-3">
        <span className="mr-auto text-sm font-semibold">SharePlan</span>
        <div className="absolute left-1/2 -translate-x-1/2">
          <ServerHealthIndicator />
        </div>
        {authState ? (
          <AccountBadge authState={authState} serverHost={serverHost} onLogout={handleLogout} />
        ) : (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-1.5 text-xs"
            onClick={() => setShowAuthDialog(true)}
          >
            <LogIn className="h-3 w-3 mr-1" />
            {t("auth:dialog.title")}
          </Button>
        )}
        {TABS.map(({ id, icon: Icon, titleKey }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-md px-3 text-xs font-medium transition-colors",
              tab === id
                ? "bg-black/10 dark:bg-white/15 text-foreground"
                : "text-muted-foreground hover:bg-black/5 dark:hover:bg-white/10",
            )}
            title={t(titleKey)}
          >
            <Icon className="h-3.5 w-3.5" />
            {t(titleKey)}
          </button>
        ))}
      </header>

      <main className="flex-1 overflow-auto p-4">
        {tab === "share" && <SharePanel onSignInNeeded={!authState ? () => setShowAuthDialog(true) : undefined} />}
        {tab === "consume" && <ConsumePanel />}
        {tab === "wallet" && <WalletPanel />}
        {tab === "providers" && <ProvidersPanel />}
        {tab === "p2p" && <P2PPanel />}
        {tab === "logs" && <SystemLogPanel />}
        {tab === "settings" && <SettingsPanel onServerHostChanged={setServerHost} onSignInNeeded={!authState && serverHost ? () => setShowAuthDialog(true) : undefined} />}
      </main>

      <AuthDialog
        open={showAuthDialog}
        onClose={() => setShowAuthDialog(false)}
        onAuth={handleAuthDialogAuth}
      />
    </div>
  );
}