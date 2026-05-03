import { useTranslation } from "react-i18next";
import { Progress } from "@/components/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Activity, Zap, AlertCircle, CheckCircle2, Clock, TrendingUp, Server, Gauge, AlertTriangle, ListChecks } from "lucide-react";
import { LineChart, Line, AreaChart, Area, XAxis, YAxis, Tooltip as RechartsTooltip, ResponsiveContainer, CartesianGrid } from "recharts";
import { buildRpsChartData } from "@/lib/load-rps-chart";
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
  const hasTargetRpsLine = rpsChartData.some((point) => typeof point.targetRpsLimit === "number");
  const runnerResourceHistory = metrics.runnerResourceHistory ?? [];
  const runnerNames = getRunnerNames(runnerResourceHistory);
  const cpuChartData = buildRunnerResourceChartData(runnerResourceHistory, "cpuUsagePercent");
  const memoryChartData = buildRunnerResourceChartData(runnerResourceHistory, "memoryMb");
  const networkChartData = buildRunnerResourceChartData(runnerResourceHistory, "networkTotalKb");

  return (
    <div className="space-y-4 p-1">
      {nodesInfo && nodesInfo.nodesUsed > 0 && (
        <div className="glass rounded-lg p-3 flex items-center gap-3">
          <Server className="h-4 w-4 text-primary shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-xs font-semibold">
                {t(nodesInfo.nodesUsed === 1 ? "loadTestResults.nodes" : "loadTestResults.nodes_plural", { count: nodesInfo.nodesUsed })}
              </span>
              <span className="text-[10px] text-muted-foreground">
                {t("loadTestResults.nodesOf", { count: nodesInfo.nodesFound, suffix: nodesInfo.nodesFound !== 1 ? "is" : "l" })}
              </span>
            </div>
            {nodesInfo.nodeNames.length > 0 && (
              <div className="flex flex-wrap gap-1 mt-1">
                {nodesInfo.nodeNames.map((name) => (
                  <span key={name} className="inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground">
                    {name}
                  </span>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
      {/* Progress */}
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

      {/* Metric cards */}
      <div className="grid grid-cols-3 gap-2">
        <MetricCard icon={Zap} label={t("loadTestResults.sent")} value={metrics.totalSent} />
        <MetricCard icon={CheckCircle2} label={t("loadTestResults.success")} value={metrics.totalSuccess} color="text-success" />
        <MetricCard icon={AlertCircle} label={t("loadTestResults.error")} value={metrics.totalError} color="text-destructive" />
      </div>
      <div className={`grid gap-2 ${metrics.avgLatency > 0 ? 'grid-cols-2 sm:grid-cols-3 xl:grid-cols-5' : 'grid-cols-2'}`}>
        <MetricCard icon={TrendingUp} label="RPS" value={metrics.rps} color="text-primary" />
        <MetricCard
          icon={Clock}
          label={t("loadTestResults.elapsedLabel", "Time")}
          value={`${Math.round(metrics.elapsedMs / 1000)}s`}
        />
        {metrics.avgLatency > 0 && (
          <>
            <MetricCard icon={Clock} label={t("loadTestResults.avg")} value={`${metrics.avgLatency}ms`} />
            <MetricCard icon={Activity} label="P95" value={`${metrics.p95}ms`} />
            <MetricCard icon={Activity} label="P99" value={`${metrics.p99}ms`} />
          </>
        )}
      </div>
      {(typeof metrics.targetIntensity === "number" ||
        typeof metrics.targetRpsLimit === "number" ||
        typeof metrics.inFlight === "number") && (
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {typeof metrics.targetIntensity === "number" && (
            <MetricCard icon={Gauge} label={t("loadTestResults.targetIntensity")} value={`${metrics.targetIntensity.toFixed(1)}%`} color="text-primary" />
          )}
          {typeof metrics.targetRpsLimit === "number" && (
            <MetricCard icon={Gauge} label={t("loadTestResults.targetRpsLimit")} value={metrics.targetRpsLimit.toFixed(1)} color="text-primary" />
          )}
          {typeof metrics.inFlight === "number" && (
            <MetricCard icon={Activity} label={t("loadTestResults.inFlight")} value={metrics.inFlight} />
          )}
        </div>
      )}
      {(typeof metrics.curveAdherence === "number" ||
        typeof metrics.missedStarts === "number" ||
        typeof metrics.readyRequests === "number" ||
        typeof metrics.outstandingRequests === "number") && (
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          {typeof metrics.curveAdherence === "number" && (
            <MetricCard
              icon={Activity}
              label={t("loadTestResults.curveAdherence")}
              value={`${parseFloat(metrics.curveAdherence.toFixed(1))}%`}
              color="text-success"
            />
          )}
          {typeof metrics.missedStarts === "number" && (
            <MetricCard
              icon={AlertTriangle}
              label={t("loadTestResults.missedStarts")}
              value={metrics.missedStarts}
              color="text-warning"
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
          {typeof metrics.outstandingRequests === "number" && (
            <MetricCard
              icon={Activity}
              label={t("loadTestResults.outstandingRequests")}
              value={metrics.outstandingRequests}
            />
          )}
        </div>
      )}
      {(typeof metrics.dispatchSubmitted === "number" ||
        typeof metrics.httpSendReturned === "number" ||
        typeof metrics.responseBodyCompleted === "number" ||
        typeof metrics.dependencyLimitedStarts === "number" ||
        typeof metrics.runtimeLaggedStarts === "number") && (
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
          {typeof metrics.dispatchSubmitted === "number" && (
            <MetricCard
              icon={ListChecks}
              label={t("loadTestResults.dispatchSubmitted")}
              value={metrics.dispatchSubmitted}
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
          {typeof metrics.runtimeLaggedStarts === "number" && (
            <MetricCard
              icon={AlertTriangle}
              label={t("loadTestResults.runtimeLaggedStarts")}
              value={metrics.runtimeLaggedStarts}
              color="text-warning"
            />
          )}
        </div>
      )}

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
                labelFormatter={(v) => `${v}s`}
              />
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

      {/* Latency chart */}
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

      {/* RPS chart */}
      {rpsChartData.length > 1 && (
        <div className="glass rounded-lg p-3 space-y-2">
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

    </div>
  );
}
