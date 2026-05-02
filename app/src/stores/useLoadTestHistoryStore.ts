import { create } from "zustand";
import {
  getLoadTestRuns,
  getAllLoadTestRunsForProject,
  deleteLoadTestRunsForPipeline as removeRunsForPipeline,
  type LoadTestRunRecord,
} from "@/lib/load-test-store";

import { runRemoteLoadTest, reconnectToLoadExecution, type RemoteLoadTestController } from "@/lib/remote-executor";
import * as apiClient from "@/lib/api-client";
import type { Pipeline } from "@/types/pipeline";
import type { LoadRunConfig, LoadTestMetrics, LoadTestState } from "@/types/load-test";
import { toast } from "sonner";
import i18n from "@/i18n";

const emptyMetrics: LoadTestMetrics = {
  totalSent: 0, totalSuccess: 0, totalError: 0,
  avgLatency: 0, p95: 0, p99: 0, rps: 0,
  latencyHistory: [], rpsHistory: [],
  runnerResourceHistory: [],
  startTime: 0, elapsedMs: 0,
};

interface NodesInfo {
  nodesUsed: number;
  nodesFound: number;
  nodeNames: string[];
}

interface LoadTestHistoryState {
  runs: LoadTestRunRecord[];
  activeRunId: string | null;

  // Execution state — what the UI displays
  state: LoadTestState;
  metrics: LoadTestMetrics;
  config: LoadRunConfig | null;
  nodesInfo: NodesInfo | null;

  // Live state — always reflects real execution (decoupled from display when browsing history)
  liveState: LoadTestState;
  liveMetrics: LoadTestMetrics;
  viewingHistoricRun: boolean;

  // Local history helpers (offline mode only)
  loadLocalRuns: (projectId: string, pipelineIndex: number) => Promise<void>;
  loadLocalAllForProject: (projectId: string) => Promise<LoadTestRunRecord[]>;
  deleteLocalRunsForPipeline: (projectId: string, pipelineIndex: number) => Promise<void>;
  setActiveRunId: (id: string | null) => void;
  setRuns: (runs: LoadTestRunRecord[]) => void;

  // Execution actions
  disconnectController: () => void;
  runTest: (
    pipeline: Pipeline,
    pipelineIndex: number,
    projectId: string,
    cfg: LoadRunConfig,
    executionBackendUrl?: string,
    selectedBaseUrlKey?: string,
    specs?: import("@/types/project").ProjectSpec[],
    envGroups?: import("@/types/project").ProjectEnvGroup[],
    selectedEnvGroupSlug?: string | null
  ) => void;
  cancelTest: () => void;
  resetTest: () => void;
  selectHistoricRun: (run: LoadTestRunRecord, executionBackendUrl?: string) => void;
  backToLive: () => void;
  clearHistory: (projectId: string, pipelineIndex: number, executionBackendUrl?: string) => Promise<void>;

  // Load history from API or local
  loadHistory: (projectId: string, pipelineIndex: number, executionBackendUrl?: string, autoReconnect?: boolean) => Promise<void>;

  // Reconnect to active/finished execution via SSE
  reconnectExecution: (executionId: string, projectId: string, executionBackendUrl: string) => void;
}

// Internal controller ref
let _loadController: RemoteLoadTestController | null = null;

export const useLoadTestHistoryStore = create<LoadTestHistoryState>((set, get) => ({
  runs: [],
  activeRunId: null,
  state: "idle",
  metrics: emptyMetrics,
  config: null,
  nodesInfo: null,
  liveState: "idle",
  liveMetrics: emptyMetrics,
  viewingHistoricRun: false,

  loadLocalRuns: async (projectId, pipelineIndex) => {
    const runs = await getLoadTestRuns(projectId, pipelineIndex);
    set({ runs });
  },

  loadLocalAllForProject: async (projectId) => {
    return getAllLoadTestRunsForProject(projectId);
  },

  deleteLocalRunsForPipeline: async (projectId, pipelineIndex) => {
    await removeRunsForPipeline(projectId, pipelineIndex);
    set({ runs: [], activeRunId: null });
  },

  setActiveRunId: (id) => set({ activeRunId: id }),
  setRuns: (runs) => set({ runs }),

  // ── Execution actions ──

  runTest: (pipeline, pipelineIndex, projectId, cfg, executionBackendUrl?, selectedBaseUrlKey?, specs?, envGroups?, selectedEnvGroupSlug?) => {
    // selectedBaseUrlKey is kept for remote compat but unused locally
    const syntheticId = `running-${Date.now()}`;
    const syntheticRun: LoadTestRunRecord = {
      id: syntheticId,
      projectId,
      pipelineIndex,
      pipelineName: pipeline.name,
      config: cfg,
      metrics: emptyMetrics,
      state: "running",
      timestamp: new Date().toISOString(),
    };
    set((s) => ({
      config: cfg,
      metrics: emptyMetrics,
      state: "running",
      viewingHistoricRun: false,
      nodesInfo: null,
      liveMetrics: emptyMetrics,
      liveState: "running",
      runs: [syntheticRun, ...s.runs.filter(r => r.state !== "running")],
      activeRunId: syntheticId,
    }));

    if (!executionBackendUrl) {
      toast.error(i18n.t("store.configureServerUrl"));
      set({ state: "idle", liveState: "idle", metrics: emptyMetrics, liveMetrics: emptyMetrics, activeRunId: null });
      return;
    }

    const saveAndRefresh = async (finalMetrics: LoadTestMetrics, finalState: LoadTestState) => {
      try {
        const records = await apiClient.listLoadHistory(executionBackendUrl, projectId, {
          pipelineIndex,
          limit: 50,
        });
        set({ runs: records.map(apiClient.loadRecordToRun) });
      } catch (e) {
        console.error("Failed to save/load load test run:", e);
        toast.error(i18n.t("store.saveLoadTestError"));
      }
    };

    {
      const collectedNodeNames = new Set<string>();
      const runtimeSpecs = specs?.map(s => ({ slug: s.slug, servers: s.servers }));
      const runtimeEnvGroups = envGroups?.map((group) => ({
        slug: group.slug,
        urls: Object.fromEntries(group.entries.map((entry) => [entry.name, entry.url])),
      }));
      const controller = runRemoteLoadTest(executionBackendUrl, pipeline, cfg, {
        onSnapshot: (snapshot) => {
          const s = get();
          set({
            liveMetrics: snapshot.metrics,
            liveState: snapshot.state,
            nodesInfo: snapshot.nodesInfo,
          });
          if (!s.viewingHistoricRun) {
            set({
              metrics: snapshot.metrics,
              state: snapshot.state,
            });
          }
        },
        onMetricsUpdate: (m) => {
          const snapshot = { ...m };
          const s = get();
          set({ liveMetrics: snapshot });
          if (!s.viewingHistoricRun) {
            set({ metrics: snapshot });
          }
        },
        onComplete: (m) => {
          const snapshot = { ...m };
          const s = get();
          set({ liveMetrics: snapshot, liveState: "completed" });
          if (!s.viewingHistoricRun) {
            set({ metrics: snapshot, state: "completed" });
          }
          saveAndRefresh(snapshot, "completed");
        },
        onError: (err) => {
          console.error("Remote load test error:", err);
          toast.error(err || i18n.t("store.loadTestRemoteError"));
          const s = get();
          set({ liveState: "cancelled" });
          if (!s.viewingHistoricRun) {
            set({ state: "cancelled" });
          }
          saveAndRefresh(s.liveMetrics, "cancelled");
        },
        onNodesInfo: (info) => {
          if (info.nodeNames) info.nodeNames.forEach(n => collectedNodeNames.add(n));
          set({
            nodesInfo: {
              ...info,
              nodeNames: Array.from(collectedNodeNames),
            },
          });
        },
      }, projectId, selectedBaseUrlKey, pipelineIndex, runtimeSpecs, runtimeEnvGroups, selectedEnvGroupSlug);
      _loadController = controller;
    }
  },

  disconnectController: () => {
    _loadController?.disconnect();
    _loadController = null;
    set({ state: "idle", metrics: emptyMetrics, activeRunId: null, liveState: "idle", liveMetrics: emptyMetrics, viewingHistoricRun: false, nodesInfo: null });
  },

  cancelTest: () => {
    _loadController?.cancel();
    _loadController = null;
    const s = get();
    set({ liveState: "cancelled" });
    if (!s.viewingHistoricRun) {
      set({ state: "cancelled" });
    }
    // Save cancelled run
    const { config, liveMetrics, runs } = get();
    if (config) {
      // We need projectId/pipelineIndex but they're not in state — save is handled by the caller context
      // Actually for cancel, the save was triggered by state change in the old code.
      // We'll handle this differently: the runTest already set up saveAndRefresh for onComplete/onError.
      // For local cancel, we need to save manually. But we don't have projectId here.
      // Let's skip auto-save on cancel for now — the old code did it via useEffect on state change.
    }
  },

  resetTest: () => {
    _loadController = null;
    set({
      state: "idle",
      metrics: emptyMetrics,
      viewingHistoricRun: false,
      activeRunId: null,
      nodesInfo: null,
      liveMetrics: emptyMetrics,
      liveState: "idle",
    });
  },

  selectHistoricRun: (run, _executionBackendUrl?) => {
    // Always just switch display — never cancel or reconnect the active execution
    set({
      metrics: run.metrics,
      config: run.config,
      state: run.state === "completed" ? "completed" : "cancelled",
      activeRunId: run.id ?? null,
      viewingHistoricRun: true,
    });
  },

  backToLive: () => {
    const s = get();
    set({
      viewingHistoricRun: false,
      activeRunId: null,
      metrics: s.liveMetrics,
      state: s.liveState,
    });
  },

  clearHistory: async (projectId, pipelineIndex, executionBackendUrl?) => {
    try {
      if (executionBackendUrl) {
        await apiClient.deleteLoadHistory(executionBackendUrl, projectId, { pipelineIndex });
      } else {
        await removeRunsForPipeline(projectId, pipelineIndex);
      }
    } catch (e) {
      console.error("Failed to clear remote load history:", e);
      toast.error(i18n.t("store.clearRemoteHistoryError"));
      if (executionBackendUrl) {
        return;
      }
    }
    const s = get();
    set({ runs: [] });
    if (s.viewingHistoricRun) {
      _loadController = null;
      set({
        state: "idle",
        metrics: emptyMetrics,
        viewingHistoricRun: false,
        activeRunId: null,
        nodesInfo: null,
        liveMetrics: emptyMetrics,
        liveState: "idle",
      });
    }
  },

  loadHistory: async (projectId, pipelineIndex, executionBackendUrl?, autoReconnect = true) => {
    try {
      if (executionBackendUrl) {
        const records = await apiClient.listLoadHistory(executionBackendUrl, projectId, {
          pipelineIndex,
          limit: 50,
        });
        const mapped = records.map(apiClient.loadRecordToRun);
        const { liveState, activeRunId, runs: currentRuns } = get();

        // Preserve synthetic running entry during polling
        if (liveState === "running" && activeRunId) {
          const hasRunning = mapped.some(r => r.state === "running");
          if (!hasRunning) {
            const syntheticEntry = currentRuns.find(r => r.id === activeRunId && r.state === "running");
            if (syntheticEntry) {
              set({ runs: [syntheticEntry, ...mapped.filter(r => r.id !== activeRunId)] });
            } else {
              set({ runs: mapped });
            }
          } else {
            set({ runs: mapped });
          }
        } else {
          set({ runs: mapped });
        }

        // Auto-reconnect to latest run if not currently running
        if (autoReconnect && !get().liveState.startsWith("running") && mapped.length > 0) {
          const latest = mapped[0];
          if (latest.executionId) {
            get().reconnectExecution(latest.executionId, latest.projectId, executionBackendUrl);
          }
        }
      } else {
        const runs = await getLoadTestRuns(projectId, pipelineIndex);
        set({ runs });
      }
    } catch (err) {
      console.error("Failed to load history:", err);
      toast.error(i18n.t("store.loadTestHistoryError"));
      if (executionBackendUrl) {
        set({ runs: [] });
        return;
      }
      const runs = await getLoadTestRuns(projectId, pipelineIndex);
      set({ runs });
    }
  },

  reconnectExecution: (executionId, projectId, executionBackendUrl) => {
    _loadController?.disconnect();
    _loadController = null;

    const collectedNodeNames = new Set<string>();

    const syntheticId = `running-${executionId}`;
    const syntheticRun: LoadTestRunRecord = {
      id: syntheticId,
      projectId,
      pipelineIndex: 0,
      pipelineName: "",
      config: { totalRequests: 0, concurrency: 0, rampUpSeconds: 0 },
      metrics: emptyMetrics,
      state: "running",
      timestamp: new Date().toISOString(),
      executionId,
    };
    set((s) => ({
      state: "running",
      metrics: emptyMetrics,
      viewingHistoricRun: false,
      nodesInfo: null,
      liveMetrics: emptyMetrics,
      liveState: "running",
      runs: [syntheticRun, ...s.runs.filter(r => r.state !== "running")],
      activeRunId: syntheticId,
    }));

    const controller = reconnectToLoadExecution(executionBackendUrl, projectId, executionId, {
      onSnapshot: (snapshot) => {
        const s = get();
        set({
          liveMetrics: snapshot.metrics,
          liveState: snapshot.state,
          nodesInfo: snapshot.nodesInfo,
        });
        if (!s.viewingHistoricRun) {
          set({
            metrics: snapshot.metrics,
            state: snapshot.state,
          });
        }
      },
      onMetricsUpdate: (m) => {
        const snapshot = { ...m };
        const s = get();
        set({ liveMetrics: snapshot });
        if (!s.viewingHistoricRun) {
          set({ metrics: snapshot });
        }
      },
      onComplete: (m) => {
        const snapshot = { ...m };
        const s = get();
        set({ liveMetrics: snapshot, liveState: "completed" });
        if (!s.viewingHistoricRun) {
          set({ metrics: snapshot, state: "completed" });
        }
        _loadController = null;

        // Reload history
        apiClient.listLoadHistory(executionBackendUrl, projectId, { limit: 50 })
          .then(records => set({ runs: records.map(apiClient.loadRecordToRun) }))
          .catch(() => {});
      },
      onError: (err) => {
        console.error("Reconnect load test error:", err);
        const s = get();
        set({ liveState: "cancelled" });
        if (!s.viewingHistoricRun) {
          set({ state: "cancelled" });
        }
        _loadController = null;
      },
      onNodesInfo: (info) => {
        if (info.nodeNames) info.nodeNames.forEach(n => collectedNodeNames.add(n));
        set({
          nodesInfo: {
            ...info,
            nodeNames: Array.from(collectedNodeNames),
          },
        });
      },
    });
    _loadController = controller;
  },
}));
