import type { LoadTestMetrics, WaveLoadConfig } from "@/types/load-test";

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

  return history.map((point, index) => {
    const previous = history[index - 1];
    const elapsedMs = point.timestamp - metrics.startTime;
    const intervalSeconds = previous ? (point.timestamp - previous.timestamp) / 1000 : 0;
    const hasTotals = previous?.totalSent !== undefined && point.totalSent !== undefined;
    const intervalRps = hasTotals && intervalSeconds > 0
      ? Math.max(0, (point.totalSent - previous.totalSent) / intervalSeconds)
      : point.rps;

    return {
      time: Math.round(elapsedMs / 1000),
      rps: roundOne(intervalRps),
      targetRpsLimit: estimateTargetRpsLimit(point, metrics, waveConfig, elapsedMs),
    };
  });
}
