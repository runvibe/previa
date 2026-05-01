import { create } from "zustand";
import {
  getRuns,
  getAllRunsForProject,
  deleteRunsForPipeline as removeRunsForPipeline,
  importRuns as persistImportRuns,
  type ExecutionRun,
} from "@/lib/execution-store";
import { runRemoteIntegrationTest, reconnectToE2eExecution, type RemoteExecutionController } from "@/lib/remote-executor";
import * as apiClient from "@/lib/api-client";
import type { Pipeline, StepExecutionResult } from "@/types/pipeline";
import { toast } from "sonner";
import i18n from "@/i18n";

interface ExecutionHistoryState {
  runs: ExecutionRun[];
  latestStatuses: Record<number, "success" | "error" | "running" | "queued">;
  activeRunId: string | null;

  // Execution state
  results: Record<string, StepExecutionResult>;
  running: boolean;
  executionNode: string | null;
  resultsGeneration: number;
  lastRunFinishedAt: number;

  // Local history helpers (offline mode only)
  loadLocalRuns: (projectId: string, pipelineIndex: number) => Promise<void>;
  loadLocalAllForProject: (projectId: string) => Promise<ExecutionRun[]>;
  deleteLocalRunsForPipeline: (projectId: string, pipelineIndex: number) => Promise<void>;
  importRuns: (runs: Omit<ExecutionRun, "id">[], projectId: string) => Promise<void>;
  setActiveRunId: (id: string | null) => void;
  setRuns: (runs: ExecutionRun[]) => void;
  setLatestStatuses: (statuses: Record<number, "success" | "error" | "running" | "queued">) => void;
  updatePipelineStatus: (pipelineIndex: number, status: "success" | "error" | "running" | "queued") => void;
  clearPipelineStatus: (pipelineIndex: number) => void;

  // Execution actions
  disconnectController: () => void;
  runTest: (
    pipeline: Pipeline,
    pipelineIndex: number,
    projectId: string,
    executionBackendUrl?: string,
    specs?: import("@/types/project").ProjectSpec[],
    envGroups?: import("@/types/project").ProjectEnvGroup[],
    selectedEnvGroupSlug?: string | null
  ) => Promise<"success" | "error">;
  cancelTest: () => void;
  selectRun: (run: ExecutionRun, executionBackendUrl?: string) => void;
  clearResults: () => void;

  // Load history from API or local
  loadHistory: (projectId: string, pipelineIndex: number, executionBackendUrl?: string, pipeline?: Pipeline) => Promise<void>;

  // Check pipeline runtime status and reconnect if running
  checkPipelineRuntime: (projectId: string, pipelineId: string, pipelineIndex: number, executionBackendUrl: string, pipeline: Pipeline) => Promise<void>;

  // Fetch latest status for all pipelines at once
  fetchAllLatestStatuses: (projectId: string, pipelineCount: number, executionBackendUrl?: string) => Promise<void>;

  // Reconnect to active execution via SSE
  reconnectExecution: (executionId: string, projectId: string, executionBackendUrl: string, pipeline?: Pipeline, pipelineIndex?: number) => void;
}

// Internal ref for the controller — not exposed in state
let _controller: RemoteExecutionController | null = null;
let _lastLoadedExecutionId: string | null = null;

function executionHintKey(projectId: string, pipelineId: string): string {
  return `previa:active-execution:${projectId}:${pipelineId}`;
}

function saveExecutionHint(projectId: string, pipelineId: string, executionId: string): void {
  if (typeof window === "undefined") return;
  window.sessionStorage.setItem(executionHintKey(projectId, pipelineId), executionId);
}

function readExecutionHint(projectId: string, pipelineId: string): string | null {
  if (typeof window === "undefined") return null;
  return window.sessionStorage.getItem(executionHintKey(projectId, pipelineId));
}

function clearExecutionHint(projectId: string, pipelineId: string): void {
  if (typeof window === "undefined") return;
  window.sessionStorage.removeItem(executionHintKey(projectId, pipelineId));
}

export const useExecutionHistoryStore = create<ExecutionHistoryState>((set, get) => ({
  runs: [],
  latestStatuses: {},
  activeRunId: null,
  results: {},
  running: false,
  executionNode: null,
  resultsGeneration: 0,
  lastRunFinishedAt: 0,

  loadLocalRuns: async (projectId, pipelineIndex) => {
    const runs = await getRuns(projectId, pipelineIndex);
    set({ runs });
  },

  loadLocalAllForProject: async (projectId) => {
    return getAllRunsForProject(projectId);
  },

  deleteLocalRunsForPipeline: async (projectId, pipelineIndex) => {
    await removeRunsForPipeline(projectId, pipelineIndex);
    set({ runs: [], activeRunId: null });
  },

  importRuns: async (runs, projectId) => {
    await persistImportRuns(runs, projectId);
  },

  setActiveRunId: (id) => set({ activeRunId: id }),
  setRuns: (runs) => set({ runs }),
  setLatestStatuses: (statuses) => set({ latestStatuses: statuses }),
  updatePipelineStatus: (pipelineIndex, status) => {
    set((state) => ({
      latestStatuses: { ...state.latestStatuses, [pipelineIndex]: status },
    }));
  },
  clearPipelineStatus: (pipelineIndex) => {
    set((state) => {
      const latestStatuses = { ...state.latestStatuses };
      delete latestStatuses[pipelineIndex];
      return { latestStatuses };
    });
  },

  // ── Execution actions ──

  runTest: async (pipeline, pipelineIndex, projectId, executionBackendUrl?, specs?, envGroups?, selectedEnvGroupSlug?) => {
    console.log("[DEBUG][runTest] pipeline received", {
      pipelineId: pipeline.id,
      pipelineName: pipeline.name,
      firstStepAsserts: pipeline.steps[0]?.asserts,
      timestamp: Date.now(),
    });
    set({ running: true, executionNode: null, activeRunId: null });

    // Initialize all steps as pending
    const initial: Record<string, StepExecutionResult> = {};
    for (const step of pipeline.steps) {
      initial[step.id] = { stepId: step.id, status: "pending" };
    }
    set({ results: initial });

    // Inject synthetic "running" entry into history
    const syntheticId = `running-${Date.now()}`;
    const syntheticRun: ExecutionRun = {
      id: syntheticId,
      projectId,
      pipelineIndex,
      pipelineName: pipeline.name,
      status: "running",
      timestamp: new Date().toISOString(),
      duration: 0,
      results: initial,
    };
    set((state) => ({
      runs: [syntheticRun, ...state.runs.filter(r => r.status !== "running")],
      activeRunId: syntheticId,
      latestStatuses: { ...state.latestStatuses, [pipelineIndex]: "running" as const },
    }));

    const startTime = Date.now();
    let finalResults: Record<string, StepExecutionResult> = { ...initial };
    let activeExecutionId: string | null = null;

    if (!executionBackendUrl) {
      toast.error(i18n.t("store.configureServerUrl"));
      set({ running: false, results: {}, activeRunId: null });
      return "error";
    }

    // Remote execution via SSE
    await new Promise<void>((resolve) => {
      const runtimeSpecs = specs?.map(s => ({ slug: s.slug, servers: s.servers }));
      const runtimeEnvGroups = envGroups?.map((group) => ({
        slug: group.slug,
        urls: Object.fromEntries(group.entries.map((entry) => [entry.name, entry.url])),
      }));
      const controller = runRemoteIntegrationTest(executionBackendUrl, pipeline, {
          onExecutionInit: (executionId) => {
            if (!executionId || !pipeline.id) return;
            activeExecutionId = executionId;
            _lastLoadedExecutionId = executionId;
            saveExecutionHint(projectId, pipeline.id, executionId);
            console.log("[DEBUG][runTest.onExecutionInit] captured execution id", {
              projectId,
              pipelineId: pipeline.id,
              pipelineIndex,
              executionId,
            });
            set((state) => ({
              runs: state.runs.map((run) => run.id === syntheticId ? { ...run, executionId } : run),
            }));
          },
          onStepStart: (stepId, meta) => {
            const prev = finalResults[stepId];
            const attempt = meta?.attempt ?? ((prev?.attempts ?? 0) + 1);
            finalResults = { ...finalResults, [stepId]: {
              ...prev,
              stepId,
              status: "running",
              attempts: attempt,
              maxAttempts: meta?.maxAttempts ?? prev?.maxAttempts,
              startedAt: meta?.startedAt ?? Date.now(),
            } };
            set({ results: { ...finalResults } });
          },
          onStepResult: (stepId, result) => {
            const prev = finalResults[stepId];
            finalResults = { ...finalResults, [stepId]: {
              ...result,
              attempts: result.attempts ?? prev?.attempts,
              maxAttempts: result.maxAttempts ?? prev?.maxAttempts,
            } };
            set({ results: { ...finalResults } });
          },
          onComplete: () => resolve(),
          onError: (err) => {
            console.error("Remote e2e test error:", err);
            toast.error(err || i18n.t("store.remoteExecutionError"));
            resolve();
          },
          onNodeInfo: (node) => set({ executionNode: node }),
        }, projectId, undefined, pipelineIndex, runtimeSpecs, runtimeEnvGroups, selectedEnvGroupSlug);
        _controller = controller;
    });

    const duration = Date.now() - startTime;
    const hasError = Object.values(finalResults).some(r => r.status === "error");
    const status: "success" | "error" = hasError ? "error" : "success";

    // Update pipeline status
    set((state) => ({
      latestStatuses: { ...state.latestStatuses, [pipelineIndex]: status },
    }));

    // Persist and reload history
    try {
      const records = await apiClient.listIntegrationHistory(executionBackendUrl, projectId, {
        pipelineIndex,
        limit: 50,
      });
      const mapped = records.map(apiClient.integrationRecordToRun);
      set({
        runs: mapped,
        activeRunId: mapped.length > 0 ? (mapped[0].id ?? null) : null,
      });
    } catch (e) {
      console.error("Failed to save/load run:", e);
      toast.error(i18n.t("store.saveHistoryError"));
    }

    set((state) => ({
      running: false,
      results: finalResults,
      resultsGeneration: state.resultsGeneration + 1,
      lastRunFinishedAt: Date.now(),
    }));
    if (pipeline.id) {
      clearExecutionHint(projectId, pipeline.id);
    }
    _controller = null;
    return status;
  },

  disconnectController: () => {
    console.log("[DEBUG][executionStore.disconnectController] disconnecting active SSE", {
      hadController: !!_controller,
      lastLoadedExecutionId: _lastLoadedExecutionId,
    });
    _controller?.disconnect();
    _controller = null;
    _lastLoadedExecutionId = null;
    set({ running: false, results: {}, activeRunId: null });
  },

  cancelTest: () => {
    _controller?.cancel();
    _controller = null;
  },

  selectRun: (run, executionBackendUrl?) => {
    if (run.status === "running" && executionBackendUrl && run.executionId) {
      get().reconnectExecution(run.executionId, run.projectId, executionBackendUrl);
    } else {
      _lastLoadedExecutionId = run.executionId ?? null;
      set({
        results: run.results,
        activeRunId: run.id ?? null,
      });
    }
  },

  clearResults: () => {
    set({ results: {}, activeRunId: null });
  },

  loadHistory: async (projectId, pipelineIndex, executionBackendUrl?, pipeline?) => {
    const genBefore = get().resultsGeneration;
    try {
      if (executionBackendUrl) {
        const records = await apiClient.listIntegrationHistory(executionBackendUrl, projectId, {
          pipelineIndex,
          limit: 50,
        });
        const mapped = records.map(apiClient.integrationRecordToRun);
        set({ runs: mapped });
        if (!get().running && mapped.length > 0) {
          const latest = mapped[0];
          if (latest.status === "running" && latest.executionId) {
            console.log("[DEBUG][loadHistory] found running execution in history; reconnecting", {
              projectId,
              pipelineIndex,
              pipelineId: pipeline?.id,
              executionId: latest.executionId,
            });
            if (pipeline?.id) {
              saveExecutionHint(projectId, pipeline.id, latest.executionId);
            }
            get().reconnectExecution(latest.executionId, projectId, executionBackendUrl, pipeline, pipelineIndex);
            return;
          }
        }
        // Only auto-select latest results if not running and no recent finish
        if (!get().running && get().resultsGeneration === genBefore && (Date.now() - get().lastRunFinishedAt) > 5000) {
          if (mapped.length > 0) {
            const latest = mapped[0];
            // Skip "running" entries — reconnection is handled by checkPipelineRuntime
            if (latest.status !== "running" && (latest.executionId !== _lastLoadedExecutionId || !get().activeRunId)) {
              _lastLoadedExecutionId = latest.executionId ?? null;
              set({ results: latest.results, activeRunId: latest.id ?? null });
            }
          } else {
            set({ results: {}, activeRunId: null });
          }
        }
      } else {
        const runs = await getRuns(projectId, pipelineIndex);
        set({ runs });
        if (!get().running && get().resultsGeneration === genBefore && (Date.now() - get().lastRunFinishedAt) > 5000) {
          if (runs.length > 0) {
            set({ results: runs[0].results, activeRunId: runs[0].id ?? null });
          } else {
            set({ results: {}, activeRunId: null });
          }
        }
      }
    } catch (e) {
      console.error("Failed to load history:", e);
      toast.error(i18n.t("store.loadHistoryError"));
    }
  },

  checkPipelineRuntime: async (projectId, pipelineId, pipelineIndex, executionBackendUrl, pipeline) => {
    try {
      const baseUrl = apiClient.ensureApiPrefix(executionBackendUrl);
      console.log("[DEBUG][checkPipelineRuntime] requesting pipeline runtime", {
        projectId,
        pipelineId,
        pipelineIndex,
        pipelineName: pipeline.name,
        baseUrl,
        storeRunning: get().running,
        lastLoadedExecutionId: _lastLoadedExecutionId,
      });
      const { runtime } = await apiClient.getPipelineWithRuntime(baseUrl, projectId, pipelineId);
      console.log("[DEBUG][checkPipelineRuntime]", { pipelineId, runtime, timestamp: Date.now() });

      if (runtime.status === "running" && runtime.activeExecution?.id) {
        const execId = runtime.activeExecution.id;
        console.log("[DEBUG][checkPipelineRuntime] pipeline is running", {
          pipelineId,
          pipelineIndex,
          executionId: execId,
          lastLoadedExecutionId: _lastLoadedExecutionId,
          storeRunning: get().running,
          willReconnect: execId !== _lastLoadedExecutionId || !get().running,
        });
        if (execId !== _lastLoadedExecutionId || !get().running) {
          get().reconnectExecution(execId, projectId, executionBackendUrl, pipeline, pipelineIndex);
        }
      } else if (runtime.status === "queued") {
        // Pipeline is queued — reflect that in the sidebar and keep polling.
        console.log("[DEBUG][checkPipelineRuntime] pipeline is queued", {
          pipelineId,
          pipelineIndex,
        });
        set((state) => ({
          latestStatuses: { ...state.latestStatuses, [pipelineIndex]: "queued" as const },
        }));
        setTimeout(() => {
          get().checkPipelineRuntime(projectId, pipelineId, pipelineIndex, executionBackendUrl, pipeline);
        }, 2000);
      } else {
        const hintedExecutionId = readExecutionHint(projectId, pipelineId);
        if (hintedExecutionId) {
          console.log("[DEBUG][checkPipelineRuntime] runtime is idle but found local execution hint", {
            pipelineId,
            pipelineIndex,
            hintedExecutionId,
          });
          get().reconnectExecution(hintedExecutionId, projectId, executionBackendUrl, pipeline, pipelineIndex);
          return;
        }
        console.log("[DEBUG][checkPipelineRuntime] pipeline is idle; clearing status", {
          pipelineId,
          pipelineIndex,
        });
        get().clearPipelineStatus(pipelineIndex);
      }
    } catch (e) {
      console.error("[DEBUG][checkPipelineRuntime] failed to check pipeline runtime", {
        projectId,
        pipelineId,
        pipelineIndex,
        error: e,
      });
      // Silently fall back — loadHistory will still work
    }
  },

  fetchAllLatestStatuses: async (projectId, pipelineCount, executionBackendUrl?) => {
    if (!executionBackendUrl || pipelineCount === 0) return;
    try {
      const promises = Array.from({ length: pipelineCount }, (_, i) =>
        apiClient.listIntegrationHistory(executionBackendUrl, projectId, {
          pipelineIndex: i,
          limit: 1,
        }).then(records => {
          if (records.length > 0) {
            const status = records[0].status as string;
            if (status === "success" || status === "error" || status === "running") {
              return [i, status] as [number, "success" | "error" | "running"];
            }
          }
          return null;
        }).catch(() => null)
      );
      const results = await Promise.all(promises);
      const statuses: Record<number, "success" | "error" | "running" | "queued"> = {};
      for (const entry of results) {
        if (entry) statuses[entry[0]] = entry[1];
      }
      // Only update statuses that aren't already set (don't overwrite active execution statuses)
      set((state) => ({
        latestStatuses: { ...statuses, ...state.latestStatuses },
      }));
    } catch (e) {
      console.error("Failed to fetch latest statuses:", e);
    }
  },

  reconnectExecution: (executionId, projectId, executionBackendUrl, pipeline?, pipelineIndex?) => {
    console.log("[DEBUG][reconnectExecution] starting reconnect", {
      projectId,
      executionId,
      pipelineId: pipeline?.id,
      pipelineName: pipeline?.name,
      pipelineIndex,
      previousExecutionId: _lastLoadedExecutionId,
    });
    _lastLoadedExecutionId = executionId;
    // Cancel any existing controller
    _controller?.disconnect();
    _controller = null;

    set({ running: true, executionNode: null, activeRunId: null });

    const initial: Record<string, StepExecutionResult> = {};
    set({ results: initial });

    // Inject synthetic "running" entry
    const syntheticId = `reconnect-${Date.now()}`;
    set((state) => ({
      runs: [{
        id: syntheticId,
        projectId,
        pipelineIndex: pipelineIndex ?? 0,
        pipelineName: pipeline?.name ?? "Reconnecting...",
        status: "running" as const,
        timestamp: new Date().toISOString(),
        duration: 0,
        results: initial,
        executionId,
      }, ...state.runs.filter(r => r.status !== "running")],
      activeRunId: syntheticId,
    }));

    let finalResults: Record<string, StepExecutionResult> = { ...initial };

    const controller = reconnectToE2eExecution(executionBackendUrl, projectId, executionId, {
      onSnapshot: (snapshot) => {
        console.log("[DEBUG][reconnectExecution.onSnapshot]", {
          executionId,
          snapshotExecutionId: snapshot.executionId,
          status: snapshot.status,
          stepsCount: Object.keys(snapshot.results).length,
          pipelineIndex,
        });
        finalResults = { ...snapshot.results };
        set((state) => ({
          results: { ...finalResults },
          running: snapshot.status === "running" || snapshot.status === "queued",
          latestStatuses:
            pipelineIndex !== undefined && (snapshot.status === "running" || snapshot.status === "queued")
              ? { ...state.latestStatuses, [pipelineIndex]: snapshot.status }
              : state.latestStatuses,
        }));
      },
      onStepStart: (stepId, meta) => {
        console.log("[DEBUG][reconnectExecution.onStepStart]", {
          executionId,
          stepId,
          attempt: meta?.attempt,
          maxAttempts: meta?.maxAttempts,
        });
        const prev = finalResults[stepId];
        const attempt = meta?.attempt ?? ((prev?.attempts ?? 0) + 1);
        finalResults = { ...finalResults, [stepId]: {
          ...prev,
          stepId,
          status: "running",
          attempts: attempt,
          maxAttempts: meta?.maxAttempts ?? prev?.maxAttempts,
          startedAt: meta?.startedAt ?? Date.now(),
        } };
        set({ results: { ...finalResults } });
      },
      onStepResult: (stepId, result) => {
        console.log("[DEBUG][reconnectExecution.onStepResult]", {
          executionId,
          stepId,
          status: result.status,
          attempt: result.attempts,
          duration: result.duration,
        });
        const prev = finalResults[stepId];
        finalResults = { ...finalResults, [stepId]: {
          ...result,
          attempts: result.attempts ?? prev?.attempts,
          maxAttempts: result.maxAttempts ?? prev?.maxAttempts,
        } };
        set({ results: { ...finalResults } });
      },
      onComplete: () => {
        console.log("[DEBUG][reconnectExecution.onComplete]", {
          executionId,
          pipelineIndex,
          finalSteps: Object.keys(finalResults).length,
        });
        const hasError = Object.values(finalResults).some(r => r.status === "error");
        const status: "success" | "error" = hasError ? "error" : "success";

        set((state) => ({
          running: false,
          results: finalResults,
          resultsGeneration: state.resultsGeneration + 1,
          lastRunFinishedAt: Date.now(),
          latestStatuses:
            pipelineIndex !== undefined
              ? { ...state.latestStatuses, [pipelineIndex]: status }
              : state.latestStatuses,
        }));
        if (pipeline?.id) {
          clearExecutionHint(projectId, pipeline.id);
        }
        _controller = null;

        // Reload history to get the final record
        apiClient.listIntegrationHistory(executionBackendUrl, projectId, { pipelineIndex, limit: 50 })
          .then(records => {
            const mapped = records.map(apiClient.integrationRecordToRun);
            set({ runs: mapped, activeRunId: mapped.length > 0 ? (mapped[0].id ?? null) : null });
          })
          .catch(() => {});
      },
      onError: (err) => {
        console.error("[DEBUG][reconnectExecution.onError]", {
          executionId,
          pipelineIndex,
          error: err,
        });
        const hasError = Object.values(finalResults).some(r => r.status === "error");
        const status: "success" | "error" = hasError ? "error" : "success";
        set((state) => ({
          running: false,
          results: finalResults,
          resultsGeneration: state.resultsGeneration + 1,
          lastRunFinishedAt: Date.now(),
          latestStatuses:
            pipelineIndex !== undefined
              ? { ...state.latestStatuses, [pipelineIndex]: status }
              : state.latestStatuses,
        }));
        if (pipeline?.id) {
          clearExecutionHint(projectId, pipeline.id);
        }
        _controller = null;
      },
      onNodeInfo: (node) => {
        console.log("[DEBUG][reconnectExecution.onNodeInfo]", {
          executionId,
          node,
        });
        set({ executionNode: node });
      },
    });
    _controller = controller;
  },
}));
