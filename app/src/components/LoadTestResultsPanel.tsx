import { useTranslation } from "react-i18next";
import { Progress } from "@/components/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Activity, Zap, AlertCircle, CheckCircle2, Clock, TrendingUp, Server } from "lucide-react";
import { LineChart, Line, AreaChart, Area, XAxis, YAxis, Tooltip as RechartsTooltip, ResponsiveContainer, CartesianGrid } from "recharts";
import type { LoadTestMetrics, LoadTestState, RunnerResourcePoint } from "@/types/load-test";

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

interface LoadTestResultsPanelProps {
  metrics: LoadTestMetrics;
  state: LoadTestState;
  totalRequests: number;
  nodesInfo?: { nodesUsed: number; nodesFound: number; nodeNames: string[] } | null;
}

export function LoadTestResultsPanel({ metrics, state, totalRequests, nodesInfo }: LoadTestResultsPanelProps) {
  const { t } = useTranslation();
  const progressPercent = totalRequests > 0 ? (metrics.totalSent / totalRequests) * 100 : 0;

  const latencyChartData = (metrics.latencyHistory ?? []).slice(-100).map((p) => ({
    idx: p.index,
    latency: p.latency,
  }));

  const rpsChartData = (metrics.rpsHistory ?? []).map((p) => ({
    time: Math.round((p.timestamp - metrics.startTime) / 1000),
    rps: p.rps,
  }));
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
      <div className="space-y-1.5">
        <div className="flex items-center gap-2">
          <Progress value={progressPercent} className="h-2 flex-1" />
          <span className="text-[10px] font-medium text-muted-foreground whitespace-nowrap">
            {metrics.totalSent}/{totalRequests}
          </span>
        </div>
      </div>

      {/* Metric cards */}
      <div className="grid grid-cols-3 gap-2">
        <MetricCard icon={Zap} label={t("loadTestResults.sent")} value={metrics.totalSent} />
        <MetricCard icon={CheckCircle2} label={t("loadTestResults.success")} value={metrics.totalSuccess} color="text-success" />
        <MetricCard icon={AlertCircle} label={t("loadTestResults.error")} value={metrics.totalError} color="text-destructive" />
      </div>
      <div className={`grid gap-2 ${metrics.avgLatency > 0 ? 'grid-cols-2 sm:grid-cols-4' : 'grid-cols-1'}`}>
        <MetricCard icon={TrendingUp} label="RPS" value={metrics.rps} color="text-primary" />
        {metrics.avgLatency > 0 && (
          <>
            <MetricCard icon={Clock} label={t("loadTestResults.avg")} value={`${metrics.avgLatency}ms`} />
            <MetricCard icon={Activity} label="P95" value={`${metrics.p95}ms`} />
            <MetricCard icon={Activity} label="P99" value={`${metrics.p99}ms`} />
          </>
        )}
      </div>

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
          <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{t("loadTestResults.rpsOverTime")}</p>
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
                formatter={(v: number) => [v, "RPS"]}
              />
              <Area type="monotone" dataKey="rps" stroke="hsl(var(--status-success))" fill="hsl(var(--status-success) / 0.15)" strokeWidth={1.5} />
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

      {/* Elapsed time */}
      <div className="text-center">
        <span className="text-sm text-muted-foreground">
          {t("loadTestResults.elapsed", { seconds: Math.round(metrics.elapsedMs / 1000) })}
        </span>
      </div>
    </div>
  );
}
