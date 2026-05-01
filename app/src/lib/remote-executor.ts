import type { Pipeline, StepExecutionResult } from "@/types/pipeline";
import type {
  LoadTestConfig,
  LoadTestMetrics,
  LoadTestState,
  RemoteMetricsEvent,
  ConsolidatedLoadMetrics,
  RpsPoint,
  RunnerResourcePoint,
  RunnerRuntimeInfo,
} from "@/types/load-test";
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
  const totalSuccess = toNumber(value.totalSuccess);
  const totalError = toNumber(value.totalError);
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
    totalSent: totalSent ?? 0,
    totalSuccess: totalSuccess ?? 0,
    totalError: totalError ?? 0,
    rps: rps ?? 0,
    startTime: toNumber(value.startTime) ?? Date.now(),
    elapsedMs: toNumber(value.elapsedMs) ?? 0,
    runtime: extractRunnerRuntime(value.runtime),
  };
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

  return metrics.reduce<RemoteMetricsEvent>(
    (acc, item) => ({
      totalSent: acc.totalSent + item.totalSent,
      totalSuccess: acc.totalSuccess + item.totalSuccess,
      totalError: acc.totalError + item.totalError,
      rps: acc.rps + item.rps,
      startTime: Math.min(acc.startTime, item.startTime),
      elapsedMs: Math.max(acc.elapsedMs, item.elapsedMs),
    }),
    {
      totalSent: 0,
      totalSuccess: 0,
      totalError: 0,
      rps: 0,
      startTime: Number.MAX_SAFE_INTEGER,
      elapsedMs: 0,
    },
  );
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

  return {
    totalSent: toNumber(consolidated?.totalSent) ?? aggregated?.totalSent ?? 0,
    totalSuccess: toNumber(consolidated?.totalSuccess) ?? aggregated?.totalSuccess ?? 0,
    totalError: toNumber(consolidated?.totalError) ?? aggregated?.totalError ?? 0,
    avgLatency: toNumber(consolidated?.avgLatency) ?? 0,
    p95: toNumber(consolidated?.p95) ?? 0,
    p99: toNumber(consolidated?.p99) ?? 0,
    rps,
    latencyHistory: [],
    rpsHistory: rps > 0 ? [{ timestamp: Date.now(), rps }] : [],
    runnerResourceHistory: Array.isArray(snapshot.lines)
      ? extractRunnerResourcePoints(snapshot.lines)
      : [],
    startTime,
    elapsedMs: toNumber(consolidated?.elapsedMs) ?? aggregated?.elapsedMs ?? 0,
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
  specs?: Array<{ slug: string; servers: Record<string, string> }>
): RemoteExecutionController {
  const abortController = new AbortController();
  const transactionId = generateUUID();
  let executionId: string | null = null;

  const run = async () => {
    try {
      const base = ensureApiPrefix(backendUrl);
      const basePath = `${base}/projects/${projectId}/tests/e2e`;
      const body = { pipelineId: pipeline.id, selectedBaseUrlKey, pipelineIndex, specs };
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
  let startTime = Infinity, maxElapsed = 0;

  for (const p of nodeMap.values()) {
    totalSent += p.totalSent;
    totalSuccess += p.totalSuccess;
    totalError += p.totalError;
    rpsSum += p.rps;
    if (p.startTime < startTime) startTime = p.startTime;
    if (p.elapsedMs > maxElapsed) maxElapsed = p.elapsedMs;
  }

  return { totalSent, totalSuccess, totalError, rps: rpsSum, startTime, elapsedMs: maxElapsed };
}

export function runRemoteLoadTest(
  backendUrl: string,
  pipeline: Pipeline,
  config: LoadTestConfig,
  callbacks: RemoteLoadTestCallbacks,
  projectId: string,
  selectedBaseUrlKey?: string,
  pipelineIndex?: number,
  specs?: Array<{ slug: string; servers: Record<string, string> }>
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
      rpsHistory.push({ timestamp: now, rps: consolidated?.rps ?? event.rps });
      lastRpsPointTime = now;
    }
    return {
      totalSent: consolidated?.totalSent ?? event.totalSent,
      totalSuccess: consolidated?.totalSuccess ?? event.totalSuccess,
      totalError: consolidated?.totalError ?? event.totalError,
      avgLatency: consolidated?.avgLatency ?? 0,
      p95: consolidated?.p95 ?? 0,
      p99: consolidated?.p99 ?? 0,
      rps: consolidated?.rps ?? event.rps,
      latencyHistory: [],
      rpsHistory: [...rpsHistory],
      runnerResourceHistory: [...runnerResourceHistory],
      startTime: consolidated?.startTime ?? event.startTime,
      elapsedMs: consolidated?.elapsedMs ?? event.elapsedMs,
    };
  }

  const run = async () => {
    try {
      const base = ensureApiPrefix(backendUrl);
      const basePath = `${base}/projects/${projectId}/tests/load`;
      const body = { pipelineId: pipeline.id, config, selectedBaseUrlKey, pipelineIndex, specs };
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
      rpsHistory.push({ timestamp: now, rps: consolidated?.rps ?? event.rps });
      lastRpsPointTime = now;
    }
    return {
      totalSent: consolidated?.totalSent ?? event.totalSent,
      totalSuccess: consolidated?.totalSuccess ?? event.totalSuccess,
      totalError: consolidated?.totalError ?? event.totalError,
      avgLatency: consolidated?.avgLatency ?? 0,
      p95: consolidated?.p95 ?? 0,
      p99: consolidated?.p99 ?? 0,
      rps: consolidated?.rps ?? event.rps,
      latencyHistory: [],
      rpsHistory: [...rpsHistory],
      runnerResourceHistory: [...runnerResourceHistory],
      startTime: consolidated?.startTime ?? event.startTime,
      elapsedMs: consolidated?.elapsedMs ?? event.elapsedMs,
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
