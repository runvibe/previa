import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Check,
  CircleHelp,
  Clipboard,
  KeyRound,
  Loader2,
  Plus,
  ShieldCheck,
  Trash2,
  UserPlus,
} from "lucide-react";
import { toast } from "sonner";

import { useAppHeader } from "@/components/AppShell";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ProjectSettingsDialog } from "@/components/ProjectSettingsDialog";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

const ROLES: AccessRole[] = ["admin", "editor", "operator", "viewer"];

function formatDate(value?: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function roleClass(role: AccessRole) {
  if (role === "root" || role === "admin") return "border-primary/30 bg-primary/10 text-primary";
  if (role === "editor") return "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300";
  if (role === "operator") return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300";
  if (role === "viewer") return "border-muted-foreground/30 bg-muted text-muted-foreground";
  return "border-muted-foreground/30 bg-muted text-muted-foreground";
}

function statusClass(active: boolean) {
  return active
    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
    : "border-muted-foreground/30 bg-muted text-muted-foreground";
}

export default function AccessManagementPage() {
  const navigate = useNavigate();
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
  const [saving, setSaving] = useState(false);
  const [userToDelete, setUserToDelete] = useState<ManagedUser | null>(null);
  const [tokenToDelete, setTokenToDelete] = useState<ApiTokenRecord | null>(null);
  const [createDialog, setCreateDialog] = useState<"user" | "token" | null>(null);
  const headerActions = useMemo(() => <ProjectSettingsDialog />, []);

  const headerConfig = useMemo(() => ({
    headerActions,
    onBackToProjects: () => navigate("/"),
  }), [headerActions, navigate]);
  useAppHeader(headerConfig);

  const canManage = useMemo(
    () => currentUser?.role === "root" || currentUser?.role === "admin",
    [currentUser?.role],
  );

  const refresh = useCallback(async () => {
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
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const accessSummary = useMemo(() => [
    { label: "Usuarios", value: users.length },
    { label: "Usuarios ativos", value: users.filter((user) => user.active).length },
    { label: "Tokens ativos", value: tokens.filter((token) => token.active).length },
  ], [tokens, users]);

  async function handleCreateUser(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    setSaving(true);
    try {
      const user = await createUser({ username, password, role: userRole, active: true });
      setUsers((items) => [user, ...items]);
      setUsername("");
      setPassword("");
      setCreateDialog(null);
      toast.success("Usuario criado");
    } catch {
      setError("Nao foi possivel criar o usuario");
      toast.error("Nao foi possivel criar o usuario");
    } finally {
      setSaving(false);
    }
  }

  async function handleCreateToken(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    setSaving(true);
    try {
      const result = await createApiToken({ name: tokenName, role: tokenRole });
      setTokens((items) => [result.record, ...items]);
      setCreatedToken(result.token);
      setTokenName("");
      toast.success("Token criado");
    } catch {
      setError("Nao foi possivel criar o token");
      toast.error("Nao foi possivel criar o token");
    } finally {
      setSaving(false);
    }
  }

  async function handleToggleUser(user: ManagedUser) {
    setError(null);
    setSaving(true);
    try {
      const updated = await updateUser(user.id, { active: !user.active });
      setUsers((items) => items.map((item) => (item.id === updated.id ? updated : item)));
      toast.success(updated.active ? "Usuario ativado" : "Usuario desativado");
    } catch {
      setError("Nao foi possivel atualizar o usuario");
      toast.error("Nao foi possivel atualizar o usuario");
    } finally {
      setSaving(false);
    }
  }

  async function handleDeleteUser() {
    if (!userToDelete) return;
    setError(null);
    setSaving(true);
    try {
      await deleteUser(userToDelete.id);
      setUsers((items) => items.filter((item) => item.id !== userToDelete.id));
      setUserToDelete(null);
      toast.success("Usuario removido");
    } catch {
      setError("Nao foi possivel remover o usuario");
      toast.error("Nao foi possivel remover o usuario");
    } finally {
      setSaving(false);
    }
  }

  async function handleToggleToken(token: ApiTokenRecord) {
    setError(null);
    setSaving(true);
    try {
      const updated = await updateApiToken(token.id, !token.active);
      setTokens((items) => items.map((item) => (item.id === updated.id ? updated : item)));
      toast.success(updated.active ? "Token ativado" : "Token desativado");
    } catch {
      setError("Nao foi possivel atualizar o token");
      toast.error("Nao foi possivel atualizar o token");
    } finally {
      setSaving(false);
    }
  }

  async function handleDeleteToken() {
    if (!tokenToDelete) return;
    setError(null);
    setSaving(true);
    try {
      await deleteApiToken(tokenToDelete.id);
      setTokens((items) => items.filter((item) => item.id !== tokenToDelete.id));
      setTokenToDelete(null);
      toast.success("Token revogado");
    } catch {
      setError("Nao foi possivel revogar o token");
      toast.error("Nao foi possivel revogar o token");
    } finally {
      setSaving(false);
    }
  }

  async function handleCopyToken() {
    if (!createdToken) return;
    await navigator.clipboard?.writeText(createdToken);
    toast.success("Token copiado");
  }

  return (
    <main className="flex-1 overflow-auto p-4 sm:p-6">
      <div className="mx-auto max-w-6xl space-y-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <h2 className="text-xl font-bold sm:text-2xl">Acesso</h2>
            <p className="text-sm text-muted-foreground">
              Logado como {currentUser?.username ?? "anonymous"}
            </p>
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-3">
          {accessSummary.map((item) => (
            <Card key={item.label}>
              <CardContent className="flex items-center justify-between p-4">
                <span className="text-sm text-muted-foreground">{item.label}</span>
                <span className="text-2xl font-semibold">{item.value}</span>
              </CardContent>
            </Card>
          ))}
        </div>

        {error ? <p className="rounded border border-destructive/30 px-3 py-2 text-sm text-destructive">{error}</p> : null}

        {loading ? (
          <div className="flex items-center justify-center py-16 text-muted-foreground">
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            Carregando
          </div>
        ) : (
          <>
            <Card>
              <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="flex items-center gap-2">
                  <CardTitle className="flex items-center gap-2 text-base">
                    <ShieldCheck className="h-4 w-4" />
                    Usuarios
                  </CardTitle>
                  <AccessTypeHelp
                    label="Ajuda sobre usuarios"
                    description="Usuarios acessam o app pelo login e recebem JWT na sessao. Use para pessoas que precisam entrar na interface do Previa."
                  />
                </div>
                {canManage ? (
                  <Button size="sm" onClick={() => setCreateDialog("user")}>
                    <Plus className="h-4 w-4" />
                    Novo usuario
                  </Button>
                ) : null}
              </CardHeader>
              <CardContent className="p-0">
                {users.length === 0 ? (
                  <div className="flex flex-col items-center justify-center px-6 py-14 text-center">
                    <UserPlus className="mb-4 h-9 w-9 text-muted-foreground" />
                    <h3 className="font-semibold">Nenhum usuario criado</h3>
                    <p className="mt-1 max-w-sm text-sm text-muted-foreground">Crie usuarios para controlar acesso ao app.</p>
                  </div>
                ) : (
                  <div className="overflow-x-auto">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead className="min-w-56">Login</TableHead>
                          <TableHead className="w-36 text-center">Role</TableHead>
                          <TableHead className="w-36 text-center">Status</TableHead>
                          <TableHead className="min-w-44">Criado em</TableHead>
                          <TableHead className="w-32 text-center">Acoes</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {users.map((user) => (
                          <TableRow key={user.id}>
                            <TableCell className="min-w-56 font-medium">{user.username}</TableCell>
                            <TableCell className="w-36 text-center">
                              <Badge variant="outline" className={cn("capitalize", roleClass(user.role))}>
                                {user.role}
                              </Badge>
                            </TableCell>
                            <TableCell className="w-36 text-center">
                              <Badge variant="outline" className={cn("capitalize", statusClass(user.active))}>
                                {user.active ? <Check className="h-3 w-3" /> : null}
                                {user.active ? "ativo" : "inativo"}
                              </Badge>
                            </TableCell>
                            <TableCell className="min-w-44 text-xs text-muted-foreground">
                              {formatDate(user.createdAt)}
                            </TableCell>
                            <TableCell className="w-32">
                              {canManage ? (
                                <div className="flex justify-center gap-2">
                                  <Switch
                                    aria-label={user.active ? `Desativar ${user.username}` : `Ativar ${user.username}`}
                                    checked={user.active}
                                    disabled={saving}
                                    onCheckedChange={() => { void handleToggleUser(user); }}
                                  />
                                  <Button
                                    variant="ghost"
                                    size="icon"
                                    className="h-9 w-9 text-destructive"
                                    disabled={saving}
                                    onClick={() => setUserToDelete(user)}
                                    aria-label={`Remover ${user.username}`}
                                    title="Remover"
                                  >
                                    <Trash2 className="h-4 w-4" />
                                  </Button>
                                </div>
                              ) : "-"}
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="flex items-center gap-2">
                  <CardTitle className="flex items-center gap-2 text-base">
                    <KeyRound className="h-4 w-4" />
                    API tokens
                  </CardTitle>
                  <AccessTypeHelp
                    label="Ajuda sobre API tokens"
                    description="API tokens sao credenciais fixas para CLI, MCP e chamadas diretas na API, sem depender de login interativo."
                  />
                </div>
                {canManage ? (
                  <Button size="sm" onClick={() => {
                    setCreatedToken(null);
                    setCreateDialog("token");
                  }}>
                    <Plus className="h-4 w-4" />
                    Novo token
                  </Button>
                ) : null}
              </CardHeader>
              <CardContent className="p-0">
                {tokens.length === 0 ? (
                  <div className="flex flex-col items-center justify-center px-6 py-14 text-center">
                    <KeyRound className="mb-4 h-9 w-9 text-muted-foreground" />
                    <h3 className="font-semibold">Nenhum token criado</h3>
                    <p className="mt-1 max-w-sm text-sm text-muted-foreground">Crie tokens fixos para CLI, MCP e integracoes diretas com a API.</p>
                  </div>
                ) : (
                  <div className="overflow-x-auto">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead className="min-w-56">Nome</TableHead>
                          <TableHead className="min-w-32">Prefixo</TableHead>
                          <TableHead className="w-36 text-center">Role</TableHead>
                          <TableHead className="w-36 text-center">Status</TableHead>
                          <TableHead className="min-w-44">Ultimo uso</TableHead>
                          <TableHead className="w-32 text-center">Acoes</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {tokens.map((token) => (
                          <TableRow key={token.id}>
                            <TableCell className="min-w-56 font-medium">{token.name}</TableCell>
                            <TableCell className="min-w-32 font-mono text-xs">{token.tokenPrefix}</TableCell>
                            <TableCell className="w-36 text-center">
                              <Badge variant="outline" className={cn("capitalize", roleClass(token.role))}>
                                {token.role}
                              </Badge>
                            </TableCell>
                            <TableCell className="w-36 text-center">
                              <Badge variant="outline" className={cn("capitalize", statusClass(token.active))}>
                                {token.active ? <Check className="h-3 w-3" /> : null}
                                {token.active ? "ativo" : "inativo"}
                              </Badge>
                            </TableCell>
                            <TableCell className="min-w-44 text-xs text-muted-foreground">
                              {formatDate(token.lastUsedAt)}
                            </TableCell>
                            <TableCell className="w-32">
                              {canManage ? (
                                <div className="flex justify-center gap-2">
                                  <Switch
                                    aria-label={token.active ? `Desativar ${token.name}` : `Ativar ${token.name}`}
                                    checked={token.active}
                                    disabled={saving}
                                    onCheckedChange={() => { void handleToggleToken(token); }}
                                  />
                                  <Button
                                    variant="ghost"
                                    size="icon"
                                    className="h-9 w-9 text-destructive"
                                    disabled={saving}
                                    onClick={() => setTokenToDelete(token)}
                                    aria-label={`Revogar ${token.name}`}
                                    title="Revogar"
                                  >
                                    <Trash2 className="h-4 w-4" />
                                  </Button>
                                </div>
                              ) : "-"}
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                )}
              </CardContent>
            </Card>
          </>
        )}
      </div>

      <Dialog open={createDialog === "user"} onOpenChange={(open) => setCreateDialog(open ? "user" : null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Novo usuario</DialogTitle>
            <DialogDescription>Crie um acesso com login, senha e role de permissao.</DialogDescription>
          </DialogHeader>
          <form id="create-access-user-form" onSubmit={handleCreateUser} className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="access-username">Login</Label>
              <Input id="access-username" value={username} onChange={(event) => setUsername(event.target.value)} required />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="access-password">Senha</Label>
              <Input id="access-password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} required />
            </div>
            <RoleSelect id="access-user-role" value={userRole} onChange={setUserRole} />
          </form>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setCreateDialog(null)}>
              Cancelar
            </Button>
            <Button
              type="submit"
              form="create-access-user-form"
              disabled={saving || !username.trim() || !password.trim()}
            >
              {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
              Criar usuario
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={createDialog === "token"} onOpenChange={(open) => setCreateDialog(open ? "token" : null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Novo API token</DialogTitle>
            <DialogDescription>Crie um token fixo para CLI, MCP ou acesso direto a API.</DialogDescription>
          </DialogHeader>
          <form id="create-access-token-form" onSubmit={handleCreateToken} className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="access-token-name">Nome</Label>
              <Input id="access-token-name" value={tokenName} onChange={(event) => setTokenName(event.target.value)} required />
            </div>
            <RoleSelect id="access-token-role" value={tokenRole} onChange={setTokenRole} />
            {createdToken ? (
              <div className="space-y-2 rounded-md border border-border/70 bg-muted/30 p-3">
                <Label>Token criado</Label>
                <div className="flex gap-2">
                  <code className="min-w-0 flex-1 overflow-auto rounded-md bg-background px-3 py-2 text-xs">
                    {createdToken}
                  </code>
                  <Button type="button" variant="outline" onClick={handleCopyToken}>
                    <Clipboard className="h-4 w-4" />
                    Copiar
                  </Button>
                </div>
              </div>
            ) : null}
          </form>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setCreateDialog(null)}>
              Fechar
            </Button>
            <Button
              type="submit"
              form="create-access-token-form"
              disabled={saving || !tokenName.trim()}
            >
              {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
              Criar token
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={!!userToDelete}
        onOpenChange={(open) => { if (!open) setUserToDelete(null); }}
        title="Remover usuario"
        description={`Remover ${userToDelete?.username ?? "este usuario"} permanentemente?`}
        confirmLabel="Remover"
        variant="destructive"
        confirmDisabled={saving}
        onConfirm={handleDeleteUser}
      />
      <ConfirmDialog
        open={!!tokenToDelete}
        onOpenChange={(open) => { if (!open) setTokenToDelete(null); }}
        title="Revogar API token"
        description={`Revogar ${tokenToDelete?.name ?? "este token"} permanentemente?`}
        confirmLabel="Revogar"
        variant="destructive"
        confirmDisabled={saving}
        onConfirm={handleDeleteToken}
      />
    </main>
  );
}

function AccessTypeHelp({ label, description }: { label: string; description: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          title={description}
          className="inline-flex h-6 w-6 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
        >
          <CircleHelp className="h-3.5 w-3.5" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right" className="max-w-xs leading-relaxed">
        {description}
      </TooltipContent>
    </Tooltip>
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
    <div className="space-y-1.5">
      <Label htmlFor={id}>Role</Label>
      <select
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value as AccessRole)}
        className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
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
