import type { Pipeline, StepExecutionResult } from "@/types/pipeline";
import type {
  LoadRunConfig,
  LoadTestMetrics,
  LoadTestState,
  RemoteMetricsEvent,
  ConsolidatedLoadMetrics,
  LoadLifecycleBucket,
  RpsPoint,
  RunnerResourcePoint,
  RunnerRuntimeInfo,
} from "@/types/load-test";
import { isWaveLoadConfig } from "@/types/load-test";
import { generateUUID } from "./uuid";
import { cancelExecution, ensureApiPrefix } from "./api-client";

// ============ SSE Parser ============

function parseSSE(chunk: string): Array<{ event: string; data: string }> {
  const events: Array<{ event: string; data: string }> = [];
  const blocks = chunk.split("\n\n").filter(Boolean);

  for (const block of blocks) {
    let event = "message";
    let data = "";

    for (const line of block.split("\n")) {
      if (line.startsWith("event: ")) {
        event = line.slice(7).trim();
      } else if (line.startsWith("data: ")) {
        data += (data ? "\n" : "") + line.slice(6);
      } else if (line.startsWith("data:")) {
        data += (data ? "\n" : "") + line.slice(5);
      }
    }

    if (data) {
      events.push({ event, data });
    }
  }

  return events;
}

// ============ SSE Attempt/Event Helpers ============

type SseObject = Record<string, unknown>;
type OrchestratorLine = { runnerEvent?: unknown; payload?: unknown; node?: unknown };

export interface RemoteIntegrationSnapshot {
  executionId: string | null;
  status: string;
  results: Record<string, StepExecutionResult>;
  summary: Record<string, unknown> | null;
  errors: string[];
}

export interface RemoteNodesInfo {
  nodesUsed: number;
  nodesFound: number;
  nodeNames: string[];
}

export interface RemoteLoadExecutionSnapshot {
  executionId: string | null;
  status: string;
  state: LoadTestState;
  metrics: LoadTestMetrics;
  nodesInfo: RemoteNodesInfo | null;
  errors: string[];
}

function isSseObject(value: unknown): value is SseObject {
  return typeof value === "object" && value !== null;
}

function toNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function maxOptional(left: number | undefined, right: number | undefined) {
  if (typeof left !== "number") return right;
  if (typeof right !== "number") return left;
  return Math.max(left, right);
}

function pickString(...values: unknown[]): string | undefined {
  for (const value of values) {
    if (typeof value === "string" && value.trim().length > 0) return value;
  }
  return undefined;
}

function pickStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string" && item.trim().length > 0);
}

function resolveStepStatus(...values: unknown[]): StepExecutionResult["status"] {
  for (const value of values) {
    if (value === "pending" || value === "running" || value === "success" || value === "error") {
      return value;
    }
  }
  return "running";
}

function readAttemptMeta(source?: SseObject): { attempt?: number; maxAttempts?: number } {
  if (!source) return {};
  const meta = isSseObject(source.meta) ? source.meta : undefined;
  return {
    attempt: toNumber(source.attempt ?? source.attempts ?? meta?.attempt ?? meta?.attempts),
    maxAttempts: toNumber(
      source.maxAttempts
        ?? source.max_attempts
        ?? source.max_retry_attempts
        ?? meta?.maxAttempts
        ?? meta?.max_attempts
        ?? meta?.max_retry_attempts
    ),
  };
}

function extractAttemptMeta(
  envelope: SseObject,
  payload: SseObject,
  linePayload?: SseObject,
): { attempt?: number; maxAttempts?: number; startedAt?: number } {
  const lineMeta = readAttemptMeta(linePayload);
  if (lineMeta.attempt !== undefined || lineMeta.maxAttempts !== undefined) {
    const startedAt = extractTimestamp(linePayload, envelope, payload);
    return { ...lineMeta, startedAt };
  }

  const payloadMeta = readAttemptMeta(payload);
  const envelopeMeta = readAttemptMeta(envelope);

  let attempt = payloadMeta.attempt ?? envelopeMeta.attempt;
  let maxAttempts = payloadMeta.maxAttempts ?? envelopeMeta.maxAttempts;

  // Fallback: check aggregated lines from orchestrator envelope
  if (attempt === undefined && Array.isArray(envelope.lines)) {
    for (const rawLine of envelope.lines) {
      if (!isSseObject(rawLine)) continue;
      const nestedPayload = isSseObject(rawLine.payload) ? rawLine.payload : undefined;
      const nestedMeta = readAttemptMeta(nestedPayload);
      attempt = nestedMeta.attempt;
      maxAttempts = nestedMeta.maxAttempts ?? maxAttempts;
      if (attempt !== undefined) break;
    }
  }

  const startedAt = extractTimestamp(linePayload, payload, envelope);
  return { attempt, maxAttempts, startedAt };
}

function extractTimestamp(...sources: (SseObject | undefined)[]): number | undefined {
  for (const source of sources) {
    if (!source) continue;
    const ts = toNumber(source.startedAt ?? source.startedAtMs ?? source.timestamp ?? source.receivedAt);
    if (ts !== undefined) return ts;
  }
  return undefined;
}

function resolveEventName(
  rawEvent: string,
  envelope: SseObject,
  payload: SseObject,
  lineRunnerEvent?: string,
): string {
  if (lineRunnerEvent) return lineRunnerEvent;
  if (rawEvent !== "message") return rawEvent;
  return pickString(envelope.event, envelope.runnerEvent, payload.event, payload.runnerEvent) ?? rawEvent;
}

function resolveStepId(
  envelope: SseObject,
  payload: SseObject,
  linePayload?: SseObject,
): string | undefined {
  const nestedStep = linePayload?.step ?? payload.step;
  const nestedStepId = isSseObject(nestedStep) ? nestedStep.id : undefined;
  return pickString(linePayload?.stepId, envelope.stepId, payload.stepId, nestedStepId);
}

function mapLoadSnapshotStatus(status: string): LoadTestState {
  if (status === "cancelled") return "cancelled";
  if (status === "running" || status === "queued") return "running";
  return "completed";
}

function extractRemoteMetrics(value: unknown): RemoteMetricsEvent | null {
  if (!isSseObject(value)) return null;

  const totalSent = toNumber(value.totalSent);
  const totalStarted = toNumber(value.totalStarted);
  const totalSuccess = toNumber(value.totalSuccess);
  const totalError = toNumber(value.totalError);
  const httpStarted = toNumber(value.httpStarted);
  const httpCompleted = toNumber(value.httpCompleted);
  const dispatchSubmitted = toNumber(value.dispatchSubmitted);
  const dispatchStarted = toNumber(value.dispatchStarted);
  const httpSendReturned = toNumber(value.httpSendReturned);
  const responseBodyCompleted = toNumber(value.responseBodyCompleted);
  const dependencyLimitedStarts = toNumber(value.dependencyLimitedStarts);
  const dispatcherLaggedStarts = toNumber(value.dispatcherLaggedStarts);
  const runtimeLaggedStarts = toNumber(value.runtimeLaggedStarts);
  const senderLaggedStarts = toNumber(value.senderLaggedStarts);
  const senderQueueDepth = toNumber(value.senderQueueDepth);
  const senderStartLagAvgMs = toNumber(value.senderStartLagAvgMs);
  const senderStartLagP95Ms = toNumber(value.senderStartLagP95Ms);
  const senderStartLagP99Ms = toNumber(value.senderStartLagP99Ms);
  const senderStartLagMaxMs = toNumber(value.senderStartLagMaxMs);
  const httpSendDurationAvgMs = toNumber(value.httpSendDurationAvgMs);
  const httpSendDurationP95Ms = toNumber(value.httpSendDurationP95Ms);
  const httpSendDurationP99Ms = toNumber(value.httpSendDurationP99Ms);
  const responseObservationDurationAvgMs = toNumber(value.responseObservationDurationAvgMs);
  const responseObservationDurationP95Ms = toNumber(value.responseObservationDurationP95Ms);
  const responseObservationDurationP99Ms = toNumber(value.responseObservationDurationP99Ms);
  const schedulerLagMs = toNumber(value.schedulerLagMs);
  const schedulerLaggedStarts = toNumber(value.schedulerLaggedStarts);
  const rps = toNumber(value.rps);

  if (
    totalSent === undefined
    && totalSuccess === undefined
    && totalError === undefined
    && rps === undefined
  ) {
    return null;
  }

  return {
    snapshotMode: value.snapshotMode === "live" || value.snapshotMode === "final"
      ? value.snapshotMode
      : undefined,
    totalSent: totalSent ?? 0,
    totalStarted,
    totalSuccess: totalSuccess ?? 0,
    totalError: totalError ?? 0,
    httpStarted,
    httpCompleted,
    dispatchSubmitted,
    dispatchStarted,
    httpSendReturned,
    responseBodyCompleted,
    dependencyLimitedStarts,
    dispatcherLaggedStarts,
    runtimeLaggedStarts,
    senderLaggedStarts,
    senderQueueDepth,
    senderStartLagAvgMs,
    senderStartLagP95Ms,
    senderStartLagP99Ms,
    senderStartLagMaxMs,
    httpSendDurationAvgMs,
    httpSendDurationP95Ms,
    httpSendDurationP99Ms,
    responseObservationDurationAvgMs,
    responseObservationDurationP95Ms,
    responseObservationDurationP99Ms,
    schedulerLagMs,
    schedulerLaggedStarts,
    rps: rps ?? 0,
    startTime: toNumber(value.startTime) ?? Date.now(),
    elapsedMs: toNumber(value.elapsedMs) ?? 0,
    dispatchBuckets: extractDispatchBuckets(value.dispatchBuckets),
    lifecycleBuckets: extractLifecycleBuckets(value.lifecycleBuckets),
    targetIntensity: toNumber(value.targetIntensity),
    targetRpsLimit: toNumber(value.targetRpsLimit),
    inFlight: toNumber(value.inFlight),
    runnerMaxRps: toNumber(value.runnerMaxRps),
    tickMs: toNumber(value.tickMs),
    scheduledStarts: toNumber(value.scheduledStarts),
    missedStarts: toNumber(value.missedStarts),
    readyRequests: toNumber(value.readyRequests),
    activePipelines: toNumber(value.activePipelines),
    outstandingRequests: toNumber(value.outstandingRequests),
    curveAdherence: toNumber(value.curveAdherence),
    runtime: extractRunnerRuntime(value.runtime),
  };
}

function extractDispatchBuckets(value: unknown) {
  if (!Array.isArray(value)) return undefined;
  const buckets = value
    .map((item) => {
      if (!isSseObject(item)) return null;
      const elapsedMs = toNumber(item.elapsedMs);
      const count = toNumber(item.count);
      return elapsedMs !== undefined && count !== undefined ? { elapsedMs, count } : null;
    })
    .filter((item): item is { elapsedMs: number; count: number } => item !== null);

  return buckets.length > 0 ? buckets : undefined;
}

function extractLifecycleBuckets(value: unknown): LoadLifecycleBucket[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const buckets = value
    .map((item) => {
      if (!isSseObject(item)) return null;
      const elapsedMs = toNumber(item.elapsedMs);
      if (elapsedMs === undefined) return null;
      return {
        elapsedMs,
        planned: toNumber(item.planned),
        slotEnqueued: toNumber(item.slotEnqueued),
        requestPrepared: toNumber(item.requestPrepared),
        requestEnqueued: toNumber(item.requestEnqueued),
        sendTaskSpawned: toNumber(item.sendTaskSpawned),
        sendStarted: toNumber(item.sendStarted),
        httpStarted: toNumber(item.httpStarted),
        httpSendReturned: toNumber(item.httpSendReturned),
        responseBodyCompleted: toNumber(item.responseBodyCompleted),
        dispatcherLagged: toNumber(item.dispatcherLagged),
        runtimeLagged: toNumber(item.runtimeLagged),
        senderLagged: toNumber(item.senderLagged),
        senderStartLagMsMax: toNumber(item.senderStartLagMsMax),
        httpSendDurationMsMax: toNumber(item.httpSendDurationMsMax),
        responseObservationDurationMsMax: toNumber(item.responseObservationDurationMsMax),
      };
    })
    .filter((item): item is LoadLifecycleBucket => item !== null);

  return buckets.length > 0 ? buckets : undefined;
}

function extractRunnerRuntime(value: unknown): RunnerRuntimeInfo | undefined {
  if (!isSseObject(value)) return undefined;
  const pid = toNumber(value.pid);
  const memoryBytes = toNumber(value.memoryBytes);
  const virtualMemoryBytes = toNumber(value.virtualMemoryBytes);
  const cpuUsagePercent = toNumber(value.cpuUsagePercent);
  const networkTxBytes = toNumber(value.networkTxBytes) ?? 0;
  const networkRxBytes = toNumber(value.networkRxBytes) ?? 0;
  const networkTotalBytes = toNumber(value.networkTotalBytes) ?? networkTxBytes + networkRxBytes;

  if (
    pid === undefined
    || memoryBytes === undefined
    || virtualMemoryBytes === undefined
    || cpuUsagePercent === undefined
  ) {
    return undefined;
  }

  return {
    pid,
    memoryBytes,
    virtualMemoryBytes,
    cpuUsagePercent,
    networkTxBytes,
    networkRxBytes,
    networkTotalBytes,
  };
}

function extractRunnerResourcePoints(lines: unknown[]): RunnerResourcePoint[] {
  const points: RunnerResourcePoint[] = [];

  for (const line of lines) {
    if (!isSseObject(line)) continue;
    const payload = isSseObject(line.payload) ? line.payload : line;
    const metrics = extractRemoteMetrics(payload);
    const node = pickString(line.node, payload.node);
    if (!metrics?.runtime || !node) continue;

    const elapsedMs = metrics.elapsedMs;
    points.push({
      node,
      timestamp: metrics.startTime + elapsedMs,
      elapsedMs,
      cpuUsagePercent: metrics.runtime.cpuUsagePercent,
      memoryBytes: metrics.runtime.memoryBytes,
      memoryMb: Math.round((metrics.runtime.memoryBytes / 1024 / 1024) * 100) / 100,
      networkTxBytes: metrics.runtime.networkTxBytes ?? 0,
      networkRxBytes: metrics.runtime.networkRxBytes ?? 0,
      networkTotalBytes: metrics.runtime.networkTotalBytes ?? 0,
      networkTotalKb: Math.round(((metrics.runtime.networkTotalBytes ?? 0) / 1024) * 100) / 100,
    });
  }

  return points;
}

const MAX_RUNNER_RESOURCE_POINTS = 800;

function computeCurveAdherence(scheduledStarts?: number, missedStarts?: number): number | undefined {
  if (scheduledStarts === undefined) return undefined;
  if (scheduledStarts === 0) return 100;
  return Math.round(((scheduledStarts - (missedStarts ?? 0)) / scheduledStarts) * 10_000) / 100;
}

function dispatchBucketFor(metrics: RemoteMetricsEvent, elapsedMs: number) {
  const bucketMs = Math.floor(elapsedMs / 1000) * 1000;
  return metrics.dispatchBuckets?.find((bucket) => bucket.elapsedMs === bucketMs)?.count;
}

function closedDispatchBucketElapsedMs(elapsedMs: number) {
  return elapsedMs >= 1000 ? Math.floor((elapsedMs - 1000) / 1000) * 1000 : undefined;
}

function appendRunnerResourceHistory(history: RunnerResourcePoint[], lines: unknown[] | undefined) {
  if (!lines) return;
  const points = extractRunnerResourcePoints(lines);
  if (points.length === 0) return;
  history.push(...points);
  if (history.length > MAX_RUNNER_RESOURCE_POINTS) {
    history.splice(0, history.length - MAX_RUNNER_RESOURCE_POINTS);
  }
}

function aggregateLineMetrics(lines: unknown[]): RemoteMetricsEvent | null {
  const metrics = lines
    .map((line) => {
      if (!isSseObject(line)) return null;
      return extractRemoteMetrics(isSseObject(line.payload) ? line.payload : line);
    })
    .filter((item): item is RemoteMetricsEvent => item !== null);

  if (metrics.length === 0) return null;
  const lifecycleByElapsed = new Map<number, LoadLifecycleBucket>();
  for (const item of metrics) {
    for (const bucket of item.lifecycleBuckets ?? []) {
      const current = lifecycleByElapsed.get(bucket.elapsedMs);
      lifecycleByElapsed.set(bucket.elapsedMs, {
        elapsedMs: bucket.elapsedMs,
        planned: (current?.planned ?? 0) + (bucket.planned ?? 0),
        slotEnqueued: (current?.slotEnqueued ?? 0) + (bucket.slotEnqueued ?? 0),
        requestPrepared: (current?.requestPrepared ?? 0) + (bucket.requestPrepared ?? 0),
        requestEnqueued: (current?.requestEnqueued ?? 0) + (bucket.requestEnqueued ?? 0),
        sendTaskSpawned: (current?.sendTaskSpawned ?? 0) + (bucket.sendTaskSpawned ?? 0),
        sendStarted: (current?.sendStarted ?? 0) + (bucket.sendStarted ?? 0),
        httpStarted: (current?.httpStarted ?? 0) + (bucket.httpStarted ?? 0),
        httpSendReturned: (current?.httpSendReturned ?? 0) + (bucket.httpSendReturned ?? 0),
        responseBodyCompleted: (current?.responseBodyCompleted ?? 0) + (bucket.responseBodyCompleted ?? 0),
        dispatcherLagged: (current?.dispatcherLagged ?? 0) + (bucket.dispatcherLagged ?? 0),
        runtimeLagged: (current?.runtimeLagged ?? 0) + (bucket.runtimeLagged ?? 0),
        senderLagged: (current?.senderLagged ?? 0) + (bucket.senderLagged ?? 0),
        senderStartLagMsMax: Math.max(
          current?.senderStartLagMsMax ?? 0,
          bucket.senderStartLagMsMax ?? 0,
        ) || undefined,
        httpSendDurationMsMax: Math.max(
          current?.httpSendDurationMsMax ?? 0,
          bucket.httpSendDurationMsMax ?? 0,
        ) || undefined,
        responseObservationDurationMsMax: Math.max(
          current?.responseObservationDurationMsMax ?? 0,
          bucket.responseObservationDurationMsMax ?? 0,
        ) || undefined,
      });
    }
  }

  const aggregated = metrics.reduce<RemoteMetricsEvent>(
    (acc, item) => {
      const nextTargetIntensity =
        item.targetIntensity !== undefined
          ? ((acc.targetIntensity ?? 0) + item.targetIntensity) / ((acc.targetIntensity === undefined ? 0 : 1) + 1)
          : acc.targetIntensity;
      return {
        totalSent: acc.totalSent + item.totalSent,
        totalStarted: item.totalStarted !== undefined
          ? (acc.totalStarted ?? 0) + item.totalStarted
          : acc.totalStarted,
        totalSuccess: acc.totalSuccess + item.totalSuccess,
        totalError: acc.totalError + item.totalError,
        httpStarted: item.httpStarted !== undefined
          ? (acc.httpStarted ?? 0) + item.httpStarted
          : acc.httpStarted,
        httpCompleted: item.httpCompleted !== undefined
          ? (acc.httpCompleted ?? 0) + item.httpCompleted
          : acc.httpCompleted,
        dispatchSubmitted: item.dispatchSubmitted !== undefined
          ? (acc.dispatchSubmitted ?? 0) + item.dispatchSubmitted
          : acc.dispatchSubmitted,
        dispatchStarted: item.dispatchStarted !== undefined
          ? (acc.dispatchStarted ?? 0) + item.dispatchStarted
          : acc.dispatchStarted,
        httpSendReturned: item.httpSendReturned !== undefined
          ? (acc.httpSendReturned ?? 0) + item.httpSendReturned
          : acc.httpSendReturned,
        responseBodyCompleted: item.responseBodyCompleted !== undefined
          ? (acc.responseBodyCompleted ?? 0) + item.responseBodyCompleted
          : acc.responseBodyCompleted,
        dependencyLimitedStarts: item.dependencyLimitedStarts !== undefined
          ? (acc.dependencyLimitedStarts ?? 0) + item.dependencyLimitedStarts
          : acc.dependencyLimitedStarts,
        dispatcherLaggedStarts: item.dispatcherLaggedStarts !== undefined
          ? (acc.dispatcherLaggedStarts ?? 0) + item.dispatcherLaggedStarts
          : acc.dispatcherLaggedStarts,
        runtimeLaggedStarts: item.runtimeLaggedStarts !== undefined
          ? (acc.runtimeLaggedStarts ?? 0) + item.runtimeLaggedStarts
          : acc.runtimeLaggedStarts,
        senderLaggedStarts: item.senderLaggedStarts !== undefined
          ? (acc.senderLaggedStarts ?? 0) + item.senderLaggedStarts
          : acc.senderLaggedStarts,
        senderQueueDepth: item.senderQueueDepth !== undefined
          ? (acc.senderQueueDepth ?? 0) + item.senderQueueDepth
          : acc.senderQueueDepth,
        senderStartLagAvgMs: maxOptional(acc.senderStartLagAvgMs, item.senderStartLagAvgMs),
        senderStartLagP95Ms: maxOptional(acc.senderStartLagP95Ms, item.senderStartLagP95Ms),
        senderStartLagP99Ms: maxOptional(acc.senderStartLagP99Ms, item.senderStartLagP99Ms),
        senderStartLagMaxMs: maxOptional(acc.senderStartLagMaxMs, item.senderStartLagMaxMs),
        httpSendDurationAvgMs: maxOptional(acc.httpSendDurationAvgMs, item.httpSendDurationAvgMs),
        httpSendDurationP95Ms: maxOptional(acc.httpSendDurationP95Ms, item.httpSendDurationP95Ms),
        httpSendDurationP99Ms: maxOptional(acc.httpSendDurationP99Ms, item.httpSendDurationP99Ms),
        responseObservationDurationAvgMs: maxOptional(
          acc.responseObservationDurationAvgMs,
          item.responseObservationDurationAvgMs,
        ),
        responseObservationDurationP95Ms: maxOptional(
          acc.responseObservationDurationP95Ms,
          item.responseObservationDurationP95Ms,
        ),
        responseObservationDurationP99Ms: maxOptional(
          acc.responseObservationDurationP99Ms,
          item.responseObservationDurationP99Ms,
        ),
        schedulerLagMs: item.schedulerLagMs !== undefined
          ? (acc.schedulerLagMs ?? 0) + item.schedulerLagMs
          : acc.schedulerLagMs,
        schedulerLaggedStarts: item.schedulerLaggedStarts !== undefined
          ? (acc.schedulerLaggedStarts ?? 0) + item.schedulerLaggedStarts
          : acc.schedulerLaggedStarts,
        dispatchBuckets: undefined,
        rps: acc.rps + item.rps,
        startTime: Math.min(acc.startTime, item.startTime),
        elapsedMs: Math.max(acc.elapsedMs, item.elapsedMs),
        targetIntensity: nextTargetIntensity,
        targetRpsLimit: (acc.targetRpsLimit ?? 0) + (item.targetRpsLimit ?? 0) || undefined,
        inFlight: (acc.inFlight ?? 0) + (item.inFlight ?? 0) || undefined,
        runnerMaxRps: (acc.runnerMaxRps ?? 0) + (item.runnerMaxRps ?? 0) || undefined,
        tickMs: Math.max(acc.tickMs ?? 0, item.tickMs ?? 0) || undefined,
        scheduledStarts: (acc.scheduledStarts ?? 0) + (item.scheduledStarts ?? 0) || undefined,
        missedStarts: (acc.missedStarts ?? 0) + (item.missedStarts ?? 0) || undefined,
        readyRequests: (acc.readyRequests ?? 0) + (item.readyRequests ?? 0) || undefined,
        activePipelines: (acc.activePipelines ?? 0) + (item.activePipelines ?? 0) || undefined,
        outstandingRequests: (acc.outstandingRequests ?? 0) + (item.outstandingRequests ?? 0) || undefined,
        curveAdherence: item.curveAdherence ?? acc.curveAdherence,
      };
    },
    {
      totalSent: 0,
      totalSuccess: 0,
      totalError: 0,
      rps: 0,
      startTime: Number.MAX_SAFE_INTEGER,
      elapsedMs: 0,
    },
  );

  return {
    ...aggregated,
    snapshotMode: metrics.some((item) => item.snapshotMode === "final") ? "final" : metrics[0]?.snapshotMode,
    lifecycleBuckets: Array.from(lifecycleByElapsed.values()).sort((a, b) => a.elapsedMs - b.elapsedMs),
    curveAdherence: computeCurveAdherence(
      aggregated.scheduledStarts,
      aggregated.missedStarts,
    ) ?? aggregated.curveAdherence,
  };
}

function extractNodesInfo(context: unknown): RemoteNodesInfo | null {
  if (!isSseObject(context)) return null;

  const usedNodes = pickStringArray(context.usedNodes);
  const activeNodes = pickStringArray(context.activeNodes);
  const registeredNodes = pickStringArray(context.registeredNodes);
  const nodeNames = usedNodes.length > 0 ? usedNodes : activeNodes;
  const nodesUsed = toNumber(context.usedNodesTotal ?? context.nodesUsed) ?? nodeNames.length;
  const nodesFound = toNumber(
    context.registeredNodesTotal ?? context.nodesFound ?? context.activeNodesTotal,
  ) ?? registeredNodes.length;

  if (nodesUsed === 0 && nodesFound === 0 && nodeNames.length === 0) {
    return null;
  }

  return {
    nodesUsed,
    nodesFound,
    nodeNames,
  };
}

function buildLoadMetricsFromSnapshot(snapshot: SseObject): LoadTestMetrics {
  const consolidated = isSseObject(snapshot.consolidated) ? snapshot.consolidated : null;
  const aggregated = Array.isArray(snapshot.lines)
    ? aggregateLineMetrics(snapshot.lines)
    : null;
  const startTime = toNumber(consolidated?.startTime) ?? aggregated?.startTime ?? Date.now();
  const rps = toNumber(consolidated?.rps) ?? aggregated?.rps ?? 0;
  const totalSent = toNumber(consolidated?.totalSent) ?? aggregated?.totalSent ?? 0;
  const totalStarted = toNumber(consolidated?.totalStarted) ?? aggregated?.totalStarted;
  const httpStarted = toNumber(consolidated?.httpStarted) ?? aggregated?.httpStarted;
  const httpCompleted = toNumber(consolidated?.httpCompleted) ?? aggregated?.httpCompleted;
  const dispatchSubmitted = toNumber(consolidated?.dispatchSubmitted) ?? aggregated?.dispatchSubmitted;
  const dispatchStarted = toNumber(consolidated?.dispatchStarted) ?? aggregated?.dispatchStarted;
  const httpSendReturned = toNumber(consolidated?.httpSendReturned) ?? aggregated?.httpSendReturned;
  const responseBodyCompleted = toNumber(consolidated?.responseBodyCompleted) ?? aggregated?.responseBodyCompleted;
  const dependencyLimitedStarts = toNumber(consolidated?.dependencyLimitedStarts) ?? aggregated?.dependencyLimitedStarts;
  const dispatcherLaggedStarts = toNumber(consolidated?.dispatcherLaggedStarts) ?? aggregated?.dispatcherLaggedStarts;
  const runtimeLaggedStarts = toNumber(consolidated?.runtimeLaggedStarts) ?? aggregated?.runtimeLaggedStarts;
  const senderLaggedStarts = toNumber(consolidated?.senderLaggedStarts) ?? aggregated?.senderLaggedStarts;
  const senderQueueDepth = toNumber(consolidated?.senderQueueDepth) ?? aggregated?.senderQueueDepth;
  const senderStartLagAvgMs = toNumber(consolidated?.senderStartLagAvgMs) ?? aggregated?.senderStartLagAvgMs;
  const senderStartLagP95Ms = toNumber(consolidated?.senderStartLagP95Ms) ?? aggregated?.senderStartLagP95Ms;
  const senderStartLagP99Ms = toNumber(consolidated?.senderStartLagP99Ms) ?? aggregated?.senderStartLagP99Ms;
  const senderStartLagMaxMs = toNumber(consolidated?.senderStartLagMaxMs) ?? aggregated?.senderStartLagMaxMs;
  const httpSendDurationAvgMs = toNumber(consolidated?.httpSendDurationAvgMs) ?? aggregated?.httpSendDurationAvgMs;
  const httpSendDurationP95Ms = toNumber(consolidated?.httpSendDurationP95Ms) ?? aggregated?.httpSendDurationP95Ms;
  const httpSendDurationP99Ms = toNumber(consolidated?.httpSendDurationP99Ms) ?? aggregated?.httpSendDurationP99Ms;
  const responseObservationDurationAvgMs = toNumber(consolidated?.responseObservationDurationAvgMs) ?? aggregated?.responseObservationDurationAvgMs;
  const responseObservationDurationP95Ms = toNumber(consolidated?.responseObservationDurationP95Ms) ?? aggregated?.responseObservationDurationP95Ms;
  const responseObservationDurationP99Ms = toNumber(consolidated?.responseObservationDurationP99Ms) ?? aggregated?.responseObservationDurationP99Ms;
  const schedulerLagMs = toNumber(consolidated?.schedulerLagMs) ?? aggregated?.schedulerLagMs;
  const schedulerLaggedStarts = toNumber(consolidated?.schedulerLaggedStarts) ?? aggregated?.schedulerLaggedStarts;
  const targetIntensity = toNumber(consolidated?.targetIntensity) ?? aggregated?.targetIntensity;
  const targetRpsLimit = toNumber(consolidated?.targetRpsLimit) ?? aggregated?.targetRpsLimit;
  const scheduledStarts = toNumber(consolidated?.scheduledStarts) ?? aggregated?.scheduledStarts;
  const missedStarts = toNumber(consolidated?.missedStarts) ?? aggregated?.missedStarts;
  const readyRequests = toNumber(consolidated?.readyRequests) ?? aggregated?.readyRequests;
  const activePipelines = toNumber(consolidated?.activePipelines) ?? aggregated?.activePipelines;
  const outstandingRequests = toNumber(consolidated?.outstandingRequests) ?? aggregated?.outstandingRequests;
  const curveAdherence = toNumber(consolidated?.curveAdherence) ?? aggregated?.curveAdherence;
  const errors = pickStringArray(snapshot.errors);

  const elapsedMs = toNumber(consolidated?.elapsedMs) ?? aggregated?.elapsedMs ?? 0;
  const lifecycleBuckets = Array.isArray(consolidated?.lifecycleBuckets)
    ? extractLifecycleBuckets(consolidated.lifecycleBuckets)
    : aggregated?.lifecycleBuckets ?? [];
  const lifecycleBucket = lifecycleBuckets.find((bucket) => bucket.elapsedMs === elapsedMs);

  return {
    snapshotMode: aggregated?.snapshotMode,
    totalSent,
    totalStarted,
    httpStarted,
    httpCompleted,
    totalSuccess: toNumber(consolidated?.totalSuccess) ?? aggregated?.totalSuccess ?? 0,
    totalError: toNumber(consolidated?.totalError) ?? aggregated?.totalError ?? 0,
    avgLatency: toNumber(consolidated?.avgLatency) ?? 0,
    p95: toNumber(consolidated?.p95) ?? 0,
    p99: toNumber(consolidated?.p99) ?? 0,
    rps,
    latencyHistory: [],
    rpsHistory: rps > 0 ? [{
      timestamp: startTime + elapsedMs,
      elapsedMs,
      rps: lifecycleBucket?.httpStarted ?? rps,
      lifecycleBucket,
      totalStarted,
      totalSent,
      httpStarted,
      httpCompleted,
      dispatchSubmitted,
      dispatchStarted,
      httpSendReturned,
      responseBodyCompleted,
      dependencyLimitedStarts,
      dispatcherLaggedStarts,
      runtimeLaggedStarts,
      senderLaggedStarts,
      senderQueueDepth,
      senderStartLagAvgMs,
      senderStartLagP95Ms,
      senderStartLagP99Ms,
      senderStartLagMaxMs,
      httpSendDurationAvgMs,
      httpSendDurationP95Ms,
      httpSendDurationP99Ms,
      responseObservationDurationAvgMs,
      responseObservationDurationP95Ms,
      responseObservationDurationP99Ms,
      schedulerLagMs,
      schedulerLaggedStarts,
      targetIntensity,
      targetRpsLimit,
      scheduledStarts,
      missedStarts,
      readyRequests,
      activePipelines,
      outstandingRequests,
      curveAdherence,
    }] : [],
    runnerResourceHistory: Array.isArray(snapshot.lines)
      ? extractRunnerResourcePoints(snapshot.lines)
      : [],
    lifecycleBuckets,
    errors,
    startTime,
    elapsedMs,
    targetIntensity,
    targetRpsLimit,
    inFlight: toNumber(consolidated?.inFlight) ?? aggregated?.inFlight,
    runnerMaxRps: toNumber(consolidated?.runnerMaxRps) ?? aggregated?.runnerMaxRps,
    tickMs: toNumber(consolidated?.tickMs) ?? aggregated?.tickMs,
    scheduledStarts,
    missedStarts,
    dispatchSubmitted,
    dispatchStarted,
    httpSendReturned,
    responseBodyCompleted,
    dependencyLimitedStarts,
    dispatcherLaggedStarts,
    runtimeLaggedStarts,
    senderLaggedStarts,
    senderQueueDepth,
    senderStartLagAvgMs,
    senderStartLagP95Ms,
    senderStartLagP99Ms,
    senderStartLagMaxMs,
    httpSendDurationAvgMs,
    httpSendDurationP95Ms,
    httpSendDurationP99Ms,
    responseObservationDurationAvgMs,
    responseObservationDurationP95Ms,
    responseObservationDurationP99Ms,
    schedulerLagMs,
    schedulerLaggedStarts,
    readyRequests,
    activePipelines,
    outstandingRequests,
    curveAdherence,
  };
}

function extractSnapshotStepId(value: unknown): string | undefined {
  if (!isSseObject(value)) return undefined;
  const nestedStep = isSseObject(value.step) ? value.step : undefined;
  return pickString(value.stepId, nestedStep?.id);
}

export function parseIntegrationSnapshot(value: unknown): RemoteIntegrationSnapshot | null {
  if (!isSseObject(value)) return null;
  if (value.kind !== undefined && value.kind !== "e2e") return null;

  const results: Record<string, StepExecutionResult> = {};
  const steps = Array.isArray(value.steps) ? value.steps : [];

  for (const rawStep of steps) {
    if (!isSseObject(rawStep)) continue;
    const stepId = extractSnapshotStepId(rawStep);
    if (!stepId) continue;

    results[stepId] = {
      ...(rawStep as Partial<StepExecutionResult>),
      stepId,
      status: resolveStepStatus(rawStep.status),
    };
  }

  return {
    executionId: pickString(value.executionId) ?? null,
    status: pickString(value.status) ?? "running",
    results,
    summary: isSseObject(value.summary) ? (value.summary as Record<string, unknown>) : null,
    errors: Array.isArray(value.errors)
      ? value.errors.filter((item): item is string => typeof item === "string")
      : [],
  };
}

export function parseLoadExecutionSnapshot(value: unknown): RemoteLoadExecutionSnapshot | null {
  if (!isSseObject(value)) return null;
  if (value.kind !== undefined && value.kind !== "load") return null;

  const status = pickString(value.status) ?? "running";

  return {
    executionId: pickString(value.executionId) ?? null,
    status,
    state: mapLoadSnapshotStatus(status),
    metrics: buildLoadMetricsFromSnapshot(value),
    nodesInfo: extractNodesInfo(value.context),
    errors: Array.isArray(value.errors)
      ? value.errors.filter((item): item is string => typeof item === "string")
      : [],
  };
}

function dispatchIntegrationEvent(
  rawEvent: string,
  envelope: SseObject,
  payload: SseObject,
  callbacks: RemoteIntegrationCallbacks,
  options?: {
    linePayload?: SseObject;
    lineRunnerEvent?: string;
    onExecutionInit?: (executionId: string | null) => void;
    skipTopLevelStepEvents?: boolean;
  },
): boolean {
  const eventName = resolveEventName(rawEvent, envelope, payload, options?.lineRunnerEvent);
  const sourcePayload = options?.linePayload ?? payload;

  if (options?.skipTopLevelStepEvents && !options?.lineRunnerEvent && (eventName === "step:start" || eventName === "step:result")) {
    return false;
  }

  switch (eventName) {
    case "execution:init": {
      options?.onExecutionInit?.(pickString(envelope.executionId, payload.executionId, sourcePayload.executionId) ?? null);
      return true;
    }
    case "execution:snapshot": {
      const snapshot = parseIntegrationSnapshot(sourcePayload);
      if (!snapshot) return false;
      callbacks.onSnapshot?.(snapshot);
      return true;
    }
    case "step:start": {
      const stepId = resolveStepId(envelope, payload, options?.linePayload);
      if (!stepId) return false;
      const meta = extractAttemptMeta(envelope, payload, options?.linePayload);
      callbacks.onStepStart(stepId, meta);
      return true;
    }
    case "step:result": {
      const stepId = resolveStepId(envelope, payload, options?.linePayload);
      if (!stepId) return false;
      const meta = extractAttemptMeta(envelope, payload, options?.linePayload);
      const mapped: StepExecutionResult = {
        ...(sourcePayload as Partial<StepExecutionResult>),
        stepId,
        status: resolveStepStatus(sourcePayload.status, payload.status, envelope.status),
        attempts: meta.attempt,
        maxAttempts: meta.maxAttempts,
      };
      callbacks.onStepResult(stepId, mapped);
      return true;
    }
    case "pipeline:complete":
      callbacks.onComplete(sourcePayload as { totalSteps: number; passed: number; failed: number; totalDuration: number });
      return true;
    case "error":
      callbacks.onError(
        pickString(envelope.message, sourcePayload.message, sourcePayload.error, payload.message, payload.error) ?? "Erro desconhecido"
      );
      return true;
    default:
      return false;
  }
}

function handleIntegrationEnvelope(
  rawEvent: string,
  envelope: SseObject,
  callbacks: RemoteIntegrationCallbacks,
  onExecutionInit?: (executionId: string | null) => void,
): void {
  const payload = isSseObject(envelope.payload) ? envelope.payload : envelope;

  const nodeName = pickString(envelope.node);
  if (nodeName && callbacks.onNodeInfo) {
    callbacks.onNodeInfo(nodeName);
  }

  const lines = Array.isArray(envelope.lines) ? (envelope.lines as OrchestratorLine[]) : [];
  let handledLineStepEvent = false;

  for (const line of lines) {
    const lineRunnerEvent = pickString(line.runnerEvent);
    const linePayload = isSseObject(line.payload) ? line.payload : undefined;
    const lineNode = pickString(line.node);

    if (lineNode && callbacks.onNodeInfo) {
      callbacks.onNodeInfo(lineNode);
    }

    if (!lineRunnerEvent) continue;

    const handled = dispatchIntegrationEvent(rawEvent, envelope, payload, callbacks, {
      linePayload,
      lineRunnerEvent,
      onExecutionInit,
    });

    if (handled && (lineRunnerEvent === "step:start" || lineRunnerEvent === "step:result")) {
      handledLineStepEvent = true;
    }
  }

  dispatchIntegrationEvent(rawEvent, envelope, payload, callbacks, {
    onExecutionInit,
    skipTopLevelStepEvents: handledLineStepEvent,
  });
}

// ============ Remote End-to-End Test ============

export interface RemoteIntegrationCallbacks {
  onExecutionInit?: (executionId: string | null) => void;
  onStepStart: (stepId: string, meta?: { attempt?: number; maxAttempts?: number; startedAt?: number }) => void;
  onStepResult: (stepId: string, result: StepExecutionResult) => void;
  onComplete: (summary: { totalSteps: number; passed: number; failed: number; totalDuration: number }) => void;
  onError: (error: string) => void;
  onNodeInfo?: (node: string) => void;
  onSnapshot?: (snapshot: RemoteIntegrationSnapshot) => void;
}

export interface RemoteExecutionController {
  cancel: () => void;
  disconnect: () => void;
}

export function runRemoteIntegrationTest(
  backendUrl: string,
  pipeline: Pipeline,
  callbacks: RemoteIntegrationCallbacks,
  projectId: string,
  selectedBaseUrlKey?: string,
  pipelineIndex?: number,
  specs?: Array<{ slug: string; servers: Record<string, string> }>,
  envGroups?: Array<{ slug: string; urls: Record<string, string> }>,
  selectedEnvGroupSlug?: string | null
): RemoteExecutionController {
  const abortController = new AbortController();
  const transactionId = generateUUID();
  let executionId: string | null = null;

  const run = async () => {
    try {
      const base = ensureApiPrefix(backendUrl);
      const basePath = `${base}/projects/${projectId}/tests/e2e`;
      const body = {
        pipelineId: pipeline.id,
        selectedBaseUrlKey,
        selectedEnvGroupSlug,
        pipelineIndex,
        specs,
        envGroups,
      };
      const response = await fetch(basePath, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Accept": "text/event-stream",
          "x-transaction-id": transactionId,
        },
        body: JSON.stringify(body),
        signal: abortController.signal,
      });

      if (!response.ok) {
        const err = await response.text();
        callbacks.onError(`HTTP ${response.status}: ${err}`);
        return;
      }

      const reader = response.body?.getReader();
      if (!reader) {
        callbacks.onError("Stream não suportado pelo servidor");
        return;
      }

      const decoder = new TextDecoder();
      let buffer = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const parts = buffer.split("\n\n");
        buffer = parts.pop() || "";

        for (const part of parts) {
          if (!part.trim()) continue;
          const events = parseSSE(part + "\n\n");

          for (const { event, data } of events) {
            try {
              const envelope = JSON.parse(data) as SseObject;
              handleIntegrationEnvelope(event, envelope, callbacks, (id) => {
                executionId = id;
                callbacks.onExecutionInit?.(id);
              });
            } catch {
              // Skip malformed JSON
            }
          }
        }
      }

      // Process remaining buffer after stream ends
      if (buffer.trim()) {
        const events = parseSSE(buffer + "\n\n");
        for (const { event, data } of events) {
          try {
            const envelope = JSON.parse(data) as SseObject;
            handleIntegrationEnvelope(event, envelope, callbacks, (id) => {
              executionId = id;
              callbacks.onExecutionInit?.(id);
            });
          } catch {
            // skip
          }
        }
      }
    } catch (err) {
      if ((err as Error).name !== "AbortError") {
        callbacks.onError(err instanceof Error ? err.message : String(err));
      }
    }
  };

  run();

  return {
    cancel: () => {
      if (executionId) {
        cancelExecution(backendUrl, executionId);
      }
      abortController.abort();
    },
    disconnect: () => {
      abortController.abort();
    },
  };
}

export function runRemoteIntegrationFromStep(
  backendUrl: string,
  pipeline: Pipeline,
  startStepId: string,
  priorResults: Record<string, StepExecutionResult>,
  callbacks: RemoteIntegrationCallbacks,
  projectId: string,
  selectedBaseUrlKey?: string,
  pipelineIndex?: number,
  specs?: Array<{ slug: string; servers: Record<string, string> }>,
  envGroups?: Array<{ slug: string; urls: Record<string, string> }>,
  selectedEnvGroupSlug?: string | null
): RemoteExecutionController {
  const abortController = new AbortController();
  const transactionId = generateUUID();
  let executionId: string | null = null;

  const run = async () => {
    try {
      const base = ensureApiPrefix(backendUrl);
      const basePath = `${base}/projects/${projectId}/tests/e2e/rerun-from-step`;
      const body = {
        pipelineId: pipeline.id,
        startStepId,
        priorResults,
        selectedBaseUrlKey,
        selectedEnvGroupSlug,
        pipelineIndex,
        specs,
        envGroups,
      };
      const response = await fetch(basePath, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Accept": "text/event-stream",
          "x-transaction-id": transactionId,
        },
        body: JSON.stringify(body),
        signal: abortController.signal,
      });

      if (!response.ok) {
        const err = await response.text();
        callbacks.onError(`HTTP ${response.status}: ${err}`);
        return;
      }

      const reader = response.body?.getReader();
      if (!reader) {
        callbacks.onError("Stream não suportado pelo servidor");
        return;
      }

      const decoder = new TextDecoder();
      let buffer = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const parts = buffer.split("\n\n");
        buffer = parts.pop() || "";

        for (const part of parts) {
          if (!part.trim()) continue;
          const events = parseSSE(part + "\n\n");

          for (const { event, data } of events) {
            try {
              const envelope = JSON.parse(data) as SseObject;
              handleIntegrationEnvelope(event, envelope, callbacks, (id) => {
                executionId = id;
                callbacks.onExecutionInit?.(id);
              });
            } catch {
              // Skip malformed JSON
            }
          }
        }
      }

      if (buffer.trim()) {
        const events = parseSSE(buffer + "\n\n");
        for (const { event, data } of events) {
          try {
            const envelope = JSON.parse(data) as SseObject;
            handleIntegrationEnvelope(event, envelope, callbacks, (id) => {
              executionId = id;
              callbacks.onExecutionInit?.(id);
            });
          } catch {
            // skip
          }
        }
      }
    } catch (err) {
      if ((err as Error).name !== "AbortError") {
        callbacks.onError(err instanceof Error ? err.message : String(err));
      }
    }
  };

  run();

  return {
    cancel: () => {
      if (executionId) {
        cancelExecution(backendUrl, executionId);
      }
      abortController.abort();
    },
    disconnect: () => {
      abortController.abort();
    },
  };
}

// ============ Remote Load Test ============

export interface RemoteLoadTestCallbacks {
  onMetricsUpdate: (metrics: LoadTestMetrics) => void;
  onComplete: (metrics: LoadTestMetrics) => void;
  onError: (error: string) => void;
  onNodesInfo?: (info: RemoteNodesInfo) => void;
  onSnapshot?: (snapshot: RemoteLoadExecutionSnapshot) => void;
}

export interface RemoteLoadTestController {
  cancel: () => void;
  disconnect: () => void;
}

/** Consolidate all known node metrics into a single snapshot. */
function consolidateNodeMetrics(nodeMap: Map<string, RemoteMetricsEvent>): RemoteMetricsEvent | null {
  if (nodeMap.size === 0) return null;

  let totalSent = 0, totalSuccess = 0, totalError = 0, rpsSum = 0;
  let totalStarted = 0, hasTotalStarted = false;
  let startTime = Infinity, maxElapsed = 0;
  let targetIntensityTotal = 0, targetIntensityCount = 0;
  let targetRpsLimit = 0, hasTargetRpsLimit = false;
  let inFlight = 0, hasInFlight = false;
  let runnerMaxRps = 0, hasRunnerMaxRps = false;
  let tickMs = 0;
  let scheduledStarts = 0, hasScheduledStarts = false;
  let missedStarts = 0, hasMissedStarts = false;
  let dispatchSubmitted = 0, hasDispatchSubmitted = false;
  let dispatchStarted = 0, hasDispatchStarted = false;
  let httpSendReturned = 0, hasHttpSendReturned = false;
  let responseBodyCompleted = 0, hasResponseBodyCompleted = false;
  let dependencyLimitedStarts = 0, hasDependencyLimitedStarts = false;
  let dispatcherLaggedStarts = 0, hasDispatcherLaggedStarts = false;
  let runtimeLaggedStarts = 0, hasRuntimeLaggedStarts = false;
  let senderLaggedStarts = 0, hasSenderLaggedStarts = false;
  let senderQueueDepth = 0, hasSenderQueueDepth = false;
  let senderStartLagAvgMs: number | undefined;
  let senderStartLagP95Ms: number | undefined;
  let senderStartLagP99Ms: number | undefined;
  let senderStartLagMaxMs: number | undefined;
  let httpSendDurationAvgMs: number | undefined;
  let httpSendDurationP95Ms: number | undefined;
  let httpSendDurationP99Ms: number | undefined;
  let responseObservationDurationAvgMs: number | undefined;
  let responseObservationDurationP95Ms: number | undefined;
  let responseObservationDurationP99Ms: number | undefined;
  let schedulerLagMs = 0, hasSchedulerLagMs = false;
  let schedulerLaggedStarts = 0, hasSchedulerLaggedStarts = false;
  let readyRequests = 0, hasReadyRequests = false;
  let activePipelines = 0, hasActivePipelines = false;
  let outstandingRequests = 0, hasOutstandingRequests = false;

  for (const p of nodeMap.values()) {
    totalSent += p.totalSent;
    if (typeof p.totalStarted === "number") {
      totalStarted += p.totalStarted;
      hasTotalStarted = true;
    }
    totalSuccess += p.totalSuccess;
    totalError += p.totalError;
    rpsSum += p.rps;
    if (p.startTime < startTime) startTime = p.startTime;
    if (p.elapsedMs > maxElapsed) maxElapsed = p.elapsedMs;
    if (typeof p.targetIntensity === "number") {
      targetIntensityTotal += p.targetIntensity;
      targetIntensityCount += 1;
    }
    if (typeof p.targetRpsLimit === "number") {
      targetRpsLimit += p.targetRpsLimit;
      hasTargetRpsLimit = true;
    }
    if (typeof p.inFlight === "number") {
      inFlight += p.inFlight;
      hasInFlight = true;
    }
    if (typeof p.runnerMaxRps === "number") {
      runnerMaxRps += p.runnerMaxRps;
      hasRunnerMaxRps = true;
    }
    if (typeof p.tickMs === "number") {
      tickMs = Math.max(tickMs, p.tickMs);
    }
    if (typeof p.scheduledStarts === "number") {
      scheduledStarts += p.scheduledStarts;
      hasScheduledStarts = true;
    }
    if (typeof p.missedStarts === "number") {
      missedStarts += p.missedStarts;
      hasMissedStarts = true;
    }
    if (typeof p.dispatchSubmitted === "number") {
      dispatchSubmitted += p.dispatchSubmitted;
      hasDispatchSubmitted = true;
    }
    if (typeof p.dispatchStarted === "number") {
      dispatchStarted += p.dispatchStarted;
      hasDispatchStarted = true;
    }
    if (typeof p.httpSendReturned === "number") {
      httpSendReturned += p.httpSendReturned;
      hasHttpSendReturned = true;
    }
    if (typeof p.responseBodyCompleted === "number") {
      responseBodyCompleted += p.responseBodyCompleted;
      hasResponseBodyCompleted = true;
    }
    if (typeof p.dependencyLimitedStarts === "number") {
      dependencyLimitedStarts += p.dependencyLimitedStarts;
      hasDependencyLimitedStarts = true;
    }
    if (typeof p.dispatcherLaggedStarts === "number") {
      dispatcherLaggedStarts += p.dispatcherLaggedStarts;
      hasDispatcherLaggedStarts = true;
    }
    if (typeof p.runtimeLaggedStarts === "number") {
      runtimeLaggedStarts += p.runtimeLaggedStarts;
      hasRuntimeLaggedStarts = true;
    }
    if (typeof p.senderLaggedStarts === "number") {
      senderLaggedStarts += p.senderLaggedStarts;
      hasSenderLaggedStarts = true;
    }
    if (typeof p.senderQueueDepth === "number") {
      senderQueueDepth += p.senderQueueDepth;
      hasSenderQueueDepth = true;
    }
    senderStartLagAvgMs = maxOptional(senderStartLagAvgMs, p.senderStartLagAvgMs);
    senderStartLagP95Ms = maxOptional(senderStartLagP95Ms, p.senderStartLagP95Ms);
    senderStartLagP99Ms = maxOptional(senderStartLagP99Ms, p.senderStartLagP99Ms);
    senderStartLagMaxMs = maxOptional(senderStartLagMaxMs, p.senderStartLagMaxMs);
    httpSendDurationAvgMs = maxOptional(httpSendDurationAvgMs, p.httpSendDurationAvgMs);
    httpSendDurationP95Ms = maxOptional(httpSendDurationP95Ms, p.httpSendDurationP95Ms);
    httpSendDurationP99Ms = maxOptional(httpSendDurationP99Ms, p.httpSendDurationP99Ms);
    responseObservationDurationAvgMs = maxOptional(
      responseObservationDurationAvgMs,
      p.responseObservationDurationAvgMs,
    );
    responseObservationDurationP95Ms = maxOptional(
      responseObservationDurationP95Ms,
      p.responseObservationDurationP95Ms,
    );
    responseObservationDurationP99Ms = maxOptional(
      responseObservationDurationP99Ms,
      p.responseObservationDurationP99Ms,
    );
    if (typeof p.schedulerLagMs === "number") {
      schedulerLagMs += p.schedulerLagMs;
      hasSchedulerLagMs = true;
    }
    if (typeof p.schedulerLaggedStarts === "number") {
      schedulerLaggedStarts += p.schedulerLaggedStarts;
      hasSchedulerLaggedStarts = true;
    }
    if (typeof p.readyRequests === "number") {
      readyRequests += p.readyRequests;
      hasReadyRequests = true;
    }
    if (typeof p.activePipelines === "number") {
      activePipelines += p.activePipelines;
      hasActivePipelines = true;
    }
    if (typeof p.outstandingRequests === "number") {
      outstandingRequests += p.outstandingRequests;
      hasOutstandingRequests = true;
    }
  }
  const curveAdherence = computeCurveAdherence(
    hasScheduledStarts ? scheduledStarts : undefined,
    hasMissedStarts ? missedStarts : undefined,
  );

  return {
    totalSent,
    totalStarted: hasTotalStarted ? totalStarted : undefined,
    totalSuccess,
    totalError,
    rps: rpsSum,
    startTime,
    elapsedMs: maxElapsed,
    targetIntensity: targetIntensityCount > 0 ? targetIntensityTotal / targetIntensityCount : undefined,
    targetRpsLimit: hasTargetRpsLimit ? targetRpsLimit : undefined,
    inFlight: hasInFlight ? inFlight : undefined,
    runnerMaxRps: hasRunnerMaxRps ? runnerMaxRps : undefined,
    tickMs: tickMs > 0 ? tickMs : undefined,
    scheduledStarts: hasScheduledStarts ? scheduledStarts : undefined,
    missedStarts: hasMissedStarts ? missedStarts : undefined,
    dispatchSubmitted: hasDispatchSubmitted ? dispatchSubmitted : undefined,
    dispatchStarted: hasDispatchStarted ? dispatchStarted : undefined,
    httpSendReturned: hasHttpSendReturned ? httpSendReturned : undefined,
    responseBodyCompleted: hasResponseBodyCompleted ? responseBodyCompleted : undefined,
    dependencyLimitedStarts: hasDependencyLimitedStarts ? dependencyLimitedStarts : undefined,
    dispatcherLaggedStarts: hasDispatcherLaggedStarts ? dispatcherLaggedStarts : undefined,
    runtimeLaggedStarts: hasRuntimeLaggedStarts ? runtimeLaggedStarts : undefined,
    senderLaggedStarts: hasSenderLaggedStarts ? senderLaggedStarts : undefined,
    senderQueueDepth: hasSenderQueueDepth ? senderQueueDepth : undefined,
    senderStartLagAvgMs,
    senderStartLagP95Ms,
    senderStartLagP99Ms,
    senderStartLagMaxMs,
    httpSendDurationAvgMs,
    httpSendDurationP95Ms,
    httpSendDurationP99Ms,
    responseObservationDurationAvgMs,
    responseObservationDurationP95Ms,
    responseObservationDurationP99Ms,
    schedulerLagMs: hasSchedulerLagMs ? schedulerLagMs : undefined,
    schedulerLaggedStarts: hasSchedulerLaggedStarts ? schedulerLaggedStarts : undefined,
    readyRequests: hasReadyRequests ? readyRequests : undefined,
    activePipelines: hasActivePipelines ? activePipelines : undefined,
    outstandingRequests: hasOutstandingRequests ? outstandingRequests : undefined,
    curveAdherence,
  };
}

function buildRpsHistoryPoint(
  fallbackTimestamp: number,
  event: RemoteMetricsEvent,
  consolidated?: ConsolidatedLoadMetrics | null,
  nodes?: Map<string, RemoteMetricsEvent>,
): RpsPoint {
  const startTime = consolidated?.startTime ?? event.startTime;
  const elapsedMs = consolidated?.elapsedMs ?? event.elapsedMs;
  const dispatchElapsedMs = closedDispatchBucketElapsedMs(elapsedMs);
  const sampleElapsedMs = dispatchElapsedMs ?? elapsedMs;
  const runners = nodes
    ? Array.from(nodes.entries()).map(([runnerId, metrics]) => {
      const dispatchBucket = dispatchElapsedMs !== undefined
        ? dispatchBucketFor(metrics, dispatchElapsedMs)
        : undefined;
      const lifecycleBucket = metrics.lifecycleBuckets?.find(
        (bucket) => bucket.elapsedMs === sampleElapsedMs,
      );
      return {
        runnerId,
        dispatchBucket,
        lifecycleBucket,
        httpStarted: metrics.httpStarted,
        httpCompleted: metrics.httpCompleted,
        dispatchSubmitted: metrics.dispatchSubmitted,
        dispatchStarted: metrics.dispatchStarted,
        httpSendReturned: metrics.httpSendReturned,
        responseBodyCompleted: metrics.responseBodyCompleted,
        dependencyLimitedStarts: metrics.dependencyLimitedStarts,
        dispatcherLaggedStarts: metrics.dispatcherLaggedStarts,
        runtimeLaggedStarts: metrics.runtimeLaggedStarts,
        senderLaggedStarts: metrics.senderLaggedStarts,
        senderQueueDepth: metrics.senderQueueDepth,
        schedulerLagMs: metrics.schedulerLagMs,
        schedulerLaggedStarts: metrics.schedulerLaggedStarts,
        totalStarted: metrics.totalStarted,
        totalSent: metrics.totalSent,
        rps: lifecycleBucket?.httpStarted ?? metrics.rps,
        scheduledStarts: metrics.scheduledStarts,
        missedStarts: metrics.missedStarts,
        readyRequests: metrics.readyRequests,
        activePipelines: metrics.activePipelines,
        outstandingRequests: metrics.outstandingRequests,
        curveAdherence: metrics.curveAdherence,
      };
    })
    : undefined;
  const dispatchBuckets = runners
    ?.map((runner) => runner.dispatchBucket)
    .filter((value): value is number => typeof value === "number");
  const dispatchBucket = dispatchBuckets && dispatchBuckets.length > 0
    ? dispatchBuckets.reduce((sum, value) => sum + value, 0)
    : undefined;
  const lifecycleBucket = consolidated?.lifecycleBuckets?.find(
    (bucket) => bucket.elapsedMs === sampleElapsedMs,
  ) ?? event.lifecycleBuckets?.find((bucket) => bucket.elapsedMs === sampleElapsedMs);

  return {
    timestamp: Number.isFinite(startTime) && Number.isFinite(sampleElapsedMs)
      ? startTime + sampleElapsedMs
      : fallbackTimestamp,
    elapsedMs: sampleElapsedMs,
    rps: lifecycleBucket?.httpStarted ?? consolidated?.rps ?? event.rps,
    dispatchBucket: dispatchBucket !== undefined ? dispatchBucket : undefined,
    lifecycleBucket,
    totalStarted: consolidated?.totalStarted ?? event.totalStarted,
    totalSent: consolidated?.totalSent ?? event.totalSent,
    httpStarted: consolidated?.httpStarted ?? event.httpStarted,
    httpCompleted: consolidated?.httpCompleted ?? event.httpCompleted,
    dispatchSubmitted: consolidated?.dispatchSubmitted ?? event.dispatchSubmitted,
    dispatchStarted: consolidated?.dispatchStarted ?? event.dispatchStarted,
    httpSendReturned: consolidated?.httpSendReturned ?? event.httpSendReturned,
    responseBodyCompleted: consolidated?.responseBodyCompleted ?? event.responseBodyCompleted,
    dependencyLimitedStarts: consolidated?.dependencyLimitedStarts ?? event.dependencyLimitedStarts,
    dispatcherLaggedStarts: consolidated?.dispatcherLaggedStarts ?? event.dispatcherLaggedStarts,
    runtimeLaggedStarts: consolidated?.runtimeLaggedStarts ?? event.runtimeLaggedStarts,
    senderLaggedStarts: consolidated?.senderLaggedStarts ?? event.senderLaggedStarts,
    senderQueueDepth: consolidated?.senderQueueDepth ?? event.senderQueueDepth,
    senderStartLagAvgMs: consolidated?.senderStartLagAvgMs ?? event.senderStartLagAvgMs,
    senderStartLagP95Ms: consolidated?.senderStartLagP95Ms ?? event.senderStartLagP95Ms,
    senderStartLagP99Ms: consolidated?.senderStartLagP99Ms ?? event.senderStartLagP99Ms,
    senderStartLagMaxMs: consolidated?.senderStartLagMaxMs ?? event.senderStartLagMaxMs,
    httpSendDurationAvgMs: consolidated?.httpSendDurationAvgMs ?? event.httpSendDurationAvgMs,
    httpSendDurationP95Ms: consolidated?.httpSendDurationP95Ms ?? event.httpSendDurationP95Ms,
    httpSendDurationP99Ms: consolidated?.httpSendDurationP99Ms ?? event.httpSendDurationP99Ms,
    responseObservationDurationAvgMs: consolidated?.responseObservationDurationAvgMs ?? event.responseObservationDurationAvgMs,
    responseObservationDurationP95Ms: consolidated?.responseObservationDurationP95Ms ?? event.responseObservationDurationP95Ms,
    responseObservationDurationP99Ms: consolidated?.responseObservationDurationP99Ms ?? event.responseObservationDurationP99Ms,
    schedulerLagMs: consolidated?.schedulerLagMs ?? event.schedulerLagMs,
    schedulerLaggedStarts: consolidated?.schedulerLaggedStarts ?? event.schedulerLaggedStarts,
    targetIntensity: consolidated?.targetIntensity ?? event.targetIntensity,
    targetRpsLimit: consolidated?.targetRpsLimit ?? event.targetRpsLimit,
    scheduledStarts: consolidated?.scheduledStarts ?? event.scheduledStarts,
    missedStarts: consolidated?.missedStarts ?? event.missedStarts,
    readyRequests: consolidated?.readyRequests ?? event.readyRequests,
    activePipelines: consolidated?.activePipelines ?? event.activePipelines,
    outstandingRequests: consolidated?.outstandingRequests ?? event.outstandingRequests,
    curveAdherence: consolidated?.curveAdherence ?? event.curveAdherence,
    runners,
  };
}

export function runRemoteLoadTest(
  backendUrl: string,
  pipeline: Pipeline,
  config: LoadRunConfig,
  callbacks: RemoteLoadTestCallbacks,
  projectId: string,
  selectedBaseUrlKey?: string,
  pipelineIndex?: number,
  specs?: Array<{ slug: string; servers: Record<string, string> }>,
  envGroups?: Array<{ slug: string; urls: Record<string, string> }>,
  selectedEnvGroupSlug?: string | null
): RemoteLoadTestController {
  const abortController = new AbortController();
  const transactionId = generateUUID();
  let executionId: string | null = null;

  // Client-side node state accumulator
  const lastKnownNodeMetrics = new Map<string, RemoteMetricsEvent>();
  const rpsHistory: RpsPoint[] = [];
  const runnerResourceHistory: RunnerResourcePoint[] = [];
  let lastRpsPointTime = 0;

  function toFullMetrics(event: RemoteMetricsEvent, consolidated?: ConsolidatedLoadMetrics | null): LoadTestMetrics {
    const now = Date.now();
    if (now - lastRpsPointTime >= 500) {
      rpsHistory.push(buildRpsHistoryPoint(now, event, consolidated, lastKnownNodeMetrics));
      lastRpsPointTime = now;
    }
    return {
      snapshotMode: event.snapshotMode,
      totalSent: consolidated?.totalSent ?? event.totalSent,
      totalStarted: consolidated?.totalStarted ?? event.totalStarted,
      httpStarted: consolidated?.httpStarted ?? event.httpStarted,
      httpCompleted: consolidated?.httpCompleted ?? event.httpCompleted,
      dispatchSubmitted: consolidated?.dispatchSubmitted ?? event.dispatchSubmitted,
      dispatchStarted: consolidated?.dispatchStarted ?? event.dispatchStarted,
      httpSendReturned: consolidated?.httpSendReturned ?? event.httpSendReturned,
      responseBodyCompleted: consolidated?.responseBodyCompleted ?? event.responseBodyCompleted,
      dependencyLimitedStarts: consolidated?.dependencyLimitedStarts ?? event.dependencyLimitedStarts,
      dispatcherLaggedStarts: consolidated?.dispatcherLaggedStarts ?? event.dispatcherLaggedStarts,
      runtimeLaggedStarts: consolidated?.runtimeLaggedStarts ?? event.runtimeLaggedStarts,
      senderLaggedStarts: consolidated?.senderLaggedStarts ?? event.senderLaggedStarts,
      senderQueueDepth: consolidated?.senderQueueDepth ?? event.senderQueueDepth,
      senderStartLagAvgMs: consolidated?.senderStartLagAvgMs ?? event.senderStartLagAvgMs,
      senderStartLagP95Ms: consolidated?.senderStartLagP95Ms ?? event.senderStartLagP95Ms,
      senderStartLagP99Ms: consolidated?.senderStartLagP99Ms ?? event.senderStartLagP99Ms,
      senderStartLagMaxMs: consolidated?.senderStartLagMaxMs ?? event.senderStartLagMaxMs,
      httpSendDurationAvgMs: consolidated?.httpSendDurationAvgMs ?? event.httpSendDurationAvgMs,
      httpSendDurationP95Ms: consolidated?.httpSendDurationP95Ms ?? event.httpSendDurationP95Ms,
      httpSendDurationP99Ms: consolidated?.httpSendDurationP99Ms ?? event.httpSendDurationP99Ms,
      responseObservationDurationAvgMs: consolidated?.responseObservationDurationAvgMs ?? event.responseObservationDurationAvgMs,
      responseObservationDurationP95Ms: consolidated?.responseObservationDurationP95Ms ?? event.responseObservationDurationP95Ms,
      responseObservationDurationP99Ms: consolidated?.responseObservationDurationP99Ms ?? event.responseObservationDurationP99Ms,
      schedulerLagMs: consolidated?.schedulerLagMs ?? event.schedulerLagMs,
      schedulerLaggedStarts: consolidated?.schedulerLaggedStarts ?? event.schedulerLaggedStarts,
      totalSuccess: consolidated?.totalSuccess ?? event.totalSuccess,
      totalError: consolidated?.totalError ?? event.totalError,
      avgLatency: consolidated?.avgLatency ?? 0,
      p95: consolidated?.p95 ?? 0,
      p99: consolidated?.p99 ?? 0,
      rps: consolidated?.rps ?? event.rps,
      latencyHistory: [],
      rpsHistory: [...rpsHistory],
      runnerResourceHistory: [...runnerResourceHistory],
      lifecycleBuckets: consolidated?.lifecycleBuckets ?? event.lifecycleBuckets ?? [],
      startTime: consolidated?.startTime ?? event.startTime,
      elapsedMs: consolidated?.elapsedMs ?? event.elapsedMs,
      targetIntensity: consolidated?.targetIntensity ?? event.targetIntensity,
      targetRpsLimit: consolidated?.targetRpsLimit ?? event.targetRpsLimit,
      inFlight: consolidated?.inFlight ?? event.inFlight,
      runnerMaxRps: consolidated?.runnerMaxRps ?? event.runnerMaxRps,
      tickMs: consolidated?.tickMs ?? event.tickMs,
      scheduledStarts: consolidated?.scheduledStarts ?? event.scheduledStarts,
      missedStarts: consolidated?.missedStarts ?? event.missedStarts,
      readyRequests: consolidated?.readyRequests ?? event.readyRequests,
      activePipelines: consolidated?.activePipelines ?? event.activePipelines,
      outstandingRequests: consolidated?.outstandingRequests ?? event.outstandingRequests,
      curveAdherence: consolidated?.curveAdherence ?? event.curveAdherence,
    };
  }

  const run = async () => {
    try {
      const base = ensureApiPrefix(backendUrl);
      const basePath = `${base}/projects/${projectId}/tests/load`;
      const body = {
        pipelineId: pipeline.id,
        ...(isWaveLoadConfig(config) ? { load: config } : { config }),
        selectedBaseUrlKey,
        selectedEnvGroupSlug,
        pipelineIndex,
        specs,
        envGroups,
      };
      const response = await fetch(basePath, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Accept": "text/event-stream",
          "x-transaction-id": transactionId,
        },
        body: JSON.stringify(body),
        signal: abortController.signal,
      });

      if (!response.ok) {
        const err = await response.text();
        callbacks.onError(`HTTP ${response.status}: ${err}`);
        return;
      }

      const reader = response.body?.getReader();
      if (!reader) {
        callbacks.onError("Stream não suportado pelo servidor");
        return;
      }

      const decoder = new TextDecoder();
      let buffer = "";

      let streamCompleted = false;
      let pendingCompleteMetrics: LoadTestMetrics | null = null;

      const processBlock = (part: string) => {
        if (!part.trim()) return;
        const events = parseSSE(part + "\n\n");

        for (const { event, data } of events) {
          try {
            const envelope = JSON.parse(data);

            // Extract node info from orchestrator envelope
            if (callbacks.onNodesInfo && (envelope.nodesUsed !== undefined || envelope.usedNodes)) {
              callbacks.onNodesInfo({
                nodesUsed: envelope.usedNodesTotal ?? envelope.nodesUsed ?? 0,
                nodesFound: envelope.registeredNodesTotal ?? envelope.nodesFound ?? 0,
                nodeNames: envelope.usedNodes ?? envelope.activeNodes ?? [],
              });
            }

            // Aggregate metrics from lines[] array (orchestrator envelope)
            const lines = envelope.lines as Array<{ node: string; payload: RemoteMetricsEvent; runnerEvent: string }> | undefined;

            const zeroMetrics: RemoteMetricsEvent = { totalSent: 0, totalSuccess: 0, totalError: 0, rps: 0, startTime: Date.now(), elapsedMs: 0 };

            switch (event) {
              case "execution:init":
                executionId = envelope.executionId ?? envelope.payload?.executionId ?? null;
                break;
              case "execution:snapshot": {
                const snapshot = parseLoadExecutionSnapshot(envelope);
                if (snapshot) {
                  callbacks.onSnapshot?.(snapshot);
                  if (snapshot.nodesInfo && callbacks.onNodesInfo) {
                    callbacks.onNodesInfo(snapshot.nodesInfo);
                  }
                }
                break;
              }
              case "metrics": {
                const envelopeConsolidated = envelope.consolidated as ConsolidatedLoadMetrics | undefined;
                let snapshot: RemoteMetricsEvent;
                if (lines !== undefined) {
                  appendRunnerResourceHistory(runnerResourceHistory, lines);
                  for (const line of lines) {
                    if (line.runnerEvent === "metrics") {
                      lastKnownNodeMetrics.set(line.node, line.payload);
                    }
                  }
                  snapshot = consolidateNodeMetrics(lastKnownNodeMetrics) ?? zeroMetrics;
                } else {
                  snapshot = (envelope.payload ?? envelope) as RemoteMetricsEvent;
                }
                callbacks.onMetricsUpdate(toFullMetrics(snapshot, envelopeConsolidated));
                break;
              }
              case "complete": {
                streamCompleted = true;
                const envelopeConsolidated = envelope.consolidated as ConsolidatedLoadMetrics | undefined;
                let snapshot: RemoteMetricsEvent;
                if (lines !== undefined) {
                  appendRunnerResourceHistory(runnerResourceHistory, lines);
                  for (const line of lines) {
                    if (line.runnerEvent === "metrics" || line.runnerEvent === "complete") {
                      lastKnownNodeMetrics.set(line.node, line.payload);
                    }
                  }
                  snapshot = consolidateNodeMetrics(lastKnownNodeMetrics) ?? zeroMetrics;
                } else {
                  snapshot = (envelope.payload ?? envelope) as RemoteMetricsEvent;
                }
                // Store metrics to call onComplete outside try/catch
                pendingCompleteMetrics = toFullMetrics(snapshot, envelopeConsolidated);
                break;
              }
              case "error":
                callbacks.onError(envelope.message || envelope.payload?.message || envelope.payload?.error || "Erro desconhecido");
                break;
            }
          } catch {
            // Skip malformed JSON
          }
        }

        // Call onComplete outside try/catch so errors are not swallowed
        if (pendingCompleteMetrics) {
          const m = pendingCompleteMetrics;
          pendingCompleteMetrics = null;
          callbacks.onComplete(m);
        }
      };

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const parts = buffer.split("\n\n");
        buffer = parts.pop() || "";

        for (const part of parts) {
          processBlock(part);
        }

        if (streamCompleted) {
          await reader.cancel();
          break;
        }
      }

      // Process any remaining data in the buffer after stream ends
      if (buffer.trim()) {
        processBlock(buffer);
      }

      // If stream ended without a "complete" event, finalize based on last known metrics
      if (!streamCompleted && !abortController.signal.aborted) {
        const lastSnapshot = consolidateNodeMetrics(lastKnownNodeMetrics);
        if (lastSnapshot) {
          callbacks.onComplete(toFullMetrics(lastSnapshot));
        } else {
          callbacks.onComplete(toFullMetrics({ totalSent: 0, totalSuccess: 0, totalError: 0, rps: 0, startTime: Date.now(), elapsedMs: 0 }));
        }
      }
    } catch (err) {
      if ((err as Error).name !== "AbortError") {
        callbacks.onError(err instanceof Error ? err.message : String(err));
      }
    }
  };

  run();

  return {
    cancel: () => {
      if (executionId) {
        cancelExecution(backendUrl, executionId);
      }
      abortController.abort();
    },
    disconnect: () => {
      abortController.abort();
    },
  };
}

// ============ SSE Reconnection ============

export function reconnectToE2eExecution(
  backendUrl: string,
  projectId: string,
  executionId: string,
  callbacks: RemoteIntegrationCallbacks,
): RemoteExecutionController {
  const abortController = new AbortController();

  const run = async () => {
    try {
      const base = ensureApiPrefix(backendUrl);
      const url = `${base}/projects/${projectId}/executions/${executionId}`;
      console.log("[DEBUG][reconnectToE2eExecution] opening SSE", {
        projectId,
        executionId,
        url,
      });
      const response = await fetch(url, {
        method: "GET",
        headers: { "Accept": "text/event-stream" },
        signal: abortController.signal,
      });

      if (!response.ok) {
        const err = await response.text();
        console.error("[DEBUG][reconnectToE2eExecution] non-ok response", {
          projectId,
          executionId,
          status: response.status,
          error: err,
        });
        callbacks.onError(`HTTP ${response.status}: ${err}`);
        return;
      }

      console.log("[DEBUG][reconnectToE2eExecution] SSE connected", {
        projectId,
        executionId,
        status: response.status,
        contentType: response.headers.get("content-type"),
      });

      const reader = response.body?.getReader();
      if (!reader) {
        console.error("[DEBUG][reconnectToE2eExecution] missing response body reader", {
          projectId,
          executionId,
        });
        callbacks.onError("Stream não suportado pelo servidor");
        return;
      }

      const decoder = new TextDecoder();
      let buffer = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const parts = buffer.split("\n\n");
        buffer = parts.pop() || "";

        for (const part of parts) {
          if (!part.trim()) continue;
          const events = parseSSE(part + "\n\n");

          for (const { event, data } of events) {
            try {
              const envelope = JSON.parse(data) as SseObject;
              console.log("[DEBUG][reconnectToE2eExecution] event received", {
                projectId,
                executionId,
                event,
                envelopeEvent: pickString(envelope.event, envelope.runnerEvent),
              });
              handleIntegrationEnvelope(event, envelope, callbacks);
            } catch { /* skip malformed JSON */ }
          }
        }
      }

      // Process remaining buffer
      if (buffer.trim()) {
        const events = parseSSE(buffer + "\n\n");
        for (const { event, data } of events) {
          try {
            const envelope = JSON.parse(data) as SseObject;
            console.log("[DEBUG][reconnectToE2eExecution] trailing event received", {
              projectId,
              executionId,
              event,
              envelopeEvent: pickString(envelope.event, envelope.runnerEvent),
            });
            handleIntegrationEnvelope(event, envelope, callbacks);
          } catch { /* skip */ }
        }
      }
    } catch (err) {
      if ((err as Error).name !== "AbortError") {
        console.error("[DEBUG][reconnectToE2eExecution] stream error", {
          projectId,
          executionId,
          error: err instanceof Error ? err.message : String(err),
        });
        callbacks.onError(err instanceof Error ? err.message : String(err));
      } else {
        console.log("[DEBUG][reconnectToE2eExecution] stream aborted", {
          projectId,
          executionId,
        });
      }
    }
  };

  run();

  return {
    cancel: () => {
      cancelExecution(backendUrl, executionId);
      abortController.abort();
    },
    disconnect: () => {
      abortController.abort();
    },
  };
}

export function reconnectToLoadExecution(
  backendUrl: string,
  projectId: string,
  executionId: string,
  callbacks: RemoteLoadTestCallbacks,
): RemoteLoadTestController {
  const abortController = new AbortController();

  const lastKnownNodeMetrics = new Map<string, RemoteMetricsEvent>();
  const rpsHistory: RpsPoint[] = [];
  const runnerResourceHistory: RunnerResourcePoint[] = [];
  let lastRpsPointTime = 0;

  function toFullMetrics(event: RemoteMetricsEvent, consolidated?: ConsolidatedLoadMetrics | null): LoadTestMetrics {
    const now = Date.now();
    if (now - lastRpsPointTime >= 500) {
      rpsHistory.push(buildRpsHistoryPoint(now, event, consolidated, lastKnownNodeMetrics));
      lastRpsPointTime = now;
    }
    return {
      snapshotMode: event.snapshotMode,
      totalSent: consolidated?.totalSent ?? event.totalSent,
      totalStarted: consolidated?.totalStarted ?? event.totalStarted,
      httpStarted: consolidated?.httpStarted ?? event.httpStarted,
      httpCompleted: consolidated?.httpCompleted ?? event.httpCompleted,
      dispatchSubmitted: consolidated?.dispatchSubmitted ?? event.dispatchSubmitted,
      dispatchStarted: consolidated?.dispatchStarted ?? event.dispatchStarted,
      httpSendReturned: consolidated?.httpSendReturned ?? event.httpSendReturned,
      responseBodyCompleted: consolidated?.responseBodyCompleted ?? event.responseBodyCompleted,
      dependencyLimitedStarts: consolidated?.dependencyLimitedStarts ?? event.dependencyLimitedStarts,
      dispatcherLaggedStarts: consolidated?.dispatcherLaggedStarts ?? event.dispatcherLaggedStarts,
      runtimeLaggedStarts: consolidated?.runtimeLaggedStarts ?? event.runtimeLaggedStarts,
      senderLaggedStarts: consolidated?.senderLaggedStarts ?? event.senderLaggedStarts,
      senderQueueDepth: consolidated?.senderQueueDepth ?? event.senderQueueDepth,
      senderStartLagAvgMs: consolidated?.senderStartLagAvgMs ?? event.senderStartLagAvgMs,
      senderStartLagP95Ms: consolidated?.senderStartLagP95Ms ?? event.senderStartLagP95Ms,
      senderStartLagP99Ms: consolidated?.senderStartLagP99Ms ?? event.senderStartLagP99Ms,
      senderStartLagMaxMs: consolidated?.senderStartLagMaxMs ?? event.senderStartLagMaxMs,
      httpSendDurationAvgMs: consolidated?.httpSendDurationAvgMs ?? event.httpSendDurationAvgMs,
      httpSendDurationP95Ms: consolidated?.httpSendDurationP95Ms ?? event.httpSendDurationP95Ms,
      httpSendDurationP99Ms: consolidated?.httpSendDurationP99Ms ?? event.httpSendDurationP99Ms,
      responseObservationDurationAvgMs: consolidated?.responseObservationDurationAvgMs ?? event.responseObservationDurationAvgMs,
      responseObservationDurationP95Ms: consolidated?.responseObservationDurationP95Ms ?? event.responseObservationDurationP95Ms,
      responseObservationDurationP99Ms: consolidated?.responseObservationDurationP99Ms ?? event.responseObservationDurationP99Ms,
      schedulerLagMs: consolidated?.schedulerLagMs ?? event.schedulerLagMs,
      schedulerLaggedStarts: consolidated?.schedulerLaggedStarts ?? event.schedulerLaggedStarts,
      totalSuccess: consolidated?.totalSuccess ?? event.totalSuccess,
      totalError: consolidated?.totalError ?? event.totalError,
      avgLatency: consolidated?.avgLatency ?? 0,
      p95: consolidated?.p95 ?? 0,
      p99: consolidated?.p99 ?? 0,
      rps: consolidated?.rps ?? event.rps,
      latencyHistory: [],
      rpsHistory: [...rpsHistory],
      runnerResourceHistory: [...runnerResourceHistory],
      lifecycleBuckets: consolidated?.lifecycleBuckets ?? event.lifecycleBuckets ?? [],
      startTime: consolidated?.startTime ?? event.startTime,
      elapsedMs: consolidated?.elapsedMs ?? event.elapsedMs,
      targetIntensity: consolidated?.targetIntensity ?? event.targetIntensity,
      targetRpsLimit: consolidated?.targetRpsLimit ?? event.targetRpsLimit,
      inFlight: consolidated?.inFlight ?? event.inFlight,
      runnerMaxRps: consolidated?.runnerMaxRps ?? event.runnerMaxRps,
      tickMs: consolidated?.tickMs ?? event.tickMs,
      scheduledStarts: consolidated?.scheduledStarts ?? event.scheduledStarts,
      missedStarts: consolidated?.missedStarts ?? event.missedStarts,
      readyRequests: consolidated?.readyRequests ?? event.readyRequests,
      activePipelines: consolidated?.activePipelines ?? event.activePipelines,
      outstandingRequests: consolidated?.outstandingRequests ?? event.outstandingRequests,
      curveAdherence: consolidated?.curveAdherence ?? event.curveAdherence,
    };
  }

  const zeroMetrics: RemoteMetricsEvent = { totalSent: 0, totalSuccess: 0, totalError: 0, rps: 0, startTime: Date.now(), elapsedMs: 0 };

  const run = async () => {
    try {
      const base = ensureApiPrefix(backendUrl);
      const url = `${base}/projects/${projectId}/executions/${executionId}`;
      const response = await fetch(url, {
        method: "GET",
        headers: { "Accept": "text/event-stream" },
        signal: abortController.signal,
      });

      if (!response.ok) {
        const err = await response.text();
        callbacks.onError(`HTTP ${response.status}: ${err}`);
        return;
      }

      const reader = response.body?.getReader();
      if (!reader) {
        callbacks.onError("Stream não suportado pelo servidor");
        return;
      }

      const decoder = new TextDecoder();
      let buffer = "";
      let streamCompleted = false;
      let pendingCompleteMetrics: LoadTestMetrics | null = null;

      const processBlock = (part: string) => {
        if (!part.trim()) return;
        const events = parseSSE(part + "\n\n");

        for (const { event, data } of events) {
          try {
            const envelope = JSON.parse(data);

            if (callbacks.onNodesInfo && (envelope.nodesUsed !== undefined || envelope.usedNodes)) {
              callbacks.onNodesInfo({
                nodesUsed: envelope.usedNodesTotal ?? envelope.nodesUsed ?? 0,
                nodesFound: envelope.registeredNodesTotal ?? envelope.nodesFound ?? 0,
                nodeNames: envelope.usedNodes ?? envelope.activeNodes ?? [],
              });
            }

            const lines = envelope.lines as Array<{ node: string; payload: RemoteMetricsEvent; runnerEvent: string }> | undefined;

            switch (event) {
              case "execution:init":
                break;
              case "execution:snapshot": {
                const snapshot = parseLoadExecutionSnapshot(envelope);
                if (snapshot) {
                  callbacks.onSnapshot?.(snapshot);
                  if (snapshot.nodesInfo && callbacks.onNodesInfo) {
                    callbacks.onNodesInfo(snapshot.nodesInfo);
                  }
                }
                break;
              }
              case "metrics": {
                const envelopeConsolidated = envelope.consolidated as ConsolidatedLoadMetrics | undefined;
                let snapshot: RemoteMetricsEvent;
                if (lines !== undefined) {
                  appendRunnerResourceHistory(runnerResourceHistory, lines);
                  for (const line of lines) {
                    if (line.runnerEvent === "metrics") {
                      lastKnownNodeMetrics.set(line.node, line.payload);
                    }
                  }
                  snapshot = consolidateNodeMetrics(lastKnownNodeMetrics) ?? zeroMetrics;
                } else {
                  snapshot = (envelope.payload ?? envelope) as RemoteMetricsEvent;
                }
                callbacks.onMetricsUpdate(toFullMetrics(snapshot, envelopeConsolidated));
                break;
              }
              case "complete": {
                streamCompleted = true;
                const envelopeConsolidated = envelope.consolidated as ConsolidatedLoadMetrics | undefined;
                let snapshot: RemoteMetricsEvent;
                if (lines !== undefined) {
                  appendRunnerResourceHistory(runnerResourceHistory, lines);
                  for (const line of lines) {
                    if (line.runnerEvent === "metrics" || line.runnerEvent === "complete") {
                      lastKnownNodeMetrics.set(line.node, line.payload);
                    }
                  }
                  snapshot = consolidateNodeMetrics(lastKnownNodeMetrics) ?? zeroMetrics;
                } else {
                  snapshot = (envelope.payload ?? envelope) as RemoteMetricsEvent;
                }
                pendingCompleteMetrics = toFullMetrics(snapshot, envelopeConsolidated);
                break;
              }
              case "error":
                callbacks.onError(envelope.message || envelope.payload?.message || envelope.payload?.error || "Erro desconhecido");
                break;
            }
          } catch { /* skip */ }
        }

        if (pendingCompleteMetrics) {
          const m = pendingCompleteMetrics;
          pendingCompleteMetrics = null;
          callbacks.onComplete(m);
        }
      };

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const parts = buffer.split("\n\n");
        buffer = parts.pop() || "";

        for (const part of parts) {
          processBlock(part);
        }

        if (streamCompleted) {
          await reader.cancel();
          break;
        }
      }

      if (buffer.trim()) {
        processBlock(buffer);
      }

      if (!streamCompleted && !abortController.signal.aborted) {
        const lastSnapshot = consolidateNodeMetrics(lastKnownNodeMetrics);
        if (lastSnapshot) {
          callbacks.onComplete(toFullMetrics(lastSnapshot));
        } else {
          callbacks.onComplete(toFullMetrics(zeroMetrics));
        }
      }
    } catch (err) {
      if ((err as Error).name !== "AbortError") {
        callbacks.onError(err instanceof Error ? err.message : String(err));
      }
    }
  };

  run();

  return {
    cancel: () => {
      cancelExecution(backendUrl, executionId);
      abortController.abort();
    },
    disconnect: () => {
      abortController.abort();
    },
  };
}
