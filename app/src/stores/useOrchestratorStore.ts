import { create } from "zustand";
import { toast } from "sonner";
import { getApiUrlFromBase, resolveApiBaseUrl } from "@/lib/api-base";
import { clearAuthSession, getAuthToken } from "@/stores/useAuthStore";
import type { OrchestratorContext } from "@/lib/orchestrator-url";

export type { OrchestratorContext } from "@/lib/orchestrator-url";

export interface OrchestratorInfo {
  context: string;
  totalRunners: number;
  activeRunners: number;
}

interface OrchestratorState {
  contexts: OrchestratorContext[];
  activeContextId: string | null;
  info: OrchestratorInfo | null;

  /** Convenience: derived active context */
  activeContext: OrchestratorContext | null;

  /** Legacy compat: returns active context URL or null */
  url: string | null;

  addContext: (name: string, url: string) => OrchestratorContext;
  removeContext: (id: string) => void;
  updateContext: (id: string, updates: Partial<Omit<OrchestratorContext, "id">>) => void;
  switchContext: (id: string) => void;
  setInfo: (info: OrchestratorInfo | null) => void;
  fetchInfo: () => Promise<OrchestratorInfo | null>;

  // Legacy compat
  setUrl: (url: string | null) => void;
}

const CURRENT_CONTEXT_ID = "current";
const API_OFFLINE_TOAST_ID = "previa-api-offline";

function timeoutSignal(timeoutMs: number): AbortSignal | undefined {
  return typeof AbortSignal !== "undefined" && "timeout" in AbortSignal
    ? AbortSignal.timeout(timeoutMs)
    : undefined;
}

function currentContext(name = "current"): OrchestratorContext {
  return {
    id: CURRENT_CONTEXT_ID,
    name,
    url: resolveApiBaseUrl(),
  };
}

function showApiOfflineToast(baseUrl: string) {
  toast.error("Sem conexão com o servidor", {
    id: API_OFFLINE_TOAST_ID,
    description: `URL do serviço: ${baseUrl}`,
  });
}

export const useOrchestratorStore = create<OrchestratorState>((set, get) => {
  const initialActive = currentContext();

  return {
    contexts: [initialActive],
    activeContextId: CURRENT_CONTEXT_ID,
    activeContext: initialActive,
    url: initialActive.url,
    info: null,

    addContext: () => {
      return get().activeContext ?? currentContext();
    },

    removeContext: () => {},

    updateContext: () => {},

    switchContext: () => {},

    setInfo: (info) => set({ info }),

    fetchInfo: async () => {
      const base = resolveApiBaseUrl();
      const token = getAuthToken();
      const headers = token ? { Authorization: `Bearer ${token}` } : undefined;
      try {
        const res = await fetch(`${base}/info`, { headers, signal: timeoutSignal(8000) });
        if (!res.ok) {
          if (res.status === 401) {
            clearAuthSession();
          }
          set({ info: null });
          showApiOfflineToast(base);
          return null;
        }
        const data = await res.json();
        const info: OrchestratorInfo = {
          context: data.context,
          totalRunners: data.totalRunners,
          activeRunners: data.activeRunners,
        };
        const active = currentContext(info.context);
        set({
          contexts: [active],
          activeContextId: CURRENT_CONTEXT_ID,
          activeContext: active,
          url: active.url,
          info,
        });
        toast.dismiss?.(API_OFFLINE_TOAST_ID);
        return info;
      } catch {
        set({ info: null });
        showApiOfflineToast(base);
        return null;
      }
    },

    // Legacy compat
    setUrl: () => {
      const active = currentContext(get().info?.context ?? "current");
      set({
        contexts: [active],
        activeContextId: CURRENT_CONTEXT_ID,
        activeContext: active,
        url: active.url,
      });
    },
  };
});

/** Returns the base API URL (with /api/v1) or null if no backend is configured. Safe to call outside React. */
export function getApiUrl(): string {
  return getApiUrlFromBase();
}
