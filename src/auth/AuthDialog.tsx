import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Globe, Loader2 } from "lucide-react";
import { authBrowserLogin, getClientConfig, type AuthState, type AuthError } from "@/lib/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";

interface AuthDialogProps {
  open: boolean;
  onClose: () => void;
  onAuth: (state: AuthState) => void;
}

function formatAuthError(err: AuthError, t: (key: string, opts?: Record<string, string>) => string): string {
  switch (err.kind) {
    case "network":
      return t("auth:errors.network");
    case "unauthorized":
      return t("auth:errors.unauthorized");
    case "email_exists":
      return t("auth:errors.emailExists");
    case "invalid_credentials":
      return t("auth:errors.invalidCredentials");
    case "token_expired":
      return t("auth:errors.tokenExpired");
    case "validation":
      return t("auth:errors.validation", { message: err.message });
    case "server":
      return t("auth:errors.server", { message: err.message });
    default:
      return t("auth:errors.unknown");
  }
}

export function AuthDialog({ open, onClose, onAuth }: AuthDialogProps) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleBrowserLogin() {
    setError(null);
    setLoading(true);
    try {
      // Read the latest config from database, not from potentially stale React state.
      const config = await getClientConfig();
      const host = config.server_host;
      if (!host) {
        setError(t("auth:errors.network"));
        return;
      }
      const state = await authBrowserLogin(host);
      onAuth(state);
    } catch (err) {
      const authErr = err as AuthError;
      setError(formatAuthError(authErr, t));
    } finally {
      setLoading(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("auth:dialog.title")}</DialogTitle>
          <DialogDescription>{t("auth:dialog.description")}</DialogDescription>
        </DialogHeader>

        {error && (
          <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
        )}

        <DialogFooter>
          <Button
            variant="ghost"
            onClick={onClose}
          >
            {t("auth:dialog.cancel")}
          </Button>
          <Button
            onClick={() => void handleBrowserLogin()}
            disabled={loading}
          >
            {loading ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <Globe className="mr-2 h-4 w-4" />
            )}
            {loading ? t("auth:browserLogin.waiting") : t("auth:browserLogin.button")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}