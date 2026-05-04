export interface LoadTestConfig {
  totalRequests: number;
  concurrency: number;
  rampUpSeconds: number;
}

export type LoadInterpolation = "smooth" | "linear" | "step";

export interface LoadPoint {
  atMs: number;
  intensity: number;
}

export interface WaveLoadConfig {
  points: LoadPoint[];
  interpolation: LoadInterpolation;
  gracePeriodMs?: number;
}

export type LoadRunConfig = LoadTestConfig | WaveLoadConfig;

export function isWaveLoadConfig(config: LoadRunConfig | null | undefined): config is WaveLoadConfig {
  return !!config && Array.isArray((config as WaveLoadConfig).points);
}

export interface LatencyPoint {
  index: number;
  latency: number;
  timestamp: number;
}

export interface RpsPoint {
  timestamp: number;
  elapsedMs?: number;
  rps: number;
  dispatchBucket?: number;
  totalStarted?: number;
  totalSent?: number;
  httpStarted?: number;
  httpCompleted?: number;
  dispatchSubmitted?: number;
  dispatchStarted?: number;
  httpSendReturned?: number;
  responseBodyCompleted?: number;
  dependencyLimitedStarts?: number;
  dispatcherLaggedStarts?: number;
  runtimeLaggedStarts?: number;
  schedulerLagMs?: number;
  schedulerLaggedStarts?: number;
  slotEnqueued?: number;
  requestPrepared?: number;
  requestEnqueued?: number;
  sendTaskSpawned?: number;
  sendStarted?: number;
  targetIntensity?: number;
  targetRpsLimit?: number;
  scheduledStarts?: number;
  missedStarts?: number;
  readyRequests?: number;
  activePipelines?: number;
  outstandingRequests?: number;
  curveAdherence?: number;
  runners?: RunnerRpsSample[];
}

export interface RunnerRpsSample {
  runnerId: string;
  dispatchBucket?: number;
  httpStarted?: number;
  httpCompleted?: number;
  dispatchSubmitted?: number;
  dispatchStarted?: number;
  httpSendReturned?: number;
  responseBodyCompleted?: number;
  dependencyLimitedStarts?: number;
  dispatcherLaggedStarts?: number;
  runtimeLaggedStarts?: number;
  schedulerLagMs?: number;
  schedulerLaggedStarts?: number;
  slotEnqueued?: number;
  requestPrepared?: number;
  requestEnqueued?: number;
  sendTaskSpawned?: number;
  sendStarted?: number;
  totalStarted?: number;
  totalSent?: number;
  rps?: number;
  scheduledStarts?: number;
  missedStarts?: number;
  readyRequests?: number;
  activePipelines?: number;
  outstandingRequests?: number;
  curveAdherence?: number;
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

export interface DispatchBucket {
  elapsedMs: number;
  count: number;
}

/** Slim payload sent by the backend SSE (no latency history/percentiles). */
export interface RemoteMetricsEvent {
  totalStarted?: number;
  totalSent: number;
  totalSuccess: number;
  totalError: number;
  httpStarted?: number;
  httpCompleted?: number;
  dispatchSubmitted?: number;
  dispatchStarted?: number;
  httpSendReturned?: number;
  responseBodyCompleted?: number;
  dependencyLimitedStarts?: number;
  dispatcherLaggedStarts?: number;
  runtimeLaggedStarts?: number;
  schedulerLagMs?: number;
  schedulerLaggedStarts?: number;
  slotEnqueued?: number;
  requestPrepared?: number;
  requestEnqueued?: number;
  sendTaskSpawned?: number;
  sendStarted?: number;
  rps: number;
  startTime: number;
  elapsedMs: number;
  dispatchBuckets?: DispatchBucket[];
  targetIntensity?: number;
  targetRpsLimit?: number;
  inFlight?: number;
  runnerMaxRps?: number;
  tickMs?: number;
  scheduledStarts?: number;
  missedStarts?: number;
  readyRequests?: number;
  activePipelines?: number;
  outstandingRequests?: number;
  curveAdherence?: number;
  runtime?: RunnerRuntimeInfo;
}

/** Rich client-side metrics used by UI & storage. */
export interface LoadTestMetrics {
  totalStarted?: number;
  totalSent: number;
  totalSuccess: number;
  totalError: number;
  httpStarted?: number;
  httpCompleted?: number;
  dispatchSubmitted?: number;
  dispatchStarted?: number;
  httpSendReturned?: number;
  responseBodyCompleted?: number;
  dependencyLimitedStarts?: number;
  dispatcherLaggedStarts?: number;
  runtimeLaggedStarts?: number;
  schedulerLagMs?: number;
  schedulerLaggedStarts?: number;
  slotEnqueued?: number;
  requestPrepared?: number;
  requestEnqueued?: number;
  sendTaskSpawned?: number;
  sendStarted?: number;
  avgLatency: number;
  p95: number;
  p99: number;
  rps: number;
  latencyHistory: LatencyPoint[];
  rpsHistory: RpsPoint[];
  runnerResourceHistory: RunnerResourcePoint[];
  errors?: string[];
  startTime: number;
  elapsedMs: number;
  targetIntensity?: number;
  targetRpsLimit?: number;
  inFlight?: number;
  runnerMaxRps?: number;
  tickMs?: number;
  scheduledStarts?: number;
  missedStarts?: number;
  readyRequests?: number;
  activePipelines?: number;
  outstandingRequests?: number;
  curveAdherence?: number;
}

/** Consolidated metrics sent by the orchestrator (includes percentiles). */
export interface ConsolidatedLoadMetrics {
  totalStarted?: number;
  totalSent: number;
  totalSuccess: number;
  totalError: number;
  httpStarted?: number;
  httpCompleted?: number;
  dispatchSubmitted?: number;
  dispatchStarted?: number;
  httpSendReturned?: number;
  responseBodyCompleted?: number;
  dependencyLimitedStarts?: number;
  dispatcherLaggedStarts?: number;
  runtimeLaggedStarts?: number;
  schedulerLagMs?: number;
  schedulerLaggedStarts?: number;
  slotEnqueued?: number;
  requestPrepared?: number;
  requestEnqueued?: number;
  sendTaskSpawned?: number;
  sendStarted?: number;
  rps: number;
  avgLatency: number;
  p95: number;
  p99: number;
  startTime: number;
  elapsedMs: number;
  nodesReporting: number;
  targetIntensity?: number;
  targetRpsLimit?: number;
  inFlight?: number;
  runnerMaxRps?: number;
  tickMs?: number;
  scheduledStarts?: number;
  missedStarts?: number;
  readyRequests?: number;
  activePipelines?: number;
  outstandingRequests?: number;
  curveAdherence?: number;
}

export type LoadTestState = "idle" | "running" | "completed" | "cancelled";

export interface LoadTestRun {
  id?: number;
  projectId: string;
  pipelineIndex: number;
  pipelineName: string;
  config: LoadRunConfig;
  metrics: LoadTestMetrics;
  state: LoadTestState;
  timestamp: string;
}
