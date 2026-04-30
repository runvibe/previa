import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Activity, Loader2, Plus, RefreshCw, Server, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { useAppHeader } from "@/components/AppShell";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ProjectSettingsDialog } from "@/components/ProjectSettingsDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from "@/components/ui/table";
import {
  createRunner, deleteRunner, listRunners, updateRunner, type RunnerRecord,
} from "@/lib/api-client";
import { cn } from "@/lib/utils";
import { getApiUrl, useOrchestratorStore } from "@/stores/useOrchestratorStore";

function formatBytes(value?: number | null): string {
  if (!value || value <= 0) return "-";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function formatDate(value?: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function healthClass(status: string) {
  if (status === "healthy") return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
  if (status === "unhealthy") return "border-destructive/30 bg-destructive/10 text-destructive";
  return "border-muted-foreground/30 bg-muted text-muted-foreground";
}

export default function RunnersPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const activeContext = useOrchestratorStore((state) => state.activeContext);
  const activeContextId = useOrchestratorStore((state) => state.activeContextId);
  const fetchInfo = useOrchestratorStore((state) => state.fetchInfo);
  const [runners, setRunners] = useState<RunnerRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [endpoint, setEndpoint] = useState("");
  const [name, setName] = useState("");
  const [editingNames, setEditingNames] = useState<Record<string, string>>({});
  const [runnerToDelete, setRunnerToDelete] = useState<RunnerRecord | null>(null);
  const headerActions = useMemo(() => <ProjectSettingsDialog />, []);

  const headerConfig = useMemo(() => ({
    headerActions,
    onBackToProjects: () => navigate("/"),
  }), [headerActions, navigate]);
  useAppHeader(headerConfig);

  const apiUrl = getApiUrl();
  const enabledCount = runners.filter((runner) => runner.enabled).length;
  const healthyCount = runners.filter((runner) => runner.healthStatus === "healthy").length;

  const load = useCallback(async () => {
    const currentApiUrl = getApiUrl();
    if (!currentApiUrl) {
      setRunners([]);
      return;
    }

    setLoading(true);
    try {
      setRunners(await listRunners(currentApiUrl));
      await fetchInfo();
    } catch {
      toast.error(t("runners.loadError"));
    } finally {
      setLoading(false);
    }
  }, [fetchInfo, t]);

  useEffect(() => {
    void load();
  }, [activeContextId, load]);

  useEffect(() => {
    setEditingNames(Object.fromEntries(runners.map((runner) => [runner.id, runner.name ?? ""])));
  }, [runners]);

  const runnerSummary = useMemo(() => [
    { label: t("runners.summary.total"), value: runners.length },
    { label: t("runners.summary.enabled"), value: enabledCount },
    { label: t("runners.summary.healthy"), value: healthyCount },
  ], [enabledCount, healthyCount, runners.length, t]);

  const handleAddRunner = async () => {
    const currentApiUrl = getApiUrl();
    if (!currentApiUrl) {
      toast.error(t("runners.noBackend"));
      return;
    }
    const trimmedEndpoint = endpoint.trim();
    if (!trimmedEndpoint) return;

    setSaving(true);
    try {
      const runner = await createRunner(currentApiUrl, {
        endpoint: trimmedEndpoint,
        name: name.trim() || null,
        enabled: true,
      });
      setRunners((current) => {
        const others = current.filter((item) => item.id !== runner.id);
        return [...others, runner].sort((a, b) => a.endpoint.localeCompare(b.endpoint));
      });
      setEndpoint("");
      setName("");
      toast.success(t("runners.addSuccess"));
      await fetchInfo();
    } catch {
      toast.error(t("runners.addError"));
    } finally {
      setSaving(false);
    }
  };

  const handleUpdateRunner = async (runner: RunnerRecord, payload: { name?: string | null; enabled?: boolean }) => {
    const currentApiUrl = getApiUrl();
    if (!currentApiUrl) return;

    setSaving(true);
    try {
      const updated = await updateRunner(currentApiUrl, runner.id, payload);
      setRunners((current) => current.map((item) => (item.id === updated.id ? updated : item)));
      toast.success(t("runners.updateSuccess"));
      await fetchInfo();
    } catch {
      toast.error(t("runners.updateError"));
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteRunner = async () => {
    const currentApiUrl = getApiUrl();
    if (!currentApiUrl || !runnerToDelete) return;

    setSaving(true);
    try {
      await deleteRunner(currentApiUrl, runnerToDelete.id);
      setRunners((current) => current.filter((runner) => runner.id !== runnerToDelete.id));
      setRunnerToDelete(null);
      toast.success(t("runners.deleteSuccess"));
      await fetchInfo();
    } catch {
      toast.error(t("runners.deleteError"));
    } finally {
      setSaving(false);
    }
  };

  if (!apiUrl) {
    return (
      <main className="flex-1 p-4 sm:p-6">
        <div className="mx-auto max-w-6xl">
          <Card>
            <CardHeader>
              <CardTitle>{t("runners.title")}</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-sm text-muted-foreground">{t("runners.noBackend")}</p>
            </CardContent>
          </Card>
        </div>
      </main>
    );
  }

  return (
    <main className="flex-1 overflow-auto p-4 sm:p-6">
      <div className="mx-auto max-w-6xl space-y-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <h2 className="text-xl font-bold sm:text-2xl">{t("runners.title")}</h2>
            <p className="text-sm text-muted-foreground">{t("runners.subtitle", { context: activeContext?.name ?? "-" })}</p>
          </div>
          <Button variant="outline" onClick={load} disabled={loading}>
            {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
            {t("runners.refresh")}
          </Button>
        </div>

        <div className="grid gap-3 sm:grid-cols-3">
          {runnerSummary.map((item) => (
            <Card key={item.label}>
              <CardContent className="flex items-center justify-between p-4">
                <span className="text-sm text-muted-foreground">{item.label}</span>
                <span className="text-2xl font-semibold">{item.value}</span>
              </CardContent>
            </Card>
          ))}
        </div>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t("runners.addTitle")}</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid gap-3 md:grid-cols-[1.4fr_1fr_auto] md:items-end">
              <div className="space-y-1.5">
                <Label htmlFor="runner-endpoint">{t("runners.endpoint")}</Label>
                <Input
                  id="runner-endpoint"
                  placeholder="http://127.0.0.1:55880"
                  value={endpoint}
                  onChange={(event) => setEndpoint(event.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="runner-name">{t("runners.name")}</Label>
                <Input
                  id="runner-name"
                  placeholder={t("runners.namePlaceholder")}
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </div>
              <Button onClick={handleAddRunner} disabled={saving || !endpoint.trim()}>
                <Plus className="h-4 w-4" />
                {t("runners.add")}
              </Button>
            </div>
          </CardContent>
        </Card>

        {loading ? (
          <div className="flex items-center justify-center py-16 text-muted-foreground">
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("common.loading")}
          </div>
        ) : runners.length === 0 ? (
          <Card>
            <CardContent className="flex flex-col items-center justify-center py-16 text-center">
              <Server className="mb-4 h-9 w-9 text-muted-foreground" />
              <h3 className="font-semibold">{t("runners.empty.title")}</h3>
              <p className="mt-1 max-w-sm text-sm text-muted-foreground">{t("runners.empty.description")}</p>
            </CardContent>
          </Card>
        ) : (
          <div className="overflow-x-auto rounded-lg border border-border/70">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("runners.name")}</TableHead>
                  <TableHead>{t("runners.endpoint")}</TableHead>
                  <TableHead>{t("runners.status")}</TableHead>
                  <TableHead>{t("runners.runtime")}</TableHead>
                  <TableHead>{t("runners.lastSeen")}</TableHead>
                  <TableHead className="text-right">{t("runners.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {runners.map((runner) => (
                  <TableRow key={runner.id}>
                    <TableCell className="min-w-52">
                      <div className="space-y-2">
                        <Input
                          aria-label={t("runners.nameFor", { endpoint: runner.endpoint })}
                          value={editingNames[runner.id] ?? ""}
                          onChange={(event) => setEditingNames((current) => ({ ...current, [runner.id]: event.target.value }))}
                        />
                      </div>
                    </TableCell>
                    <TableCell className="min-w-64">
                      <div className="font-mono text-xs">{runner.endpoint}</div>
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline" className={cn("capitalize", healthClass(runner.healthStatus))}>
                        <Activity className="h-3 w-3" />
                        {runner.healthStatus}
                      </Badge>
                    </TableCell>
                    <TableCell className="min-w-44 text-xs text-muted-foreground">
                      {runner.runtime ? (
                        <div className="space-y-0.5">
                          <div>PID {runner.runtime.pid}</div>
                          <div>{formatBytes(runner.runtime.memoryBytes)}</div>
                          <div>{runner.runtime.cpuUsagePercent.toFixed(1)}% CPU</div>
                        </div>
                      ) : "-"}
                    </TableCell>
                    <TableCell className="min-w-40 text-xs text-muted-foreground">
                      {formatDate(runner.lastSeenAt)}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-2">
                        <Switch
                          aria-label={runner.enabled ? t("runners.disableRunner") : t("runners.enableRunner")}
                          checked={runner.enabled}
                          disabled={saving}
                          onCheckedChange={(enabled) => handleUpdateRunner(runner, { enabled })}
                        />
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={saving || (editingNames[runner.id] ?? "") === (runner.name ?? "")}
                          onClick={() => handleUpdateRunner(runner, { name: editingNames[runner.id]?.trim() || null })}
                        >
                          {t("common.save")}
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-9 w-9 text-destructive"
                          disabled={saving}
                          onClick={() => setRunnerToDelete(runner)}
                          aria-label={t("runners.deleteRunner", { endpoint: runner.endpoint })}
                          title={t("common.delete")}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>

      <ConfirmDialog
        open={!!runnerToDelete}
        onOpenChange={(open) => { if (!open) setRunnerToDelete(null); }}
        title={t("runners.deleteConfirm.title")}
        description={t("runners.deleteConfirm.description", { endpoint: runnerToDelete?.endpoint })}
        confirmLabel={t("common.delete")}
        variant="destructive"
        onConfirm={handleDeleteRunner}
      />
    </main>
  );
}
