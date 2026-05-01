import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createRunner,
  deleteRunner,
  loadRecordToRun,
  listRunners,
  projectEnvGroupsToRuntime,
  updateRunner,
  type LoadHistoryRecord,
  type RunnerRecord,
} from "@/lib/api-client";

const baseUrl = "http://127.0.0.1:5588/api/v1";

const runner: RunnerRecord = {
  id: "runner-a",
  endpoint: "http://127.0.0.1:55880",
  name: "Local runner",
  source: "manual",
  enabled: true,
  healthStatus: "healthy",
  lastSeenAt: "2026-04-30T10:00:00Z",
  lastError: null,
  runtime: {
    pid: 123,
    memoryBytes: 1048576,
    virtualMemoryBytes: 2097152,
    cpuUsagePercent: 1.5,
  },
  createdAt: "2026-04-30T09:00:00Z",
  updatedAt: "2026-04-30T10:00:00Z",
};

describe("api-client runners", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("lists runners", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => [runner],
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(listRunners(baseUrl)).resolves.toEqual([runner]);
    expect(fetchMock).toHaveBeenCalledWith(`${baseUrl}/runners`, undefined);
  });

  it("creates a runner", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 201,
      json: async () => runner,
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(createRunner(baseUrl, {
      endpoint: runner.endpoint,
      name: runner.name,
      enabled: true,
    })).resolves.toEqual(runner);
    expect(fetchMock).toHaveBeenCalledWith(
      `${baseUrl}/runners`,
      expect.objectContaining({
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          endpoint: runner.endpoint,
          name: runner.name,
          enabled: true,
        }),
      }),
    );
  });

  it("updates runner enabled state", async () => {
    const updatedRunner = { ...runner, enabled: false };
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => updatedRunner,
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(updateRunner(baseUrl, runner.id, { enabled: false })).resolves.toEqual(updatedRunner);
    expect(fetchMock).toHaveBeenCalledWith(
      `${baseUrl}/runners/${runner.id}`,
      expect.objectContaining({
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ enabled: false }),
      }),
    );
  });

  it("deletes a runner", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 204,
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(deleteRunner(baseUrl, runner.id)).resolves.toBeUndefined();
    expect(fetchMock).toHaveBeenCalledWith(`${baseUrl}/runners/${runner.id}`, { method: "DELETE" });
  });
});

describe("api-client load history mapping", () => {
  it("rebuilds runner resource history from final runner lines", () => {
    const run = loadRecordToRun({
      id: "run-1",
      executionId: "exec-1",
      pipelineName: "Load",
      status: "success",
      startedAtMs: 1_000,
      finishedAtMs: 2_000,
      durationMs: 1_000,
      requestedConfig: {
        totalRequests: 3,
        concurrency: 1,
        rampUpSeconds: 0,
      },
      finalConsolidated: {
        totalSent: 3,
        totalSuccess: 3,
        totalError: 0,
        rps: 3,
        avgLatency: 10,
        p95: 12,
        p99: 13,
        startTime: 1_000,
        elapsedMs: 1_000,
      },
      finalLines: [
        {
          node: "runner-a",
          payload: {
            totalSent: 3,
            totalSuccess: 3,
            totalError: 0,
            rps: 3,
            startTime: 1_000,
            elapsedMs: 1_000,
            runtime: {
              pid: 123,
              memoryBytes: 104_857_600,
              virtualMemoryBytes: 209_715_200,
              cpuUsagePercent: 11.5,
              networkTxBytes: 2_048,
              networkRxBytes: 4_096,
              networkTotalBytes: 6_144,
            },
          },
          runnerEvent: "complete",
          receivedAt: 2_000,
        },
      ],
      errors: [],
      request: {},
      context: {},
      projectId: "project-1",
      pipelineIndex: 0,
    } satisfies LoadHistoryRecord);

    expect(run.metrics.runnerResourceHistory).toEqual([
      {
        node: "runner-a",
        timestamp: 2_000,
        elapsedMs: 1_000,
        cpuUsagePercent: 11.5,
        memoryBytes: 104_857_600,
        memoryMb: 100,
        networkTxBytes: 2_048,
        networkRxBytes: 4_096,
        networkTotalBytes: 6_144,
        networkTotalKb: 6,
      },
    ]);
  });
});

describe("api-client env group mapping", () => {
  it("converts project env group entries to runtime url maps", () => {
    expect(projectEnvGroupsToRuntime([
      {
        id: "env-group-1",
        projectId: "project-1",
        slug: "hml",
        name: "Homolog",
        entries: [
          { name: "api", url: "https://api-hml.example.com" },
          { name: "auth", url: "https://auth-hml.example.com" },
        ],
        createdAt: "2026-05-01T00:00:00Z",
        updatedAt: "2026-05-01T00:00:00Z",
      },
    ])).toEqual([
      {
        slug: "hml",
        urls: {
          api: "https://api-hml.example.com",
          auth: "https://auth-hml.example.com",
        },
      },
    ]);
  });
});
