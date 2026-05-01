import { describe, expect, it } from "vitest";

import {
  parseIntegrationSnapshot,
  parseLoadExecutionSnapshot,
} from "@/lib/remote-executor";

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
    });
    expect(snapshot?.nodesInfo?.nodeNames).toEqual(["runner-a", "runner-b"]);
  });

  it("ignores incompatible snapshot kinds", () => {
    expect(parseIntegrationSnapshot({ kind: "load" })).toBeNull();
    expect(parseLoadExecutionSnapshot({ kind: "e2e" })).toBeNull();
  });
});
