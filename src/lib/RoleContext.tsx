import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from "react";

/** UI view role — controls which tabs are visible. Different from backend Role (supplier/consumer/idle). */
export type ViewRole = "supplier" | "consumer" | "both";

const STORAGE_KEY = "shareplan:role";

function loadViewRole(): ViewRole {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "supplier" || stored === "consumer" || stored === "both") {
    return stored;
  }
  return "both";
}

interface RoleContextValue {
  viewRole: ViewRole;
  setViewRole: (role: ViewRole) => void;
}

const RoleContext = createContext<RoleContextValue>({
  viewRole: "both",
  setViewRole: () => {},
});

export function RoleProvider({ children }: { children: ReactNode }) {
  const [viewRole, setViewRoleState] = useState<ViewRole>(loadViewRole);

  const setViewRole = useCallback((role: ViewRole) => {
    localStorage.setItem(STORAGE_KEY, role);
    setViewRoleState(role);
  }, []);

  // Sync if another tab changes the role
  useEffect(() => {
    function onStorage(e: StorageEvent) {
      if (e.key === STORAGE_KEY && e.newValue) {
        const v = e.newValue;
        if (v === "supplier" || v === "consumer" || v === "both") {
          setViewRoleState(v);
        }
      }
    }
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  return (
    <RoleContext.Provider value={{ viewRole, setViewRole }}>
      {children}
    </RoleContext.Provider>
  );
}

export function useViewRole() {
  return useContext(RoleContext);
}