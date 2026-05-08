import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Pipeline, StepExecutionResult } from "@/types/pipeline";

const mocks = vi.hoisted(() => ({
  runRemoteIntegrationFromStep: vi.fn(),
}));

vi.mock("@/lib/remote-executor", () => ({
  runRemoteIntegrationTest: vi.fn(),
  runRemoteIntegrationFromStep: mocks.runRemoteIntegrationFromStep,
  reconnectToE2eExecution: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  listIntegrationHistory: vi.fn(async () => []),
  integrationRecordToRun: vi.fn(),
  ensureApiPrefix: (value: string) => value,
  getPipelineWithRuntime: vi.fn(),
  projectEnvGroupsToRuntime: vi.fn(() => []),
}));

vi.mock("@/lib/execution-store", () => ({
  getRuns: vi.fn(async () => []),
  getAllRunsForProject: vi.fn(async () => []),
  deleteRunsForPipeline: vi.fn(async () => {}),
  importRuns: vi.fn(async () => {}),
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

import { useExecutionHistoryStore } from "@/stores/useExecutionHistoryStore";

const pipeline: Pipeline = {
  id: "pipeline-1",
  name: "Pipeline",
  description: "Pipeline",
  steps: [
    {
      id: "login",
      name: "Login",
      description: "Login",
      headers: {},
      method: "POST",
      url: "https://example.com/login",
    },
    {
      id: "protected",
      name: "Protected",
      description: "Protected",
      headers: { Authorization: "Bearer {{steps.login.token}}" },
      method: "GET",
      url: "https://example.com/protected",
    },
  ],
};

describe("useExecutionHistoryStore rerunFromStep", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useExecutionHistoryStore.setState({
      runs: [],
      latestStatuses: {},
      activeRunId: null,
      results: {},
      running: false,
      executionNode: null,
      resultsGeneration: 0,
      lastRunFinishedAt: 0,
    });
  });

  it("reruns from the selected step while preserving previous step results", async () => {
    const loginResult: StepExecutionResult = {
      stepId: "login",
      status: "success",
      response: {
        status: 200,
        statusText: "OK",
        headers: {},
        body: { token: "abc123" },
      },
    };
    const protectedResult: StepExecutionResult = {
      stepId: "protected",
      status: "success",
    };
    useExecutionHistoryStore.setState({
      results: { login: loginResult, protected: protectedResult },
    });

    mocks.runRemoteIntegrationFromStep.mockImplementation(
      (_backend, _pipeline, startStepId, priorResults, callbacks) => {
        expect(startStepId).toBe("protected");
        expect(priorResults.login.response.body.token).toBe("abc123");
        callbacks.onStepStart("protected");
        callbacks.onStepResult("protected", {
          stepId: "protected",
          status: "success",
          response: {
            status: 200,
            statusText: "OK",
            headers: {},
            body: { ok: true },
          },
        });
        callbacks.onComplete({ totalSteps: 1, passed: 1, failed: 0, totalDuration: 4 });
        return { cancel: vi.fn(), disconnect: vi.fn() };
      },
    );

    await useExecutionHistoryStore.getState().rerunFromStep(
      pipeline,
      0,
      "project-1",
      "protected",
      "http://127.0.0.1:5610",
      [],
      [],
      "local",
    );

    const results = useExecutionHistoryStore.getState().results;
    expect(results.login).toEqual(loginResult);
    expect(results.protected.response?.body).toEqual({ ok: true });
  });
});
