import { describe, expect, it } from "vitest";

import { buildStatusCodeChartData } from "@/lib/load-status-code-chart";
import type { LoadTestMetrics } from "@/types/load-test";

function metricsWithStatusBuckets(statusCodeBuckets: LoadTestMetrics["statusCodeBuckets"]): LoadTestMetrics {
  return {
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
    statusCodeBuckets,
    startTime: 0,
    elapsedMs: 0,
  };
}

describe("buildStatusCodeChartData", () => {
  it("groups status code buckets by second and keeps network errors last", () => {
    const chart = buildStatusCodeChartData(metricsWithStatusBuckets([
      { elapsedMs: 1_000, code: "200", count: 10 },
      { elapsedMs: 1_000, code: "502", count: 2 },
      { elapsedMs: 2_000, code: "200", count: 8 },
      { elapsedMs: 2_000, code: "network_error", count: 1 },
    ]));

    expect(chart.series.map((series) => series.code)).toEqual(["200", "502", "network_error"]);
    expect(chart.data).toEqual([
      { time: 1, "200": 10, "502": 2, network_error: 0 },
      { time: 2, "200": 8, "502": 0, network_error: 1 },
    ]);
  });
});
