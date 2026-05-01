/**
 * Centralized HTTP client for the Previa Orchestrator API.
 * Every function receives `baseUrl` (e.g. "http://localhost:5588/api/v1") as its first argument.
 */

import { useEventStore } from "@/stores/useEventStore";
import type { Pipeline, OpenAPISpec } from "@/types/pipeline";
import type { Project, ProjectEnvEntry, ProjectEnvGroup, ProjectSpec } from "@/types/project";
import { getMergedSpec } from "@/types/project";
import type { ExecutionRun } from "@/lib/execution-store";
import type { LoadTestRunRecord } from "@/lib/load-test-store";
import type { LoadTestMetrics, LoadTestState, RunnerResourcePoint } from "@/types/load-test";

// ============ API Types (from OpenAPI spec) ============

export interface ProjectRecord {
  id: string;
  name: string;
  description?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ApiPipeline {
  id?: string | null;
  name: string;
  description?: string | null;
  steps: ApiPipelineStep[];
}

export interface ApiPipelineStep {
  id: string;
  name: string;
  method: string;
  url: string;
  description?: string | null;
  headers?: Record<string, string>;
  body?: Record<string, unknown> | null;
  operationId?: string | null;
  asserts?: Array<{ field: string; operator: string; expected?: string | null }>;
  delay?: number | null;
  retry?: number | null;
}

export interface PipelineInput {
  name: string;
  description?: string | null;
  steps: ApiPipelineStep[];
}

export interface ProjectUpsertRequest {
  name: string;
  description?: string | null;
  spec?: Record<string, unknown> | null;
  executionBackendUrl?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
}

export interface ProjectUpdateRequest {
  name: string;
  description?: string | null;
  executionBackendUrl?: string | null;
}

export interface ProjectSpecRecord {
  id: string;
  projectId: string;
  spec: Record<string, unknown>;
  sync: boolean;
  url?: string | null;
  slug?: string | null;
  servers?: Record<string, string> | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectSpecUpsertRequest {
  spec: Record<string, unknown>;
  sync?: boolean;
  url?: string | null;
  slug?: string | null;
  servers?: Record<string, string> | null;
}

export interface ProjectEnvGroupRecord {
  id: string;
  projectId: string;
  slug: string;
  name: string;
  entries: ProjectEnvEntry[];
  createdAt: string;
  updatedAt: string;
}

export interface ProjectEnvGroupUpsertRequest {
  slug: string;
  name: string;
  entries: ProjectEnvEntry[];
}

export interface RuntimeEnvGroup {
  slug: string;
  urls: Record<string, string>;
}

export interface IntegrationHistoryRecord {
  id: string;
  executionId: string;
  pipelineName: string;
  status: string;
  startedAtMs: number;
  finishedAtMs: number;
  durationMs: number;
  summary: Record<string, unknown> | null;
  steps: Array<Record<string, unknown>>;
  errors: string[];
  request: Record<string, unknown>;
  projectId?: string | null;
  pipelineIndex?: number | null;
  pipelineId?: string | null;
  transactionId?: string | null;
  selectedBaseUrlKey?: string | null;
}

export interface LoadHistoryRecord {
  id: string;
  executionId: string;
  pipelineName: string;
  status: string;
  startedAtMs: number;
  finishedAtMs: number;
  durationMs: number;
  requestedConfig: Record<string, unknown>;
  finalConsolidated: Record<string, unknown> | null;
  finalLines: Array<Record<string, unknown>>;
  errors: string[];
  request: Record<string, unknown>;
  context: Record<string, unknown>;
  projectId?: string | null;
  pipelineIndex?: number | null;
  pipelineId?: string | null;
  transactionId?: string | null;
  selectedBaseUrlKey?: string | null;
}

export interface HistoryQuery {
  pipelineIndex?: number;
  limit?: number;
  offset?: number;
  order?: "asc" | "desc";
}

export interface RunnerRuntimeInfo {
  pid: number;
  memoryBytes: number;
  virtualMemoryBytes: number;
  cpuUsagePercent: number;
  networkTxBytes?: number;
  networkRxBytes?: number;
  networkTotalBytes?: number;
}

export interface RunnerRecord {
  id: string;
  endpoint: string;
  name?: string | null;
  source: string;
  enabled: boolean;
  healthStatus: string;
  lastSeenAt?: string | null;
  lastError?: string | null;
  runtime?: RunnerRuntimeInfo | null;
  createdAt: string;
  updatedAt: string;
}

export interface RunnerUpsertRequest {
  endpoint: string;
  name?: string | null;
  enabled?: boolean;
}

export interface RunnerUpdateRequest {
  name?: string | null;
  enabled?: boolean;
}

// ============ Helper ============

function simpleHash(input: string): string {
  let hash = 5381;
  for (let i = 0; i < input.length; i++) {
    hash = ((hash << 5) + hash + input.charCodeAt(i)) >>> 0;
  }
  return hash.toString(36);
}

function detectOperation(url: string, method: string): string {
  if (/\/specs/.test(url)) return "Specs";
  if (/\/pipelines/.test(url)) return "Pipelines";
  if (/\/tests\/load/.test(url)) return "Load Tests";
  if (/\/tests\/e2e/.test(url)) return "End-to-End Tests";
  if (/\/projects/.test(url)) return "Projetos";
  return method === "GET" ? "Consulta" : "Operação";
}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const method = init?.method ?? "GET";
  try {
    const res = await fetch(url, init);
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      const statusCode = res.status;
      useEventStore.getState().addEvent({
        uid: simpleHash(`${method}:${url}:${text || `HTTP ${statusCode}`}`),
        type: "error",
        title: detectOperation(url, method),
        message: text || `HTTP ${statusCode}`,
        details: { method, url, statusCode },
      });
      throw new Error(`HTTP ${statusCode}: ${text}`);
    }
    if (res.status === 204) return undefined as unknown as T;
    return res.json();
  } catch (err) {
    // Network errors (no response at all)
    if (err instanceof TypeError) {
      useEventStore.getState().addEvent({
        uid: simpleHash(`${method}:${url}:${err.message || "Erro de rede"}`),
        type: "error",
        title: detectOperation(url, method),
        message: err.message || "Erro de rede",
        details: { method, url },
      });
    }
    throw err;
  }
}

export function ensureApiPrefix(url: string): string {
  const clean = url.replace(/\/+$/, "");
  return clean.endsWith("/api/v1") ? clean : `${clean}/api/v1`;
}

function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const entries = Object.entries(params).filter(([, v]) => v !== undefined && v !== null);
  if (entries.length === 0) return "";
  return "?" + entries.map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`).join("&");
}

// ============ Pipeline Mapping ============

function pipelineToInput(p: Pipeline): PipelineInput {
  return {
    name: p.name,
    description: p.description || null,
    steps: p.steps.map((s) => ({
      id: s.id,
      name: s.name,
      method: s.method,
      url: s.url,
      description: s.description || null,
      headers: s.headers,
      body: s.body ?? null,
      operationId: s.operationId ?? null,
      asserts: s.asserts?.map((a) => ({ field: a.field, operator: a.operator, expected: a.expected ?? null })),
      delay: s.delay ?? null,
      retry: s.retry ?? null,
    })),
  };
}

function apiPipelineToLocal(p: ApiPipeline): Pipeline {
  return {
    id: p.id ?? undefined,
    name: p.name,
    description: p.description ?? "",
    steps: p.steps.map((s) => ({
      id: s.id,
      name: s.name,
      method: s.method as Pipeline["steps"][0]["method"],
      url: s.url,
      description: s.description ?? "",
      headers: s.headers ?? {},
      body: s.body ?? undefined,
      operationId: s.operationId ?? undefined,
      asserts: s.asserts?.map((a) => ({
        field: a.field,
        operator: a.operator as any,
        expected: a.expected ?? undefined,
      })),
      delay: s.delay ?? undefined,
      retry: s.retry ?? undefined,
    })),
  };
}

// ============ Project Mapping ============

function specRecordToProjectSpec(record: ProjectSpecRecord): import("@/types/project").ProjectSpec {
  const { slugify } = await_slugify();
  const name = (record.spec as any)?.info?.title || record.url || `Spec ${record.id.slice(0, 6)}`;
  return {
    id: record.id,
    slug: record.slug || slugify(name),
    name,
    spec: record.spec as unknown as OpenAPISpec,
    url: record.url ?? undefined,
    sync: record.sync,
    servers: record.servers || {},
    specMd5: (record as any).specMd5 ?? undefined,
  };
}

// Lazy import cache for slugify to avoid sync require()
let _slugifyFn: ((name: string) => string) | null = null;
function await_slugify() {
  if (!_slugifyFn) {
    // Dynamic import is async, but we need sync access here.
    // Import at module level instead.
    _slugifyFn = (name: string) =>
      name.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^[-_]+|[-_]+$/g, "") || "spec";
  }
  return { slugify: _slugifyFn };
}

function projectRecordToLocal(
  r: ProjectRecord,
  pipelines: Pipeline[] = [],
  specRecords: ProjectSpecRecord[] = [],
  envGroups: ProjectEnvGroup[] = [],
): Project {
  const specs = specRecords.map(specRecordToProjectSpec);
  // Backward compat: set legacy spec to merged routes from all specs
  let spec: OpenAPISpec | undefined;
  if (specs.length === 1) {
    spec = specs[0].spec;
  } else if (specs.length > 1) {
    spec = getMergedSpec(specs);
  }

  return {
    id: r.id,
    name: r.name,
    description: r.description ?? undefined,
    createdAt: r.createdAt,
    updatedAt: r.updatedAt,
    spec,
    specs,
    envGroups,
    pipelines,
  };
}

// ============ Projects ============

export async function listProjects(baseUrl: string, opts?: { limit?: number; offset?: number; order?: "asc" | "desc" }): Promise<Project[]> {
  console.log("[DEBUG][api] GET /projects START", { timestamp: Date.now() });
  const records = await request<ProjectRecord[]>(
    `${baseUrl}/projects${qs({ limit: opts?.limit, offset: opts?.offset, order: opts?.order })}`
  );
  console.log("[DEBUG][api] GET /projects END", { count: records.length, timestamp: Date.now() });
  // List returns metadata only — no pipelines or specs
  return records.map((r) => projectRecordToLocal(r, [], [], []));
}

export async function getProject(baseUrl: string, id: string): Promise<Project> {
  console.log("[DEBUG][api] GET /projects/:id START (parallel: project+pipelines+specs+envGroups)", { id, timestamp: Date.now() });
  const [record, pipelines, specs, envGroups] = await Promise.all([
    request<ProjectRecord>(`${baseUrl}/projects/${id}`),
    listPipelines(baseUrl, id).catch(() => []),
    listSpecs(baseUrl, id).catch(() => []),
    listProjectEnvGroups(baseUrl, id).catch(() => []),
  ]);
  console.log("[DEBUG][api] GET /projects/:id END", { id, pipelinesCount: pipelines.length, specsCount: specs.length, envGroupsCount: envGroups.length, timestamp: Date.now() });
  return projectRecordToLocal(record, pipelines, specs, envGroups);
}

export async function createProject(baseUrl: string, data: ProjectUpsertRequest): Promise<ProjectRecord> {
  return request<ProjectRecord>(`${baseUrl}/projects`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function upsertProject(baseUrl: string, id: string, data: ProjectUpdateRequest): Promise<ProjectRecord> {
  return request<ProjectRecord>(`${baseUrl}/projects/${id}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function deleteProject(baseUrl: string, id: string): Promise<void> {
  await request<void>(`${baseUrl}/projects/${id}`, { method: "DELETE" });
}

// ============ Runners ============

export async function listRunners(baseUrl: string): Promise<RunnerRecord[]> {
  return request<RunnerRecord[]>(`${baseUrl}/runners`);
}

export async function createRunner(baseUrl: string, payload: RunnerUpsertRequest): Promise<RunnerRecord> {
  return request<RunnerRecord>(`${baseUrl}/runners`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export async function updateRunner(
  baseUrl: string,
  runnerId: string,
  payload: RunnerUpdateRequest,
): Promise<RunnerRecord> {
  return request<RunnerRecord>(`${baseUrl}/runners/${runnerId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export async function deleteRunner(baseUrl: string, runnerId: string): Promise<void> {
  await request<void>(`${baseUrl}/runners/${runnerId}`, { method: "DELETE" });
}

// ============ Pipelines ============

export async function listPipelines(baseUrl: string, projectId: string): Promise<Pipeline[]> {
  console.log("[DEBUG][api] GET /pipelines START", { projectId, timestamp: Date.now() });
  const records = await request<ApiPipeline[]>(`${baseUrl}/projects/${projectId}/pipelines`);
  console.log("[DEBUG][api] GET /pipelines END", { projectId, count: records.length, timestamp: Date.now() });
  return records.map(apiPipelineToLocal);
}

export async function getPipeline(baseUrl: string, projectId: string, pipelineId: string): Promise<Pipeline> {
  const record = await request<ApiPipeline>(`${baseUrl}/projects/${projectId}/pipelines/${pipelineId}`);
  return apiPipelineToLocal(record);
}

// ============ Pipeline Runtime ============

export interface PipelineRuntime {
  status: "idle" | "queued" | "running";
  activeExecution?: { id: string; kind?: "e2e" | "load" };
  activeQueue?: { id: string };
}

export interface PipelineWithRuntime {
  pipeline: Pipeline;
  runtime: PipelineRuntime;
}

export async function getPipelineWithRuntime(baseUrl: string, projectId: string, pipelineId: string): Promise<PipelineWithRuntime> {
  const raw = await request<ApiPipeline & { runtime?: PipelineRuntime }>(`${baseUrl}/projects/${projectId}/pipelines/${pipelineId}`);
  const pipeline = apiPipelineToLocal(raw);
  const runtime: PipelineRuntime = raw.runtime ?? { status: "idle" };
  return { pipeline, runtime };
}

export async function createPipeline(baseUrl: string, projectId: string, pipeline: Pipeline): Promise<Pipeline> {
  console.log("[DEBUG][api] POST /pipelines START", { projectId, name: pipeline.name, timestamp: Date.now() });
  const record = await request<ApiPipeline>(`${baseUrl}/projects/${projectId}/pipelines`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(pipelineToInput(pipeline)),
  });
  console.log("[DEBUG][api] POST /pipelines END", { projectId, name: pipeline.name, timestamp: Date.now() });
  return apiPipelineToLocal(record);
}

export async function upsertPipeline(baseUrl: string, projectId: string, pipelineId: string, pipeline: Pipeline): Promise<Pipeline> {
  console.log("[DEBUG][api] PUT /pipelines START", { projectId, pipelineId, name: pipeline.name, timestamp: Date.now() });
  const record = await request<ApiPipeline>(`${baseUrl}/projects/${projectId}/pipelines/${pipelineId}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(pipelineToInput(pipeline)),
  });
  console.log("[DEBUG][api] PUT /pipelines END", { projectId, pipelineId, timestamp: Date.now() });
  return apiPipelineToLocal(record);
}

export async function deletePipeline(baseUrl: string, projectId: string, pipelineId: string): Promise<void> {
  console.log("[DEBUG][api] DELETE /pipelines START", { projectId, pipelineId, timestamp: Date.now() });
  await request<void>(`${baseUrl}/projects/${projectId}/pipelines/${pipelineId}`, { method: "DELETE" });
  console.log("[DEBUG][api] DELETE /pipelines END", { projectId, pipelineId, timestamp: Date.now() });
}

// ============ Specs ============

export async function listSpecs(baseUrl: string, projectId: string): Promise<ProjectSpecRecord[]> {
  return request<ProjectSpecRecord[]>(`${baseUrl}/projects/${projectId}/specs`);
}

export async function getSpec(baseUrl: string, projectId: string, specId: string): Promise<ProjectSpecRecord> {
  return request<ProjectSpecRecord>(`${baseUrl}/projects/${projectId}/specs/${specId}`);
}

export async function createSpec(baseUrl: string, projectId: string, data: ProjectSpecUpsertRequest): Promise<ProjectSpecRecord> {
  console.log("[DEBUG][api] POST /specs START", { projectId, timestamp: Date.now() });
  const result = await request<ProjectSpecRecord>(`${baseUrl}/projects/${projectId}/specs`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
  console.log("[DEBUG][api] POST /specs END", { projectId, timestamp: Date.now() });
  return result;
}

export async function upsertSpec(
  baseUrl: string,
  projectId: string,
  specId: string,
  data: ProjectSpecUpsertRequest,
): Promise<ProjectSpecRecord> {
  return request<ProjectSpecRecord>(`${baseUrl}/projects/${projectId}/specs/${specId}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function deleteSpec(baseUrl: string, projectId: string, specId: string): Promise<void> {
  await request<void>(`${baseUrl}/projects/${projectId}/specs/${specId}`, { method: "DELETE" });
}

// ============ Env Groups ============

export async function listProjectEnvGroups(baseUrl: string, projectId: string): Promise<ProjectEnvGroup[]> {
  return request<ProjectEnvGroup[]>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups`);
}

export async function createProjectEnvGroup(baseUrl: string, projectId: string, data: ProjectEnvGroupUpsertRequest): Promise<ProjectEnvGroup> {
  return request<ProjectEnvGroup>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function updateProjectEnvGroup(baseUrl: string, projectId: string, envGroupId: string, data: ProjectEnvGroupUpsertRequest): Promise<ProjectEnvGroup> {
  return request<ProjectEnvGroup>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups/${envGroupId}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function deleteProjectEnvGroup(baseUrl: string, projectId: string, envGroupId: string): Promise<void> {
  await request<void>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups/${envGroupId}`, { method: "DELETE" });
}

export function projectEnvGroupsToRuntime(envGroups: ProjectEnvGroup[]): RuntimeEnvGroup[] {
  return envGroups.map((group) => ({
    slug: group.slug,
    urls: Object.fromEntries(group.entries.map((entry) => [entry.name, entry.url])),
  }));
}

// ============ Integration History ============

export async function listIntegrationHistory(
  baseUrl: string,
  projectId: string,
  query: HistoryQuery
): Promise<IntegrationHistoryRecord[]> {
  return request<IntegrationHistoryRecord[]>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e${qs(query as Record<string, string | number | undefined>)}`
  );
}

export async function getIntegrationTest(
  baseUrl: string,
  projectId: string,
  testId: string
): Promise<IntegrationHistoryRecord> {
  return request<IntegrationHistoryRecord>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e/${testId}`);
}

export async function deleteIntegrationHistory(
  baseUrl: string,
  projectId: string,
  query?: { pipelineIndex?: number }
): Promise<void> {
  const params = query?.pipelineIndex !== undefined ? qs({ pipelineIndex: query.pipelineIndex } as Record<string, string | number | undefined>) : "";
  await request<void>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e${params}`,
    { method: "DELETE" }
  );
}

// ============ Load History ============

export async function listLoadHistory(
  baseUrl: string,
  projectId: string,
  query: HistoryQuery
): Promise<LoadHistoryRecord[]> {
  return request<LoadHistoryRecord[]>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/load${qs(query as Record<string, string | number | undefined>)}`
  );
}

export async function getLoadTest(
  baseUrl: string,
  projectId: string,
  testId: string
): Promise<LoadHistoryRecord> {
  return request<LoadHistoryRecord>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/load/${testId}`);
}

export async function deleteLoadHistory(
  baseUrl: string,
  projectId: string,
  query?: { pipelineIndex?: number }
): Promise<void> {
  const params = query?.pipelineIndex !== undefined ? qs({ pipelineIndex: query.pipelineIndex } as Record<string, string | number | undefined>) : "";
  await request<void>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/load${params}`,
    { method: "DELETE" }
  );
}

// ============ History Mapping ============

function toSafeNumber(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function stringifyAssertionValue(value: unknown): string | undefined {
  if (value === undefined) return undefined;
  if (value === null) return "null";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function buildRunnerResourceHistoryFromLines(lines: Array<Record<string, unknown>>): RunnerResourcePoint[] {
  return lines.flatMap((line) => {
    const payload = isRecord(line.payload) ? line.payload : line;
    const runtime = isRecord(payload.runtime) ? payload.runtime : null;
    const node = typeof line.node === "string" ? line.node : typeof payload.node === "string" ? payload.node : null;
    if (!runtime || !node) return [];

    const startTime = toSafeNumber(payload.startTime, Date.now());
    const elapsedMs = toSafeNumber(payload.elapsedMs, 0);
    const memoryBytes = toSafeNumber(runtime.memoryBytes, 0);
    const networkTxBytes = toSafeNumber(runtime.networkTxBytes, 0);
    const networkRxBytes = toSafeNumber(runtime.networkRxBytes, 0);
    const networkTotalBytes = toSafeNumber(
      runtime.networkTotalBytes,
      networkTxBytes + networkRxBytes,
    );

    return [{
      node,
      timestamp: startTime + elapsedMs,
      elapsedMs,
      cpuUsagePercent: toSafeNumber(runtime.cpuUsagePercent, 0),
      memoryBytes,
      memoryMb: Math.round((memoryBytes / 1024 / 1024) * 100) / 100,
      networkTxBytes,
      networkRxBytes,
      networkTotalBytes,
      networkTotalKb: Math.round((networkTotalBytes / 1024) * 100) / 100,
    }];
  });
}

function normalizeAssertionResults(raw: unknown): Array<{
  assertion: { field: string; operator: "equals" | "not_equals" | "contains" | "exists" | "not_exists" | "gt" | "lt"; expected?: string };
  passed: boolean;
  actual?: string;
}> | undefined {
  if (!Array.isArray(raw)) return undefined;
  const allowed = new Set(["equals", "not_equals", "contains", "exists", "not_exists", "gt", "lt"]);
  return raw
    .filter((item) => item && typeof item === "object")
    .map((item: any) => {
      const assertion = item.assertion && typeof item.assertion === "object" ? item.assertion : {};
      const operatorRaw = typeof assertion.operator === "string" ? assertion.operator : "equals";
      const operator = (allowed.has(operatorRaw) ? operatorRaw : "equals") as
        | "equals"
        | "not_equals"
        | "contains"
        | "exists"
        | "not_exists"
        | "gt"
        | "lt";
      const expected = stringifyAssertionValue(assertion.expected);
      return {
        assertion: {
          field: typeof assertion.field === "string" ? assertion.field : "unknown",
          operator,
          expected,
        },
        passed: item.passed === true,
        actual: stringifyAssertionValue(item.actual),
      };
    });
}

export function integrationRecordToRun(r: IntegrationHistoryRecord): ExecutionRun {
  const results: Record<string, any> = {};
  if (Array.isArray(r.steps)) {
    for (const step of r.steps) {
      const s = step as any;
      const stepId = s.stepId ?? s.id ?? String(Object.keys(results).length);
      const normalizedAssertResults = normalizeAssertionResults(s.assertResults ?? s.assertFailures);
      const normalizedFailures =
        normalizeAssertionResults(s.assertFailures) ??
        (normalizedAssertResults ? normalizedAssertResults.filter((item) => item.passed === false) : undefined);
      results[stepId] = {
        stepId,
        status: s.status ?? "success",
        request: s.request,
        response: s.response,
        error: s.error,
        duration: toSafeNumber(s.duration, 0),
        assertResults: normalizedAssertResults,
        assertFailures: normalizedFailures,
      };
    }
  }

  const startedAtMs = toSafeNumber(r.startedAtMs, Date.now());
  return {
    id: r.id ?? r.executionId ?? "",
    projectId: r.projectId ?? "",
    pipelineIndex: r.pipelineIndex ?? 0,
    pipelineName: r.pipelineName,
    status:
      r.status === "success"
        ? "success"
        : r.status === "running"
          ? "running"
          : "error",
    timestamp: new Date(startedAtMs).toISOString(),
    duration: toSafeNumber(r.durationMs, 0),
    results,
    executionId: r.executionId,
  };
}

export function loadRecordToRun(r: LoadHistoryRecord): LoadTestRunRecord {
  const cfg = r.requestedConfig as any;
  const consolidated = r.finalConsolidated as any;

  const metrics: LoadTestMetrics = {
    totalSent: consolidated?.totalSent ?? 0,
    totalSuccess: consolidated?.totalSuccess ?? 0,
    totalError: consolidated?.totalError ?? 0,
    avgLatency: consolidated?.avgLatency ?? 0,
    p95: consolidated?.p95 ?? 0,
    p99: consolidated?.p99 ?? 0,
    rps: consolidated?.rps ?? 0,
    latencyHistory: consolidated?.latencyHistory ?? [],
    rpsHistory: consolidated?.rpsHistory ?? [],
    runnerResourceHistory: consolidated?.runnerResourceHistory ?? buildRunnerResourceHistoryFromLines(r.finalLines),
    startTime: consolidated?.startTime ?? r.startedAtMs,
    elapsedMs: consolidated?.elapsedMs ?? r.durationMs,
  };

  const state: LoadTestState =
    r.status === "completed" ? "completed"
    : r.status === "running" ? "running"
    : "cancelled";

  return {
    id: r.id ?? r.executionId ?? "",
    projectId: r.projectId ?? "",
    pipelineIndex: r.pipelineIndex ?? 0,
    pipelineName: r.pipelineName,
    config: {
      totalRequests: cfg?.totalRequests ?? 0,
      concurrency: cfg?.concurrency ?? 1,
      rampUpSeconds: cfg?.rampUpSeconds ?? 0,
    },
    metrics,
    state,
    timestamp: new Date(r.startedAtMs).toISOString(),
    executionId: r.executionId,
  };
}

// ============ Spec Validation ============

export interface SpecValidationResponse {
  spec: Record<string, unknown> | null;
  sourceMd5: string;
  status: "valid" | "invalid";
  points?: Array<{ severity: string; comment: string; pointer?: string; line?: number }>;
}

export async function validateSpec(
  baseUrl: string,
  source: string,
): Promise<SpecValidationResponse> {
  return request<SpecValidationResponse>(`${ensureApiPrefix(baseUrl)}/specs/validate`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ source }),
  });
}

// ============ Execution Cancel ============

export async function cancelExecution(baseUrl: string, executionId: string): Promise<void> {
  const url = `${ensureApiPrefix(baseUrl)}/executions/${executionId}/cancel`;
  await fetch(url, { method: "POST" }).catch(() => {});
}

// ============ E2E Queue (Serial Execution) ============

export type E2eQueueStatus = "pending" | "running" | "failed" | "completed" | "cancelled";

export interface E2eQueuePipelineRecord {
  id: string;
  status: E2eQueueStatus;
  updatedAt: string;
}

export interface E2eQueueRecord {
  id: string;
  status: E2eQueueStatus;
  pipelines: E2eQueuePipelineRecord[];
  updatedAt: string;
}

export interface ProjectE2eQueueRequest {
  pipelineIds: string[];
  selectedBaseUrlKey?: string | null;
  selectedEnvGroupSlug?: string | null;
  specs?: Array<{ slug: string; servers: Record<string, string> }>;
  envGroups?: RuntimeEnvGroup[];
}

export async function createE2eQueue(
  baseUrl: string,
  projectId: string,
  data: ProjectE2eQueueRequest,
): Promise<E2eQueueRecord> {
  return request<E2eQueueRecord>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e/queue`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(data),
    },
  );
}

export async function getCurrentE2eQueue(
  baseUrl: string,
  projectId: string,
): Promise<E2eQueueRecord | null> {
  try {
    return await request<E2eQueueRecord>(
      `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e/queue`,
    );
  } catch {
    return null;
  }
}

export async function getE2eQueue(
  baseUrl: string,
  projectId: string,
  queueId: string,
): Promise<E2eQueueRecord> {
  return request<E2eQueueRecord>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e/queue/${queueId}`,
  );
}

export async function deleteE2eQueue(
  baseUrl: string,
  projectId: string,
  queueId: string,
): Promise<void> {
  await request<void>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e/queue/${queueId}`,
    { method: "DELETE" },
  );
}

export interface E2eQueueStreamCallbacks {
  onSnapshot: (record: E2eQueueRecord) => void;
  onError?: (error: string) => void;
  onClose?: () => void;
}

export interface E2eQueueStreamController {
  disconnect: () => void;
}

function parseSseBlocks(chunk: string): Array<{ event: string; data: string }> {
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

export function connectE2eQueue(
  baseUrl: string,
  projectId: string,
  queueId: string,
  callbacks: E2eQueueStreamCallbacks,
): E2eQueueStreamController {
  const abortController = new AbortController();

  const run = async () => {
    try {
      const url = `${ensureApiPrefix(baseUrl)}/projects/${projectId}/tests/e2e/queue/${queueId}`;
      const response = await fetch(url, {
        method: "GET",
        headers: { Accept: "text/event-stream, application/json" },
        signal: abortController.signal,
      });

      if (!response.ok) {
        const errorText = await response.text().catch(() => "");
        callbacks.onError?.(`HTTP ${response.status}: ${errorText}`);
        return;
      }

      const contentType = response.headers.get("content-type") ?? "";
      if (!contentType.includes("text/event-stream")) {
        const record = await response.json() as E2eQueueRecord;
        callbacks.onSnapshot(record);
        callbacks.onClose?.();
        return;
      }

      const reader = response.body?.getReader();
      if (!reader) {
        callbacks.onError?.("Stream da fila indisponível");
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
          const events = parseSseBlocks(part + "\n\n");
          for (const { event, data } of events) {
            if (event !== "queue:update") continue;
            try {
              callbacks.onSnapshot(JSON.parse(data) as E2eQueueRecord);
            } catch {
              // ignore malformed queue events
            }
          }
        }
      }

      if (buffer.trim()) {
        const events = parseSseBlocks(buffer + "\n\n");
        for (const { event, data } of events) {
          if (event !== "queue:update") continue;
          try {
            callbacks.onSnapshot(JSON.parse(data) as E2eQueueRecord);
          } catch {
            // ignore malformed queue events
          }
        }
      }

      callbacks.onClose?.();
    } catch (error) {
      if ((error as Error).name === "AbortError") {
        callbacks.onClose?.();
        return;
      }
      callbacks.onError?.(error instanceof Error ? error.message : String(error));
    }
  };

  void run();

  return {
    disconnect: () => abortController.abort(),
  };
}

// ============ Project Export / Import ============

export interface ProjectExportEnvelope {
  format: string;
  version: number;
  exportedAt: string;
  project: Record<string, unknown>;
  history?: Record<string, unknown>[] | null;
  loadTestHistory?: Record<string, unknown>[] | null;
}

export interface ProjectImportResponse {
  id: string;
  name: string;
  envGroupsImported?: number;
}

export interface SqliteProjectImportResponse {
  includeHistory: boolean;
  projectsImported: number;
  projects: Array<{
    sourceProjectId: string;
    projectId: string;
    projectName: string;
    pipelinesImported: number;
    specsImported: number;
    envGroupsImported?: number;
    e2eHistoryImported: number;
    loadHistoryImported: number;
  }>;
}

export interface ProjectSqliteExportRequest {
  all: boolean;
  projectIds: string[];
  includeHistory: boolean;
}

export async function exportProjectRemote(
  baseUrl: string,
  projectId: string,
  includeHistory: boolean,
): Promise<ProjectExportEnvelope> {
  const qs = includeHistory ? "?includeHistory=true" : "";
  return request<ProjectExportEnvelope>(
    `${ensureApiPrefix(baseUrl)}/projects/${projectId}/export${qs}`,
  );
}

export async function exportProjectsSqliteRemote(
  baseUrl: string,
  payload: ProjectSqliteExportRequest,
): Promise<Blob> {
  const url = `${ensureApiPrefix(baseUrl)}/projects/export`;
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${text}`);
  }
  return res.blob();
}

export async function importProjectsSqliteRemote(
  baseUrl: string,
  bytes: ArrayBuffer,
  includeHistory: boolean,
): Promise<SqliteProjectImportResponse> {
  const query = includeHistory ? "?includeHistory=true" : "?includeHistory=false";
  return request<SqliteProjectImportResponse>(
    `${ensureApiPrefix(baseUrl)}/projects/import${query}`,
    {
      method: "POST",
      headers: { "Content-Type": "application/vnd.sqlite3" },
      body: bytes,
    },
  );
}

export async function importProjectRemote(
  baseUrl: string,
  envelope: ProjectExportEnvelope,
  includeHistory: boolean,
): Promise<ProjectImportResponse> {
  const qs = includeHistory ? "?includeHistory=true" : "";
  return request<ProjectImportResponse>(
    `${ensureApiPrefix(baseUrl)}/projects/import${qs}`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(envelope),
    },
  );
}
