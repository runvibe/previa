import { useEffect, useState } from "react";
import { Navigate, Outlet, useLocation } from "react-router-dom";

import { fetchCurrentUser } from "@/lib/auth-client";
import { useAuthStore } from "@/stores/useAuthStore";
import { DotsLoader } from "@/components/DotsLoader";

export function AuthGate() {
  const location = useLocation();
  const token = useAuthStore((state) => state.token);
  const setSession = useAuthStore((state) => state.setSession);
  const clearSession = useAuthStore((state) => state.clearSession);
  const [status, setStatus] = useState<"checking" | "allowed" | "login">("checking");

  useEffect(() => {
    let cancelled = false;
    setStatus("checking");
    fetchCurrentUser()
      .then((user) => {
        if (cancelled) return;
        setSession(token, user);
        setStatus("allowed");
      })
      .catch(() => {
        if (cancelled) return;
        clearSession();
        setStatus("login");
      });
    return () => {
      cancelled = true;
    };
  }, [clearSession, setSession, token]);

  if (status === "checking") {
    return (
      <div className="flex h-full min-h-screen items-center justify-center bg-background">
        <DotsLoader />
      </div>
    );
  }

  if (status === "login") {
    return <Navigate to="/login" replace state={{ from: location }} />;
  }

  return <Outlet />;
}
