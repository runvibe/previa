import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LoadTestTab } from "@/components/LoadTestTab";
import type { Pipeline } from "@/types/pipeline";

const loadHistory = vi.fn();
let isMobile = false;

vi.mock("@/hooks/use-mobile", () => ({
  useIsMobile: () => isMobile,
}));

vi.mock("@/stores/useLoadTestHistoryStore", () => ({
  useLoadTestHistoryStore: () => ({
    state: "idle",
    liveState: "idle",
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
    runs: [
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
        state: "completed",
        timestamp: "2026-05-02T00:00:00.000Z",
      },
    ],
    activeRunId: null,
    viewingHistoricRun: false,
    loadHistory,
    clearHistory: vi.fn(),
    runTest: vi.fn(),
    resetTest: vi.fn(),
    cancelTest: vi.fn(),
    backToLive: vi.fn(),
    reconnectExecution: vi.fn(),
    selectHistoricRun: vi.fn(),
  }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    i18n: { language: "en-US" },
    t: (key: string) => {
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
      };
      return labels[key] ?? key;
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

describe("LoadTestTab", () => {
  beforeEach(() => {
    isMobile = false;
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
});
