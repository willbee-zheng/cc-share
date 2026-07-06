import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronRight, Rocket } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DEFAULT_CLIENT_CONFIG,
  getClientConfig,
  setClientConfig,
  type ClientConfig,
} from "../lib/api";
import { useViewRole, type ViewRole } from "@/lib/RoleContext";

const STORAGE_KEY = "shareplan:onboarded";

type Step = "welcome" | "config" | "role" | "done";

function isOnboarded(): boolean {
  return localStorage.getItem(STORAGE_KEY) === "1";
}

function markOnboarded(role: ViewRole) {
  localStorage.setItem(STORAGE_KEY, "1");
  localStorage.setItem("shareplan:role", role);
}

/**
 * 首次使用引导：检测 share.db 是否已配置（server_host+auth_token 都非空）→
 * 未配置则弹出向导。完成后写 localStorage 标记，下次不再弹。
 */
export function FirstRunGuide() {
  const { t } = useTranslation("share");
  const { setViewRole } = useViewRole();
  const [open, setOpen] = useState(false);
  const [step, setStep] = useState<Step>("welcome");
  const [config, setConfig] = useState<ClientConfig>(DEFAULT_CLIENT_CONFIG);
  const [role, setRole] = useState<ViewRole>("supplier");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (isOnboarded()) return;
    let cancelled = false;
    void getClientConfig()
      .then((c) => {
        if (cancelled) return;
        setConfig(c);
        // 已经手动配置过的用户也算 onboarded，避免再次打扰
        if (c.server_host && c.auth_token && c.node_id) {
          markOnboarded("supplier");
          return;
        }
        setOpen(true);
      })
      .catch(() => {
        // 加载配置失败也展示引导
        if (!cancelled) setOpen(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function update<K extends keyof ClientConfig>(key: K, value: ClientConfig[K]) {
    setConfig((prev) => ({ ...prev, [key]: value }));
  }

  async function handleSaveAndContinue() {
    setError(null);
    if (!config.server_host || !config.auth_token || !config.node_id) {
      setError(t("guide.errors.required"));
      return;
    }
    setSaving(true);
    try {
      await setClientConfig(config);
      setStep("role");
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  function handleFinish() {
    markOnboarded(role);
    setViewRole(role);
    setStep("done");
    setTimeout(() => setOpen(false), 800);
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        // 只允许向导内部关；用户点 X 时也标记完成（不强制）
        if (!o) markOnboarded(role);
        setOpen(o);
      }}
    >
      <DialogContent className="max-w-md">
        {step === "welcome" && (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <Rocket className="w-5 h-5" />
                {t("guide.welcome.title")}
              </DialogTitle>
              <DialogDescription>{t("guide.welcome.description")}</DialogDescription>
            </DialogHeader>
            <ul className="text-sm text-muted-foreground list-disc pl-5 space-y-1.5">
              <li>{t("guide.welcome.bulletEarn")}</li>
              <li>{t("guide.welcome.bulletConsume")}</li>
              <li>{t("guide.welcome.bulletSafety")}</li>
            </ul>
            <DialogFooter>
              <Button onClick={() => setStep("config")}>
                {t("guide.welcome.start")}
                <ChevronRight className="w-4 h-4 ml-1" />
              </Button>
            </DialogFooter>
          </>
        )}

        {step === "config" && (
          <>
            <DialogHeader>
              <DialogTitle>{t("guide.config.title")}</DialogTitle>
              <DialogDescription>{t("guide.config.description")}</DialogDescription>
            </DialogHeader>
            <div className="space-y-3">
              <div className="space-y-1.5">
                <Label htmlFor="guide-url">{t("config.serverHost")}</Label>
                <Input
                  id="guide-url"
                  value={config.server_host}
                  onChange={(e) => update("server_host", e.target.value)}
                  placeholder="api.cc-share.com 或 192.168.1.60:8080"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="guide-token">{t("config.authToken")}</Label>
                <Input
                  id="guide-token"
                  type="password"
                  value={config.auth_token}
                  onChange={(e) => update("auth_token", e.target.value)}
                  autoComplete="off"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="guide-node">{t("config.nodeId")}</Label>
                <Input
                  id="guide-node"
                  value={config.node_id}
                  onChange={(e) => update("node_id", e.target.value)}
                  placeholder="my-laptop"
                />
              </div>
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
            <DialogFooter>
              <Button variant="ghost" onClick={() => setStep("welcome")}>
                {t("guide.back")}
              </Button>
              <Button onClick={handleSaveAndContinue} disabled={saving}>
                {saving ? t("config.saving") : t("guide.next")}
              </Button>
            </DialogFooter>
          </>
        )}

        {step === "role" && (
          <>
            <DialogHeader>
              <DialogTitle>{t("guide.role.title")}</DialogTitle>
              <DialogDescription>{t("guide.role.description")}</DialogDescription>
            </DialogHeader>
            <div className="space-y-2">
              {(["supplier", "consumer", "both"] as const).map((r) => (
                <button
                  key={r}
                  type="button"
                  onClick={() => setRole(r)}
                  className={`w-full text-left rounded-lg border p-3 transition ${
                    role === r ? "border-primary bg-accent/50" : "hover:bg-accent/30"
                  }`}
                >
                  <p className="font-medium text-sm">{t(`guide.role.${r}.title`)}</p>
                  <p className="text-xs text-muted-foreground mt-1">
                    {t(`guide.role.${r}.description`)}
                  </p>
                </button>
              ))}
            </div>
            <DialogFooter>
              <Button variant="ghost" onClick={() => setStep("config")}>
                {t("guide.back")}
              </Button>
              <Button onClick={handleFinish}>{t("guide.finish")}</Button>
            </DialogFooter>
          </>
        )}

        {step === "done" && (
          <div className="flex flex-col items-center py-8">
            <Check className="w-12 h-12 text-emerald-500 mb-3" />
            <p className="font-medium">{t("guide.done")}</p>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
