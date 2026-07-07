import { useEffect, useState, useCallback, useMemo } from "react";
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
import { authLogout, getClientConfig, type AuthState, type ClientConfig } from "@/lib/api";
import { subscribeAuthStateChanged, subscribeConnectionState, subscribeHealthUpdate, type ConnectionState as WsConnectionState, type HealthUpdateEvent } from "@/lib/events";
import { RoleProvider, useViewRole, type ViewRole } from "@/lib/RoleContext";
import { RoleSwitcher } from "@/components/RoleSwitcher";
import { cn } from "@/components/ui/cn";
import { Button } from "@/components/ui/button";

type Tab = "share" | "consume" | "wallet" | "providers" | "p2p" | "logs" | "settings";

interface TabDef {
  id: Tab;
  icon: typeof Share2;
  titleKey: string;
  roles: ViewRole[];
}

const TABS: TabDef[] = [
  { id: "share", icon: Share2, titleKey: "tabs.share", roles: ["supplier", "both"] },
  { id: "consume", icon: ArrowDownToLine, titleKey: "tabs.consume", roles: ["consumer", "both"] },
  { id: "wallet", icon: Wallet, titleKey: "tabs.wallet", roles: ["supplier", "consumer", "both"] },
  { id: "providers", icon: Server, titleKey: "tabs.providers", roles: ["supplier", "both"] },
  { id: "p2p", icon: Radio, titleKey: "tabs.p2p", roles: ["supplier", "consumer", "both"] },
  { id: "logs", icon: ScrollText, titleKey: "tabs.logs", roles: ["supplier", "consumer", "both"] },
  { id: "settings", icon: Settings, titleKey: "tabs.settings", roles: ["supplier", "consumer", "both"] },
];

const DEFAULT_TAB: Record<ViewRole, Tab> = {
  supplier: "share",
  consumer: "consume",
  both: "share",
};

type HealthStatus = "unknown" | "connected" | "healthy" | "degraded" | "disconnected";

function ServerHealthIndicator() {
  const { t } = useTranslation("share");
  const [connectionState, setConnectionState] = useState<WsConnectionState>("disconnected");
  const [latencyMs, setLatencyMs] = useState<number | null>(null);

  useEffect(() => {
    let unlistenConn: (() => void) | null = null;
    let unlistenHealth: (() => void) | null = null;

    subscribeConnectionState((state) => {
      setConnectionState(state);
      // Reset latency when disconnected/reconnecting
      if (state === "disconnected" || state === "connecting") {
        setLatencyMs(null);
      }
    }).then((fn) => { unlistenConn = fn; }).catch(() => {});

    subscribeHealthUpdate((event: HealthUpdateEvent) => {
      if (event.healthy) {
        setLatencyMs(event.latency_ms);
      }
    }).then((fn) => { unlistenHealth = fn; }).catch(() => {});

    return () => {
      unlistenConn?.();
      unlistenHealth?.();
    };
  }, []);

  const status: HealthStatus = (() => {
    if (connectionState === "disconnected") return "disconnected";
    if (connectionState === "connecting" || connectionState === "reconnecting") return "unknown";
    // connected
    if (latencyMs === null) return "connected"; // Connected but no heartbeat pong yet
    if (latencyMs < 200) return "healthy";
    return "degraded";
  })();

  if (status === "disconnected") {
    return (
      <div className="flex items-center gap-1.5 text-xs text-red-500 dark:text-red-400" title={t("health.unhealthy")}>
        <div className="w-2 h-2 rounded-full bg-red-500" />
        {t("health.unhealthy")}
      </div>
    );
  }

  if (status === "unknown") {
    return (
      <div className="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400">
        <div className="w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
        {t("connection.connecting")}
      </div>
    );
  }

  if (status === "connected") {
    return (
      <div className="flex items-center gap-1.5 text-xs text-blue-600 dark:text-blue-400">
        <div className="w-2 h-2 rounded-full bg-blue-500 animate-pulse" />
        {t("health.checking")}
      </div>
    );
  }

  if (status === "degraded") {
    return (
      <div className="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400" title={`${latencyMs}ms`}>
        <div className="w-2 h-2 rounded-full bg-amber-500" />
        {t("health.healthy")} ({latencyMs}ms)
      </div>
    );
  }

  // healthy
  return (
    <div className="flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
      <div className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse" />
      {t("health.healthy")} ({latencyMs}ms)
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
  return (
    <RoleProvider>
      <AppInner />
    </RoleProvider>
  );
}

function AppInner() {
  const { t } = useTranslation();
  const { viewRole } = useViewRole();
  const visibleTabs = useMemo(() => TABS.filter((tab) => tab.roles.includes(viewRole)), [viewRole]);
  const [tab, setTab] = useState<Tab>("share");
  const { authState, setAuthState, loading } = useAuthState("");
  const [serverHost, setServerHost] = useState("");

  // When viewRole changes, if the current tab is no longer visible, switch to the default
  useEffect(() => {
    if (!visibleTabs.some((t) => t.id === tab)) {
      setTab(DEFAULT_TAB[viewRole]);
    }
  }, [viewRole, visibleTabs, tab]);

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
        <span className="mr-2 text-sm font-semibold">SharePlan</span>
        <RoleSwitcher />
        <div className="flex-1" />
        <ServerHealthIndicator />
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
      </header>

      <nav className="flex h-9 shrink-0 items-center gap-1 border-b border-black/5 dark:border-white/5 px-3 overflow-x-auto">
        {visibleTabs.map(({ id, icon: Icon, titleKey }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={cn(
              "inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors whitespace-nowrap",
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
      </nav>

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