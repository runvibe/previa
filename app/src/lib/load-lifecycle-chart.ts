import type { LoadTestMetrics, RpsPoint } from "@/types/load-test";

export type LifecycleSeriesKey =
  | "planned"
  | "sendStarted"
  | "httpStarted"
  | "httpSendReturned"
  | "responseBodyCompleted"
  | "senderStartLagMsMax"
  | "httpSendDurationMsMax"
  | "responseObservationDurationMsMax";

export type LifecycleSeriesTone = "planned" | "send" | "http" | "returned" | "body" | "startLag" | "sendLag" | "observeLag";

export interface LifecycleSeries {
  key: LifecycleSeriesKey;
  labelKey: string;
  tone: LifecycleSeriesTone;
  axis: "count" | "ms";
}

export interface LifecycleChartRow {
  time: number;
  planned: number;
  sendStarted: number;
  httpStarted: number;
  httpSendReturned: number;
  responseBodyCompleted: number;
  senderStartLagMsMax: number;
  httpSendDurationMsMax: number;
  responseObservationDurationMsMax: number;
}

export interface LifecycleChartData {
  data: LifecycleChartRow[];
  series: LifecycleSeries[];
}

const SERIES: LifecycleSeries[] = [
  { key: "planned", labelKey: "loadTestResults.lifecyclePlanned", tone: "planned", axis: "count" },
  { key: "sendStarted", labelKey: "loadTestResults.lifecycleSendStarted", tone: "send", axis: "count" },
  { key: "httpStarted", labelKey: "loadTestResults.lifecycleHttpStarted", tone: "http", axis: "count" },
  { key: "httpSendReturned", labelKey: "loadTestResults.lifecycleHttpSendReturned", tone: "returned", axis: "count" },
  { key: "responseBodyCompleted", labelKey: "loadTestResults.lifecycleBodyCompleted", tone: "body", axis: "count" },
  { key: "senderStartLagMsMax", labelKey: "loadTestResults.lifecycleSenderStartLag", tone: "startLag", axis: "ms" },
  { key: "httpSendDurationMsMax", labelKey: "loadTestResults.lifecycleHttpSendDuration", tone: "sendLag", axis: "ms" },
  { key: "responseObservationDurationMsMax", labelKey: "loadTestResults.lifecycleResponseObservation", tone: "observeLag", axis: "ms" },
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
    senderStartLagMsMax: 0,
    httpSendDurationMsMax: 0,
    responseObservationDurationMsMax: 0,
  };
  rows.set(time, row);
  return row;
}

function roundOne(value: number) {
  return Math.round(value * 10) / 10;
}

export function buildLifecycleChartData(metrics: LoadTestMetrics): LifecycleChartData {
  const history = metrics.rpsHistory ?? [];
  if (history.length === 0 && (!metrics.lifecycleBuckets || metrics.lifecycleBuckets.length === 0)) {
    return { data: [], series: SERIES };
  }

  const rows = new Map<number, LifecycleChartRow>();
  const directRowTimes = new Set<number>();

  for (const bucket of metrics.lifecycleBuckets ?? []) {
    const time = Math.max(0, Math.floor(bucket.elapsedMs / 1000));
    const row = ensureRow(rows, time);
    row.planned += bucket.planned ?? 0;
    row.sendStarted += bucket.sendStarted ?? 0;
    row.httpStarted += bucket.httpStarted ?? 0;
    row.httpSendReturned += bucket.httpSendReturned ?? 0;
    row.responseBodyCompleted += bucket.responseBodyCompleted ?? 0;
    row.senderStartLagMsMax = Math.max(row.senderStartLagMsMax, bucket.senderStartLagMsMax ?? 0);
    row.httpSendDurationMsMax = Math.max(row.httpSendDurationMsMax, bucket.httpSendDurationMsMax ?? 0);
    row.responseObservationDurationMsMax = Math.max(
      row.responseObservationDurationMsMax,
      bucket.responseObservationDurationMsMax ?? 0,
    );
    directRowTimes.add(time);
  }

  for (let index = 0; index < history.length; index += 1) {
    const point = history[index];
    const previous = history[index - 1];
    const time = bucketSecond(point, metrics);
    if (directRowTimes.has(time)) continue;
    const row = ensureRow(rows, time);
    const directBucket = point.lifecycleBucket
      ?? metrics.lifecycleBuckets?.find((bucket) => bucket.elapsedMs === time * 1000);

    if (directBucket) {
      row.planned += directBucket.planned ?? 0;
      row.sendStarted += directBucket.sendStarted ?? 0;
      row.httpStarted += directBucket.httpStarted ?? 0;
      row.httpSendReturned += directBucket.httpSendReturned ?? 0;
      row.responseBodyCompleted += directBucket.responseBodyCompleted ?? 0;
      row.senderStartLagMsMax = Math.max(row.senderStartLagMsMax, directBucket.senderStartLagMsMax ?? 0);
      row.httpSendDurationMsMax = Math.max(row.httpSendDurationMsMax, directBucket.httpSendDurationMsMax ?? 0);
      row.responseObservationDurationMsMax = Math.max(
        row.responseObservationDurationMsMax,
        directBucket.responseObservationDurationMsMax ?? 0,
      );
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
      senderStartLagMsMax: roundOne(row.senderStartLagMsMax),
      httpSendDurationMsMax: roundOne(row.httpSendDurationMsMax),
      responseObservationDurationMsMax: roundOne(row.responseObservationDurationMsMax),
    }))
    .filter(
      (row) =>
        row.planned > 0 ||
        row.sendStarted > 0 ||
        row.httpStarted > 0 ||
        row.httpSendReturned > 0 ||
        row.responseBodyCompleted > 0 ||
        row.senderStartLagMsMax > 0 ||
        row.httpSendDurationMsMax > 0 ||
        row.responseObservationDurationMsMax > 0,
    );

  const series = SERIES.filter((series) => {
    if (series.axis === "count") return true;
    return data.some((row) => row[series.key] > 0);
  });

  return { data, series };
}
