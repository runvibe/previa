import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Pipeline } from "@/types/pipeline";
import type { LoadRunConfig } from "@/types/load-test";

const mocks = vi.hoisted(() => ({
  runRemoteLoadTest: vi.fn(),
  reconnectToLoadExecution: vi.fn(),
  listLoadHistory: vi.fn(async () => []),
}));

vi.mock("@/lib/remote-executor", () => ({
  runRemoteLoadTest: mocks.runRemoteLoadTest,
  reconnectToLoadExecution: mocks.reconnectToLoadExecution,
}));

vi.mock("@/lib/api-client", () => ({
  listLoadHistory: mocks.listLoadHistory,
  loadRecordToRun: vi.fn(),
}));

vi.mock("@/lib/load-test-store", () => ({
  getLoadTestRuns: vi.fn(async () => []),
  getAllLoadTestRunsForProject: vi.fn(async () => []),
  deleteLoadTestRunsForPipeline: vi.fn(async () => {}),
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
  },
}));

vi.mock("@/i18n", () => ({
  default: {
    t: (key: string) => key,
  },
}));

import { useLoadTestHistoryStore } from "@/stores/useLoadTestHistoryStore";

const pipeline: Pipeline = {
  id: "pipeline-1",
  name: "Pipeline",
  description: "Pipeline",
  steps: [
    {
      id: "step-1",
      name: "Step",
      description: "Step",
      headers: {},
      method: "GET",
      url: "https://example.com",
    },
  ],
};

const config: LoadRunConfig = {
  points: [
    { atMs: 0, intensity: 10 },
    { atMs: 60_000, intensity: 100 },
  ],
  interpolation: "smooth",
};

async function flushAsyncState(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("useLoadTestHistoryStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useLoadTestHistoryStore.getState().disconnectController();
    useLoadTestHistoryStore.setState({
      runs: [],
      activeRunId: null,
      state: "idle",
      metrics: {
        totalSent: 0,
        totalSuccess: 0,
        totalError: 0,
        avgLatency: 0,
        p95: 0,
        p99: 0,
        rps: 0,
        latencyHistory: [],
        rpsHistory: [],
        runnerResourceHistory: [],
        startTime: 0,
        elapsedMs: 0,
      },
      config: null,
      nodesInfo: null,
      provisioningStatus: null,
      provisioningStartedAt: null,
      liveState: "idle",
      liveMetrics: {
        totalSent: 0,
        totalSuccess: 0,
        totalError: 0,
        avgLatency: 0,
        p95: 0,
        p99: 0,
        rps: 0,
        latencyHistory: [],
        rpsHistory: [],
        runnerResourceHistory: [],
        startTime: 0,
        elapsedMs: 0,
      },
      viewingHistoricRun: false,
    });
  });

  it("clears stale reconnect runs when the load execution no longer exists", async () => {
    mocks.reconnectToLoadExecution.mockImplementation(
      (_backend, _projectId, _executionId, callbacks) => {
        queueMicrotask(() => {
          callbacks.onError(
            'HTTP 404: {"error":"not_found","message":"execution not found for project"}',
          );
        });
        return { cancel: vi.fn(), disconnect: vi.fn() };
      },
    );

    useLoadTestHistoryStore.getState().reconnectExecution(
      "exec-stale",
      "project-1",
      "http://127.0.0.1:5610",
    );

    await flushAsyncState();

    const state = useLoadTestHistoryStore.getState();
    expect(state.state).toBe("idle");
    expect(state.liveState).toBe("idle");
    expect(state.activeRunId).toBeNull();
    expect(state.runs).toEqual([]);
    expect(state.provisioningStatus).toBeNull();
    expect(state.provisioningStartedAt).toBeNull();
  });

  it("returns to idle when a load test fails before an execution id is assigned", async () => {
    mocks.runRemoteLoadTest.mockImplementation(
      (_backend, _pipeline, _config, callbacks) => {
        queueMicrotask(() => {
          callbacks.onError(
            'HTTP 503: {"error":"service_unavailable","message":"No active runners found via /health"}',
          );
        });
        return { cancel: vi.fn(), disconnect: vi.fn() };
      },
    );

    useLoadTestHistoryStore.getState().runTest(
      pipeline,
      0,
      "project-1",
      config,
      "http://127.0.0.1:5610",
    );

    await flushAsyncState();

    const state = useLoadTestHistoryStore.getState();
    expect(state.state).toBe("idle");
    expect(state.liveState).toBe("idle");
    expect(state.activeRunId).toBeNull();
    expect(state.runs).toEqual([]);
    expect(state.provisioningStatus).toBeNull();
    expect(state.provisioningStartedAt).toBeNull();
  });

  it("ignores synthetic completion after a provisioning error", async () => {
    mocks.runRemoteLoadTest.mockImplementation(
      (_backend, _pipeline, _config, callbacks) => {
        queueMicrotask(() => {
          callbacks.onExecutionStarted?.("exec-provisioning-failed");
          callbacks.onError(
            'HTTP 404: {"error":"not_found","message":"execution not found for project"}',
          );
          callbacks.onComplete({
            totalSent: 0,
            totalSuccess: 0,
            totalError: 0,
            avgLatency: 0,
            p95: 0,
            p99: 0,
            rps: 0,
            latencyHistory: [],
            rpsHistory: [],
            runnerResourceHistory: [],
            startTime: 0,
            elapsedMs: 0,
          });
        });
        return { cancel: vi.fn(), disconnect: vi.fn() };
      },
    );

    useLoadTestHistoryStore.getState().runTest(
      pipeline,
      0,
      "project-1",
      config,
      "http://127.0.0.1:5610",
    );

    await flushAsyncState();

    const state = useLoadTestHistoryStore.getState();
    expect(state.state).toBe("idle");
    expect(state.liveState).toBe("idle");
    expect(state.activeRunId).toBeNull();
    expect(state.runs).toEqual([]);
  });
});
