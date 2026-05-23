import type { LoadTestMetrics } from "@/types/load-test";

export interface StatusCodeSeries {
  code: string;
  labelKey?: string;
  color: string;
}

export interface StatusCodeChartData {
  data: Array<Record<string, number>>;
  series: StatusCodeSeries[];
}

function statusCodeColor(code: string) {
  if (code === "network_error") return "#a855f7";
  const status = Number.parseInt(code, 10);
  if (Number.isNaN(status)) return "hsl(var(--muted-foreground))";
  if (status >= 500) return "hsl(var(--status-error))";
  if (status >= 400) return "hsl(var(--warning))";
  if (status >= 300) return "hsl(var(--status-running))";
  if (status >= 200) return "hsl(var(--status-success))";
  return "hsl(var(--muted-foreground))";
}

function compareStatusCodes(a: string, b: string) {
  if (a === "network_error") return 1;
  if (b === "network_error") return -1;
  const left = Number.parseInt(a, 10);
  const right = Number.parseInt(b, 10);
  if (!Number.isNaN(left) && !Number.isNaN(right)) return left - right;
  return a.localeCompare(b);
}

export function buildStatusCodeChartData(metrics: LoadTestMetrics): StatusCodeChartData {
  const buckets = metrics.statusCodeBuckets ?? [];
  if (buckets.length === 0) return { data: [], series: [] };

  const rows = new Map<number, Record<string, number>>();
  const codes = new Set<string>();

  for (const bucket of buckets) {
    const time = Math.max(0, Math.floor(bucket.elapsedMs / 1000));
    const row = rows.get(time) ?? { time };
    row[bucket.code] = (row[bucket.code] ?? 0) + bucket.count;
    rows.set(time, row);
    codes.add(bucket.code);
  }

  const orderedCodes = Array.from(codes).sort(compareStatusCodes);
  const data = Array.from(rows.values())
    .sort((a, b) => a.time - b.time)
    .map((row) => {
      const completeRow = { ...row };
      for (const code of orderedCodes) {
        completeRow[code] = completeRow[code] ?? 0;
      }
      return completeRow;
    });

  return {
    data,
    series: orderedCodes.map((code) => ({
      code,
      labelKey: code === "network_error" ? "loadTestResults.networkError" : undefined,
      color: statusCodeColor(code),
    })),
  };
}
