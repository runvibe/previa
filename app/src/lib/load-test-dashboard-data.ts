import { format } from "date-fns";
import type { LoadTestRunRecord } from "@/lib/load-test-store";
import { isWaveLoadConfig } from "@/types/load-test";

export function buildLatencyHistory(runs: LoadTestRunRecord[]) {
  return runs
    .filter((r) => r.metrics.avgLatency > 0)
    .map((r) => ({
      timestamp: format(new Date(r.timestamp), "dd/MM HH:mm"),
      avg: Math.round(r.metrics.avgLatency),
      p95: Math.round(r.metrics.p95),
      p99: Math.round(r.metrics.p99),
    }));
}

export function buildRpsHistory(runs: LoadTestRunRecord[]) {
  return runs.map((r) => ({
    timestamp: format(new Date(r.timestamp), "dd/MM HH:mm"),
    rps: Math.round(r.metrics.rps * 100) / 100,
  }));
}

export function buildLoadTestSuccessRate(runs: LoadTestRunRecord[]) {
  const totals = runs.reduce(
    (acc, r) => ({
      success: acc.success + r.metrics.totalSuccess,
      error: acc.error + r.metrics.totalError,
    }),
    { success: 0, error: 0 }
  );
  return [
    { name: "Sucesso", value: totals.success },
    { name: "Erro", value: totals.error },
  ];
}

export function buildLatencyDistribution(runs: LoadTestRunRecord[]) {
  const buckets = [
    { label: "0-50ms", min: 0, max: 50, count: 0 },
    { label: "50-100ms", min: 50, max: 100, count: 0 },
    { label: "100-200ms", min: 100, max: 200, count: 0 },
    { label: "200-500ms", min: 200, max: 500, count: 0 },
    { label: "500ms+", min: 500, max: Infinity, count: 0 },
  ];
  for (const run of runs) {
    if (!run.metrics.latencyHistory?.length) continue;
    for (const point of run.metrics.latencyHistory) {
      const bucket = buckets.find((b) => point.latency >= b.min && point.latency < b.max);
      if (bucket) bucket.count++;
    }
  }
  return buckets.map((b) => ({ bucket: b.label, count: b.count }));
}

export function buildConfigComparison(runs: LoadTestRunRecord[]) {
  const groups = new Map<number, { avgLatencies: number[]; p95s: number[]; rpsList: number[] }>();
  for (const r of runs) {
    const c = isWaveLoadConfig(r.config)
      ? Math.max(...r.config.points.map((point) => point.intensity))
      : r.config.concurrency;
    if (!groups.has(c)) groups.set(c, { avgLatencies: [], p95s: [], rpsList: [] });
    const g = groups.get(c)!;
    if (r.metrics.avgLatency > 0) {
      g.avgLatencies.push(r.metrics.avgLatency);
      g.p95s.push(r.metrics.p95);
    }
    g.rpsList.push(r.metrics.rps);
  }
  const avg = (arr: number[]) => arr.length > 0 ? Math.round(arr.reduce((a, b) => a + b, 0) / arr.length) : 0;
  return Array.from(groups.entries())
    .sort(([a], [b]) => a - b)
    .map(([concurrency, g]) => ({
      config: `${concurrency}${runs.some((run) => isWaveLoadConfig(run.config)) ? "% peak" : " conc."}`,
      avgLatency: avg(g.avgLatencies),
      p95: avg(g.p95s),
      rps: Math.round((g.rpsList.reduce((a, b) => a + b, 0) / g.rpsList.length) * 100) / 100,
    }));
}

export function buildThroughputVsLatency(runs: LoadTestRunRecord[]) {
  return runs.map((r) => ({
    rps: Math.round(r.metrics.rps * 100) / 100,
    avgLatency: Math.round(r.metrics.avgLatency),
    label: isWaveLoadConfig(r.config)
      ? `${Math.max(...r.config.points.map((point) => point.intensity))}% peak`
      : `${r.config.concurrency} conc.`,
  }));
}

export function buildLoadTestTimeline(runs: LoadTestRunRecord[]) {
  const byDay = new Map<string, number>();
  for (const r of runs) {
    const day = format(new Date(r.timestamp), "dd/MM");
    byDay.set(day, (byDay.get(day) || 0) + 1);
  }
  return Array.from(byDay.entries()).map(([date, count]) => ({ date, tests: count }));
}
