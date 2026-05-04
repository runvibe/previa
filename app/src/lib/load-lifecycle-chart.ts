import type { LoadTestMetrics, RpsPoint } from "@/types/load-test";

export type LifecycleSeriesKey =
  | "planned"
  | "sendStarted"
  | "httpStarted"
  | "httpSendReturned"
  | "responseBodyCompleted";

export type LifecycleSeriesTone = "planned" | "send" | "http" | "returned" | "body";

export interface LifecycleSeries {
  key: LifecycleSeriesKey;
  labelKey: string;
  tone: LifecycleSeriesTone;
}

export interface LifecycleChartRow {
  time: number;
  planned: number;
  sendStarted: number;
  httpStarted: number;
  httpSendReturned: number;
  responseBodyCompleted: number;
}

export interface LifecycleChartData {
  data: LifecycleChartRow[];
  series: LifecycleSeries[];
}

const SERIES: LifecycleSeries[] = [
  { key: "planned", labelKey: "loadTestResults.lifecyclePlanned", tone: "planned" },
  { key: "sendStarted", labelKey: "loadTestResults.lifecycleSendStarted", tone: "send" },
  { key: "httpStarted", labelKey: "loadTestResults.lifecycleHttpStarted", tone: "http" },
  { key: "httpSendReturned", labelKey: "loadTestResults.lifecycleHttpSendReturned", tone: "returned" },
  { key: "responseBodyCompleted", labelKey: "loadTestResults.lifecycleBodyCompleted", tone: "body" },
];

function elapsedMsForPoint(point: RpsPoint, metrics: LoadTestMetrics) {
  return typeof point.elapsedMs === "number" ? point.elapsedMs : point.timestamp - metrics.startTime;
}

function bucketSecond(point: RpsPoint, metrics: LoadTestMetrics) {
  return Math.max(0, Math.floor(elapsedMsForPoint(point, metrics) / 1000));
}

function cumulativeDelta(current: number | undefined, previous: number | undefined) {
  if (typeof current !== "number") return 0;
  if (typeof previous !== "number") return Math.max(0, current);
  return Math.max(0, current - previous);
}

function ensureRow(rows: Map<number, LifecycleChartRow>, time: number): LifecycleChartRow {
  const existing = rows.get(time);
  if (existing) return existing;

  const row: LifecycleChartRow = {
    time,
    planned: 0,
    sendStarted: 0,
    httpStarted: 0,
    httpSendReturned: 0,
    responseBodyCompleted: 0,
  };
  rows.set(time, row);
  return row;
}

function roundOne(value: number) {
  return Math.round(value * 10) / 10;
}

export function buildLifecycleChartData(metrics: LoadTestMetrics): LifecycleChartData {
  const history = metrics.rpsHistory ?? [];
  if (history.length === 0) return { data: [], series: SERIES };

  const rows = new Map<number, LifecycleChartRow>();

  for (let index = 0; index < history.length; index += 1) {
    const point = history[index];
    const previous = history[index - 1];
    const time = bucketSecond(point, metrics);
    const row = ensureRow(rows, time);
    const directBucket = point.lifecycleBucket
      ?? metrics.lifecycleBuckets?.find((bucket) => bucket.elapsedMs === time * 1000);

    if (directBucket) {
      row.planned += directBucket.planned ?? 0;
      row.sendStarted += directBucket.sendStarted ?? 0;
      row.httpStarted += directBucket.httpStarted ?? 0;
      row.httpSendReturned += directBucket.httpSendReturned ?? 0;
      row.responseBodyCompleted += directBucket.responseBodyCompleted ?? 0;
      continue;
    }

    row.planned += cumulativeDelta(point.scheduledStarts, previous?.scheduledStarts);
    row.sendStarted += cumulativeDelta(point.sendStarted, previous?.sendStarted);
    if (typeof point.dispatchBucket === "number") {
      row.httpStarted = Math.max(row.httpStarted, point.dispatchBucket);
    } else {
      row.httpStarted += cumulativeDelta(
        point.httpStarted ?? point.dispatchStarted,
        previous?.httpStarted ?? previous?.dispatchStarted,
      );
    }
    row.httpSendReturned += cumulativeDelta(point.httpSendReturned, previous?.httpSendReturned);
    row.responseBodyCompleted += cumulativeDelta(
      point.responseBodyCompleted,
      previous?.responseBodyCompleted,
    );
  }

  const data = Array.from(rows.values())
    .sort((a, b) => a.time - b.time)
    .map((row) => ({
      ...row,
      planned: roundOne(row.planned),
      sendStarted: roundOne(row.sendStarted),
      httpStarted: roundOne(row.httpStarted),
      httpSendReturned: roundOne(row.httpSendReturned),
      responseBodyCompleted: roundOne(row.responseBodyCompleted),
    }))
    .filter(
      (row) =>
        row.planned > 0 ||
        row.sendStarted > 0 ||
        row.httpStarted > 0 ||
        row.httpSendReturned > 0 ||
        row.responseBodyCompleted > 0,
    );

  return { data, series: SERIES };
}
