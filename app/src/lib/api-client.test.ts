import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createRunner,
  deleteRunner,
  listRunners,
  updateRunner,
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
