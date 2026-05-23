import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Progress } from "@/components/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Activity, Zap, AlertCircle, CheckCircle2, Clock, TrendingUp, Server, Gauge, AlertTriangle, ListChecks, ChevronDown, Copy, Check } from "lucide-react";
import { LineChart, Line, AreaChart, Area, XAxis, YAxis, Tooltip as RechartsTooltip, ResponsiveContainer, CartesianGrid, ReferenceLine } from "recharts";
import { buildLifecycleChartData, type LifecycleSeriesTone } from "@/lib/load-lifecycle-chart";
import { buildRpsChartData, buildWaveSecondMarkers, formatPlannedRequests } from "@/lib/load-rps-chart";
import { buildStatusCodeChartData } from "@/lib/load-status-code-chart";
import { deriveWaveDiagnostics } from "@/lib/wave-diagnostics";
import { isWaveLoadConfig } from "@/types/load-test";
import type { LoadInterpolation, LoadPoint, LoadRunConfig, LoadTestMetrics, LoadTestState, RunnerResourcePoint, WaveLoadConfig } from "@/types/load-test";

const RUNNER_RESOURCE_COLORS = [
  "hsl(var(--primary))",
  "hsl(var(--status-success))",
  "hsl(var(--status-running))",
  "hsl(var(--status-error))",
  "#a855f7",
  "#06b6d4",
  "#f97316",
  "#84cc16",
];

const LIFECYCLE_COLORS: Record<LifecycleSeriesTone, string> = {
  planned: "hsl(var(--primary))",
  send: "hsl(var(--status-running))",
  http: "hsl(var(--status-success))",
  returned: "hsl(var(--warning))",
  body: "hsl(var(--muted-foreground))",
  startLag: "#a855f7",
  sendLag: "#06b6d4",
  observeLag: "#f97316",
};

function formatCompact(value: string | number): { display: string; full: string; needsTooltip: boolean } {
  const raw = typeof value === "string" ? value : String(value);
  const numMatch = raw.match(/^(-?[\d.]+)(.*)?$/);
  if (!numMatch) return { display: raw, full: raw, needsTooltip: false };

  const num = parseFloat(numMatch[1]);
  const suffix = numMatch[2] ?? "";
  if (isNaN(num)) return { display: raw, full: raw, needsTooltip: false };

  const absNum = Math.abs(num);
  let display: string;

  if (absNum >= 1_000_000) display = `${(num / 1_000_000).toFixed(1)}M${suffix}`;
  else if (absNum >= 10_000) display = `${(num / 1_000).toFixed(1)}K${suffix}`;
  else if (absNum >= 100) display = `${Math.round(num)}${suffix}`;
  else if (absNum >= 1) display = `${parseFloat(num.toFixed(1))}${suffix}`;
  else display = `${parseFloat(num.toFixed(2))}${suffix}`;

  return { display, full: raw, needsTooltip: display !== raw };
}

function MetricCard({ icon: Icon, label, value, color }: { icon: React.ElementType; label: string; value: string | number; color?: string }) {
  const { display, full, needsTooltip } = formatCompact(value);

  const content = (
    <div className="glass rounded-lg p-3 flex flex-col items-center gap-1 min-w-0">
      <Icon className={`h-3.5 w-3.5 ${color || "text-muted-foreground"}`} />
      <span className="text-lg font-bold leading-none">{display}</span>
      <span className="text-[9px] text-muted-foreground uppercase tracking-wider whitespace-nowrap">{label}</span>
    </div>
  );

  if (!needsTooltip) return content;

  return (
    <Tooltip>
      <TooltipTrigger asChild>{content}</TooltipTrigger>
      <TooltipContent className="font-mono text-xs">{full}</TooltipContent>
    </Tooltip>
  );
}

function ResultsSection({
  title,
  testId,
  children,
}: {
  title: string;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <section data-testid={testId} className="space-y-2">
      <h3 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </h3>
      <div className="space-y-2">{children}</div>
    </section>
  );
}

function buildRunnerResourceChartData(
  points: RunnerResourcePoint[],
  valueKey: "cpuUsagePercent" | "memoryMb" | "networkTotalKb",
) {
  const rows = new Map<number, Record<string, number>>();

  for (const point of points.slice(-300)) {
    const second = Math.round(point.elapsedMs / 1000);
    const row = rows.get(second) ?? { time: second };
    row[point.node] = Math.round(point[valueKey] * 100) / 100;
    rows.set(second, row);
  }

  return Array.from(rows.values()).sort((a, b) => a.time - b.time);
}

function getRunnerNames(points: RunnerResourcePoint[]) {
  return Array.from(new Set(points.map((point) => point.node)));
}

function formatMemory(value: number) {
  if (value >= 1024) return `${(value / 1024).toFixed(1)} GB`;
  return `${Math.round(value)} MB`;
}

function formatNetwork(value: number) {
  if (value >= 1024) return `${(value / 1024).toFixed(1)} MB`;
  return `${Math.round(value)} KB`;
}

function buildWaveChartData(config: WaveLoadConfig) {
  return config.points.map((point) => ({
    time: Math.round(point.atMs / 1000),
    intensity: point.intensity,
  }));
}

function waveChartType(interpolation: LoadInterpolation) {
  if (interpolation === "step") return "stepAfter";
  if (interpolation === "linear") return "linear";
  return "monotone";
}

function interpolationLabelKey(interpolation: LoadInterpolation) {
  if (interpolation === "step") return "loadTest.interpolationStep";
  if (interpolation === "linear") return "loadTest.interpolationLinear";
  return "loadTest.interpolationSmooth";
}

function formatPointTime(point: LoadPoint) {
  const seconds = point.atMs / 1000;
  if (Number.isInteger(seconds)) return `${seconds}s`;
  return `${seconds.toFixed(1)}s`;
}

function formatWaveMarkerSecond(second: number) {
  if (Number.isInteger(second)) return `${second}s`;
  return `${second.toFixed(1)}s`;
}

function resolveWaveRunnerCount(
  nodesInfo: LoadTestResultsPanelProps["nodesInfo"],
  runnerSeriesCount: number,
) {
  if (nodesInfo && nodesInfo.nodesUsed > 0) return nodesInfo.nodesUsed;
  if (runnerSeriesCount > 0) return runnerSeriesCount;
  return 1;
}

function compactEndpoint(endpoint: string) {
  try {
    const parsed = new URL(endpoint);
    return parsed.hostname;
  } catch {
    return endpoint;
  }
}

function runnerDisplayName(endpoint: string, index: number) {
  const host = compactEndpoint(endpoint);
  const podName = host.split(".")[0] ?? host;
  const ordinal = podName.match(/-(\d+)$/)?.[1];
  if (ordinal !== undefined) return `runner-${ordinal}`;
  return `runner-${index + 1}`;
}

function NodeSummaryPanel({ nodesInfo }: { nodesInfo: NonNullable<LoadTestResultsPanelProps["nodesInfo"]> }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [copiedEndpoint, setCopiedEndpoint] = useState<string | null>(null);
  const hasEndpoints = nodesInfo.nodeNames.length > 0;
  const availabilityLabel =
    nodesInfo.nodesFound > 0
      ? t("loadTestResults.nodesOf", { count: nodesInfo.nodesFound, suffix: nodesInfo.nodesFound !== 1 ? "is" : "l" })
      : t("loadTestResults.dynamicReservation", { defaultValue: "dynamic reservation" });

  const copyEndpoint = async (endpoint: string) => {
    try {
      await navigator.clipboard.writeText(endpoint);
      setCopiedEndpoint(endpoint);
      window.setTimeout(() => setCopiedEndpoint((current) => current === endpoint ? null : current), 1500);
    } catch {
      setCopiedEndpoint(null);
    }
  };

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="glass rounded-lg p-3 space-y-3">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-3">
          <Server data-testid="load-results-nodes-icon" className="h-4 w-4 shrink-0 text-white" />
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-xs font-semibold">
                {t(nodesInfo.nodesUsed === 1 ? "loadTestResults.nodes" : "loadTestResults.nodes_plural", { count: nodesInfo.nodesUsed })}
              </span>
              <span className="text-[10px] text-muted-foreground">{availabilityLabel}</span>
            </div>
            <div className="mt-1 flex flex-wrap gap-1.5">
              <Badge variant="secondary" className="px-2 py-0 text-[10px]">
                {t("loadTestResults.runnersUsed", { count: nodesInfo.nodesUsed, defaultValue: "{{count}} runners used" })}
              </Badge>
              {hasEndpoints && (
                <Badge variant="outline" className="px-2 py-0 text-[10px]">
                  {t("loadTestResults.endpointsCount", { count: nodesInfo.nodeNames.length, defaultValue: "{{count}} endpoints" })}
                </Badge>
              )}
            </div>
          </div>
        </div>

        {hasEndpoints && (
          <CollapsibleTrigger asChild>
            <Button variant="ghost" size="sm" className="h-7 justify-between gap-2 px-2 text-[11px] sm:justify-center">
              {open
                ? t("loadTestResults.hideEndpoints", { defaultValue: "Hide endpoints" })
                : t("loadTestResults.showEndpoints", { defaultValue: "Show endpoints" })}
              <ChevronDown className={`h-3.5 w-3.5 transition-transform ${open ? "rotate-180" : ""}`} />
            </Button>
          </CollapsibleTrigger>
        )}
      </div>

      {hasEndpoints && (
        <CollapsibleContent className="space-y-2">
          <div className="max-h-[21rem] overflow-y-auto overscroll-contain pr-1" data-testid="load-results-runner-endpoints-scroll">
            <div className="grid gap-2 md:grid-cols-2">
              {nodesInfo.nodeNames.map((endpoint, index) => {
                const copied = copiedEndpoint === endpoint;
                return (
                  <div key={`${endpoint}-${index}`} className="rounded-lg border border-border/60 bg-background/35 p-2.5">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="text-xs font-semibold">{runnerDisplayName(endpoint, index)}</p>
                        <p className="mt-0.5 truncate text-[10px] text-muted-foreground">{compactEndpoint(endpoint)}</p>
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0"
                        onClick={() => copyEndpoint(endpoint)}
                        title={t("loadTestResults.copyEndpoint", { defaultValue: "Copy endpoint" })}
                        aria-label={t("loadTestResults.copyEndpoint", { defaultValue: "Copy endpoint" })}
                      >
                        {copied ? <Check className="h-3.5 w-3.5 text-success" /> : <Copy className="h-3.5 w-3.5" />}
                      </Button>
                    </div>
                    <p className="mt-2 break-all rounded-md bg-muted/30 px-2 py-1.5 font-mono text-[10px] leading-relaxed text-muted-foreground">
                      {endpoint}
                    </p>
                  </div>
                );
              })}
            </div>
          </div>
        </CollapsibleContent>
      )}
    </Collapsible>
  );
}

interface LoadTestResultsPanelProps {
  metrics: LoadTestMetrics;
  state: LoadTestState;
  totalRequests: number;
  config?: LoadRunConfig | null;
  nodesInfo?: { nodesUsed: number; nodesFound: number; nodeNames: string[] } | null;
}

export function LoadTestResultsPanel({ metrics, state, totalRequests, config, nodesInfo }: LoadTestResultsPanelProps) {
  const { t } = useTranslation();
  const progressPercent = totalRequests > 0 ? (metrics.totalSent / totalRequests) * 100 : 0;
  const waveConfig = isWaveLoadConfig(config) ? config : null;
  const waveChartData = waveConfig ? buildWaveChartData(waveConfig) : [];

  const latencyChartData = (metrics.latencyHistory ?? []).slice(-100).map((p) => ({
    idx: p.index,
    latency: p.latency,
  }));

  const rpsChart = buildRpsChartData(metrics, waveConfig);
  const rpsChartData = rpsChart.data;
  const waveRunnerCount = resolveWaveRunnerCount(nodesInfo, rpsChart.runnerSeries.length);
  const waveSecondMarkers = waveConfig
    ? buildWaveSecondMarkers(waveConfig, {
      runnerCount: waveRunnerCount,
      runnerMaxRps: waveConfig.runnerMaxRps ?? metrics.runnerMaxRps,
    })
    : [];
  const visibleWaveSecondMarkers = waveSecondMarkers.filter((marker) => marker.showLabel);
  const lifecycleChart = buildLifecycleChartData(metrics);
  const lifecycleChartData = lifecycleChart.data;
  const statusCodeChart = buildStatusCodeChartData(metrics);
  const statusCodeChartData = statusCodeChart.data;
  const waveDiagnostics = deriveWaveDiagnostics(metrics);
  const hasTargetRpsLine = rpsChartData.some((point) => typeof point.targetRpsLimit === "number");
  const runnerResourceHistory = metrics.runnerResourceHistory ?? [];
  const runnerNames = getRunnerNames(runnerResourceHistory);
  const cpuChartData = buildRunnerResourceChartData(runnerResourceHistory, "cpuUsagePercent");
  const memoryChartData = buildRunnerResourceChartData(runnerResourceHistory, "memoryMb");
  const networkChartData = buildRunnerResourceChartData(runnerResourceHistory, "networkTotalKb");
  const hasTrafficSection = rpsChartData.length > 1;
  const hasWavePlanSection = (waveConfig && waveChartData.length > 1) || lifecycleChartData.length > 0;
  const hasResponseSection =
    metrics.avgLatency > 0 ||
    typeof metrics.inFlight === "number" ||
    statusCodeChartData.length > 0 ||
    latencyChartData.length > 1 ||
    Boolean(metrics.errors && metrics.errors.length > 0);
  const hasGeneratorSummary =
    typeof metrics.schedulerLaggedStarts === "number" ||
    typeof metrics.readyRequests === "number" ||
    typeof metrics.schedulerLagMs === "number";
  const hasGeneratorDetails =
    typeof metrics.dispatchSubmitted === "number" ||
    typeof metrics.dispatchStarted === "number" ||
    typeof metrics.httpSendReturned === "number" ||
    typeof metrics.responseBodyCompleted === "number" ||
    typeof metrics.dependencyLimitedStarts === "number" ||
    typeof metrics.dispatcherLaggedStarts === "number" ||
    typeof metrics.runtimeLaggedStarts === "number" ||
    typeof metrics.senderLaggedStarts === "number" ||
    typeof metrics.senderQueueDepth === "number" ||
    typeof metrics.senderStartLagP95Ms === "number" ||
    typeof metrics.httpSendDurationP95Ms === "number" ||
    typeof metrics.responseObservationDurationP95Ms === "number" ||
    typeof metrics.slotEnqueued === "number" ||
    typeof metrics.requestPrepared === "number" ||
    typeof metrics.requestEnqueued === "number" ||
    typeof metrics.sendTaskSpawned === "number" ||
    typeof metrics.sendStarted === "number" ||
    typeof metrics.httpStarted === "number" ||
    typeof metrics.outstandingRequests === "number";
  const hasRunnerInfraSection = runnerNames.length > 0 && (cpuChartData.length > 0 || memoryChartData.length > 0 || networkChartData.length > 0);

  return (
    <div className="space-y-4 p-1">
      <ResultsSection title={t("loadTestResults.sectionOutcome")} testId="load-results-outcome">
        {nodesInfo && nodesInfo.nodesUsed > 0 && (
          <NodeSummaryPanel nodesInfo={nodesInfo} />
        )}

        {totalRequests > 0 && (
          <div className="space-y-1.5">
            <div className="flex items-center gap-2">
              <Progress value={progressPercent} className="h-3.5 flex-1" />
              <span className="text-[10px] font-medium text-muted-foreground whitespace-nowrap">
                {metrics.totalSent}/{totalRequests}
              </span>
            </div>
          </div>
        )}

        <div className="grid grid-cols-3 gap-2">
          <MetricCard icon={Zap} label={t("loadTestResults.sent")} value={metrics.totalSent} />
          <MetricCard icon={CheckCircle2} label={t("loadTestResults.success")} value={metrics.totalSuccess} color="text-success" />
          <MetricCard icon={AlertCircle} label={t("loadTestResults.error")} value={metrics.totalError} color="text-destructive" />
        </div>

        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          <MetricCard icon={TrendingUp} label="RPS" value={metrics.rps} color="text-primary" />
          <MetricCard
            icon={Clock}
            label={t("loadTestResults.elapsedLabel", "Time")}
            value={`${Math.round(metrics.elapsedMs / 1000)}s`}
          />
          {typeof metrics.targetIntensity === "number" && (
            <MetricCard icon={Gauge} label={t("loadTestResults.targetIntensity")} value={`${metrics.targetIntensity.toFixed(1)}%`} color="text-primary" />
          )}
          {typeof metrics.targetRpsLimit === "number" && (
            <MetricCard icon={Gauge} label={t("loadTestResults.targetRpsLimit")} value={metrics.targetRpsLimit.toFixed(1)} color="text-primary" />
          )}
        </div>

        {(typeof metrics.curveAdherence === "number" || waveDiagnostics.actualMissedStarts > 0) && (
          <div className="grid grid-cols-2 gap-2">
            {typeof metrics.curveAdherence === "number" && (
              <MetricCard
                icon={Activity}
                label={t("loadTestResults.curveAdherence")}
                value={`${parseFloat(metrics.curveAdherence.toFixed(1))}%`}
                color="text-success"
              />
            )}
            <MetricCard
              icon={waveDiagnostics.hasActualWaveLoss ? AlertTriangle : Activity}
              label={t("loadTestResults.actualMissedStarts")}
              value={waveDiagnostics.hasActualWaveLoss ? waveDiagnostics.actualMissedStarts : 0}
              color={waveDiagnostics.hasActualWaveLoss ? "text-warning" : "text-success"}
            />
          </div>
        )}
      </ResultsSection>

      {hasTrafficSection && (
        <ResultsSection title={t("loadTestResults.sectionTraffic")} testId="load-results-wave">
          {rpsChartData.length > 1 && (
            <div data-testid="rps-over-time-chart" className="glass rounded-lg p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                  {rpsChart.usesHttpRps ? t("loadTestResults.httpRpsOverTime") : t("loadTestResults.rpsOverTime")}
                </p>
                <div className="flex items-center gap-2 text-[9px] text-muted-foreground flex-wrap justify-end">
                  {rpsChart.runnerSeries.slice(0, 4).map((runner, index) => (
                    <span key={runner.key} className="inline-flex items-center gap-1">
                      <span
                        className="h-0 w-3 border-t"
                        style={{ borderColor: RUNNER_RESOURCE_COLORS[index % RUNNER_RESOURCE_COLORS.length] }}
                      />
                      {runner.label}
                    </span>
                  ))}
                  {rpsChart.runnerSeries.length > 4 && <span>+{rpsChart.runnerSeries.length - 4}</span>}
                  <span className="inline-flex items-center gap-1">
                    <span className="h-0 w-3 border-t border-dashed border-success" />
                    {rpsChart.usesHttpRps ? t("loadTestResults.rpsTotal") : t("loadTestResults.rpsActual")}
                  </span>
                  {hasTargetRpsLine && (
                    <span data-testid="rps-target-legend" className="inline-flex items-center gap-1">
                      <span className="h-0 w-3 border-t border-dotted border-primary" />
                      {t("loadTestResults.rpsTarget")}
                    </span>
                  )}
                </div>
              </div>
              <ResponsiveContainer width="100%" height={100}>
                <AreaChart data={rpsChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}s`} />
                  <YAxis tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" />
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    formatter={(v: number, name: string) => [
                      typeof v === "number" ? v.toFixed(1) : v,
                      name === "targetRpsLimit"
                        ? t("loadTestResults.rpsTarget")
                        : name === "rpsTotal"
                          ? rpsChart.usesHttpRps ? t("loadTestResults.rpsTotal") : t("loadTestResults.rpsActual")
                          : rpsChart.runnerSeries.find((runner) => runner.key === name)?.label ?? name,
                    ]}
                    labelFormatter={(v) => `${v}s`}
                  />
                  {rpsChart.runnerSeries.map((runner, index) => (
                    <Line
                      key={runner.key}
                      type="monotone"
                      dataKey={runner.key}
                      stroke={RUNNER_RESOURCE_COLORS[index % RUNNER_RESOURCE_COLORS.length]}
                      strokeWidth={1.5}
                      dot={false}
                      connectNulls
                    />
                  ))}
                  <Area
                    type="monotone"
                    dataKey="rpsTotal"
                    stroke="hsl(var(--status-success))"
                    fill="hsl(var(--status-success) / 0.10)"
                    strokeWidth={1.75}
                    strokeDasharray={rpsChart.usesHttpRps ? "5 4" : undefined}
                  />
                  {hasTargetRpsLine && (
                    <Line
                      type={waveConfig ? waveChartType(waveConfig.interpolation) : "monotone"}
                      dataKey="targetRpsLimit"
                      stroke="hsl(var(--primary))"
                      strokeDasharray="2 5"
                      strokeWidth={1.5}
                      dot={false}
                      connectNulls
                    />
                  )}
                </AreaChart>
              </ResponsiveContainer>
            </div>
          )}
        </ResultsSection>
      )}

      {hasResponseSection && (
        <ResultsSection title={t("loadTestResults.sectionResponse")} testId="load-results-response">
          {metrics.avgLatency > 0 && (
            <div className="grid grid-cols-3 gap-2">
              <MetricCard icon={Clock} label={t("loadTestResults.avg")} value={`${metrics.avgLatency}ms`} />
              <MetricCard icon={Activity} label="P95" value={`${metrics.p95}ms`} />
              <MetricCard icon={Activity} label="P99" value={`${metrics.p99}ms`} />
            </div>
          )}

          {typeof metrics.inFlight === "number" && (
            <div className="grid grid-cols-2 gap-2">
              <MetricCard icon={Activity} label={t("loadTestResults.inFlight")} value={metrics.inFlight} />
            </div>
          )}

          {statusCodeChartData.length > 0 && (
            <div data-testid="status-code-timeline-chart" className="glass rounded-lg p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                  {t("loadTestResults.statusCodeTimeline")}
                </p>
                <div className="flex items-center gap-2 text-[9px] text-muted-foreground flex-wrap justify-end">
                  {statusCodeChart.series.map((series) => (
                    <span key={series.code} className="inline-flex items-center gap-1">
                      <span className="h-0 w-3 border-t" style={{ borderColor: series.color }} />
                      {series.labelKey ? t(series.labelKey) : series.code}
                    </span>
                  ))}
                </div>
              </div>
              <ResponsiveContainer width="100%" height={120}>
                <LineChart data={statusCodeChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}s`} />
                  <YAxis tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" allowDecimals={false} />
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    formatter={(v: number, name: string) => [
                      typeof v === "number" ? Math.round(v) : v,
                      name === "network_error" ? t("loadTestResults.networkError") : name,
                    ]}
                    labelFormatter={(v) => `${v}s`}
                  />
                  {statusCodeChart.series.map((series) => (
                    <Line
                      key={series.code}
                      type="monotone"
                      dataKey={series.code}
                      stroke={series.color}
                      strokeWidth={1.6}
                      dot={statusCodeChartData.length === 1}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}

          {latencyChartData.length > 1 && (
            <div className="glass rounded-lg p-3 space-y-2">
              <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{t("loadTestResults.latencyOverTime")}</p>
              <ResponsiveContainer width="100%" height={120}>
                <LineChart data={latencyChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="idx" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" />
                  <YAxis tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" />
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    labelFormatter={(v) => `#${v}`}
                    formatter={(v: number) => [`${v}ms`, t("loadTestResults.latency")]}
                  />
                  <Line type="monotone" dataKey="latency" stroke="hsl(var(--primary))" strokeWidth={1.5} dot={false} />
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}

          {metrics.errors && metrics.errors.length > 0 && (
            <div className="glass rounded-lg p-3 space-y-2">
              <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                {t("loadTestResults.errorSamples")}
              </p>
              <div className="space-y-1">
                {metrics.errors.slice(0, 5).map((error, index) => (
                  <p key={`${error}-${index}`} className="break-words text-xs text-destructive">
                    {error}
                  </p>
                ))}
              </div>
            </div>
          )}
        </ResultsSection>
      )}

      {hasWavePlanSection && (
        <ResultsSection title={t("loadTestResults.sectionWavePlan")} testId="load-results-wave-plan">
          {waveConfig && waveChartData.length > 1 && (
            <div data-testid="configured-wave-chart" className="glass rounded-lg p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                  {t("loadTestResults.configuredWave")}
                </p>
                <span className="text-[10px] text-muted-foreground">
                  {t(interpolationLabelKey(waveConfig.interpolation))}
                </span>
              </div>
              <ResponsiveContainer width="100%" height={120}>
                <AreaChart data={waveChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}s`} />
                  <YAxis domain={[0, 100]} tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}%`} />
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    formatter={(v: number) => [`${v}%`, t("loadTestResults.targetIntensity")]}
                    labelFormatter={(v) => {
                      const marker = waveSecondMarkers.find((item) => item.second === Number(v));
                      if (!marker) return `${v}s`;
                      return `${formatWaveMarkerSecond(marker.second)} - ${formatPlannedRequests(marker.plannedRequests)}`;
                    }}
                  />
                  {visibleWaveSecondMarkers.map((marker) => (
                    <ReferenceLine
                      key={`wave-second-${marker.second}`}
                      x={marker.second}
                      stroke="hsl(var(--muted-foreground))"
                      strokeDasharray="2 4"
                      strokeOpacity={0.42}
                      ifOverflow="extendDomain"
                      label={{
                        value: formatPlannedRequests(marker.plannedRequests),
                        position: "top",
                        fontSize: 9,
                        fill: "hsl(var(--muted-foreground))",
                      }}
                    />
                  ))}
                  <Area
                    type={waveChartType(waveConfig.interpolation)}
                    dataKey="intensity"
                    stroke="hsl(var(--primary))"
                    fill="hsl(var(--primary) / 0.14)"
                    strokeWidth={1.8}
                    dot={waveChartData.length <= 12}
                  />
                </AreaChart>
              </ResponsiveContainer>
              <div className="flex flex-wrap gap-1">
                {waveConfig.points.map((point, index) => (
                  <span key={`${point.atMs}-${point.intensity}-${index}`} className="inline-flex items-center gap-1 rounded-md bg-muted/50 px-1.5 py-0.5 text-[10px]">
                    <span className="font-mono text-muted-foreground">{formatPointTime(point)}</span>
                    <span className="font-semibold">{point.intensity}%</span>
                  </span>
                ))}
              </div>
            </div>
          )}

          {lifecycleChartData.length > 0 && (
            <div data-testid="wave-lifecycle-chart" className="glass rounded-lg p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                  {t("loadTestResults.waveLifecycle")}
                </p>
                <div className="flex items-center gap-2 text-[9px] text-muted-foreground flex-wrap justify-end">
                  {lifecycleChart.series.map((series) => (
                    <span key={series.key} className="inline-flex items-center gap-1">
                      <span
                        className="h-0 w-3 border-t"
                        style={{ borderColor: LIFECYCLE_COLORS[series.tone] }}
                      />
                      {t(series.labelKey)}
                    </span>
                  ))}
                </div>
              </div>
              <ResponsiveContainer width="100%" height={120}>
                <LineChart data={lifecycleChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}s`} />
                  <YAxis yAxisId="count" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" />
                  {lifecycleChart.series.some((series) => series.axis === "ms") && (
                    <YAxis
                      yAxisId="ms"
                      orientation="right"
                      tick={{ fontSize: 9 }}
                      stroke="hsl(var(--muted-foreground))"
                      tickFormatter={(v) => `${v}ms`}
                    />
                  )}
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    formatter={(v: number, name: string) => [
                      typeof v === "number"
                        ? `${v.toFixed(1)}${lifecycleChart.series.find((series) => series.key === name)?.axis === "ms" ? "ms" : ""}`
                        : v,
                      t(lifecycleChart.series.find((series) => series.key === name)?.labelKey ?? name),
                    ]}
                    labelFormatter={(v) => `${v}s`}
                  />
                  {lifecycleChart.series.map((series) => (
                    <Line
                      key={series.key}
                      type="monotone"
                      dataKey={series.key}
                      yAxisId={series.axis}
                      stroke={LIFECYCLE_COLORS[series.tone]}
                      strokeWidth={series.key === "planned" || series.key === "httpStarted" ? 1.75 : 1.4}
                      strokeDasharray={series.key === "planned" ? "2 4" : series.axis === "ms" ? "4 3" : undefined}
                      dot={lifecycleChartData.length === 1}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}
        </ResultsSection>
      )}

      {(hasGeneratorSummary || hasGeneratorDetails) && (
        <ResultsSection title={t("loadTestResults.sectionGenerator")} testId="load-results-generator">
          {hasGeneratorSummary && (
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
              {typeof metrics.schedulerLaggedStarts === "number" && (
                <MetricCard
                  icon={Clock}
                  label={t("loadTestResults.compensatedSchedulerStarts")}
                  value={metrics.schedulerLaggedStarts}
                  color={waveDiagnostics.schedulerDelayWasCompensated ? "text-success" : "text-warning"}
                />
              )}
              {typeof metrics.readyRequests === "number" && (
                <MetricCard
                  icon={ListChecks}
                  label={t("loadTestResults.readyRequests")}
                  value={metrics.readyRequests}
                  color="text-primary"
                />
              )}
              {typeof metrics.schedulerLagMs === "number" && (
                <MetricCard
                  icon={Clock}
                  label={t("loadTestResults.schedulerLagMs")}
                  value={`${metrics.schedulerLagMs}ms`}
                  color="text-warning"
                />
              )}
            </div>
          )}

          {hasGeneratorDetails && (
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4 xl:grid-cols-5">
              {typeof metrics.dispatchSubmitted === "number" && (
                <MetricCard
                  icon={ListChecks}
                  label={t("loadTestResults.dispatchSubmitted")}
                  value={metrics.dispatchSubmitted}
                  color="text-primary"
                />
              )}
              {typeof metrics.dispatchStarted === "number" && (
                <MetricCard
                  icon={Activity}
                  label={t("loadTestResults.dispatchStarted")}
                  value={metrics.dispatchStarted}
                  color="text-primary"
                />
              )}
              {typeof metrics.slotEnqueued === "number" && (
                <MetricCard
                  icon={ListChecks}
                  label={t("loadTestResults.slotEnqueued")}
                  value={metrics.slotEnqueued}
                  color="text-primary"
                />
              )}
              {typeof metrics.requestPrepared === "number" && (
                <MetricCard
                  icon={ListChecks}
                  label={t("loadTestResults.requestPrepared")}
                  value={metrics.requestPrepared}
                  color="text-primary"
                />
              )}
              {typeof metrics.requestEnqueued === "number" && (
                <MetricCard
                  icon={ListChecks}
                  label={t("loadTestResults.requestEnqueued")}
                  value={metrics.requestEnqueued}
                  color="text-primary"
                />
              )}
              {typeof metrics.sendTaskSpawned === "number" && (
                <MetricCard
                  icon={Activity}
                  label={t("loadTestResults.sendTaskSpawned")}
                  value={metrics.sendTaskSpawned}
                  color="text-primary"
                />
              )}
              {typeof metrics.sendStarted === "number" && (
                <MetricCard
                  icon={Activity}
                  label={t("loadTestResults.sendStarted")}
                  value={metrics.sendStarted}
                  color="text-primary"
                />
              )}
              {typeof metrics.httpStarted === "number" && (
                <MetricCard
                  icon={Activity}
                  label={t("loadTestResults.httpStarted")}
                  value={metrics.httpStarted}
                  color="text-primary"
                />
              )}
              {typeof metrics.httpSendReturned === "number" && (
                <MetricCard
                  icon={Activity}
                  label={t("loadTestResults.httpSendReturned")}
                  value={metrics.httpSendReturned}
                />
              )}
              {typeof metrics.responseBodyCompleted === "number" && (
                <MetricCard
                  icon={Activity}
                  label={t("loadTestResults.responseBodyCompleted")}
                  value={metrics.responseBodyCompleted}
                />
              )}
              {typeof metrics.dependencyLimitedStarts === "number" && (
                <MetricCard
                  icon={AlertTriangle}
                  label={t("loadTestResults.dependencyLimitedStarts")}
                  value={metrics.dependencyLimitedStarts}
                  color="text-warning"
                />
              )}
              {typeof metrics.dispatcherLaggedStarts === "number" && (
                <MetricCard
                  icon={AlertTriangle}
                  label={t("loadTestResults.dispatcherLaggedStarts")}
                  value={metrics.dispatcherLaggedStarts}
                  color="text-warning"
                />
              )}
              {typeof metrics.runtimeLaggedStarts === "number" && (
                <MetricCard
                  icon={AlertTriangle}
                  label={t("loadTestResults.runtimeLaggedStarts")}
                  value={metrics.runtimeLaggedStarts}
                  color="text-warning"
                />
              )}
              {typeof metrics.senderLaggedStarts === "number" && (
                <MetricCard
                  icon={AlertTriangle}
                  label={t("loadTestResults.senderLaggedStarts")}
                  value={metrics.senderLaggedStarts}
                  color="text-warning"
                />
              )}
              {typeof metrics.senderQueueDepth === "number" && (
                <MetricCard
                  icon={ListChecks}
                  label={t("loadTestResults.senderQueueDepth")}
                  value={metrics.senderQueueDepth}
                  color="text-primary"
                />
              )}
              {typeof metrics.senderStartLagP95Ms === "number" && (
                <MetricCard
                  icon={Clock}
                  label={t("loadTestResults.senderStartLagP95Ms")}
                  value={`${metrics.senderStartLagP95Ms}ms`}
                  color="text-warning"
                />
              )}
              {typeof metrics.httpSendDurationP95Ms === "number" && (
                <MetricCard
                  icon={Clock}
                  label={t("loadTestResults.httpSendDurationP95Ms")}
                  value={`${metrics.httpSendDurationP95Ms}ms`}
                />
              )}
              {typeof metrics.responseObservationDurationP95Ms === "number" && (
                <MetricCard
                  icon={Clock}
                  label={t("loadTestResults.responseObservationDurationP95Ms")}
                  value={`${metrics.responseObservationDurationP95Ms}ms`}
                />
              )}
              {typeof metrics.outstandingRequests === "number" && (
                <MetricCard
                  icon={Activity}
                  label={t("loadTestResults.observerBacklog")}
                  value={metrics.outstandingRequests}
                />
              )}
            </div>
          )}
        </ResultsSection>
      )}

      {hasRunnerInfraSection && (
        <ResultsSection title={t("loadTestResults.sectionRunnerInfra")} testId="load-results-runner-infra">
          {runnerNames.length > 0 && cpuChartData.length > 0 && (
            <div className="glass rounded-lg p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">Runner CPU</p>
                <div className="flex flex-wrap justify-end gap-x-2 gap-y-1">
                  {runnerNames.map((name, index) => (
                    <span key={name} className="inline-flex items-center gap-1 text-[9px] text-muted-foreground">
                      <span
                        className="h-1.5 w-1.5 rounded-full"
                        style={{ backgroundColor: RUNNER_RESOURCE_COLORS[index % RUNNER_RESOURCE_COLORS.length] }}
                      />
                      <span className="max-w-28 truncate font-mono">{name}</span>
                    </span>
                  ))}
                </div>
              </div>
              <ResponsiveContainer width="100%" height={110}>
                <LineChart data={cpuChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}s`} />
                  <YAxis tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}%`} />
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    formatter={(v: number, name: string) => [`${v}%`, name]}
                    labelFormatter={(v) => `${v}s`}
                  />
                  {runnerNames.map((name, index) => (
                    <Line
                      key={name}
                      type="monotone"
                      dataKey={name}
                      stroke={RUNNER_RESOURCE_COLORS[index % RUNNER_RESOURCE_COLORS.length]}
                      strokeWidth={1.5}
                      dot={cpuChartData.length === 1}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}

          {runnerNames.length > 0 && memoryChartData.length > 0 && (
            <div className="glass rounded-lg p-3 space-y-2">
              <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">Runner memory</p>
              <ResponsiveContainer width="100%" height={110}>
                <LineChart data={memoryChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}s`} />
                  <YAxis tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => formatMemory(Number(v))} width={42} />
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    formatter={(v: number, name: string) => [formatMemory(v), name]}
                    labelFormatter={(v) => `${v}s`}
                  />
                  {runnerNames.map((name, index) => (
                    <Line
                      key={name}
                      type="monotone"
                      dataKey={name}
                      stroke={RUNNER_RESOURCE_COLORS[index % RUNNER_RESOURCE_COLORS.length]}
                      strokeWidth={1.5}
                      dot={memoryChartData.length === 1}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}

          {runnerNames.length > 0 && networkChartData.length > 0 && (
            <div className="glass rounded-lg p-3 space-y-2">
              <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">Runner network</p>
              <ResponsiveContainer width="100%" height={110}>
                <LineChart data={networkChartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                  <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => `${v}s`} />
                  <YAxis tick={{ fontSize: 9 }} stroke="hsl(var(--muted-foreground))" tickFormatter={(v) => formatNetwork(Number(v))} width={42} />
                  <RechartsTooltip
                    contentStyle={{
                      background: "hsl(var(--popover))",
                      border: "1px solid hsl(var(--border))",
                      borderRadius: "var(--radius)",
                      fontSize: 11,
                    }}
                    formatter={(v: number, name: string) => [formatNetwork(v), name]}
                    labelFormatter={(v) => `${v}s`}
                  />
                  {runnerNames.map((name, index) => (
                    <Line
                      key={name}
                      type="monotone"
                      dataKey={name}
                      stroke={RUNNER_RESOURCE_COLORS[index % RUNNER_RESOURCE_COLORS.length]}
                      strokeWidth={1.5}
                      dot={networkChartData.length === 1}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}
        </ResultsSection>
      )}

    </div>
  );
}
