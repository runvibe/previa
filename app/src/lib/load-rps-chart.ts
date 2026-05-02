import type { LoadTestMetrics, WaveLoadConfig } from "@/types/load-test";

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

function sampleWaveIntensity(config: WaveLoadConfig, elapsedMs: number) {
  const points = [...config.points].sort((a, b) => a.atMs - b.atMs);
  const last = points[points.length - 1];
  if (!last) return undefined;
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
  if (typeof point.targetRpsLimit === "number") return roundOne(point.targetRpsLimit);
  if (typeof metrics.runnerMaxRps !== "number") return undefined;

  const intensity = typeof point.targetIntensity === "number"
    ? point.targetIntensity
    : waveConfig
      ? sampleWaveIntensity(waveConfig, elapsedMs)
      : undefined;

  return typeof intensity === "number" ? roundOne((metrics.runnerMaxRps * intensity) / 100) : undefined;
}

export function buildRpsChartData(metrics: LoadTestMetrics, waveConfig: WaveLoadConfig | null) {
  const history = metrics.rpsHistory ?? [];
  const usesHttpRps = history.some((point) =>
    typeof point.httpStarted === "number"
    || point.runners?.some((runner) => typeof runner.httpStarted === "number"),
  );
  const runnerIds = usesHttpRps
    ? Array.from(
      new Set(history.flatMap((point) => point.runners?.map((runner) => runner.runnerId) ?? [])),
    ).sort()
    : [];
  const runnerSeries = runnerIds.map((label, index) => ({ key: `runner${index}`, label }));

  const legacyData = () => history.map((point, index) => {
    const previous = history[index - 1];
    const elapsedMs = point.timestamp - metrics.startTime;
    const intervalSeconds = previous ? (point.timestamp - previous.timestamp) / 1000 : 0;
    const hasStartedTotals = previous?.totalStarted !== undefined && point.totalStarted !== undefined;
    const hasSentTotals = previous?.totalSent !== undefined && point.totalSent !== undefined;
    const currentTotal = hasStartedTotals ? point.totalStarted : point.totalSent;
    const previousTotal = hasStartedTotals ? previous?.totalStarted : previous?.totalSent;
    const hasTotals = hasStartedTotals || hasSentTotals;
    const intervalRps = hasTotals && intervalSeconds > 0 && currentTotal !== undefined && previousTotal !== undefined
      ? Math.max(0, (currentTotal - previousTotal) / intervalSeconds)
      : point.rps;

    return {
      time: Math.round(elapsedMs / 1000),
      rpsTotal: roundOne(intervalRps),
      targetRpsLimit: estimateTargetRpsLimit(point, metrics, waveConfig, elapsedMs),
    };
  });

  if (!usesHttpRps) {
    return { data: legacyData(), runnerSeries, usesHttpRps };
  }

  const runnerKeyById = new Map(runnerSeries.map((runner) => [runner.label, runner.key]));
  const data = history.map((point, index) => {
    const previous = history[index - 1];
    const elapsedMs = point.timestamp - metrics.startTime;
    const intervalSeconds = previous ? (point.timestamp - previous.timestamp) / 1000 : 0;
    const row: RpsChartRow = {
      time: Math.round(elapsedMs / 1000),
      rpsTotal: 0,
      targetRpsLimit: estimateTargetRpsLimit(point, metrics, waveConfig, elapsedMs),
    };

    if (point.runners && point.runners.length > 0) {
      let total = 0;
      for (const runner of point.runners) {
        const key = runnerKeyById.get(runner.runnerId);
        if (!key) continue;
        const previousRunner = previous?.runners?.find((item) => item.runnerId === runner.runnerId);
        const intervalRps = previousRunner
          && typeof previousRunner.httpStarted === "number"
          && typeof runner.httpStarted === "number"
          && intervalSeconds > 0
          ? Math.max(0, (runner.httpStarted - previousRunner.httpStarted) / intervalSeconds)
          : runner.rps ?? 0;
        const rounded = roundOne(intervalRps);
        row[key] = rounded;
        total += rounded;
      }
      row.rpsTotal = roundOne(total);
      return row;
    }

    const previousHttpStarted = previous?.httpStarted;
    const total = typeof previousHttpStarted === "number"
      && typeof point.httpStarted === "number"
      && intervalSeconds > 0
      ? Math.max(0, (point.httpStarted - previousHttpStarted) / intervalSeconds)
      : point.rps;
    row.rpsTotal = roundOne(total);
    return row;
  });

  return { data, runnerSeries, usesHttpRps };
}
