import { create } from "zustand";
import { getApiUrlFromBase, resolveApiBaseUrl } from "@/lib/api-base";
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

function currentContext(name = "current"): OrchestratorContext {
  return {
    id: CURRENT_CONTEXT_ID,
    name,
    url: resolveApiBaseUrl(),
  };
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
      try {
        const base = resolveApiBaseUrl();
        const res = await fetch(`${base}/info`, { signal: AbortSignal.timeout(8000) });
        if (!res.ok) {
          set({ info: null });
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
        return info;
      } catch {
        set({ info: null });
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
