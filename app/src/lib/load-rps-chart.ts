import type { LoadTestMetrics, RpsPoint, RunnerRpsSample, WaveLoadConfig } from "@/types/load-test";

export interface RpsRunnerSeries {
  key: string;
  label: string;
}

export interface RpsChartRow {
  time: number;
  rpsTotal: number;
  targetRpsLimit?: number;
  [key: string]: number | undefined;
}

export interface RpsChartData {
  data: RpsChartRow[];
  runnerSeries: RpsRunnerSeries[];
  usesHttpRps: boolean;
}

export interface WaveSecondMarker {
  second: number;
  plannedRequests: number;
  showLabel: boolean;
}

export interface WaveSecondMarkerOptions {
  runnerCount?: number;
  runnerMaxRps?: number;
  maxLabels?: number;
}

function roundOne(value: number) {
  return Math.round(value * 10) / 10;
}

export function formatPlannedRequests(value: number) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 10_000) return `${(value / 1_000).toFixed(1)}K`;
  return `${value}`;
}

function dispatchStarted(point: { dispatchStarted?: number; httpStarted?: number }) {
  return point.dispatchStarted ?? point.httpStarted;
}

function elapsedMsForPoint(point: RpsPoint, metrics: LoadTestMetrics) {
  return typeof point.elapsedMs === "number"
    ? point.elapsedMs
    : point.timestamp - metrics.startTime;
}

function bucketSecondForPoint(point: RpsPoint, metrics: LoadTestMetrics) {
  return Math.max(0, Math.floor(elapsedMsForPoint(point, metrics) / 1000));
}

function bucketSpan(previousBucket: number, currentBucket: number) {
  return Math.max(1, currentBucket - previousBucket);
}

function firstBucketToUpdate(previousBucket: number, currentBucket: number) {
  return currentBucket === previousBucket ? currentBucket : previousBucket + 1;
}

function runnerStarted(point: RunnerRpsSample | undefined) {
  return point ? dispatchStarted(point) : undefined;
}

function lifecycleHttpStarted(point: RpsPoint | RunnerRpsSample) {
  return point.lifecycleBucket?.httpStarted;
}

function hasDirectBucket(point: RpsPoint) {
  return typeof lifecycleHttpStarted(point) === "number"
    || typeof point.dispatchBucket === "number"
    || point.runners?.some((runner) =>
      typeof lifecycleHttpStarted(runner) === "number" || typeof runner.dispatchBucket === "number"
    ) === true;
}

function sampleWaveIntensity(config: WaveLoadConfig, elapsedMs: number) {
  const points = [...config.points].sort((a, b) => a.atMs - b.atMs);
  const last = points[points.length - 1];
  if (!last) return undefined;
  if (elapsedMs > last.atMs) return 0;
  if (elapsedMs >= last.atMs) return last.intensity;

  const segment = points
    .slice(0, -1)
    .map((point, index) => [point, points[index + 1]] as const)
    .find(([start, end]) => elapsedMs >= start.atMs && elapsedMs < end.atMs);

  if (!segment) return points[0]?.intensity;

  const [start, end] = segment;
  if (config.interpolation === "step") return start.intensity;

  const span = Math.max(1, end.atMs - start.atMs);
  const rawT = Math.min(1, Math.max(0, (elapsedMs - start.atMs) / span));
  const t = config.interpolation === "smooth" ? rawT * rawT * (3 - 2 * rawT) : rawT;
  return start.intensity + (end.intensity - start.intensity) * t;
}

function roundTimeSeconds(elapsedMs: number) {
  return roundOne(elapsedMs / 1000);
}

export function buildWaveSecondMarkers(
  config: WaveLoadConfig,
  options: WaveSecondMarkerOptions = {},
): WaveSecondMarker[] {
  const sortedPoints = [...config.points].sort((a, b) => a.atMs - b.atMs);
  const durationMs = sortedPoints.at(-1)?.atMs ?? 0;
  const runnerMaxRps = options.runnerMaxRps ?? config.runnerMaxRps ?? 0;
  const runnerCount = Math.max(1, Math.floor(options.runnerCount ?? 1));

  if (durationMs <= 0 || runnerMaxRps <= 0) return [];

  const markers: WaveSecondMarker[] = [];
  for (let bucketStartMs = 0; bucketStartMs < durationMs; bucketStartMs += 1000) {
    const bucketEndMs = Math.min(durationMs, bucketStartMs + 1000);
    const bucketDurationMs = bucketEndMs - bucketStartMs;
    let plannedRequests = 0;

    for (let sliceIndex = 0; sliceIndex < 3; sliceIndex += 1) {
      const sliceStartMs = bucketStartMs + (bucketDurationMs * sliceIndex) / 3;
      const sliceEndMs = bucketStartMs + (bucketDurationMs * (sliceIndex + 1)) / 3;
      const sampleAtMs = sliceStartMs + (sliceEndMs - sliceStartMs) / 2;
      const intensity = sampleWaveIntensity(config, sampleAtMs) ?? 0;
      plannedRequests += runnerMaxRps * runnerCount * (intensity / 100) * ((sliceEndMs - sliceStartMs) / 1000);
    }

    markers.push({
      second: roundTimeSeconds(bucketEndMs),
      plannedRequests: Math.round(plannedRequests),
      showLabel: true,
    });
  }

  const labelEvery = Math.max(1, Math.ceil(markers.length / Math.max(1, options.maxLabels ?? 8)));
  return markers.map((marker, index) => ({
    ...marker,
    showLabel: index % labelEvery === 0 || index === markers.length - 1,
  }));
}

function estimateTargetRpsLimit(
  point: { targetIntensity?: number; targetRpsLimit?: number },
  metrics: LoadTestMetrics,
  waveConfig: WaveLoadConfig | null,
  elapsedMs: number,
) {
  if (waveConfig && typeof metrics.runnerMaxRps === "number") {
    const waveEndMs = [...waveConfig.points].sort((a, b) => a.atMs - b.atMs).at(-1)?.atMs;
    if (typeof waveEndMs === "number") {
      if (elapsedMs >= waveEndMs) return undefined;
    }
    const intensity = sampleWaveIntensity(waveConfig, elapsedMs);
    if (typeof intensity !== "number") return undefined;
    const bucketCoverage = typeof waveEndMs === "number" && elapsedMs + 1000 > waveEndMs
      ? Math.max(0, waveEndMs - elapsedMs) / 1000
      : 1;
    return roundOne((metrics.runnerMaxRps * intensity * bucketCoverage) / 100);
  }

  if (typeof point.targetRpsLimit === "number") return roundOne(point.targetRpsLimit);
  if (typeof metrics.runnerMaxRps !== "number") return undefined;

  const intensity = typeof point.targetIntensity === "number"
    ? point.targetIntensity
    : undefined;

  return typeof intensity === "number" ? roundOne((metrics.runnerMaxRps * intensity) / 100) : undefined;
}

export function buildRpsChartData(metrics: LoadTestMetrics, waveConfig: WaveLoadConfig | null) {
  const history = metrics.rpsHistory ?? [];
  const usesHttpRps = history.some((point) =>
    typeof lifecycleHttpStarted(point) === "number"
    || typeof dispatchStarted(point) === "number"
    || typeof point.dispatchBucket === "number"
    || point.runners?.some((runner) =>
      typeof lifecycleHttpStarted(runner) === "number"
      || typeof dispatchStarted(runner) === "number"
      || typeof runner.dispatchBucket === "number"
    ),
  ) || metrics.lifecycleBuckets?.some((bucket) => typeof bucket.httpStarted === "number") === true;
  const runnerIds = usesHttpRps
    ? Array.from(
      new Set(history.flatMap((point) => point.runners?.map((runner) => runner.runnerId) ?? [])),
    ).sort()
    : [];
  const runnerSeries = runnerIds.map((label, index) => ({ key: `runner${index}`, label }));
  const runnerKeyById = new Map(runnerSeries.map((runner) => [runner.label, runner.key]));

  const ensureRow = (rows: Map<number, RpsChartRow>, bucket: number, point: RpsPoint) => {
    const existing = rows.get(bucket);
    if (existing) return existing;

    const row: RpsChartRow = {
      time: bucket,
      rpsTotal: 0,
      targetRpsLimit: estimateTargetRpsLimit(point, metrics, waveConfig, bucket * 1000),
    };
    for (const runner of runnerSeries) {
      row[runner.key] = 0;
    }
    rows.set(bucket, row);
    return row;
  };

  const rows = new Map<number, RpsChartRow>();
  for (const bucket of metrics.lifecycleBuckets ?? []) {
    const row = ensureRow(rows, Math.max(0, Math.floor(bucket.elapsedMs / 1000)), {
      timestamp: metrics.startTime + bucket.elapsedMs,
      elapsedMs: bucket.elapsedMs,
      rps: bucket.httpStarted ?? 0,
      lifecycleBucket: bucket,
    });
    row.rpsTotal = Math.max(row.rpsTotal, bucket.httpStarted ?? 0);
  }

  if (history.length === 0) {
    return {
      data: Array.from(rows.values()).sort((a, b) => a.time - b.time),
      runnerSeries,
      usesHttpRps,
    };
  }

  const applyDirectBucket = (row: RpsChartRow, point: RpsPoint) => {
    if (point.runners && point.runners.length > 0) {
      for (const runner of point.runners) {
        const key = runnerKeyById.get(runner.runnerId);
        const value = lifecycleHttpStarted(runner) ?? runner.dispatchBucket;
        if (!key || typeof value !== "number") continue;
        row[key] = Math.max(row[key] ?? 0, value);
      }
      row.rpsTotal = runnerSeries.reduce((sum, runner) => sum + (row[runner.key] ?? 0), 0);
      return;
    }

    const value = lifecycleHttpStarted(point) ?? point.dispatchBucket;
    if (typeof value === "number") {
      row.rpsTotal = Math.max(row.rpsTotal, value);
    }
  };
  const firstPoint = history[0];
  const firstRow = ensureRow(rows, bucketSecondForPoint(firstPoint, metrics), firstPoint);
  if (hasDirectBucket(firstPoint)) {
    applyDirectBucket(firstRow, firstPoint);
  }

  for (let index = 1; index < history.length; index += 1) {
    const point = history[index];
    const directBucket = hasDirectBucket(point);
    const previous = history[index - 1];
    const previousBucket = bucketSecondForPoint(previous, metrics);
    const currentBucket = bucketSecondForPoint(point, metrics);
    const span = bucketSpan(previousBucket, currentBucket);
    const startBucket = firstBucketToUpdate(previousBucket, currentBucket);

    if (directBucket && point.runners && point.runners.length > 0) {
      applyDirectBucket(ensureRow(rows, currentBucket, point), point);
      continue;
    }

    if (directBucket && typeof point.dispatchBucket === "number") {
      applyDirectBucket(ensureRow(rows, currentBucket, point), point);
      continue;
    }

    if (point.runners && point.runners.length > 0) {
      for (const runner of point.runners) {
        const key = runnerKeyById.get(runner.runnerId);
        if (!key) continue;
        const previousRunner = previous?.runners?.find((item) => item.runnerId === runner.runnerId);
        const currentStarted = dispatchStarted(runner);
        const previousStarted = runnerStarted(previousRunner);
        const bucketRps = previousRunner
          && typeof previousStarted === "number"
          && typeof currentStarted === "number"
          ? Math.max(0, currentStarted - previousStarted) / span
          : 0;
        for (let bucket = startBucket; bucket <= currentBucket; bucket += 1) {
          const row = ensureRow(rows, bucket, point);
          row[key] = (row[key] ?? 0) + bucketRps;
          row.rpsTotal += bucketRps;
        }
      }
      continue;
    }

    const previousStarted = dispatchStarted(previous);
    const currentStarted = dispatchStarted(point);
    const hasStartedTotals = typeof previousStarted === "number" && typeof currentStarted === "number";
    const hasTotalStarted = previous.totalStarted !== undefined && point.totalStarted !== undefined;
    const hasTotalSent = previous.totalSent !== undefined && point.totalSent !== undefined;
    const currentTotal = hasStartedTotals
      ? currentStarted
      : hasTotalStarted
        ? point.totalStarted
        : point.totalSent;
    const previousTotal = hasStartedTotals
      ? previousStarted
      : hasTotalStarted
        ? previous.totalStarted
        : previous.totalSent;
    const bucketRps = (hasStartedTotals || hasTotalStarted || hasTotalSent)
      && currentTotal !== undefined
      && previousTotal !== undefined
      ? Math.max(0, currentTotal - previousTotal) / span
      : point.rps;

    for (let bucket = startBucket; bucket <= currentBucket; bucket += 1) {
      ensureRow(rows, bucket, point).rpsTotal += bucketRps;
    }
  }

  const data = Array.from(rows.values())
    .sort((a, b) => a.time - b.time)
    .map((row) => {
      const rounded: RpsChartRow = {
        ...row,
        rpsTotal: roundOne(row.rpsTotal),
      };
      for (const runner of runnerSeries) {
        rounded[runner.key] = roundOne(row[runner.key] ?? 0);
      }
      return rounded;
    });

  return { data, runnerSeries, usesHttpRps };
}
