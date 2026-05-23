import { useEffect, useMemo, useState } from "react";
import { Globe2, Loader2, Trash2, UserPlus } from "lucide-react";
import { toast } from "sonner";

import * as api from "@/lib/api-client";
import { listUsers, type ManagedUser } from "@/lib/auth-client";
import type { Pipeline } from "@/types/pipeline";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

interface PipelineSharingDialogProps {
  open: boolean;
  baseUrl?: string;
  projectId: string;
  pipeline: Pipeline | null;
  onOpenChange: (open: boolean) => void;
}

export function PipelineSharingDialog({
  open,
  baseUrl,
  projectId,
  pipeline,
  onOpenChange,
}: PipelineSharingDialogProps) {
  const [sharing, setSharing] = useState<api.PipelineSharingRecord | null>(null);
  const [users, setUsers] = useState<ManagedUser[]>([]);
  const [selectedUserId, setSelectedUserId] = useState("");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  const pipelineId = pipeline?.id;

  useEffect(() => {
    if (!open || !baseUrl || !pipelineId) return;
    let cancelled = false;
    setLoading(true);
    Promise.all([
      api.getPipelineSharing(baseUrl, projectId, pipelineId),
      listUsers().catch(() => [] as ManagedUser[]),
    ])
      .then(([nextSharing, nextUsers]) => {
        if (cancelled) return;
        setSharing(nextSharing);
        setUsers(nextUsers.filter((user) => user.active));
      })
      .catch(() => toast.error("Nao foi possivel carregar compartilhamento"))
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [baseUrl, open, pipelineId, projectId]);

  const shareableUsers = useMemo(() => {
    const shared = new Set(sharing?.shares.map((share) => share.userId) ?? []);
    return users.filter((user) => user.id !== sharing?.ownerUserId && !shared.has(user.id));
  }, [sharing, users]);

  async function handleTogglePublic(checked: boolean) {
    if (!baseUrl || !pipelineId) return;
    setSaving(true);
    try {
      const next = await api.updatePipelineVisibility(
        baseUrl,
        projectId,
        pipelineId,
        checked ? "public" : "private",
      );
      setSharing(next);
      toast.success(checked ? "Pipeline publica" : "Pipeline privada");
    } catch {
      toast.error("Nao foi possivel alterar a visibilidade");
    } finally {
      setSaving(false);
    }
  }

  async function handleShare() {
    if (!baseUrl || !pipelineId || !selectedUserId) return;
    const user = users.find((item) => item.id === selectedUserId);
    if (!user) return;
    setSaving(true);
    try {
      const next = await api.sharePipelineWithUser(baseUrl, projectId, pipelineId, {
        userId: user.id,
        username: user.username,
      });
      setSharing(next);
      setSelectedUserId("");
      toast.success("Acesso compartilhado");
    } catch {
      toast.error("Nao foi possivel compartilhar");
    } finally {
      setSaving(false);
    }
  }

  async function handleRevoke(userId: string) {
    if (!baseUrl || !pipelineId) return;
    setSaving(true);
    try {
      await api.revokePipelineShare(baseUrl, projectId, pipelineId, userId);
      setSharing((current) => current
        ? { ...current, shares: current.shares.filter((share) => share.userId !== userId) }
        : current);
      toast.success("Acesso revogado");
    } catch {
      toast.error("Nao foi possivel revogar acesso");
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Compartilhar pipeline</DialogTitle>
          <DialogDescription>{pipeline?.name ?? "Pipeline"}</DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="flex h-32 items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : (
          <div className="space-y-5">
            <div className="flex items-center justify-between rounded-md border p-3">
              <div className="flex items-center gap-2">
                <Globe2 className="h-4 w-4 text-muted-foreground" />
                <div>
                  <Label>Publica</Label>
                  <p className="text-xs text-muted-foreground">
                    Qualquer pessoa pode ver, editar e executar.
                  </p>
                </div>
              </div>
              <Switch
                checked={sharing?.visibility === "public"}
                disabled={saving || !sharing}
                onCheckedChange={handleTogglePublic}
              />
            </div>

            <div className="space-y-2">
              <Label>Dono</Label>
              <div className="rounded-md border px-3 py-2 text-sm">
                {sharing?.ownerUsername ?? "-"}
              </div>
            </div>

            <div className="space-y-2">
              <Label>Usuarios</Label>
              <div className="flex gap-2">
                <select
                  className="h-9 flex-1 rounded-md border bg-background px-3 text-sm"
                  value={selectedUserId}
                  disabled={saving}
                  onChange={(event) => setSelectedUserId(event.target.value)}
                >
                  <option value="">Selecionar usuario</option>
                  {shareableUsers.map((user) => (
                    <option key={user.id} value={user.id}>{user.username}</option>
                  ))}
                </select>
                <Button type="button" size="icon" disabled={!selectedUserId || saving} onClick={handleShare}>
                  <UserPlus className="h-4 w-4" />
                </Button>
              </div>

              <div className="space-y-1">
                {sharing?.shares.map((share) => (
                  <div key={share.userId} className="flex items-center justify-between rounded-md border px-3 py-2 text-sm">
                    <span>{share.username}</span>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7"
                      disabled={saving}
                      onClick={() => handleRevoke(share.userId)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
                {sharing && sharing.shares.length === 0 && (
                  <p className="text-xs text-muted-foreground">Nenhum usuario compartilhado.</p>
                )}
              </div>
            </div>
          </div>
        )}

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Fechar</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
