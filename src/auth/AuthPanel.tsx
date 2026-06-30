import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { LogIn, UserPlus, Eye, EyeOff, Loader2, Globe } from "lucide-react";
import {
  authRegister,
  authLogin,
  authBrowserLogin,
  getAuthState,
  type AuthState,
  type AuthError,
} from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface AuthPanelProps {
  serverHost: string;
  onAuth: (state: AuthState) => void;
}

type View = "browser" | "login" | "register";

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

export function AuthPanel({ serverHost, onAuth }: AuthPanelProps) {
  const { t } = useTranslation();
  const [view, setView] = useState<View>("browser");

  return (
    <div className="flex min-h-screen items-center justify-center bg-white dark:bg-zinc-950">
      <div className="w-full max-w-sm">
        {view === "browser" ? (
          <BrowserLoginPanel
            serverHost={serverHost}
            onAuth={onAuth}
            onManualLogin={() => setView("login")}
          />
        ) : view === "login" ? (
          <LoginForm
            serverHost={serverHost}
            onAuth={onAuth}
            onSwitchToRegister={() => setView("register")}
            onSwitchToBrowser={() => setView("browser")}
          />
        ) : (
          <RegisterForm
            serverHost={serverHost}
            onAuth={onAuth}
            onSwitchToLogin={() => setView("login")}
            onSwitchToBrowser={() => setView("browser")}
          />
        )}
      </div>
    </div>
  );
}

function BrowserLoginPanel({
  serverHost,
  onAuth,
  onManualLogin,
}: {
  serverHost: string;
  onAuth: (state: AuthState) => void;
  onManualLogin: () => void;
}) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleBrowserLogin() {
    setError(null);
    setLoading(true);
    try {
      const state = await authBrowserLogin(serverHost);
      onAuth(state);
    } catch (err) {
      const authErr = err as AuthError;
      setError(formatAuthError(authErr, t));
    } finally {
      setLoading(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Globe className="h-5 w-5" />
          {t("auth:browserLogin.title")}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {t("auth:browserLogin.description")}
        </p>
        {error && (
          <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
        )}
        <Button
          type="button"
          className="w-full"
          disabled={loading}
          onClick={() => void handleBrowserLogin()}
        >
          {loading ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Globe className="mr-2 h-4 w-4" />
          )}
          {loading ? t("auth:browserLogin.waiting") : t("auth:browserLogin.button")}
        </Button>
        <p className="text-center text-sm text-muted-foreground">
          <button
            type="button"
            className="text-primary underline underline-offset-2"
            onClick={onManualLogin}
            disabled={loading}
          >
            {t("auth:browserLogin.manualLink")}
          </button>
        </p>
      </CardContent>
    </Card>
  );
}

function LoginForm({
  serverHost,
  onAuth,
  onSwitchToRegister,
  onSwitchToBrowser,
}: {
  serverHost: string;
  onAuth: (state: AuthState) => void;
  onSwitchToRegister: () => void;
  onSwitchToBrowser: () => void;
}) {
  const { t } = useTranslation();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!email.trim()) return;
    setLoading(true);
    try {
      const state = await authLogin(serverHost, email.trim(), password);
      onAuth(state);
    } catch (err) {
      const authErr = err as AuthError;
      setError(formatAuthError(authErr, t));
    } finally {
      setLoading(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <LogIn className="h-5 w-5" />
          {t("auth:login.title")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="login-email">{t("auth:login.email")}</Label>
            <Input
              id="login-email"
              type="email"
              autoComplete="email"
              placeholder={t("auth:login.emailPlaceholder")}
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              disabled={loading}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="login-password">{t("auth:login.password")}</Label>
            <div className="relative">
              <Input
                id="login-password"
                type={showPassword ? "text" : "password"}
                autoComplete="current-password"
                placeholder={t("auth:login.passwordPlaceholder")}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                disabled={loading}
                className="pr-9"
              />
              <button
                type="button"
                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                onClick={() => setShowPassword(!showPassword)}
                tabIndex={-1}
              >
                {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </button>
            </div>
          </div>
          {error && (
            <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
          )}
          <Button type="submit" className="w-full" disabled={loading}>
            {loading ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <LogIn className="mr-2 h-4 w-4" />}
            {loading ? t("auth:login.submitting") : t("auth:login.submit")}
          </Button>
        </form>
        <p className="mt-4 text-center text-sm text-muted-foreground">
          {t("auth:login.noAccount")}{" "}
          <button
            type="button"
            className="text-primary underline underline-offset-2"
            onClick={onSwitchToRegister}
          >
            {t("auth:login.registerLink")}
          </button>
        </p>
        <p className="mt-2 text-center text-sm text-muted-foreground">
          <button
            type="button"
            className="text-primary underline underline-offset-2"
            onClick={onSwitchToBrowser}
          >
            {t("auth:browserLogin.button")}
          </button>
        </p>
      </CardContent>
    </Card>
  );
}

function RegisterForm({
  serverHost,
  onAuth,
  onSwitchToLogin,
  onSwitchToBrowser,
}: {
  serverHost: string;
  onAuth: (state: AuthState) => void;
  onSwitchToLogin: () => void;
  onSwitchToBrowser: () => void;
}) {
  const { t } = useTranslation();
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function validate(): string | null {
    if (!email.trim()) return t("auth:errors.emailInvalid");
    if (password.length < 8) return t("auth:errors.passwordTooShort");
    if (password !== confirmPassword) return t("auth:errors.passwordMismatch");
    return null;
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const validationError = validate();
    if (validationError) {
      setError(validationError);
      return;
    }
    setError(null);
    setLoading(true);
    try {
      const state = await authRegister(
        serverHost,
        email.trim(),
        password,
        displayName.trim() || undefined,
      );
      onAuth(state);
    } catch (err) {
      const authErr = err as AuthError;
      setError(formatAuthError(authErr, t));
    } finally {
      setLoading(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <UserPlus className="h-5 w-5" />
          {t("auth:register.title")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="reg-email">{t("auth:register.email")}</Label>
            <Input
              id="reg-email"
              type="email"
              autoComplete="email"
              placeholder={t("auth:register.emailPlaceholder")}
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              disabled={loading}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="reg-display-name">{t("auth:register.displayName")}</Label>
            <Input
              id="reg-display-name"
              type="text"
              placeholder={t("auth:register.displayNamePlaceholder")}
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              disabled={loading}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="reg-password">{t("auth:register.password")}</Label>
            <div className="relative">
              <Input
                id="reg-password"
                type={showPassword ? "text" : "password"}
                autoComplete="new-password"
                placeholder={t("auth:register.passwordPlaceholder")}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                disabled={loading}
                className="pr-9"
              />
              <button
                type="button"
                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                onClick={() => setShowPassword(!showPassword)}
                tabIndex={-1}
              >
                {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </button>
            </div>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="reg-confirm-password">{t("auth:register.confirmPassword")}</Label>
            <Input
              id="reg-confirm-password"
              type="password"
              autoComplete="new-password"
              placeholder={t("auth:register.confirmPassword")}
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              disabled={loading}
            />
          </div>
          {error && (
            <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
          )}
          <Button type="submit" className="w-full" disabled={loading}>
            {loading ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <UserPlus className="mr-2 h-4 w-4" />}
            {loading ? t("auth:register.submitting") : t("auth:register.submit")}
          </Button>
        </form>
        <p className="mt-4 text-center text-sm text-muted-foreground">
          {t("auth:register.hasAccount")}{" "}
          <button
            type="button"
            className="text-primary underline underline-offset-2"
            onClick={onSwitchToLogin}
          >
            {t("auth:register.loginLink")}
          </button>
        </p>
        <p className="mt-2 text-center text-sm text-muted-foreground">
          <button
            type="button"
            className="text-primary underline underline-offset-2"
            onClick={onSwitchToBrowser}
          >
            {t("auth:browserLogin.button")}
          </button>
        </p>
      </CardContent>
    </Card>
  );
}

/** Hook to load auth state on mount and listen for changes. */
export function useAuthState(_serverHost: string) {
  const [authState, setAuthState] = useState<AuthState | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    getAuthState()
      .then((state) => {
        if (!cancelled) setAuthState(state);
      })
      .catch(() => {
        if (!cancelled) setAuthState(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, []);

  return { authState, setAuthState, loading, loadState: () => { /* unused */ } };
}