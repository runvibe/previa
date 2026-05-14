import { FormEvent, type ReactNode, useEffect, useMemo, useState } from "react";
import { KeyRound, ShieldCheck, UserPlus } from "lucide-react";

import {
  createApiToken,
  createUser,
  deleteApiToken,
  deleteUser,
  listApiTokens,
  listUsers,
  updateApiToken,
  updateUser,
  type AccessRole,
  type ApiTokenRecord,
  type ManagedUser,
} from "@/lib/auth-client";
import { useAuthStore } from "@/stores/useAuthStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";

const ROLES: AccessRole[] = ["admin", "editor", "operator", "viewer"];

export default function AccessManagementPage() {
  const currentUser = useAuthStore((state) => state.user);
  const [users, setUsers] = useState<ManagedUser[]>([]);
  const [tokens, setTokens] = useState<ApiTokenRecord[]>([]);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [userRole, setUserRole] = useState<AccessRole>("viewer");
  const [tokenName, setTokenName] = useState("");
  const [tokenRole, setTokenRole] = useState<AccessRole>("viewer");
  const [createdToken, setCreatedToken] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const canManage = useMemo(
    () => currentUser?.role === "root" || currentUser?.role === "admin",
    [currentUser?.role],
  );

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const [nextUsers, nextTokens] = await Promise.all([listUsers(), listApiTokens()]);
      setUsers(nextUsers);
      setTokens(nextTokens);
    } catch {
      setError("Nao foi possivel carregar acessos");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  async function handleCreateUser(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    try {
      const user = await createUser({ username, password, role: userRole, active: true });
      setUsers((items) => [user, ...items]);
      setUsername("");
      setPassword("");
    } catch {
      setError("Nao foi possivel criar o usuario");
    }
  }

  async function handleCreateToken(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    try {
      const result = await createApiToken({ name: tokenName, role: tokenRole });
      setTokens((items) => [result.record, ...items]);
      setCreatedToken(result.token);
      setTokenName("");
    } catch {
      setError("Nao foi possivel criar o token");
    }
  }

  async function handleToggleUser(user: ManagedUser) {
    setError(null);
    try {
      const updated = await updateUser(user.id, { active: !user.active });
      setUsers((items) => items.map((item) => (item.id === updated.id ? updated : item)));
    } catch {
      setError("Nao foi possivel atualizar o usuario");
    }
  }

  async function handleDeleteUser(user: ManagedUser) {
    setError(null);
    try {
      await deleteUser(user.id);
      setUsers((items) => items.filter((item) => item.id !== user.id));
    } catch {
      setError("Nao foi possivel remover o usuario");
    }
  }

  async function handleToggleToken(token: ApiTokenRecord) {
    setError(null);
    try {
      const updated = await updateApiToken(token.id, !token.active);
      setTokens((items) => items.map((item) => (item.id === updated.id ? updated : item)));
    } catch {
      setError("Nao foi possivel atualizar o token");
    }
  }

  async function handleDeleteToken(token: ApiTokenRecord) {
    setError(null);
    try {
      await deleteApiToken(token.id);
      setTokens((items) => items.filter((item) => item.id !== token.id));
    } catch {
      setError("Nao foi possivel revogar o token");
    }
  }

  async function handleCopyToken() {
    if (!createdToken) return;
    await navigator.clipboard?.writeText(createdToken);
  }

  return (
    <main className="flex min-h-0 flex-1 flex-col overflow-auto px-6 py-5">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
        <header className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold">Acesso</h2>
            <p className="text-sm text-muted-foreground">
              {currentUser?.username ?? "anonymous"} - {currentUser?.role ?? "viewer"}
            </p>
          </div>
          <Badge variant={canManage ? "default" : "secondary"}>
            {canManage ? "gestao ativa" : "somente leitura"}
          </Badge>
        </header>

        {error ? <p className="rounded border border-destructive/30 px-3 py-2 text-sm text-destructive">{error}</p> : null}

        {canManage ? (
          <div className="grid gap-4 lg:grid-cols-2">
            <form onSubmit={handleCreateUser} className="space-y-3 rounded-md border bg-card p-4">
              <div className="flex items-center gap-2 text-sm font-medium">
                <UserPlus className="h-4 w-4" />
                Usuario
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="access-username">Login</Label>
                  <Input id="access-username" value={username} onChange={(event) => setUsername(event.target.value)} required />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="access-password">Senha</Label>
                  <Input id="access-password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} required />
                </div>
              </div>
              <RoleSelect id="access-user-role" value={userRole} onChange={setUserRole} />
              <Button type="submit" size="sm">Criar usuario</Button>
            </form>

            <form onSubmit={handleCreateToken} className="space-y-3 rounded-md border bg-card p-4">
              <div className="flex items-center gap-2 text-sm font-medium">
                <KeyRound className="h-4 w-4" />
                API token
              </div>
              <div className="space-y-2">
                <Label htmlFor="access-token-name">Nome</Label>
                <Input id="access-token-name" value={tokenName} onChange={(event) => setTokenName(event.target.value)} required />
              </div>
              <RoleSelect id="access-token-role" value={tokenRole} onChange={setTokenRole} />
              <Button type="submit" size="sm">Criar token</Button>
              {createdToken ? (
                <div className="flex gap-2">
                  <code className="min-w-0 flex-1 overflow-auto rounded bg-muted px-3 py-2 text-xs">
                    {createdToken}
                  </code>
                  <Button type="button" variant="outline" size="sm" onClick={handleCopyToken}>
                    Copiar
                  </Button>
                </div>
              ) : null}
            </form>
          </div>
        ) : null}

        <section className="grid gap-4 lg:grid-cols-2">
          <Panel title="Usuarios" icon={<ShieldCheck className="h-4 w-4" />}>
            {loading ? <p className="text-sm text-muted-foreground">Carregando...</p> : null}
            {users.map((user) => (
              <AccessRow
                key={user.id}
                label={user.username}
                role={user.role}
                active={user.active}
                actions={canManage ? (
                  <>
                    <Button type="button" variant="outline" size="sm" onClick={() => handleToggleUser(user)}>
                      {user.active ? "Desativar" : "Ativar"}
                    </Button>
                    <Button type="button" variant="ghost" size="sm" onClick={() => handleDeleteUser(user)}>
                      Remover
                    </Button>
                  </>
                ) : null}
              />
            ))}
          </Panel>
          <Panel title="Tokens" icon={<KeyRound className="h-4 w-4" />}>
            {loading ? <p className="text-sm text-muted-foreground">Carregando...</p> : null}
            {tokens.map((token) => (
              <AccessRow
                key={token.id}
                label={`${token.name} - ${token.tokenPrefix}`}
                role={token.role}
                active={token.active}
                actions={canManage ? (
                  <>
                    <Button type="button" variant="outline" size="sm" onClick={() => handleToggleToken(token)}>
                      {token.active ? "Desativar" : "Ativar"}
                    </Button>
                    <Button type="button" variant="ghost" size="sm" onClick={() => handleDeleteToken(token)}>
                      Revogar
                    </Button>
                  </>
                ) : null}
              />
            ))}
          </Panel>
        </section>
      </div>
    </main>
  );
}

function RoleSelect({
  id,
  value,
  onChange,
}: {
  id: string;
  value: AccessRole;
  onChange: (role: AccessRole) => void;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>Role</Label>
      <select
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value as AccessRole)}
        className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
      >
        {ROLES.map((role) => (
          <option key={role} value={role}>
            {role}
          </option>
        ))}
      </select>
    </div>
  );
}

function Panel({ title, icon, children }: { title: string; icon: ReactNode; children: ReactNode }) {
  return (
    <div className="space-y-3 rounded-md border bg-card p-4">
      <div className="flex items-center gap-2 text-sm font-medium">
        {icon}
        {title}
      </div>
      <div className="space-y-2">{children}</div>
    </div>
  );
}

function AccessRow({
  label,
  role,
  active,
  actions,
}: {
  label: string;
  role: AccessRole;
  active: boolean;
  actions?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded border px-3 py-2 text-sm">
      <span className="min-w-0 truncate">{label}</span>
      <span className="flex shrink-0 items-center gap-2">
        <Badge variant="secondary">{role}</Badge>
        <Badge variant={active ? "default" : "outline"}>{active ? "ativo" : "inativo"}</Badge>
        {actions}
      </span>
    </div>
  );
}
