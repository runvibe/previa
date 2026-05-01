export interface LoadTestConfig {
  totalRequests: number;
  concurrency: number;
  rampUpSeconds: number;
}

export interface LatencyPoint {
  index: number;
  latency: number;
  timestamp: number;
}

export interface RpsPoint {
  timestamp: number;
  rps: number;
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

export interface RunnerResourcePoint {
  node: string;
  timestamp: number;
  elapsedMs: number;
  cpuUsagePercent: number;
  memoryBytes: number;
  memoryMb: number;
  networkTxBytes: number;
  networkRxBytes: number;
  networkTotalBytes: number;
  networkTotalKb: number;
}

/** Slim payload sent by the backend SSE (no latency history/percentiles). */
export interface RemoteMetricsEvent {
  totalSent: number;
  totalSuccess: number;
  totalError: number;
  rps: number;
  startTime: number;
  elapsedMs: number;
  runtime?: RunnerRuntimeInfo;
}

/** Rich client-side metrics used by UI & storage. */
export interface LoadTestMetrics {
  totalSent: number;
  totalSuccess: number;
  totalError: number;
  avgLatency: number;
  p95: number;
  p99: number;
  rps: number;
  latencyHistory: LatencyPoint[];
  rpsHistory: RpsPoint[];
  runnerResourceHistory: RunnerResourcePoint[];
  startTime: number;
  elapsedMs: number;
}

/** Consolidated metrics sent by the orchestrator (includes percentiles). */
export interface ConsolidatedLoadMetrics {
  totalSent: number;
  totalSuccess: number;
  totalError: number;
  rps: number;
  avgLatency: number;
  p95: number;
  p99: number;
  startTime: number;
  elapsedMs: number;
  nodesReporting: number;
}

export type LoadTestState = "idle" | "running" | "completed" | "cancelled";

export interface LoadTestRun {
  id?: number;
  projectId: string;
  pipelineIndex: number;
  pipelineName: string;
  config: LoadTestConfig;
  metrics: LoadTestMetrics;
  state: LoadTestState;
  timestamp: string;
}
