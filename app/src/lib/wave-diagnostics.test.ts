import { describe, expect, it } from "vitest";

import { deriveWaveDiagnostics } from "@/lib/wave-diagnostics";
import type { LoadTestMetrics } from "@/types/load-test";

const baseMetrics: LoadTestMetrics = {
  totalSent: 0,
  totalSuccess: 0,
  totalError: 0,
  avgLatency: 0,
  p95: 0,
  p99: 0,
  rps: 0,
  latencyHistory: [],
  rpsHistory: [],
  errors: [],
  startTime: 0,
  elapsedMs: 0,
};

describe("deriveWaveDiagnostics", () => {
  it("treats compensated scheduler delay as no actual wave loss", () => {
    const diagnostics = deriveWaveDiagnostics({
      ...baseMetrics,
      schedulerLagMs: 3107,
      schedulerLaggedStarts: 705,
      lifecycleBuckets: [
        { elapsedMs: 0, planned: 300, httpStarted: 300 },
        { elapsedMs: 1_000, planned: 300, httpStarted: 300 },
        { elapsedMs: 2_000, planned: 303, httpStarted: 305 },
        { elapsedMs: 3_000, planned: 303, httpStarted: 301 },
        { elapsedMs: 120_000, planned: 0, httpStarted: 7 },
      ],
    });

    expect(diagnostics.actualMissedStarts).toBe(2);
    expect(diagnostics.surplusStarts).toBe(9);
    expect(diagnostics.schedulerDelayWasCompensated).toBe(true);
    expect(diagnostics.hasActualWaveLoss).toBe(false);
  });

  it("reports actual wave loss when planned HTTP starts are not emitted", () => {
    const diagnostics = deriveWaveDiagnostics({
      ...baseMetrics,
      schedulerLagMs: 500,
      schedulerLaggedStarts: 30,
      lifecycleBuckets: [
        { elapsedMs: 0, planned: 100, httpStarted: 100 },
        { elapsedMs: 1_000, planned: 120, httpStarted: 90 },
        { elapsedMs: 2_000, planned: 130, httpStarted: 120 },
      ],
    });

    expect(diagnostics.actualMissedStarts).toBe(40);
    expect(diagnostics.surplusStarts).toBe(0);
    expect(diagnostics.schedulerDelayWasCompensated).toBe(false);
    expect(diagnostics.hasActualWaveLoss).toBe(true);
  });

  it("falls back to cumulative planned and http started counters", () => {
    const diagnostics = deriveWaveDiagnostics({
      ...baseMetrics,
      dispatchSubmitted: 1_000,
      httpStarted: 990,
    });

    expect(diagnostics.actualMissedStarts).toBe(10);
    expect(diagnostics.hasActualWaveLoss).toBe(true);
  });
});
