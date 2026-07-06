import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Rocket, Wallet, Recycle, AlertTriangle } from "lucide-react";
import { useViewRole, type ViewRole } from "@/lib/RoleContext";
import { shareGetStatus, getLocalServerAddr } from "@/lib/api";
import { cn } from "@/components/ui/cn";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

const VIEW_ROLES: { id: ViewRole; icon: typeof Rocket; titleKey: string }[] = [
  { id: "supplier", icon: Rocket, titleKey: "roleSwitcher.supplier" },
  { id: "consumer", icon: Wallet, titleKey: "roleSwitcher.consumer" },
  // { id: "both", icon: Recycle, titleKey: "roleSwitcher.both" },
];

export function RoleSwitcher() {
  const { t } = useTranslation("share");
  const { viewRole, setViewRole } = useViewRole();
  const [blockDialog, setBlockDialog] = useState<{ target: ViewRole; reason: string } | null>(null);

  async function handleSwitch(target: ViewRole) {
    if (target === viewRole) return;

    // If leaving supplier, check if sharing is active
    if (viewRole === "supplier" && target !== "supplier") {
      try {
        const status = await shareGetStatus();
        if (status === "connected" || status === "connecting" || status === "reconnecting") {
          setBlockDialog({
            target,
            reason: t("roleSwitcher.blockedBySharing"),
          });
          return;
        }
      } catch {
        // If we can't check status, allow the switch
      }
    }

    // If leaving consumer, check if proxy is running
    if (viewRole === "consumer" && target !== "consumer") {
      try {
        const addr = await getLocalServerAddr();
        if (addr) {
          setBlockDialog({
            target,
            reason: t("roleSwitcher.blockedByProxy"),
          });
          return;
        }
      } catch {
        // If we can't check status, allow the switch
      }
    }

    setViewRole(target);
  }

  return (
    <>
      <div className="inline-flex rounded-md border border-black/10 dark:border-white/10 p-0.5">
        {VIEW_ROLES.map(({ id, icon: Icon, titleKey }) => (
          <button
            key={id}
            onClick={() => void handleSwitch(id)}
            className={cn(
              "inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
              viewRole === id
                ? "bg-black/10 dark:bg-white/15 text-foreground"
                : "text-muted-foreground hover:bg-black/5 dark:hover:bg-white/10",
            )}
            title={t(titleKey)}
          >
            <Icon className="h-3.5 w-3.5" />
            {t(titleKey)}
          </button>
        ))}
      </div>

      <Dialog open={blockDialog !== null} onOpenChange={(open) => { if (!open) setBlockDialog(null); }}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <AlertTriangle className="h-5 w-5 text-amber-500" />
              {t("roleSwitcher.blockedTitle")}
            </DialogTitle>
            <DialogDescription>
              {blockDialog?.reason}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setBlockDialog(null)}>
              {t("roleSwitcher.blockedDismiss")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}