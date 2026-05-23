import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LoadTestTab } from "@/components/LoadTestTab";
import type { LoadProvisioningStatus, LoadTestState } from "@/types/load-test";
import type { LoadTestRunRecord } from "@/lib/load-test-store";
import type { Pipeline } from "@/types/pipeline";

const loadHistory = vi.fn();
const backToLive = vi.fn();
let isMobile = false;
let mockedRuns: LoadTestRunRecord[] = [];
let mockedStoreState: {
  state: LoadTestState;
  liveState: LoadTestState;
  provisioningStatus: LoadProvisioningStatus | null;
  provisioningStartedAt: number | null;
} = {
  state: "idle",
  liveState: "idle",
  provisioningStatus: null,
  provisioningStartedAt: null,
};

vi.mock("@/hooks/use-mobile", () => ({
  useIsMobile: () => isMobile,
}));

vi.mock("@/stores/useLoadTestHistoryStore", () => ({
  useLoadTestHistoryStore: () => ({
    ...mockedStoreState,
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
    runs: mockedRuns,
    activeRunId: null,
    viewingHistoricRun: false,
    loadHistory,
    clearHistory: vi.fn(),
    runTest: vi.fn(),
    resetTest: vi.fn(),
    cancelTest: vi.fn(),
    backToLive,
    reconnectExecution: vi.fn(),
    selectHistoricRun: vi.fn(),
  }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    i18n: { language: "en-US" },
    t: (key: string, options?: Record<string, string | number>) => {
      const labels: Record<string, string> = {
        "history.title": "History",
        "loadTest.wavePoints": "Wave points",
        "loadTest.wavePoints.help": "Timeline points for the load wave.",
        "loadTest.wavePoints.hint": "Set the test duration, then shape the wave directly on the graph.",
        "loadTest.duration": "Duration",
        "loadTest.pointTimeMs": "Time in milliseconds",
        "loadTest.pointIntensity": "Intensity percent",
        "loadTest.pointTimeColumn": "Time (ms)",
        "loadTest.pointIntensityColumn": "Intensity (%)",
        "loadTest.removePoint": "Remove point",
        "loadTest.interpolation": "Interpolation",
        "loadTest.interpolationSmooth": "Smooth",
        "loadTest.interpolationLinear": "Linear",
        "loadTest.interpolationStep": "Step",
        "loadTest.gracePeriod": "Grace period",
        "loadTest.wavePreview": "Wave editor",
        "loadTest.previewIntensityAxis": "Intensity (%)",
        "loadTest.previewTimeAxis": "Time (ms)",
        "loadTest.selectedPoint": "Selected point",
        "loadTest.configureManually": "Configure manually",
        "loadTest.estimatedTime": "Estimated time",
        "loadTest.provisioning.title": "Provisioning Kubernetes runners",
        "loadTest.provisioning.subtitle": "{{ready}} of {{requested}} runners ready",
        "loadTest.provisioning.waiting": "Waiting for reservation state",
        "loadTest.provisioning.unavailable": "Provisioning state is unavailable",
        "loadTest.provisioning.reservation": "Reservation",
        "loadTest.provisioning.targetRps": "Target RPS",
        "loadTest.provisioning.nodeProfile": "Node profile",
        "loadTest.provisioning.elapsed": "Elapsed",
        "loadTest.provisioning.status": "Status",
      };
      return Object.entries(options ?? {}).reduce(
        (text, [name, value]) => text.replace(`{{${name}}}`, String(value)),
        labels[key] ?? key,
      );
    },
  }),
}));

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

function defaultRuns(state: LoadTestState = "completed"): LoadTestRunRecord[] {
  return [
    {
      id: "run-1",
      projectId: "project-1",
      pipelineIndex: 0,
      pipelineName: "Pipeline",
      config: {
        points: [
          { atMs: 0, intensity: 10 },
          { atMs: 120_000, intensity: 80 },
        ],
        interpolation: "smooth",
      },
      metrics: {
        totalSent: 1,
        totalSuccess: 1,
        totalError: 0,
        avgLatency: 10,
        p95: 10,
        p99: 10,
        rps: 1,
        latencyHistory: [],
        rpsHistory: [],
        runnerResourceHistory: [],
        startTime: 0,
        elapsedMs: 1000,
      },
      state,
      timestamp: "2026-05-02T00:00:00.000Z",
    },
  ];
}

describe("LoadTestTab", () => {
  beforeEach(() => {
    isMobile = false;
    mockedStoreState = {
      state: "idle",
      liveState: "idle",
      provisioningStatus: null,
      provisioningStartedAt: null,
    };
    mockedRuns = defaultRuns();
    backToLive.mockClear();
    window.localStorage.clear();
  });

  it("keeps the load configuration panel scrollable when history is visible", () => {
    render(
      <LoadTestTab
        pipeline={pipeline}
        projectId="project-1"
        pipelineIndex={0}
      />,
    );

    expect(screen.getByTestId("load-test-config-scroll")).toHaveClass(
      "h-full",
      "min-h-0",
      "overflow-y-auto",
    );
  });

  it("collapses and reopens the load test history panel", () => {
    render(
      <LoadTestTab
        pipeline={pipeline}
        projectId="project-1"
        pipelineIndex={0}
      />,
    );

    fireEvent.click(screen.getByTitle("Collapse history"));

    expect(screen.getByTitle("Show history")).toBeInTheDocument();
    expect(screen.queryByText("1 reqs")).not.toBeInTheDocument();

    fireEvent.click(screen.getByTitle("Show history"));

    expect(screen.getByTitle("Collapse history")).toBeInTheDocument();
    expect(screen.getByText("1 reqs")).toBeInTheDocument();
  });

  it("collapses load test history downward on mobile", () => {
    isMobile = true;

    render(
      <LoadTestTab
        pipeline={pipeline}
        projectId="project-1"
        pipelineIndex={0}
      />,
    );

    fireEvent.click(screen.getByText("History"));

    const collapsedRegion = screen.getByTestId("mobile-load-test-history");
    expect(collapsedRegion).toHaveClass("max-h-10", "border-t");
    expect(window.localStorage.getItem("api-pipeline-studio:test-history-collapsed:loadtest")).toBe("true");
    expect(screen.getByTitle("Show history").querySelector(".lucide-history")).toBeInTheDocument();
    expect(screen.queryByText("1 reqs")).not.toBeInTheDocument();

    fireEvent.click(collapsedRegion);

    expect(window.localStorage.getItem("api-pipeline-studio:test-history-collapsed:loadtest")).toBe("false");
    expect(screen.getByText("1 reqs")).toBeInTheDocument();
  });

  it("restores the load test history collapse state from local storage", () => {
    window.localStorage.setItem("api-pipeline-studio:test-history-collapsed:loadtest", "true");

    render(
      <LoadTestTab
        pipeline={pipeline}
        projectId="project-1"
        pipelineIndex={0}
      />,
    );

    expect(screen.getByTitle("Show history")).toBeInTheDocument();
    expect(screen.queryByText("1 reqs")).not.toBeInTheDocument();
  });

  it("shows runner provisioning progress while a load test is provisioning", () => {
    mockedStoreState = {
      state: "provisioning",
      liveState: "provisioning",
      provisioningStartedAt: Date.now() - 2_000,
      provisioningStatus: {
        executionId: "exec-1",
        pipelineId: "pipeline-1",
        capacityMode: "kubernetes",
        requestedRunnerCount: 4,
        readyRunnerCount: 2,
        targetRps: 2500,
        nodeProfile: "4gn.nano",
        reservationId: "rr-1",
        reservationExpiresAt: "2026-05-14T10:00:00Z",
        reservationStatus: "provisioning",
        runnerEndpoints: ["http://10.0.0.1:55880", "http://10.0.0.2:55880"],
        createdAt: "2026-05-14T09:55:00Z",
        updatedAt: "2026-05-14T09:56:00Z",
      },
    };

    render(
      <LoadTestTab
        pipeline={pipeline}
        projectId="project-1"
        pipelineIndex={0}
      />,
    );

    expect(screen.getByTestId("load-provisioning-status")).toBeInTheDocument();
    expect(screen.getByText("Provisioning Kubernetes runners")).toBeInTheDocument();
    expect(screen.getByText(/2.*4/)).toBeInTheDocument();
    expect(screen.getByText(/rr-1/)).toBeInTheDocument();
    expect(screen.getByTestId("load-provisioning-icon")).toHaveClass("text-white");
    expect(screen.getByTestId("load-provisioning-icon").parentElement).not.toHaveClass("bg-primary/10");
  });

  it("treats provisioning history entries as active live executions", () => {
    mockedStoreState = {
      state: "provisioning",
      liveState: "provisioning",
      provisioningStatus: null,
      provisioningStartedAt: Date.now() - 1_000,
    };
    mockedRuns = defaultRuns("provisioning");

    render(
      <LoadTestTab
        pipeline={pipeline}
        projectId="project-1"
        pipelineIndex={0}
      />,
    );

    fireEvent.click(screen.getByText("1 reqs"));

    expect(backToLive).toHaveBeenCalledTimes(1);
  });
});
