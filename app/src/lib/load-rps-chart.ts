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

function roundOne(value: number) {
  return Math.round(value * 10) / 10;
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

function hasDirectDispatchBucket(point: RpsPoint) {
  return typeof point.dispatchBucket === "number"
    || point.runners?.some((runner) => typeof runner.dispatchBucket === "number") === true;
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
    typeof dispatchStarted(point) === "number"
    || typeof point.dispatchBucket === "number"
    || point.runners?.some((runner) =>
      typeof dispatchStarted(runner) === "number" || typeof runner.dispatchBucket === "number"
    ),
  );
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

  if (history.length === 0) {
    return { data: [], runnerSeries, usesHttpRps };
  }

  const rows = new Map<number, RpsChartRow>();
  const applyDirectDispatchBucket = (row: RpsChartRow, point: RpsPoint) => {
    if (point.runners && point.runners.length > 0) {
      for (const runner of point.runners) {
        const key = runnerKeyById.get(runner.runnerId);
        if (!key || typeof runner.dispatchBucket !== "number") continue;
        row[key] = Math.max(row[key] ?? 0, runner.dispatchBucket);
      }
      row.rpsTotal = runnerSeries.reduce((sum, runner) => sum + (row[runner.key] ?? 0), 0);
      return;
    }

    if (typeof point.dispatchBucket === "number") {
      row.rpsTotal = Math.max(row.rpsTotal, point.dispatchBucket);
    }
  };
  const firstPoint = history[0];
  const firstRow = ensureRow(rows, bucketSecondForPoint(firstPoint, metrics), firstPoint);
  if (hasDirectDispatchBucket(firstPoint)) {
    applyDirectDispatchBucket(firstRow, firstPoint);
  }

  for (let index = 1; index < history.length; index += 1) {
    const point = history[index];
    const directBucket = hasDirectDispatchBucket(point);
    const previous = history[index - 1];
    const previousBucket = bucketSecondForPoint(previous, metrics);
    const currentBucket = bucketSecondForPoint(point, metrics);
    const span = bucketSpan(previousBucket, currentBucket);
    const startBucket = firstBucketToUpdate(previousBucket, currentBucket);

    if (directBucket && point.runners && point.runners.length > 0) {
      applyDirectDispatchBucket(ensureRow(rows, currentBucket, point), point);
      continue;
    }

    if (directBucket && typeof point.dispatchBucket === "number") {
      applyDirectDispatchBucket(ensureRow(rows, currentBucket, point), point);
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
