import { FormEvent, useEffect, useState } from "react";
import { Navigate, useLocation, useNavigate } from "react-router-dom";

import { PreviaLogo } from "@/components/PreviaLogo";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { fetchCurrentUser, login } from "@/lib/auth-client";
import { useAuthStore } from "@/stores/useAuthStore";

export default function LoginPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const token = useAuthStore((state) => state.token);
  const setSession = useAuthStore((state) => state.setSession);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const from = (location.state as { from?: { pathname?: string } } | null)?.from?.pathname ?? "/";

  useEffect(() => {
    if (token) return;
    let cancelled = false;
    fetchCurrentUser()
      .then((user) => {
        if (cancelled) return;
        setSession(null, user);
        navigate(from, { replace: true });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [from, navigate, setSession, token]);

  if (token) {
    return <Navigate to={from} replace />;
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      const response = await login(username, password);
      setSession(response.token, response.user);
      navigate(from, { replace: true });
    } catch {
      setError("Usuario ou senha invalidos");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <main className="flex min-h-screen items-center justify-center bg-background px-4">
      <form onSubmit={handleSubmit} className="w-full max-w-sm space-y-5">
        <div className="flex items-center gap-3">
          <PreviaLogo className="h-9 w-9" />
          <div>
            <h1 className="text-xl font-semibold">Previa</h1>
            <p className="text-sm text-muted-foreground">Acesso protegido</p>
          </div>
        </div>
        <div className="space-y-2">
          <Label htmlFor="username">Usuario</Label>
          <Input id="username" value={username} onChange={(event) => setUsername(event.target.value)} autoComplete="username" />
        </div>
        <div className="space-y-2">
          <Label htmlFor="password">Senha</Label>
          <Input id="password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} autoComplete="current-password" />
        </div>
        {error ? <p className="text-sm text-destructive">{error}</p> : null}
        <Button type="submit" className="w-full" disabled={submitting}>
          Entrar
        </Button>
      </form>
    </main>
  );
}
