import type { LoadLifecycleBucket, LoadTestMetrics } from "@/types/load-test";

export interface WaveDiagnostics {
  plannedStarts: number;
  actualHttpStarts: number;
  actualMissedStarts: number;
  surplusStarts: number;
  hasActualWaveLoss: boolean;
  schedulerDelayWasCompensated: boolean;
}

function isFinalZeroPlanBucket(bucket: LoadLifecycleBucket): boolean {
  return (bucket.planned ?? 0) === 0 && (bucket.httpStarted ?? 0) > 0;
}

function fromLifecycleBuckets(metrics: LoadTestMetrics): WaveDiagnostics | null {
  const buckets = metrics.lifecycleBuckets ?? [];
  if (buckets.length === 0) return null;

  let plannedStarts = 0;
  let actualHttpStarts = 0;
  let actualMissedStarts = 0;
  let surplusStarts = 0;

  for (const bucket of buckets) {
    if (isFinalZeroPlanBucket(bucket)) {
      actualHttpStarts += bucket.httpStarted ?? 0;
      surplusStarts += bucket.httpStarted ?? 0;
      continue;
    }

    const planned = bucket.planned ?? 0;
    const actual = bucket.httpStarted ?? 0;
    plannedStarts += planned;
    actualHttpStarts += actual;

    if (planned > actual) {
      actualMissedStarts += planned - actual;
    } else {
      surplusStarts += actual - planned;
    }
  }

  const hasActualWaveLoss = actualMissedStarts > surplusStarts;

  return {
    plannedStarts,
    actualHttpStarts,
    actualMissedStarts,
    surplusStarts,
    hasActualWaveLoss,
    schedulerDelayWasCompensated:
      typeof metrics.schedulerLaggedStarts === "number"
      && metrics.schedulerLaggedStarts > 0
      && !hasActualWaveLoss,
  };
}

function fromCumulativeCounters(metrics: LoadTestMetrics): WaveDiagnostics {
  const plannedStarts = metrics.dispatchSubmitted ?? metrics.scheduledStarts ?? 0;
  const actualHttpStarts = metrics.httpStarted ?? metrics.totalStarted ?? 0;
  const actualMissedStarts = Math.max(0, plannedStarts - actualHttpStarts);
  const surplusStarts = Math.max(0, actualHttpStarts - plannedStarts);
  const hasActualWaveLoss = actualMissedStarts > 0;

  return {
    plannedStarts,
    actualHttpStarts,
    actualMissedStarts,
    surplusStarts,
    hasActualWaveLoss,
    schedulerDelayWasCompensated:
      typeof metrics.schedulerLaggedStarts === "number"
      && metrics.schedulerLaggedStarts > 0
      && !hasActualWaveLoss,
  };
}

export function deriveWaveDiagnostics(metrics: LoadTestMetrics): WaveDiagnostics {
  return fromLifecycleBuckets(metrics) ?? fromCumulativeCounters(metrics);
}
