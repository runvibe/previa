import { afterEach, describe, expect, it, vi } from "vitest";

import {
  parseIntegrationSnapshot,
  parseLoadExecutionSnapshot,
  runRemoteLoadTest,
} from "@/lib/remote-executor";
import { fetchLatestRunnerReservation } from "@/lib/api-client";
import type { Pipeline } from "@/types/pipeline";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("remote execution snapshot parsing", () => {
  it("parses e2e execution snapshots into step results", () => {
    const snapshot = parseIntegrationSnapshot({
      executionId: "exec-1",
      status: "running",
      kind: "e2e",
      steps: [
        {
          stepId: "step-1",
          status: "success",
          duration: 120,
        },
        {
          stepId: "step-2",
          status: "running",
          attempts: 2,
          maxAttempts: 3,
        },
      ],
      summary: null,
      errors: [],
    });

    expect(snapshot).not.toBeNull();
    expect(snapshot?.executionId).toBe("exec-1");
    expect(snapshot?.results["step-1"]).toMatchObject({
      stepId: "step-1",
      status: "success",
      duration: 120,
    });
    expect(snapshot?.results["step-2"]).toMatchObject({
      stepId: "step-2",
      status: "running",
      attempts: 2,
      maxAttempts: 3,
    });
  });

  it("parses load execution snapshots using consolidated metrics and context", () => {
    const snapshot = parseLoadExecutionSnapshot({
      executionId: "exec-2",
      status: "running",
      kind: "load",
      context: {
        registeredNodesTotal: 3,
        usedNodesTotal: 2,
        usedNodes: ["runner-a", "runner-b"],
      },
      lines: [
        {
          node: "runner-a",
          runnerEvent: "metrics",
          payload: {
            totalSent: 50,
            totalSuccess: 50,
            totalError: 0,
            rps: 12,
            startTime: 1000,
            elapsedMs: 5000,
            runtime: {
              pid: 101,
              memoryBytes: 104857600,
              virtualMemoryBytes: 209715200,
              cpuUsagePercent: 37.5,
              networkTxBytes: 2048,
              networkRxBytes: 4096,
              networkTotalBytes: 6144,
            },
          },
        },
      ],
      consolidated: {
        totalSent: 100,
        totalSuccess: 96,
        totalError: 4,
        rps: 24,
        avgLatency: 120,
        p95: 180,
        p99: 250,
        startTime: 1000,
        elapsedMs: 5000,
      },
      errors: ["boom"],
    });

    expect(snapshot).not.toBeNull();
    expect(snapshot?.state).toBe("running");
    expect(snapshot?.metrics).toMatchObject({
      totalSent: 100,
      totalSuccess: 96,
      totalError: 4,
      avgLatency: 120,
      p95: 180,
      p99: 250,
      rps: 24,
      startTime: 1000,
      elapsedMs: 5000,
    });
    expect(snapshot?.metrics.rpsHistory[0]).toMatchObject({
      timestamp: 6000,
      elapsedMs: 5000,
      rps: 24,
    });
    expect(snapshot?.metrics.runnerResourceHistory).toEqual([
      {
        node: "runner-a",
        timestamp: 6000,
        elapsedMs: 5000,
        cpuUsagePercent: 37.5,
        memoryBytes: 104857600,
        memoryMb: 100,
        networkTxBytes: 2048,
        networkRxBytes: 4096,
        networkTotalBytes: 6144,
        networkTotalKb: 6,
      },
    ]);
    expect(snapshot?.nodesInfo).toEqual({
      nodesUsed: 2,
      nodesFound: 3,
      nodeNames: ["runner-a", "runner-b"],
    });
    expect(snapshot?.errors).toEqual(["boom"]);
  });

  it("parses load execution snapshots with top-level runner context", () => {
    const snapshot = parseLoadExecutionSnapshot({
      executionId: "exec-top-level",
      status: "running",
      nodesFound: 1,
      usedNodesTotal: 1,
      usedNodes: ["http://runner.local"],
      consolidated: {
        totalSent: 30,
        totalSuccess: 29,
        totalError: 1,
        rps: 10,
        startTime: 1000,
        elapsedMs: 2000,
      },
    });

    expect(snapshot).not.toBeNull();
    expect(snapshot?.nodesInfo).toEqual({
      nodesUsed: 1,
      nodesFound: 1,
      nodeNames: ["http://runner.local"],
    });
    expect(snapshot?.metrics).toMatchObject({
      totalSent: 30,
      totalSuccess: 29,
      totalError: 1,
      rps: 10,
    });
  });

  it("falls back to aggregating line metrics when consolidated data is absent", () => {
    const snapshot = parseLoadExecutionSnapshot({
      executionId: "exec-3",
      status: "success",
      kind: "load",
      context: {
        nodesFound: 2,
        usedNodes: ["runner-a", "runner-b"],
      },
      lines: [
        {
          node: "runner-a",
          runnerEvent: "metrics",
          payload: {
            totalSent: 20,
            totalSuccess: 19,
            totalError: 1,
            rps: 5,
            startTime: 100,
            elapsedMs: 2000,
            senderLaggedStarts: 2,
            senderQueueDepth: 4,
            lifecycleBuckets: [
              { elapsedMs: 2000, planned: 10, httpStarted: 8, senderLagged: 1 },
            ],
          },
        },
        {
          node: "runner-b",
          runnerEvent: "metrics",
          payload: {
            totalSent: 30,
            totalSuccess: 30,
            totalError: 0,
            rps: 7,
            startTime: 90,
            elapsedMs: 2200,
            senderLaggedStarts: 3,
            senderQueueDepth: 6,
            lifecycleBuckets: [
              { elapsedMs: 2000, planned: 12, httpStarted: 9, senderLagged: 2 },
            ],
          },
        },
      ],
    });

    expect(snapshot).not.toBeNull();
    expect(snapshot?.state).toBe("completed");
    expect(snapshot?.metrics).toMatchObject({
      totalSent: 50,
      totalSuccess: 49,
      totalError: 1,
      rps: 12,
      startTime: 90,
      elapsedMs: 2200,
      senderLaggedStarts: 5,
      senderQueueDepth: 10,
    });
    expect(snapshot?.metrics.lifecycleBuckets?.[0]).toMatchObject({
      elapsedMs: 2000,
      senderLagged: 3,
    });
    expect(snapshot?.nodesInfo?.nodeNames).toEqual(["runner-a", "runner-b"]);
  });

  it("ignores incompatible snapshot kinds", () => {
    expect(parseIntegrationSnapshot({ kind: "load" })).toBeNull();
    expect(parseLoadExecutionSnapshot({ kind: "e2e" })).toBeNull();
  });

  it("maps provisioning load snapshots to provisioning UI state", () => {
    const snapshot = parseLoadExecutionSnapshot({
      executionId: "exec-provisioning",
      status: "provisioning",
      kind: "load",
      context: {
        registeredNodesTotal: 0,
        usedNodesTotal: 0,
        usedNodes: [],
      },
    });

    expect(snapshot).toMatchObject({
      executionId: "exec-provisioning",
      status: "provisioning",
      state: "provisioning",
    });
  });
});

describe("remote load execution requests", () => {
  const pipeline: Pipeline = {
    id: "pipeline-1",
    name: "Pipeline",
    description: null,
    steps: [
      {
        id: "step-1",
        name: "Step",
        description: null,
        method: "GET",
        url: "https://example.com",
        headers: {},
      },
    ],
  };

  function sseResponse(chunks: string[], status = 200): Response {
    const encoder = new TextEncoder();
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        for (const chunk of chunks) {
          controller.enqueue(encoder.encode(chunk));
        }
        controller.close();
      },
    });

    return new Response(stream, {
      status,
      headers: { "content-type": "text/event-stream" },
    });
  }

  it("fetches the latest runner reservation for a pipeline without exposing a token", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue({
      ok: true,
      json: async () => ({
        executionId: "exec-1",
        pipelineId: "pipe-1",
        capacityMode: "kubernetes",
        requestedRunnerCount: 3,
        readyRunnerCount: 2,
        targetRps: 2500,
        nodeProfile: "4gn.nano",
        reservationId: "rr-1",
        reservationExpiresAt: "2026-05-14T10:00:00Z",
        reservationStatus: "provisioning",
        runnerEndpoints: ["http://10.0.0.1:55880"],
        createdAt: "2026-05-14T09:55:00Z",
        updatedAt: "2026-05-14T09:56:00Z",
      }),
    } as Response);

    const status = await fetchLatestRunnerReservation(
      "http://localhost:5589",
      "project-1",
      "pipe-1",
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "http://localhost:5589/api/v1/projects/project-1/pipelines/pipe-1/runner-reservation/latest",
      expect.objectContaining({ method: "GET" }),
    );
    expect(status?.reservationId).toBe("rr-1");
    expect("reservationToken" in status!).toBe(false);
  });

  it("polls provisioning status while the load stream request is pending", async () => {
    vi.useFakeTimers();
    const callbacks = {
      onError: vi.fn(),
      onProvisioningUpdate: vi.fn(),
    };
    let resolveLoadRequest: ((value: Response) => void) | null = null;
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation((input) => {
      const url = String(input);
      if (url.includes("/runner-reservation/latest")) {
        return Promise.resolve({
          ok: true,
          status: 200,
          json: async () => ({
            executionId: "exec-1",
            pipelineId: "pipeline-1",
            capacityMode: "kubernetes",
            requestedRunnerCount: 3,
            readyRunnerCount: 1,
            targetRps: 2500,
            nodeProfile: "4gn.nano",
            reservationId: "rr-1",
            reservationExpiresAt: "2026-05-14T10:00:00Z",
            reservationStatus: "provisioning",
            runnerEndpoints: ["http://10.0.0.1:55880"],
            createdAt: "2026-05-14T09:55:00Z",
            updatedAt: "2026-05-14T09:56:00Z",
          }),
        } as Response);
      }

      return new Promise<Response>((resolve) => {
        resolveLoadRequest = resolve;
      });
    });

    const controller = runRemoteLoadTest(
      "http://localhost:5589",
      pipeline,
      { points: [{ atMs: 0, intensity: 10 }], interpolation: "smooth" },
      callbacks,
      "project-1",
      undefined,
      0,
      [],
      [],
      null,
      2500,
    );

    await vi.advanceTimersByTimeAsync(1_100);

    expect(callbacks.onProvisioningUpdate).toHaveBeenCalledWith(
      expect.objectContaining({
        reservationId: "rr-1",
        readyRunnerCount: 1,
        requestedRunnerCount: 3,
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "http://localhost:5589/api/v1/projects/project-1/pipelines/pipeline-1/runner-reservation/latest",
      expect.objectContaining({ method: "GET" }),
    );

    controller.disconnect();
    resolveLoadRequest?.({
      ok: false,
      status: 499,
      text: async () => "cancelled",
    } as Response);
  });

  it("opens the execution event stream after an async load start response", async () => {
    const onSnapshot = vi.fn();
    const onError = vi.fn();
    const onExecutionStarted = vi.fn();
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation((input) => {
      const url = String(input);
      if (url.includes("/executions/exec-async")) {
        return Promise.resolve(sseResponse([
          `event: execution:snapshot\ndata: ${JSON.stringify({
            executionId: "exec-async",
            status: "running",
            kind: "load",
            context: {
              registeredNodesTotal: 1,
              usedNodesTotal: 1,
              usedNodes: ["http://runner.local"],
            },
            consolidated: {
              totalSent: 12,
              totalSuccess: 11,
              totalError: 1,
              rps: 4,
              startTime: 1000,
              elapsedMs: 3000,
            },
          })}\n\n`,
        ]));
      }

      return Promise.resolve(new Response(
        JSON.stringify({ executionId: "exec-async", status: "running" }),
        {
          status: 202,
          headers: {
            "content-type": "application/json",
            "x-execution-id": "exec-async",
          },
        },
      ));
    });

    const controller = runRemoteLoadTest(
      "http://localhost:5589",
      pipeline,
      { points: [{ atMs: 0, intensity: 10 }], interpolation: "smooth" },
      {
        onMetricsUpdate: vi.fn(),
        onComplete: vi.fn(),
        onError,
        onExecutionStarted,
        onSnapshot,
      },
      "project-1",
      undefined,
      0,
      [],
      [],
      null,
      1000,
    );

    await vi.waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "http://localhost:5589/api/v1/projects/project-1/executions/exec-async",
        expect.objectContaining({ method: "GET" }),
      );
      expect(onSnapshot).toHaveBeenCalledWith(expect.objectContaining({
        executionId: "exec-async",
        state: "running",
        metrics: expect.objectContaining({ totalSent: 12, totalSuccess: 11 }),
      }));
    });
    expect(onExecutionStarted).toHaveBeenCalledWith("exec-async");
    expect(onError).not.toHaveBeenCalled();

    controller.disconnect();
  });

  it("treats execution status events as live load snapshots", async () => {
    const onSnapshot = vi.fn();
    const onError = vi.fn();
    vi.spyOn(globalThis, "fetch").mockResolvedValue(sseResponse([
      `event: execution:status\ndata: ${JSON.stringify({
        executionId: "exec-status",
        status: "running",
        kind: "load",
        context: {
          registeredNodesTotal: 2,
          usedNodesTotal: 1,
          usedNodes: ["runner-a"],
        },
        consolidated: {
          totalSent: 7,
          totalSuccess: 7,
          totalError: 0,
          rps: 2,
          startTime: 2000,
          elapsedMs: 1000,
        },
      })}\n\n`,
    ]));

    const controller = runRemoteLoadTest(
      "http://localhost:5589",
      pipeline,
      { points: [{ atMs: 0, intensity: 10 }], interpolation: "smooth" },
      {
        onMetricsUpdate: vi.fn(),
        onComplete: vi.fn(),
        onError,
        onSnapshot,
      },
      "project-1",
      undefined,
      0,
      [],
      [],
      null,
      1000,
    );

    await vi.waitFor(() => {
      expect(onSnapshot).toHaveBeenCalledWith(expect.objectContaining({
        executionId: "exec-status",
        state: "running",
        nodesInfo: {
          nodesUsed: 1,
          nodesFound: 2,
          nodeNames: ["runner-a"],
        },
        metrics: expect.objectContaining({ totalSent: 7, rps: 2 }),
      }));
    });
    expect(onError).not.toHaveBeenCalled();

    controller.disconnect();
  });

  it("sends global targetRps alongside the per-runner load profile", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue({
      ok: false,
      text: async () => "stop",
    } as Response);
    runRemoteLoadTest(
      "http://localhost:5589",
      pipeline,
      {
        points: [
          { atMs: 0, intensity: 10 },
          { atMs: 60_000, intensity: 80 },
        ],
        interpolation: "smooth",
        runnerMaxRps: 500,
        gracePeriodMs: 30_000,
      },
      { onError: vi.fn() },
      "project-1",
      undefined,
      0,
      [],
      [],
      null,
      2500,
    );

    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
    const requestBody = JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body));

    expect(requestBody).toMatchObject({
      pipelineId: "pipeline-1",
      targetRps: 2500,
      load: {
        runnerMaxRps: 500,
      },
    });
  });
});
