import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LoadTestResultsPanel } from "@/components/LoadTestResultsPanel";
import { buildRpsChartData } from "@/lib/load-rps-chart";
import type { LoadTestMetrics, WaveLoadConfig } from "@/types/load-test";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, number>) => {
      if (key === "loadTestResults.elapsedLabel") return "TIME";
      return key;
    },
  }),
}));

describe("LoadTestResultsPanel", () => {
  const emptyMetrics: LoadTestMetrics = {
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
    startTime: 1_000,
    elapsedMs: 0,
  };

  it("shows runner resource charts when a single runtime sample exists", () => {
    const metrics: LoadTestMetrics = {
      totalSent: 1,
      totalSuccess: 1,
      totalError: 0,
      avgLatency: 0,
      p95: 0,
      p99: 0,
      rps: 1,
      latencyHistory: [],
      rpsHistory: [],
      runnerResourceHistory: [
        {
          node: "runner-a",
          timestamp: 1_000,
          elapsedMs: 250,
          cpuUsagePercent: 12.5,
          memoryBytes: 104_857_600,
          memoryMb: 100,
          networkTxBytes: 2_048,
          networkRxBytes: 4_096,
          networkTotalBytes: 6_144,
          networkTotalKb: 6,
        },
      ],
      startTime: 750,
      elapsedMs: 250,
    };

    render(<LoadTestResultsPanel metrics={metrics} state="running" totalRequests={10} />);

    expect(screen.getByText("Runner CPU")).toBeInTheDocument();
    expect(screen.getByText("Runner memory")).toBeInTheDocument();
    expect(screen.getByText("Runner network")).toBeInTheDocument();
    expect(screen.getAllByText("runner-a").length).toBeGreaterThan(0);
  });

  it("shows elapsed time as a metric card instead of loose footer text", () => {
    const metrics: LoadTestMetrics = {
      totalSent: 10,
      totalSuccess: 10,
      totalError: 0,
      avgLatency: 100,
      p95: 150,
      p99: 200,
      rps: 2,
      latencyHistory: [],
      rpsHistory: [],
      runnerResourceHistory: [],
      startTime: 1_000,
      elapsedMs: 1_500,
    };

    render(<LoadTestResultsPanel metrics={metrics} state="completed" totalRequests={10} />);

    expect(screen.getByText("2s")).toBeInTheDocument();
    expect(screen.getByText("TIME")).toBeInTheDocument();
    expect(screen.queryByText(/elapsed/i)).not.toBeInTheDocument();
  });

  it("renders load error samples", () => {
    render(
      <LoadTestResultsPanel
        metrics={{
          ...emptyMetrics,
          totalSent: 10,
          totalError: 10,
          errors: ["runner-a create_user HTTP 409: HTTP 409 Conflict (x10)"],
        }}
        state="completed"
        totalRequests={0}
      />,
    );

    expect(screen.getByText("loadTestResults.errorSamples")).toBeInTheDocument();
    expect(screen.getByText(/create_user HTTP 409/)).toBeInTheDocument();
  });

  it("shows the configured wave profile on wave load results", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 30_000, intensity: 80 },
        { atMs: 60_000, intensity: 25 },
      ],
      interpolation: "smooth",
      gracePeriodMs: 30_000,
    };

    render(
      <LoadTestResultsPanel
        metrics={emptyMetrics}
        state="completed"
        totalRequests={0}
        config={config}
      />,
    );

    expect(screen.getByText("loadTestResults.configuredWave")).toBeInTheDocument();
    expect(screen.getByTestId("configured-wave-chart")).toBeInTheDocument();
    expect(screen.getByText("10%")).toBeInTheDocument();
    expect(screen.getByText("80%")).toBeInTheDocument();
    expect(screen.getByText("25%")).toBeInTheDocument();
  });

  it("builds the RPS chart from interval throughput with target RPS", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      runnerMaxRps: 200,
      rpsHistory: [
        { timestamp: 1_000, rps: 0, totalSent: 0 },
        { timestamp: 1_500, rps: 4, totalSent: 5, targetRpsLimit: 20 },
        { timestamp: 2_500, rps: 10, totalSent: 30, targetRpsLimit: 80 },
      ],
    };

    expect(buildRpsChartData(metrics, null)).toEqual({
      data: [
        { time: 0, rpsTotal: 0, targetRpsLimit: undefined },
        { time: 1, rpsTotal: 10, targetRpsLimit: 20 },
        { time: 2, rpsTotal: 25, targetRpsLimit: 80 },
      ],
      runnerSeries: [],
      usesHttpRps: false,
    });
  });

  it("prefers started throughput for the RPS chart when it is available", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        { timestamp: 1_000, rps: 0, totalSent: 0, totalStarted: 0 },
        { timestamp: 2_000, rps: 0, totalSent: 0, totalStarted: 50, targetRpsLimit: 50 },
        { timestamp: 3_000, rps: 2, totalSent: 2, totalStarted: 100, targetRpsLimit: 50 },
      ],
    };

    expect(buildRpsChartData(metrics, null)).toEqual({
      data: [
        { time: 0, rpsTotal: 0, targetRpsLimit: undefined },
        { time: 1, rpsTotal: 50, targetRpsLimit: 50 },
        { time: 2, rpsTotal: 50, targetRpsLimit: 50 },
      ],
      runnerSeries: [],
      usesHttpRps: false,
    });
  });

  it("estimates target RPS from the configured wave when history has no target samples", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 1_000, intensity: 50 },
      ],
      interpolation: "linear",
    };
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      runnerMaxRps: 200,
      rpsHistory: [
        { timestamp: 1_000, rps: 0 },
        { timestamp: 1_500, rps: 4 },
        { timestamp: 2_000, rps: 10 },
      ],
    };

    expect(buildRpsChartData(metrics, config)).toEqual({
      data: [
        { time: 0, rpsTotal: 0, targetRpsLimit: 20 },
        { time: 1, rpsTotal: 4, targetRpsLimit: 60 },
        { time: 1, rpsTotal: 10, targetRpsLimit: 100 },
      ],
      runnerSeries: [],
      usesHttpRps: false,
    });
  });

  it("builds HTTP RPS chart lines from per-runner started counters", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          rps: 0,
          httpStarted: 0,
          runners: [
            { runnerId: "runner-a", httpStarted: 0, rps: 0 },
            { runnerId: "runner-b", httpStarted: 0, rps: 0 },
          ],
        },
        {
          timestamp: 2_000,
          rps: 0,
          httpStarted: 30,
          targetRpsLimit: 40,
          runners: [
            { runnerId: "runner-a", httpStarted: 10, rps: 10 },
            { runnerId: "runner-b", httpStarted: 20, rps: 20 },
          ],
        },
      ],
    };

    expect(buildRpsChartData(metrics, null)).toEqual({
      data: [
        { time: 0, rpsTotal: 0, runner0: 0, runner1: 0, targetRpsLimit: undefined },
        { time: 1, rpsTotal: 30, runner0: 10, runner1: 20, targetRpsLimit: 40 },
      ],
      runnerSeries: [
        { key: "runner0", label: "runner-a" },
        { key: "runner1", label: "runner-b" },
      ],
      usesHttpRps: true,
    });
  });

  it("does not use cumulative runner RPS for the first HTTP chart sample", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          rps: 50_000,
          httpStarted: 300,
          runners: [
            { runnerId: "runner-a", httpStarted: 100, rps: 16_000 },
            { runnerId: "runner-b", httpStarted: 200, rps: 34_000 },
          ],
        },
      ],
    };

    expect(buildRpsChartData(metrics, null).data[0]).toEqual({
      time: 0,
      rpsTotal: 0,
      runner0: 0,
      runner1: 0,
      targetRpsLimit: undefined,
    });
  });

  it("uses the configured wave for target RPS instead of stale samples", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 3_000, intensity: 80 },
      ],
      interpolation: "smooth",
    };
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      runnerMaxRps: 3_000,
      rpsHistory: [
        { timestamp: 1_000, rps: 0, httpStarted: 0, targetRpsLimit: 300 },
        { timestamp: 4_000, rps: 0, httpStarted: 2_400, targetRpsLimit: 300 },
      ],
    };

    expect(buildRpsChartData(metrics, config).data[1].targetRpsLimit).toBe(2400);
  });

  it("drops configured target RPS to zero after the wave duration", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 3_000, intensity: 80 },
      ],
      interpolation: "smooth",
    };
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      runnerMaxRps: 3_000,
      rpsHistory: [
        { timestamp: 1_000, rps: 0, httpStarted: 0 },
        { timestamp: 4_000, rps: 0, httpStarted: 2_400 },
        { timestamp: 5_000, rps: 0, httpStarted: 2_400 },
      ],
    };

    expect(buildRpsChartData(metrics, config).data[2].targetRpsLimit).toBe(0);
  });

  it("shows wave dispatch adherence metrics when available", () => {
    render(
      <LoadTestResultsPanel
        metrics={{
          ...emptyMetrics,
          curveAdherence: 95,
          missedStarts: 20,
          readyRequests: 50,
          runtimeLaggedStarts: 7,
          dependencyLimitedStarts: 3,
        }}
        state="running"
        totalRequests={0}
      />,
    );

    expect(screen.getByText("loadTestResults.curveAdherence")).toBeInTheDocument();
    expect(screen.getByText("95%")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.missedStarts")).toBeInTheDocument();
    expect(screen.getByText("20")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.readyRequests")).toBeInTheDocument();
    expect(screen.getByText("50")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.runtimeLaggedStarts")).toBeInTheDocument();
    expect(screen.getByText("7")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.dependencyLimitedStarts")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("shows actual and target RPS legend when target data exists", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        { timestamp: 1_000, rps: 0, totalSent: 0, targetRpsLimit: 10 },
        { timestamp: 2_000, rps: 20, totalSent: 20, targetRpsLimit: 80 },
      ],
    };

    render(<LoadTestResultsPanel metrics={metrics} state="running" totalRequests={0} />);

    expect(screen.getByText("loadTestResults.rpsActual")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.rpsTarget")).toBeInTheDocument();
    expect(screen.getByTestId("rps-target-legend")).toBeInTheDocument();
  });
});
