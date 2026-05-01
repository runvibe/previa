import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";
import {
  ChevronDown,
  Plus,
  Trash2,
  Server,
  Loader2,
  CheckCircle2,
  X,
  Radio,
  Sparkles,
  ShieldAlert,
  RefreshCw,
} from "lucide-react";
import { toast } from "sonner";

const LOCAL_ORCHESTRATOR_URL = "http://localhost:5588";
const LOCAL_PERMISSION_ERROR = "local_permission_blocked";

interface DetectedPreviaApi {
  url: string;
  name: string;
  info: {
    context: string;
    totalRunners: number;
    activeRunners: number;
  };
}

function getFetchErrorType(error: unknown) {
  if (error instanceof DOMException && error.name === "TimeoutError") {
    return "timeout";
  }

  if (error instanceof TypeError) {
    return LOCAL_PERMISSION_ERROR;
  }

  return "unknown";
}

function normalizeContextUrl(url: string) {
  return url.replace(/\/api\/v1\/?$/, "").replace(/\/+$/, "");
}

function parsePreviaInfo(payload: unknown): DetectedPreviaApi["info"] | null {
  if (!payload || typeof payload !== "object") return null;

  const data = payload as Record<string, unknown>;
  if (typeof data.context !== "string" || !data.context.trim()) return null;
  if (typeof data.totalRunners !== "number") return null;
  if (typeof data.activeRunners !== "number") return null;

  return {
    context: data.context,
    totalRunners: data.totalRunners,
    activeRunners: data.activeRunners,
  };
}

async function detectPreviaApi(baseUrl: string, timeoutMs = 2500): Promise<DetectedPreviaApi | null> {
  const url = normalizeContextUrl(baseUrl);
  const healthRes = await fetch(`${url}/health`, {
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!healthRes.ok) return null;

  const infoRes = await fetch(`${url}/info`, {
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!infoRes.ok) return null;

  const info = parsePreviaInfo(await infoRes.json());
  if (!info) return null;

  return {
    url,
    name: info.context,
    info,
  };
}

export function ContextSwitcher() {
  const { t } = useTranslation();
  const contexts = useOrchestratorStore((s) => s.contexts);
  const activeContext = useOrchestratorStore((s) => s.activeContext);
  const info = useOrchestratorStore((s) => s.info);
  const { addContext, removeContext, switchContext, updateContext, setInfo } = useOrchestratorStore();

  const [open, setOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  const [newUrl, setNewUrl] = useState("");
  const [testStatus, setTestStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [detectedLocalContext, setDetectedLocalContext] = useState<{ url: string; name?: string } | null>(null);
  const [isCheckingLocalContext, setIsCheckingLocalContext] = useState(false);
  const [isActivatingLocalContext, setIsActivatingLocalContext] = useState(false);
  const [dismissedLocalPrompt, setDismissedLocalPrompt] = useState(false);
  const [permissionBlockedUrl, setPermissionBlockedUrl] = useState<string | null>(null);
  const [isRetryingPermission, setIsRetryingPermission] = useState(false);
  const [localPromptPosition, setLocalPromptPosition] = useState({ top: 56, right: 16 });
  const switcherRef = useRef<HTMLDivElement>(null);
  const hasCheckedStartupContextsRef = useRef(false);

  const hasLocalContext = useMemo(
    () => contexts.some((ctx) => normalizeContextUrl(ctx.url) === LOCAL_ORCHESTRATOR_URL),
    [contexts],
  );
  const showDetectedLocalPrompt = !!detectedLocalContext && !hasLocalContext && !open;

  useEffect(() => {
    if (!showDetectedLocalPrompt) return;

    const updatePromptPosition = () => {
      const rect = switcherRef.current?.getBoundingClientRect();
      if (!rect) return;

      setLocalPromptPosition({
        top: Math.round(rect.bottom + 12),
        right: Math.max(12, Math.round(window.innerWidth - rect.right)),
      });
    };

    updatePromptPosition();
    window.addEventListener("resize", updatePromptPosition);
    window.addEventListener("scroll", updatePromptPosition, true);

    return () => {
      window.removeEventListener("resize", updatePromptPosition);
      window.removeEventListener("scroll", updatePromptPosition, true);
    };
  }, [showDetectedLocalPrompt]);

  const buildUniqueContextName = (baseName: string, existingContexts = contexts) => {
    const existingNames = existingContexts.map((c) => c.name);
    if (!existingNames.includes(baseName)) return baseName;

    let suffix = 2;
    while (existingNames.includes(`${baseName} (${suffix})`)) suffix++;
    return `${baseName} (${suffix})`;
  };

  const resolveContextName = async (baseUrl: string) => {
    try {
      const base = normalizeContextUrl(baseUrl);
      const infoRes = await fetch(`${base}/info`, { signal: AbortSignal.timeout(4000) });
      if (infoRes.ok) {
        const infoData = await infoRes.json();
        if (infoData.context) return String(infoData.context);
      }
    } catch {
      // ignore name resolution errors
    }

    return baseUrl;
  };

  const activateDetectedApi = (detected: DetectedPreviaApi) => {
    const store = useOrchestratorStore.getState();
    const existing = store.contexts.find((ctx) => normalizeContextUrl(ctx.url) === detected.url);
    if (existing) {
      if (existing.name !== detected.name) {
        updateContext(existing.id, {
          name: buildUniqueContextName(
            detected.name,
            store.contexts.filter((ctx) => ctx.id !== existing.id),
          ),
        });
      }
      switchContext(existing.id);
      setInfo(detected.info);
      return;
    }

    const ctx = addContext(buildUniqueContextName(detected.name, store.contexts), detected.url);
    if (store.contexts.length > 0) {
      switchContext(ctx.id);
    }
    setInfo(detected.info);
  };

  const showPermissionBlockedFeedback = (url: string) => {
    setPermissionBlockedUrl(url);
    toast.error("O Chrome bloqueou o acesso ao app local. Permita o localhost e tente novamente.");
  };

  const clearPermissionBlocked = () => {
    setPermissionBlockedUrl(null);
  };

  useEffect(() => {
    if (hasCheckedStartupContextsRef.current || permissionBlockedUrl) {
      return;
    }
    hasCheckedStartupContextsRef.current = true;

    let cancelled = false;

    const checkStartupContexts = async () => {
      setIsCheckingLocalContext(true);
      try {
        const currentOrigin = normalizeContextUrl(window.location.origin);
        try {
          const sameOriginApi = await detectPreviaApi(currentOrigin);
          if (cancelled) return;
          if (sameOriginApi) activateDetectedApi(sameOriginApi);
        } catch {
          // Keep the fallback localhost probe even when the current origin is not a Previa API.
        }

        const store = useOrchestratorStore.getState();
        const localAlreadySaved = store.contexts.some(
          (ctx) => normalizeContextUrl(ctx.url) === LOCAL_ORCHESTRATOR_URL,
        );
        if (currentOrigin !== LOCAL_ORCHESTRATOR_URL && !localAlreadySaved && !dismissedLocalPrompt) {
          try {
            const localApi = await detectPreviaApi(LOCAL_ORCHESTRATOR_URL);
            if (cancelled) return;
            if (localApi) {
              setDetectedLocalContext({ url: localApi.url, name: localApi.name });
              clearPermissionBlocked();
            }
          } catch (error) {
            if (!cancelled && getFetchErrorType(error) === LOCAL_PERMISSION_ERROR) {
              showPermissionBlockedFeedback(LOCAL_ORCHESTRATOR_URL);
            }
          }
        }
      } finally {
        if (!cancelled) {
          setIsCheckingLocalContext(false);
        }
      }
    };

    void checkStartupContexts();

    return () => {
      cancelled = true;
    };
  }, [dismissedLocalPrompt, permissionBlockedUrl]);

  const handleTestAndAdd = async () => {
    const trimmedUrl = newUrl.trim().replace(/\/+$/, "");
    if (!trimmedUrl) return;

    setTestStatus("loading");
    clearPermissionBlocked();
    try {
      const apiBase = trimmedUrl.endsWith("/api/v1") ? trimmedUrl : `${trimmedUrl}/api/v1`;
      const res = await fetch(`${apiBase}/projects`, { method: "GET", signal: AbortSignal.timeout(8000) });
      if (!res.ok) {
        setTestStatus("error");
        toast.error(t("settings.backend.connectError", { status: res.status }));
        return;
      }

      const autoName = buildUniqueContextName(await resolveContextName(trimmedUrl));
      const ctx = addContext(autoName, trimmedUrl);
      switchContext(ctx.id);
      setTestStatus("success");
      toast.success(t("settings.backend.connectSuccess"));
      resetForm();
    } catch (error) {
      setTestStatus("error");
      const errorType = getFetchErrorType(error);
      if (errorType === LOCAL_PERMISSION_ERROR) {
        showPermissionBlockedFeedback(trimmedUrl);
        return;
      }
      toast.error(t("settings.backend.connectError", { status: errorType === "timeout" ? "timeout" : "erro" }));
    }
  };

  const resetForm = () => {
    setAdding(false);
    setNewUrl("");
    setTestStatus("idle");
  };

  const handleRemove = (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    removeContext(id);
  };

  const handleActivateDetectedContext = async () => {
    if (!detectedLocalContext) return;

    setIsActivatingLocalContext(true);
    try {
      const existing = contexts.find((ctx) => normalizeContextUrl(ctx.url) === detectedLocalContext.url);
      if (existing) {
        const resolvedName = detectedLocalContext.name ?? await resolveContextName(detectedLocalContext.url);
        if (resolvedName && existing.name === existing.url) {
          updateContext(existing.id, { name: buildUniqueContextName(resolvedName) });
        }
        switchContext(existing.id);
      } else {
        const resolvedName = buildUniqueContextName(detectedLocalContext.name ?? await resolveContextName(detectedLocalContext.url));
        const ctx = addContext(resolvedName, detectedLocalContext.url);
        switchContext(ctx.id);
      }

      clearPermissionBlocked();
      setDetectedLocalContext(null);
      setDismissedLocalPrompt(true);
      toast.success("Contexto local ativado com sucesso.");
    } catch {
      toast.error("Não foi possível ativar o contexto local.");
    } finally {
      setIsActivatingLocalContext(false);
    }
  };

  const handleRetryLocalPermission = async () => {
    if (!permissionBlockedUrl) return;

    setIsRetryingPermission(true);
    try {
      const detected = await detectPreviaApi(permissionBlockedUrl);
      if (!detected) {
        toast.error("Ainda não foi possível acessar o app local.");
        return;
      }

      clearPermissionBlocked();
      setDismissedLocalPrompt(false);
      setDetectedLocalContext({ url: detected.url, name: detected.name });
      toast.success("Acesso local liberado. Você já pode ativar o app local.");
    } catch (error) {
      if (getFetchErrorType(error) === LOCAL_PERMISSION_ERROR) {
        showPermissionBlockedFeedback(permissionBlockedUrl);
      } else {
        toast.error("Ainda não foi possível acessar o app local.");
      }
    } finally {
      setIsRetryingPermission(false);
    }
  };

  return (
    <div ref={switcherRef} className="relative">
      <Popover open={open} onOpenChange={(o) => { setOpen(o); if (!o) resetForm(); }}>
        <PopoverTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className="gap-1.5 h-8 px-2.5 max-w-[200px] font-normal"
          >
            <span
              className={`h-2 w-2 rounded-full shrink-0 ${
                activeContext && info ? "bg-success" : activeContext ? "bg-muted-foreground/40" : "bg-muted-foreground/20"
              }`}
            />
            <span className="truncate text-xs">
              {activeContext ? (info?.context || activeContext.name) : t("settings.backend.noContext", "No context")}
            </span>
            <ChevronDown className="h-3 w-3 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent 
          align="end" 
          className="w-80 p-0 border-border"
          style={{ backgroundColor: "hsl(var(--card))" }}
        >
          <div className="p-3 border-b border-border">
            <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
              {t("settings.backend.contexts", "Contexts")}
            </p>
          </div>

          {permissionBlockedUrl && (
            <div className="border-b border-border bg-warning/10 px-3 py-3">
              <div className="flex items-start gap-3 rounded-lg border border-warning/20 bg-background/80 p-3">
                <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-warning/15 text-warning">
                  <ShieldAlert className="h-4 w-4" />
                </div>
                <div className="min-w-0 flex-1 space-y-2">
                  <div className="space-y-1">
                    <p className="text-sm font-semibold text-foreground">Permita o acesso ao app local</p>
                    <p className="text-xs leading-5 text-muted-foreground">
                      Se o Chrome mostrar o aviso para abrir apps localmente, clique em <strong>Permitir</strong>. Se você bloqueou, libere o acesso ao <span className="font-mono text-foreground">localhost</span> nas permissões do navegador e tente novamente.
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      size="sm"
                      className="h-8 gap-1.5"
                      onClick={handleRetryLocalPermission}
                      disabled={isRetryingPermission}
                    >
                      {isRetryingPermission ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                      Tentar novamente
                    </Button>
                    <span className="truncate text-[11px] font-mono text-muted-foreground">{permissionBlockedUrl}</span>
                  </div>
                </div>
              </div>
            </div>
          )}

          <div className="max-h-[240px] overflow-y-auto">
            {contexts.length === 0 && !adding && (
              <div className="px-3 py-6 text-center text-xs text-muted-foreground">
                {t("settings.backend.noContexts", "No contexts configured")}
              </div>
            )}
            {contexts.map((ctx) => {
              const isActive = ctx.id === activeContext?.id;
              return (
                <button
                  key={ctx.id}
                  onClick={() => { switchContext(ctx.id); setOpen(false); }}
                  className={`w-full flex items-center gap-2.5 px-3 py-2.5 text-left hover:bg-accent/50 transition-colors group ${
                    isActive ? "bg-accent/30" : ""
                  }`}
                >
                  <Radio className={`h-3.5 w-3.5 shrink-0 ${isActive ? "text-primary" : "text-muted-foreground/40"}`} />
                  <div className="flex-1 min-w-0">
                    <p className="text-sm truncate">{ctx.name}</p>
                    <p className="text-[10px] text-muted-foreground font-mono truncate">{ctx.url}</p>
                  </div>
                  <button
                    onClick={(e) => handleRemove(ctx.id, e)}
                    className="opacity-0 group-hover:opacity-100 transition-opacity p-1 hover:bg-destructive/10 rounded"
                  >
                    <Trash2 className="h-3 w-3 text-destructive" />
                  </button>
                </button>
              );
            })}
          </div>

          <div className="border-t border-border">
            {adding ? (
              <div className="p-3 space-y-2">
                <Input
                  placeholder="http://localhost:5588"
                  value={newUrl}
                  onChange={(e) => { setNewUrl(e.target.value); setTestStatus("idle"); clearPermissionBlocked(); }}
                  className="h-8 text-xs font-mono"
                />
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 text-xs flex-1"
                    onClick={resetForm}
                  >
                    {t("common.cancel")}
                  </Button>
                  <Button
                    size="sm"
                    className="h-7 text-xs flex-1 gap-1.5"
                    disabled={!newUrl.trim() || testStatus === "loading"}
                    onClick={handleTestAndAdd}
                  >
                    {testStatus === "loading" ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : testStatus === "success" ? (
                      <CheckCircle2 className="h-3 w-3" />
                    ) : testStatus === "error" ? (
                      <X className="h-3 w-3" />
                    ) : (
                      <Server className="h-3 w-3" />
                    )}
                    {t("settings.backend.connect")}
                  </Button>
                </div>
              </div>
            ) : (
              <button
                onClick={() => setAdding(true)}
                className="w-full flex items-center gap-2 px-3 py-2.5 text-xs text-muted-foreground hover:text-foreground hover:bg-accent/50 transition-colors"
              >
                <Plus className="h-3.5 w-3.5" />
                {t("settings.backend.addContext", "Add context")}
              </button>
            )}
          </div>
        </PopoverContent>
      </Popover>

      {showDetectedLocalPrompt && detectedLocalContext && createPortal(
        <div
          className="fixed z-[9999] w-[320px] max-w-[calc(100vw-24px)] rounded-xl border border-border p-4 shadow-lg"
          style={{
            backgroundColor: "hsl(var(--card))",
            top: localPromptPosition.top,
            right: localPromptPosition.right,
          }}
        >
          <div
            className="absolute -top-2 right-6 h-4 w-4 rotate-45 border-l border-t border-border"
            style={{ backgroundColor: "hsl(var(--card))" }}
          />
          <div className="flex items-start gap-3">
            <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
              <Sparkles className="h-4 w-4" />
            </div>
            <div className="min-w-0 flex-1 space-y-3">
              <div className="space-y-1">
                <p className="text-sm font-semibold text-foreground">Contexto local encontrado</p>
                <p className="text-sm text-muted-foreground">
                  Encontramos <span className="font-mono text-foreground">{detectedLocalContext.url}</span>. Deseja ativá-lo no menu de contexts?
                </p>
              </div>
              <div className="flex gap-2">
                <Button
                  type="button"
                  size="sm"
                  className="gap-1.5"
                  disabled={isActivatingLocalContext}
                  onClick={handleActivateDetectedContext}
                >
                  {isActivatingLocalContext ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Sparkles className="h-3.5 w-3.5" />}
                  Ativar
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setDismissedLocalPrompt(true);
                    setDetectedLocalContext(null);
                  }}
                >
                  Agora não
                </Button>
              </div>
            </div>
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}
