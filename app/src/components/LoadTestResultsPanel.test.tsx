import { fireEvent, render, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { LoadTestResultsPanel } from "@/components/LoadTestResultsPanel";
import { TooltipProvider } from "@/components/ui/tooltip";
import { buildLifecycleChartData } from "@/lib/load-lifecycle-chart";
import { buildRpsChartData, buildWaveSecondMarkers } from "@/lib/load-rps-chart";
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
  function expectMetricValue(label: string, value: string) {
    const labelElement = screen.getByText(label);
    expect(labelElement.previousElementSibling).toHaveTextContent(value);
  }

  function expectBefore(first: HTMLElement, second: HTMLElement) {
    expect(Boolean(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING)).toBe(true);
  }

  function renderWithTooltipProvider(ui: ReactElement) {
    return render(<TooltipProvider>{ui}</TooltipProvider>);
  }

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

  it("shows executive outcome, live traffic, response, wave plan, diagnostics, and runner infrastructure in customer priority order", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      totalSent: 1200,
      totalSuccess: 1198,
      totalError: 2,
      rps: 80,
      avgLatency: 42,
      p95: 90,
      p99: 110,
      elapsedMs: 15_000,
      targetIntensity: 50,
      targetRpsLimit: 100,
      curveAdherence: 99.7,
      schedulerLaggedStarts: 3,
      readyRequests: 0,
      dispatchSubmitted: 1200,
      dispatchStarted: 1200,
      httpStarted: 1200,
      httpSendReturned: 1200,
      responseBodyCompleted: 1198,
      senderQueueDepth: 0,
      senderStartLagP95Ms: 1,
      httpSendDurationP95Ms: 1,
      responseObservationDurationP95Ms: 2,
      schedulerLagMs: 12,
      inFlight: 2,
      latencyHistory: [
        { index: 1, latency: 40, timestamp: 1_000 },
        { index: 2, latency: 42, timestamp: 2_000 },
      ],
      rpsHistory: [
        { timestamp: 1_000, elapsedMs: 1_000, rps: 50, totalSent: 50 },
        { timestamp: 2_000, elapsedMs: 2_000, rps: 80, totalSent: 130 },
      ],
      lifecycleBuckets: [
        { elapsedMs: 1_000, planned: 50, httpStarted: 50 },
        { elapsedMs: 2_000, planned: 80, httpStarted: 80 },
      ],
      runnerResourceHistory: [
        {
          node: "runner-a",
          timestamp: 1_000,
          elapsedMs: 1_000,
          cpuUsagePercent: 10,
          memoryBytes: 134_217_728,
          memoryMb: 128,
          networkRxBytes: 1_024,
          networkTxBytes: 2_048,
          networkTotalBytes: 3_072,
          networkTotalKb: 3,
        },
      ],
    };

    renderWithTooltipProvider(
      <LoadTestResultsPanel
        metrics={metrics}
        state="completed"
        totalRequests={0}
        config={{
          points: [
            { atMs: 0, intensity: 10 },
            { atMs: 2000, intensity: 80 },
          ],
          interpolation: "linear",
        }}
      />,
    );

    const outcome = screen.getByTestId("load-results-outcome");
    const wave = screen.getByTestId("load-results-wave");
    const response = screen.getByTestId("load-results-response");
    const wavePlan = screen.getByTestId("load-results-wave-plan");
    const generator = screen.getByTestId("load-results-generator");
    const runnerInfra = screen.getByTestId("load-results-runner-infra");

    expectBefore(outcome, wave);
    expectBefore(wave, response);
    expectBefore(response, wavePlan);
    expectBefore(wavePlan, generator);
    expectBefore(generator, runnerInfra);
    expect(screen.getByText("loadTestResults.httpStarted")).toBeInTheDocument();
    expectBefore(wavePlan, screen.getByText("loadTestResults.httpStarted").closest("div")!);
  });

  it("prioritizes observed RPS and application response before configured wave and lifecycle diagnostics", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      totalSent: 100,
      totalSuccess: 100,
      rps: 50,
      elapsedMs: 2000,
      avgLatency: 40,
      p95: 80,
      p99: 100,
      statusCodeBuckets: [
        { elapsedMs: 1_000, code: "200", count: 20 },
        { elapsedMs: 2_000, code: "409", count: 3 },
      ],
      rpsHistory: [
        { timestamp: 1_000, elapsedMs: 1_000, rps: 20, totalSent: 20 },
        { timestamp: 2_000, elapsedMs: 2_000, rps: 50, totalSent: 70 },
      ],
      lifecycleBuckets: [
        { elapsedMs: 1_000, planned: 20, httpStarted: 20 },
        { elapsedMs: 2_000, planned: 50, httpStarted: 50 },
      ],
    };

    render(
      <LoadTestResultsPanel
        metrics={metrics}
        state="completed"
        totalRequests={0}
        config={{
          points: [
            { atMs: 0, intensity: 10 },
            { atMs: 2000, intensity: 80 },
          ],
          interpolation: "linear",
        }}
      />,
    );

    expectBefore(screen.getByTestId("rps-over-time-chart"), screen.getByTestId("configured-wave-chart"));
    expectBefore(screen.getByTestId("status-code-timeline-chart"), screen.getByTestId("configured-wave-chart"));
    expectBefore(screen.getByTestId("configured-wave-chart"), screen.getByTestId("wave-lifecycle-chart"));
  });

  it("does not render an empty runner infrastructure section", () => {
    render(<LoadTestResultsPanel metrics={emptyMetrics} state="completed" totalRequests={0} />);

    expect(screen.queryByTestId("load-results-runner-infra")).not.toBeInTheDocument();
  });

  it("keeps runner endpoints collapsed behind a compact summary", () => {
    renderWithTooltipProvider(
      <LoadTestResultsPanel
        metrics={emptyMetrics}
        state="completed"
        totalRequests={0}
        nodesInfo={{
          nodesUsed: 6,
          nodesFound: 0,
          nodeNames: [
            "http://previa-runner-reserve-0.previa-runner-reserve.previa.svc.cluster.local:7373",
            "http://previa-runner-reserve-1.previa-runner-reserve.previa.svc.cluster.local:7373",
          ],
        }}
      />,
    );

    expect(screen.getByText("loadTestResults.nodes_plural")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.dynamicReservation")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.runnersUsed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /loadTestResults.showEndpoints/ })).toBeInTheDocument();
    expect(screen.getByTestId("load-results-nodes-icon")).toHaveClass("text-white");
    expect(screen.getByTestId("load-results-nodes-icon").parentElement).not.toHaveClass("bg-primary/10", "border-primary/20");
    expect(screen.queryByText("runner-0")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /loadTestResults.showEndpoints/ }));

    expect(screen.getByText("runner-0")).toBeInTheDocument();
    expect(screen.getByText("runner-1")).toBeInTheDocument();
    expect(screen.getByText("http://previa-runner-reserve-0.previa-runner-reserve.previa.svc.cluster.local:7373")).toBeInTheDocument();
    expect(screen.getByTestId("load-results-runner-endpoints-scroll")).toHaveClass("max-h-[21rem]", "overflow-y-auto");
  });

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

  it("builds lifecycle chart rows from cumulative counters", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          elapsedMs: 0,
          rps: 0,
          scheduledStarts: 0,
          sendStarted: 0,
          httpStarted: 0,
          httpSendReturned: 0,
          responseBodyCompleted: 0,
        },
        {
          timestamp: 2_000,
          elapsedMs: 1_000,
          rps: 0,
          scheduledStarts: 100,
          sendStarted: 98,
          httpStarted: 97,
          httpSendReturned: 40,
          responseBodyCompleted: 10,
          senderStartLagMsMax: 0,
          httpSendDurationMsMax: 0,
          responseObservationDurationMsMax: 0,
        },
        {
          timestamp: 3_000,
          elapsedMs: 2_000,
          rps: 0,
          scheduledStarts: 250,
          sendStarted: 245,
          httpStarted: 244,
          httpSendReturned: 90,
          responseBodyCompleted: 20,
        },
      ],
    };

    expect(buildLifecycleChartData(metrics)).toEqual({
      data: [
        {
          time: 1,
          planned: 100,
          sendStarted: 98,
          httpStarted: 97,
          httpSendReturned: 40,
          responseBodyCompleted: 10,
          senderStartLagMsMax: 0,
          httpSendDurationMsMax: 0,
          responseObservationDurationMsMax: 0,
        },
        {
          time: 2,
          planned: 150,
          sendStarted: 147,
          httpStarted: 147,
          httpSendReturned: 50,
          responseBodyCompleted: 10,
          senderStartLagMsMax: 0,
          httpSendDurationMsMax: 0,
          responseObservationDurationMsMax: 0,
        },
      ],
      series: [
        { key: "planned", labelKey: "loadTestResults.lifecyclePlanned", tone: "planned", axis: "count" },
        { key: "sendStarted", labelKey: "loadTestResults.lifecycleSendStarted", tone: "send", axis: "count" },
        { key: "httpStarted", labelKey: "loadTestResults.lifecycleHttpStarted", tone: "http", axis: "count" },
        { key: "httpSendReturned", labelKey: "loadTestResults.lifecycleHttpSendReturned", tone: "returned", axis: "count" },
        { key: "responseBodyCompleted", labelKey: "loadTestResults.lifecycleBodyCompleted", tone: "body", axis: "count" },
      ],
    });
  });

  it("builds lifecycle chart with lag series from direct buckets", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      lifecycleBuckets: [
        {
          elapsedMs: 2_000,
          planned: 10,
          httpStarted: 10,
          senderStartLagMsMax: 12,
          httpSendDurationMsMax: 34,
          responseObservationDurationMsMax: 56,
        },
      ],
    };

    const chart = buildLifecycleChartData(metrics);

    expect(chart.data[0]).toMatchObject({
      time: 2,
      senderStartLagMsMax: 12,
      httpSendDurationMsMax: 34,
      responseObservationDurationMsMax: 56,
    });
    expect(chart.series.map((series) => series.key)).toContain("senderStartLagMsMax");
    expect(chart.series.map((series) => series.key)).toContain("httpSendDurationMsMax");
    expect(chart.series.map((series) => series.key)).toContain("responseObservationDurationMsMax");
  });

  it("builds lifecycle chart from direct lifecycle buckets", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 2_000,
          elapsedMs: 1_000,
          rps: 0,
          lifecycleBucket: {
            elapsedMs: 1_000,
            planned: 30,
            sendStarted: 29,
            httpStarted: 28,
            httpSendReturned: 20,
            responseBodyCompleted: 10,
          },
        },
      ],
    };

    expect(buildLifecycleChartData(metrics).data).toEqual([
      {
        time: 1,
        planned: 30,
        sendStarted: 29,
        httpStarted: 28,
        httpSendReturned: 20,
        responseBodyCompleted: 10,
        senderStartLagMsMax: 0,
        httpSendDurationMsMax: 0,
        responseObservationDurationMsMax: 0,
      },
    ]);
  });

  it("uses direct dispatch buckets for the HTTP started lifecycle line when available", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          elapsedMs: 0,
          rps: 0,
          scheduledStarts: 100,
          dispatchBucket: 99,
          sendStarted: 100,
          httpStarted: 10_000,
        },
        {
          timestamp: 2_000,
          elapsedMs: 1_000,
          rps: 0,
          scheduledStarts: 200,
          dispatchBucket: 125,
          sendStarted: 200,
          httpStarted: 20_000,
        },
      ],
    };

    expect(buildLifecycleChartData(metrics).data).toEqual([
      {
        time: 0,
        planned: 100,
        sendStarted: 100,
        httpStarted: 99,
        httpSendReturned: 0,
        responseBodyCompleted: 0,
        senderStartLagMsMax: 0,
        httpSendDurationMsMax: 0,
        responseObservationDurationMsMax: 0,
      },
      {
        time: 1,
        planned: 100,
        sendStarted: 100,
        httpStarted: 125,
        httpSendReturned: 0,
        responseBodyCompleted: 0,
        senderStartLagMsMax: 0,
        httpSendDurationMsMax: 0,
        responseObservationDurationMsMax: 0,
      },
    ]);
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

  it("renders wave lifecycle chart when lifecycle history exists", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          elapsedMs: 0,
          rps: 0,
          scheduledStarts: 0,
          sendStarted: 0,
          httpStarted: 0,
          httpSendReturned: 0,
          responseBodyCompleted: 0,
        },
        {
          timestamp: 2_000,
          elapsedMs: 1_000,
          rps: 0,
          scheduledStarts: 100,
          sendStarted: 99,
          httpStarted: 99,
          httpSendReturned: 30,
          responseBodyCompleted: 5,
        },
      ],
    };

    render(<LoadTestResultsPanel metrics={metrics} state="running" totalRequests={0} />);

    expect(screen.getByTestId("wave-lifecycle-chart")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.waveLifecycle")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.lifecyclePlanned")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.lifecycleHttpStarted")).toBeInTheDocument();
  });

  it("renders lifecycle lag series with separate count and millisecond axes", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      lifecycleBuckets: [
        {
          elapsedMs: 1_000,
          planned: 100,
          sendStarted: 98,
          httpStarted: 97,
          httpSendReturned: 30,
          responseBodyCompleted: 5,
          senderStartLagMsMax: 12,
          httpSendDurationMsMax: 34,
          responseObservationDurationMsMax: 56,
        },
      ],
    };

    render(<LoadTestResultsPanel metrics={metrics} state="completed" totalRequests={0} />);

    expect(screen.getByTestId("wave-lifecycle-chart")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.lifecycleSenderStartLag")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.lifecycleHttpSendDuration")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.lifecycleResponseObservation")).toBeInTheDocument();
  });

  it("renders status codes over time when status buckets are available", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      statusCodeBuckets: [
        { elapsedMs: 1_000, code: "200", count: 10 },
        { elapsedMs: 1_000, code: "502", count: 2 },
        { elapsedMs: 2_000, code: "network_error", count: 1 },
      ],
    };

    render(<LoadTestResultsPanel metrics={metrics} state="completed" totalRequests={0} />);

    expect(screen.getByTestId("status-code-timeline-chart")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.statusCodeTimeline")).toBeInTheDocument();
    expect(screen.getByText("200")).toBeInTheDocument();
    expect(screen.getByText("502")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.networkError")).toBeInTheDocument();
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
        { time: 0, rpsTotal: 5, targetRpsLimit: undefined },
        { time: 1, rpsTotal: 25, targetRpsLimit: 80 },
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
        { atMs: 2_000, intensity: 50 },
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
        { time: 0, rpsTotal: 4, targetRpsLimit: 20 },
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

  it("groups irregular HTTP samples into one-second RPS buckets", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          elapsedMs: 0,
          rps: 0,
          dispatchStarted: 0,
          runners: [
            { runnerId: "runner-a", dispatchStarted: 0 },
            { runnerId: "runner-b", dispatchStarted: 0 },
          ],
        },
        {
          timestamp: 1_500,
          elapsedMs: 500,
          rps: 0,
          dispatchStarted: 10,
          runners: [
            { runnerId: "runner-a", dispatchStarted: 4 },
            { runnerId: "runner-b", dispatchStarted: 6 },
          ],
        },
        {
          timestamp: 2_500,
          elapsedMs: 1_500,
          rps: 0,
          dispatchStarted: 35,
          targetRpsLimit: 25,
          runners: [
            { runnerId: "runner-a", dispatchStarted: 14 },
            { runnerId: "runner-b", dispatchStarted: 21 },
          ],
        },
      ],
    };

    expect(buildRpsChartData(metrics, null)).toEqual({
      data: [
        { time: 0, rpsTotal: 10, runner0: 4, runner1: 6, targetRpsLimit: undefined },
        { time: 1, rpsTotal: 25, runner0: 10, runner1: 15, targetRpsLimit: 25 },
      ],
      runnerSeries: [
        { key: "runner0", label: "runner-a" },
        { key: "runner1", label: "runner-b" },
      ],
      usesHttpRps: true,
    });
  });

  it("uses runner dispatch buckets instead of cumulative snapshot deltas when available", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          elapsedMs: 0,
          rps: 0,
          dispatchStarted: 0,
          dispatchBucket: 12,
          runners: [
            { runnerId: "runner-a", dispatchStarted: 0, dispatchBucket: 5 },
            { runnerId: "runner-b", dispatchStarted: 0, dispatchBucket: 7 },
          ],
        },
        {
          timestamp: 3_000,
          elapsedMs: 2_000,
          rps: 0,
          dispatchStarted: 1_000,
          dispatchBucket: 20,
          targetRpsLimit: 20,
          runners: [
            { runnerId: "runner-a", dispatchStarted: 400, dispatchBucket: 8 },
            { runnerId: "runner-b", dispatchStarted: 600, dispatchBucket: 12 },
          ],
        },
      ],
    };

    expect(buildRpsChartData(metrics, null)).toEqual({
      data: [
        { time: 0, rpsTotal: 12, runner0: 5, runner1: 7, targetRpsLimit: undefined },
        { time: 2, rpsTotal: 20, runner0: 8, runner1: 12, targetRpsLimit: 20 },
      ],
      runnerSeries: [
        { key: "runner0", label: "runner-a" },
        { key: "runner1", label: "runner-b" },
      ],
      usesHttpRps: true,
    });
  });

  it("does not double count repeated direct dispatch bucket snapshots", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          elapsedMs: 0,
          rps: 0,
          dispatchBucket: 12,
          runners: [
            { runnerId: "runner-a", dispatchBucket: 5 },
            { runnerId: "runner-b", dispatchBucket: 7 },
          ],
        },
        {
          timestamp: 1_500,
          elapsedMs: 0,
          rps: 0,
          dispatchBucket: 12,
          runners: [
            { runnerId: "runner-a", dispatchBucket: 5 },
            { runnerId: "runner-b", dispatchBucket: 7 },
          ],
        },
      ],
    };

    expect(buildRpsChartData(metrics, null).data).toEqual([
      { time: 0, rpsTotal: 12, runner0: 5, runner1: 7, targetRpsLimit: undefined },
    ]);
  });

  it("prefers dispatch started counters over HTTP started counters", () => {
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      rpsHistory: [
        {
          timestamp: 1_000,
          rps: 0,
          httpStarted: 0,
          dispatchStarted: 0,
          runners: [
            { runnerId: "runner-a", httpStarted: 0, dispatchStarted: 0, rps: 0 },
            { runnerId: "runner-b", httpStarted: 0, dispatchStarted: 0, rps: 0 },
          ],
        },
        {
          timestamp: 2_000,
          rps: 0,
          httpStarted: 10,
          dispatchStarted: 30,
          targetRpsLimit: 40,
          runners: [
            { runnerId: "runner-a", httpStarted: 1, dispatchStarted: 10, rps: 1 },
            { runnerId: "runner-b", httpStarted: 2, dispatchStarted: 20, rps: 2 },
          ],
        },
      ],
    };

    expect(buildRpsChartData(metrics, null).data[1]).toEqual({
      time: 1,
      rpsTotal: 30,
      runner0: 10,
      runner1: 20,
      targetRpsLimit: 40,
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

  it("uses the configured wave for target RPS instead of stale samples before the wave ends", () => {
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

    expect(buildRpsChartData(metrics, config).data.find((point) => point.time === 2)?.targetRpsLimit).toBe(1855.6);
  });

  it("does not compare active target RPS at the exact wave end bucket", () => {
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

    expect(buildRpsChartData(metrics, config).data.find((point) => point.time === 3)?.targetRpsLimit).toBeUndefined();
    expect(buildRpsChartData(metrics, config).data.find((point) => point.time === 4)?.targetRpsLimit).toBeUndefined();
  });

  it("scales configured target RPS for a partial final wave bucket", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 3_500, intensity: 80 },
      ],
      interpolation: "step",
    };
    const metrics: LoadTestMetrics = {
      ...emptyMetrics,
      runnerMaxRps: 3_000,
      rpsHistory: [
        { timestamp: 1_000, rps: 0, httpStarted: 0 },
        { timestamp: 4_000, rps: 0, httpStarted: 300 },
      ],
    };

    expect(buildRpsChartData(metrics, config).data.find((point) => point.time === 3)?.targetRpsLimit).toBe(150);
  });

  it("builds per-second planned request markers from constant wave intensity", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 50 },
        { atMs: 2_000, intensity: 50 },
      ],
      interpolation: "linear",
      runnerMaxRps: 600,
    };

    expect(buildWaveSecondMarkers(config, { runnerCount: 3 })).toEqual([
      { second: 1, plannedRequests: 900, showLabel: true },
      { second: 2, plannedRequests: 900, showLabel: true },
    ]);
  });

  it("samples linear wave intensity in thirds before summing each second", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 0 },
        { atMs: 3_000, intensity: 100 },
      ],
      interpolation: "linear",
      runnerMaxRps: 600,
    };

    expect(buildWaveSecondMarkers(config, { runnerCount: 3 })).toEqual([
      { second: 1, plannedRequests: 300, showLabel: true },
      { second: 2, plannedRequests: 900, showLabel: true },
      { second: 3, plannedRequests: 1500, showLabel: true },
    ]);
  });

  it("scales the final planned request marker for a partial second", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 2_500, intensity: 10 },
      ],
      interpolation: "step",
      runnerMaxRps: 600,
    };

    expect(buildWaveSecondMarkers(config, { runnerCount: 3 })).toEqual([
      { second: 1, plannedRequests: 180, showLabel: true },
      { second: 2, plannedRequests: 180, showLabel: true },
      { second: 2.5, plannedRequests: 90, showLabel: true },
    ]);
  });

  it("shows wave dispatch adherence metrics when available", () => {
    render(
      <LoadTestResultsPanel
        metrics={{
          ...emptyMetrics,
          curveAdherence: 95,
          missedStarts: 20,
          readyRequests: 50,
          dispatchStarted: 120,
          schedulerLagMs: 400,
          schedulerLaggedStarts: 12,
          outstandingRequests: 90,
          dispatcherLaggedStarts: 9,
          runtimeLaggedStarts: 7,
          senderLaggedStarts: 5,
          senderQueueDepth: 11,
          senderStartLagP95Ms: 12,
          httpSendDurationP95Ms: 34,
          responseObservationDurationP95Ms: 56,
          dependencyLimitedStarts: 3,
          slotEnqueued: 118,
          requestPrepared: 117,
          requestEnqueued: 116,
          sendTaskSpawned: 115,
          sendStarted: 114,
          httpStarted: 113,
          lifecycleBuckets: [
            { elapsedMs: 0, planned: 100, httpStarted: 99 },
            { elapsedMs: 1_000, planned: 100, httpStarted: 100 },
          ],
        }}
        state="running"
        totalRequests={0}
      />,
    );

    expect(screen.getByText("loadTestResults.curveAdherence")).toBeInTheDocument();
    expect(screen.getByText("95%")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.actualMissedStarts")).toBeInTheDocument();
    expect(screen.getByText("1")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.compensatedSchedulerStarts")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.readyRequests")).toBeInTheDocument();
    expect(screen.getByText("50")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.dispatchStarted")).toBeInTheDocument();
    expect(screen.getByText("120")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.schedulerLagMs")).toBeInTheDocument();
    expect(screen.getByText("400ms")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.observerBacklog")).toBeInTheDocument();
    expect(screen.getByText("90")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.dispatcherLaggedStarts")).toBeInTheDocument();
    expect(screen.getByText("9")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.runtimeLaggedStarts")).toBeInTheDocument();
    expect(screen.getByText("7")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.senderLaggedStarts")).toBeInTheDocument();
    expect(screen.getByText("5")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.senderQueueDepth")).toBeInTheDocument();
    expect(screen.getByText("11")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.senderStartLagP95Ms")).toBeInTheDocument();
    expect(screen.getByText("12ms")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.httpSendDurationP95Ms")).toBeInTheDocument();
    expect(screen.getByText("34ms")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.responseObservationDurationP95Ms")).toBeInTheDocument();
    expect(screen.getByText("56ms")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.dependencyLimitedStarts")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.slotEnqueued")).toBeInTheDocument();
    expect(screen.getByText("118")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.requestPrepared")).toBeInTheDocument();
    expect(screen.getByText("117")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.requestEnqueued")).toBeInTheDocument();
    expect(screen.getByText("116")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.sendTaskSpawned")).toBeInTheDocument();
    expect(screen.getByText("115")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.sendStarted")).toBeInTheDocument();
    expect(screen.getByText("114")).toBeInTheDocument();
    expect(screen.getByText("loadTestResults.httpStarted")).toBeInTheDocument();
    expect(screen.getByText("113")).toBeInTheDocument();
  });

  it("shows compensated scheduler delay without marking the wave as actually lost", () => {
    render(
      <LoadTestResultsPanel
        metrics={{
          ...emptyMetrics,
          curveAdherence: 99.9,
          schedulerLagMs: 3107,
          schedulerLaggedStarts: 705,
          lifecycleBuckets: [
            { elapsedMs: 0, planned: 300, httpStarted: 300 },
            { elapsedMs: 1_000, planned: 300, httpStarted: 300 },
            { elapsedMs: 2_000, planned: 303, httpStarted: 305 },
            { elapsedMs: 3_000, planned: 303, httpStarted: 301 },
            { elapsedMs: 120_000, planned: 0, httpStarted: 7 },
          ],
        }}
        state="completed"
        totalRequests={0}
      />,
    );

    expect(screen.getByText("loadTestResults.compensatedSchedulerStarts")).toBeInTheDocument();
    expectMetricValue("loadTestResults.compensatedSchedulerStarts", "705");
    expect(screen.getByText("loadTestResults.actualMissedStarts")).toBeInTheDocument();
    expectMetricValue("loadTestResults.actualMissedStarts", "0");
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
