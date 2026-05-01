import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LoadTestResultsPanel } from "@/components/LoadTestResultsPanel";
import type { LoadTestMetrics } from "@/types/load-test";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, number>) => {
      if (key === "loadTestResults.elapsed") return `${params?.seconds ?? 0}s elapsed`;
      return key;
    },
  }),
}));

describe("LoadTestResultsPanel", () => {
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
});
