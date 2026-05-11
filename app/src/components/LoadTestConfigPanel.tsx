import { useState, useEffect, useRef } from "react";
import type { MouseEvent, PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { HelpCircle, Pencil, Trash2 } from "lucide-react";
import type { LoadInterpolation, LoadPoint, LoadRunConfig, WaveLoadConfig } from "@/types/load-test";
import { isWaveLoadConfig } from "@/types/load-test";
import type { Pipeline } from "@/types/pipeline";
import type { ProjectEnvGroup } from "@/types/project";
import { formatPlannedRequests } from "@/lib/load-rps-chart";

const DEFAULT_RUNNER_MAX_RPS = 600;
const MIN_RUNNER_MAX_RPS = 1;
const MAX_RUNNER_MAX_RPS = 1000;
const DURATION_PRESETS = [
  { key: "1m", label: "1m", value: 60_000 },
  { key: "10m", label: "10m", value: 600_000 },
  { key: "30m", label: "30m", value: 1_800_000 },
] as const;

interface LoadTestConfigPanelProps {
  pipeline: Pipeline;
  onStart: (config: LoadRunConfig, selectedBaseUrlKey?: string) => void;
  onConfigChange?: (config: LoadRunConfig, selectedBaseUrlKey?: string) => void;
  lastAvgLatencyMs?: number;
  initialConfig?: LoadRunConfig | null;
  envGroups?: ProjectEnvGroup[];
  selectedEnvGroupSlug?: string | null;
  runnerCount?: number;
}

function HelpPopover({ text }: { text: string }) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" className="text-muted-foreground hover:text-foreground transition-colors">
          <HelpCircle className="h-3.5 w-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent side="top" className="max-w-[240px] text-xs leading-relaxed p-3">
        {text}
      </PopoverContent>
    </Popover>
  );
}

function SliderWithManual({
  value, onChange, min, max, step, suffix, ariaLabel, manualButtonLabel,
}: {
  value: number; onChange: (v: number) => void;
  min: number; max: number; step: number; suffix?: string;
  ariaLabel?: string; manualButtonLabel?: string;
}) {
  const { t } = useTranslation();
  const [manualMode, setManualMode] = useState(false);
  const [manualValue, setManualValue] = useState(String(value));

  const commitManual = () => {
    const parsed = parseInt(manualValue, 10);
    if (!isNaN(parsed)) {
      onChange(clamp(parsed, min, max));
    }
    setManualMode(false);
  };

  if (manualMode) {
    return (
      <div className="flex items-center gap-2">
        <Input
          type="number"
          value={manualValue}
          onChange={(e) => setManualValue(e.target.value)}
          onBlur={commitManual}
          onKeyDown={(e) => { if (e.key === "Enter") commitManual(); if (e.key === "Escape") setManualMode(false); }}
          className="h-7 text-xs font-mono flex-1"
          aria-label={ariaLabel}
          autoFocus
          min={min}
          max={max}
          step={step}
        />
        {suffix && <span className="text-[10px] text-muted-foreground">{suffix}</span>}
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <Slider
        value={[clamp(value, min, max)]}
        onValueChange={([v]) => onChange(v)}
        min={min}
        max={max}
        step={step}
        aria-label={ariaLabel}
        className="flex-1"
      />
      <button
        type="button"
        onClick={() => { setManualValue(String(value)); setManualMode(true); }}
        className="text-muted-foreground hover:text-foreground transition-colors shrink-0"
        aria-label={manualButtonLabel ?? t("loadTest.configureManually")}
        title={t("loadTest.configureManually")}
      >
        <Pencil className="h-3 w-3" />
      </button>
    </div>
  );
}

export function LoadTestConfigPanel({ pipeline, onStart, onConfigChange, lastAvgLatencyMs, initialConfig, envGroups = [], selectedEnvGroupSlug, runnerCount = 1 }: LoadTestConfigPanelProps) {
  const { t } = useTranslation();
  const initialWave = isWaveLoadConfig(initialConfig) ? initialConfig : defaultWaveConfig();
  const [points, setPoints] = useState<LoadPoint[]>(initialWave.points);
  const [durationMs, setDurationMs] = useState(Math.max(initialWave.points.at(-1)?.atMs ?? 120_000, 100));
  const [durationMode, setDurationMode] = useState<"preset" | "custom">(() =>
    findDurationPreset(Math.max(initialWave.points.at(-1)?.atMs ?? 120_000, 100)) ? "preset" : "custom",
  );
  const [selectedPointIndex, setSelectedPointIndex] = useState(0);
  const [interpolation, setInterpolation] = useState<LoadInterpolation>(initialWave.interpolation);
  const [runnerMaxRps, setRunnerMaxRps] = useState(
    clamp(
      typeof initialWave.runnerMaxRps === "number" ? initialWave.runnerMaxRps : DEFAULT_RUNNER_MAX_RPS,
      MIN_RUNNER_MAX_RPS,
      MAX_RUNNER_MAX_RPS,
    ),
  );
  const [gracePeriodMs, setGracePeriodMs] = useState(initialWave.gracePeriodMs ?? 30_000);
  const [selectedEnv, setSelectedEnv] = useState<string | undefined>(undefined);

  const selectedEnvGroup = envGroups.find((group) => group.slug === selectedEnvGroupSlug);
  const sortedPoints = normalizeWavePoints(points, durationMs);
  const selectedPoint = sortedPoints[selectedPointIndex] ?? sortedPoints[0];
  const selectedDurationPreset = findDurationPreset(durationMs);
  const isCustomDuration = durationMode === "custom" || !selectedDurationPreset;
  const waveConfig: WaveLoadConfig = {
    points: sortedPoints,
    interpolation,
    runnerMaxRps,
    gracePeriodMs,
  };

  useEffect(() => {
    onConfigChange?.(waveConfig, selectedEnv);
  }, [points, durationMs, interpolation, runnerMaxRps, gracePeriodMs, selectedEnv, onConfigChange]);

  const setPoint = (index: number, patch: Partial<LoadPoint>) => {
    setPoints((current) =>
      normalizeWavePoints(current.map((point, currentIndex) =>
        currentIndex === index ? { ...point, ...patch } : point
      ), durationMs),
    );
  };

  const setDuration = (nextDurationMs: number) => {
    const clampedDuration = Math.max(100, Math.round(nextDurationMs));
    setDurationMs(clampedDuration);
    setPoints((current) => normalizeWavePoints(current, clampedDuration));
    setSelectedPointIndex((current) => Math.min(current, points.length - 1));
  };

  return (
    <div className="space-y-6 p-1">
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1.5">
            <Label className="text-xs font-medium">{t("loadTest.wavePoints")}</Label>
            <HelpPopover text={t("loadTest.wavePoints.help")} />
          </div>
        </div>
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          {t("loadTest.wavePoints.hint")}
        </p>
        <div className="grid grid-cols-[1fr_auto] items-end gap-3">
          <div className="space-y-1">
            <Label className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              {t("loadTest.duration")}
            </Label>
            <div className="grid grid-cols-4 gap-1 rounded-md bg-muted/40 p-1">
              {DURATION_PRESETS.map((preset) => {
                const active = !isCustomDuration && selectedDurationPreset?.key === preset.key;
                return (
                  <button
                    key={preset.key}
                    type="button"
                    className={`h-7 rounded text-xs font-medium transition-colors ${
                      active ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:bg-background/60 hover:text-foreground"
                    }`}
                    onClick={() => {
                      setDurationMode("preset");
                      setDuration(preset.value);
                    }}
                  >
                    {preset.label}
                  </button>
                );
              })}
              <button
                type="button"
                className={`h-7 rounded text-xs font-medium transition-colors ${
                  isCustomDuration ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:bg-background/60 hover:text-foreground"
                }`}
                onClick={() => setDurationMode("custom")}
              >
                {t("loadTest.durationCustom")}
              </button>
            </div>
            {isCustomDuration && (
              <Input
                id="load-wave-duration"
                type="number"
                min={100}
                value={durationMs}
                onChange={(event) => {
                  setDurationMode("custom");
                  setDuration(Number(event.target.value));
                }}
                className="h-8 text-xs"
                aria-label={t("loadTest.duration")}
              />
            )}
          </div>
          <span className="pb-2 text-[10px] text-muted-foreground">{formatDurationMs(durationMs)}</span>
        </div>

        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-1.5">
              <Label className="text-xs font-medium">{t("loadTest.interpolation")}</Label>
            </div>
          </div>
          <Select value={interpolation} onValueChange={(value) => setInterpolation(value as LoadInterpolation)}>
            <SelectTrigger className="h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="smooth">{t("loadTest.interpolationSmooth")}</SelectItem>
              <SelectItem value="linear">{t("loadTest.interpolationLinear")}</SelectItem>
              <SelectItem value="step">{t("loadTest.interpolationStep")}</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <Label className="text-xs font-medium">{t("loadTest.runnerMaxRps")}</Label>
            <span className="text-xs font-bold text-primary">{runnerMaxRps} RPS</span>
          </div>
          <SliderWithManual
            value={runnerMaxRps}
            onChange={setRunnerMaxRps}
            min={MIN_RUNNER_MAX_RPS}
            max={MAX_RUNNER_MAX_RPS}
            step={1}
            suffix="RPS"
            ariaLabel={t("loadTest.runnerMaxRps")}
            manualButtonLabel={`${t("loadTest.configureManually")} ${t("loadTest.runnerMaxRps")}`}
          />
        </div>

        <div className="rounded-md border border-border/60 p-3 text-primary" data-testid="wave-point-editor-card">
          <WaveEditor
            points={sortedPoints}
            durationMs={durationMs}
            interpolation={interpolation}
            runnerMaxRps={runnerMaxRps}
            runnerCount={runnerCount}
            selectedPointIndex={selectedPointIndex}
            onPointsChange={setPoints}
            onSelectedPointIndex={setSelectedPointIndex}
          />

          {selectedPoint && (
            <div className="mt-4 space-y-2 border-t border-border/60 pt-3">
              <div className="flex items-center justify-between">
                <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">{t("loadTest.selectedPoint")}</p>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  disabled={sortedPoints.length <= 2}
                  onClick={() => {
                    setPoints((current) => normalizeWavePoints(current.filter((_, index) => index !== selectedPointIndex), durationMs));
                    setSelectedPointIndex((current) => Math.max(0, current - 1));
                  }}
                  aria-label={t("loadTest.removePoint")}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div className="space-y-1">
                  <Label className="text-[10px] uppercase tracking-wide text-muted-foreground">{t("loadTest.pointTimeColumn")}</Label>
                  <Input
                    type="number"
                    min={0}
                    max={durationMs}
                    value={selectedPoint.atMs}
                    disabled={selectedPointIndex === 0 || selectedPointIndex === sortedPoints.length - 1}
                    onChange={(event) => setPoint(selectedPointIndex, { atMs: Math.max(0, Math.min(durationMs, Number(event.target.value))) })}
                    className="h-8 text-xs"
                    aria-label={t("loadTest.pointTimeMs")}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-[10px] uppercase tracking-wide text-muted-foreground">{t("loadTest.pointIntensityColumn")}</Label>
                  <Input
                    type="number"
                    min={0}
                    max={100}
                    value={selectedPoint.intensity}
                    onChange={(event) => setPoint(selectedPointIndex, { intensity: Math.min(100, Math.max(0, Number(event.target.value))) })}
                    className="h-8 text-xs"
                    aria-label={t("loadTest.pointIntensity")}
                  />
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <Label className="text-xs font-medium">{t("loadTest.gracePeriod")}</Label>
          <span className="text-xs font-bold text-primary">{Math.round(gracePeriodMs / 1000)}s</span>
        </div>
        <SliderWithManual value={Math.round(gracePeriodMs / 1000)} onChange={(seconds) => setGracePeriodMs(seconds * 1000)} min={0} max={120} step={1} suffix="s" />
      </div>

      {selectedEnvGroup && (
        <div className="rounded-md border border-border/60 px-3 py-2">
          <p className="text-[10px] text-muted-foreground">Env group</p>
          <p className="truncate text-xs font-medium">{selectedEnvGroup.name}</p>
        </div>
      )}

      <div className="rounded-lg p-3 text-center">
        <p className="text-[10px] text-muted-foreground">{t("loadTest.estimatedTime")}</p>
        <p className="text-sm font-semibold">
          ~{durationMs < 60_000 ? `${Math.ceil(durationMs / 1000)}s` : `${Math.ceil(durationMs / 60_000)}min`}
        </p>
      </div>
    </div>
  );
}

function WaveEditor({
  points,
  durationMs,
  interpolation,
  runnerMaxRps,
  runnerCount,
  selectedPointIndex,
  onPointsChange,
  onSelectedPointIndex,
}: {
  points: LoadPoint[];
  durationMs: number;
  interpolation: LoadInterpolation;
  runnerMaxRps: number;
  runnerCount: number;
  selectedPointIndex: number;
  onPointsChange: (points: LoadPoint[]) => void;
  onSelectedPointIndex: (index: number) => void;
}) {
  const { t } = useTranslation();
  const [draggingIndex, setDraggingIndex] = useState<number | null>(null);
  const draggingIndexRef = useRef<number | null>(null);
  const graphRef = useRef<SVGSVGElement | null>(null);
  const plotWidth = 100;
  const plotHeight = 48;
  const graphPoints = points.map((point) => pointToGraph(point, durationMs, plotWidth, plotHeight));
  const pathData = buildWavePath(graphPoints, interpolation);
  const pointMarkers = points.map((point, index) => ({
    key: `${point.atMs}-${point.intensity}-${index}`,
    x: graphPoints[index]?.x ?? 0,
    y: graphPoints[index]?.y ?? 0,
    plannedRequests: Math.round(runnerMaxRps * Math.max(1, runnerCount) * (point.intensity / 100)),
  }));

  const pointFromClient = (clientX: number, clientY: number): LoadPoint => {
    const rect = graphRef.current?.getBoundingClientRect();
    const xRatio = rect ? clamp((clientX - rect.left) / Math.max(rect.width, 1), 0, 1) : 0;
    const yRatio = rect ? clamp((clientY - rect.top) / Math.max(rect.height, 1), 0, 1) : 0;
    return {
      atMs: snapMs(xRatio * durationMs),
      intensity: Math.round((1 - yRatio) * 100),
    };
  };

  const updatePoint = (index: number, point: LoadPoint) => {
    const nextPoint = {
      atMs: index === 0 ? 0 : index === points.length - 1 ? durationMs : clamp(point.atMs, 0, durationMs),
      intensity: clamp(point.intensity, 0, 100),
    };
    const nextPoints = normalizeWavePoints(points.map((current, currentIndex) => currentIndex === index ? nextPoint : current), durationMs);
    const nextIndex = nextPoints.findIndex((current) => current.atMs === nextPoint.atMs && current.intensity === nextPoint.intensity);
    onPointsChange(nextPoints);
    onSelectedPointIndex(Math.max(0, nextIndex));
  };

  const addPoint = (point: LoadPoint) => {
    const nextPoint = {
      atMs: clamp(point.atMs, 0, durationMs),
      intensity: clamp(point.intensity, 0, 100),
    };
    const nextPoints = normalizeWavePoints([...points, nextPoint], durationMs);
    onPointsChange(nextPoints);
    onSelectedPointIndex(nextPoints.findIndex((current) => current.atMs === nextPoint.atMs && current.intensity === nextPoint.intensity));
  };

  const handlePointerMove = (event: PointerEvent<SVGSVGElement>) => {
    const activeIndex = draggingIndexRef.current;
    if (activeIndex === null) return;
    updatePoint(activeIndex, pointFromClient(event.clientX, event.clientY));
  };

  const handleMouseMove = (event: MouseEvent<SVGSVGElement>) => {
    const activeIndex = draggingIndexRef.current;
    if (activeIndex === null) return;
    updatePoint(activeIndex, pointFromClient(event.clientX, event.clientY));
  };

  const stopDragging = () => {
    draggingIndexRef.current = null;
    setDraggingIndex(null);
  };

  return (
    <div>
      <div className="mb-2 flex items-center justify-between text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        <span>{t("loadTest.previewIntensityAxis")}</span>
        <span>{t("loadTest.previewTimeAxis")}</span>
      </div>
      <div className="grid grid-cols-[2.75rem_1fr] gap-2">
        <div className="flex flex-col justify-between py-1 text-[10px] text-muted-foreground">
          <span>100%</span>
          <span>0%</span>
        </div>
        <div className="min-w-0">
          <div className="relative h-40 w-full overflow-visible">
            <svg
              ref={graphRef}
              data-testid="wave-editor-graph"
              viewBox={`0 0 ${plotWidth} ${plotHeight}`}
              preserveAspectRatio="none"
              className="h-full w-full cursor-crosshair touch-none overflow-visible rounded bg-muted/20"
              role="img"
              aria-label={t("loadTest.wavePreview")}
              onClick={(event) => {
                if (event.detail > 1) return;
                addPoint(pointFromClient(event.clientX, event.clientY));
              }}
              onMouseMove={handleMouseMove}
              onMouseUp={stopDragging}
              onMouseLeave={stopDragging}
              onPointerMove={handlePointerMove}
              onPointerUp={stopDragging}
              onPointerLeave={stopDragging}
            >
              {[0.25, 0.5, 0.75].map((line) => (
                <line
                  key={`h-${line}`}
                  x1="0"
                  x2={plotWidth}
                  y1={plotHeight * line}
                  y2={plotHeight * line}
                  stroke="currentColor"
                  strokeOpacity="0.08"
                  strokeWidth="0.5"
                  vectorEffect="non-scaling-stroke"
                />
              ))}
              {[0.25, 0.5, 0.75].map((line) => (
                <line
                  key={`v-${line}`}
                  x1={plotWidth * line}
                  x2={plotWidth * line}
                  y1="0"
                  y2={plotHeight}
                  stroke="currentColor"
                  strokeOpacity="0.08"
                  strokeWidth="0.5"
                  vectorEffect="non-scaling-stroke"
                />
              ))}
              <path
                data-testid="wave-editor-path"
                d={pathData}
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                vectorEffect="non-scaling-stroke"
              />
            </svg>
            <div className="pointer-events-none absolute inset-0">
              {pointMarkers.map((marker, index) => {
                const left = graphPercent(marker.x, plotWidth);
                const top = graphPercent(marker.y, plotHeight);
                const markerSize = index === selectedPointIndex ? 18 : 14;
                return (
                  <button
                    key={marker.key}
                    type="button"
                    data-testid={`wave-point-${index}`}
                    aria-label={`Wave point ${index + 1}`}
                    className="pointer-events-auto absolute -translate-x-1/2 -translate-y-1/2 cursor-grab touch-none rounded-full border border-background bg-primary p-0 shadow-sm transition-[width,height]"
                    style={{
                      left: `${left}%`,
                      top: `${top}%`,
                      width: `${markerSize}px`,
                      height: `${markerSize}px`,
                    }}
                    onPointerDown={(event) => {
                      event.stopPropagation();
                      draggingIndexRef.current = index;
                      setDraggingIndex(index);
                      onSelectedPointIndex(index);
                      event.currentTarget.setPointerCapture?.(event.pointerId);
                    }}
                    onPointerMove={(event) => {
                      const activeIndex = draggingIndexRef.current;
                      if (activeIndex === null) return;
                      updatePoint(activeIndex, pointFromClient(event.clientX, event.clientY));
                    }}
                    onPointerUp={stopDragging}
                    onMouseDown={(event) => {
                      event.stopPropagation();
                      draggingIndexRef.current = index;
                      setDraggingIndex(index);
                      onSelectedPointIndex(index);
                    }}
                    onMouseMove={(event) => {
                      const activeIndex = draggingIndexRef.current;
                      if (activeIndex === null) return;
                      updatePoint(activeIndex, pointFromClient(event.clientX, event.clientY));
                    }}
                    onMouseUp={stopDragging}
                    onClick={(event) => event.stopPropagation()}
                  />
                );
              })}
            </div>
          </div>
          <div className="mt-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("loadTest.pointMaxRequests")}
          </div>
          <div data-testid="wave-point-value-strip" className="relative mt-1 h-5 text-[10px] font-mono text-muted-foreground">
            {pointMarkers.map((marker, index) => {
              const percent = clamp((marker.x / Math.max(plotWidth, 1)) * 100, 0, 100);
              const edgeClass = index === pointMarkers.length - 1
                ? "-translate-x-full"
                : index === 0
                  ? "translate-x-0"
                  : "-translate-x-1/2";
              return (
                <span
                  key={marker.key}
                  data-testid={`wave-point-marker-value-${index}`}
                  className={`absolute top-0 rounded bg-background/70 px-1 leading-4 ${edgeClass}`}
                  style={{ left: `${percent}%` }}
                >
                  {formatPlannedRequests(marker.plannedRequests)}
                </span>
              );
            })}
          </div>
        </div>
      </div>
      <div className="ml-[3.25rem] mt-1 flex justify-between text-[10px] text-muted-foreground">
        <span>0 ms</span>
        <span>{formatDurationMs(durationMs)}</span>
      </div>
    </div>
  );
}

function defaultWaveConfig(): WaveLoadConfig {
  return {
    points: [
      { atMs: 0, intensity: 10 },
      { atMs: 120_000, intensity: 80 },
    ],
    interpolation: "smooth",
    runnerMaxRps: DEFAULT_RUNNER_MAX_RPS,
    gracePeriodMs: 30_000,
  };
}

function findDurationPreset(durationMs: number) {
  return DURATION_PRESETS.find((preset) => preset.value === durationMs);
}

function normalizeWavePoints(points: LoadPoint[], durationMs: number): LoadPoint[] {
  const clamped = points.map((point) => ({
    atMs: clamp(point.atMs, 0, durationMs),
    intensity: clamp(point.intensity, 0, 100),
  }));
  const sorted = clamped.sort((a, b) => a.atMs - b.atMs);
  const first = sorted[0] ?? { atMs: 0, intensity: 10 };
  const last = sorted.at(-1) ?? { atMs: durationMs, intensity: first.intensity };
  const middle = sorted.filter((point) => point.atMs > 0 && point.atMs < durationMs);
  return [
    { ...first, atMs: 0 },
    ...middle,
    { ...last, atMs: durationMs },
  ];
}

function formatDurationMs(ms: number): string {
  if (ms >= 1000) {
    return `${ms.toLocaleString()} ms (${Math.round(ms / 1000).toLocaleString()}s)`;
  }
  return `${ms.toLocaleString()} ms`;
}

function pointToGraph(point: LoadPoint, durationMs: number, width: number, height: number) {
  return {
    x: (point.atMs / Math.max(durationMs, 1)) * width,
    y: height - (point.intensity / 100) * height,
  };
}

function graphPercent(value: number, total: number): number {
  return Math.round(clamp((value / Math.max(total, 1)) * 100, 0, 100) * 1000) / 1000;
}

function buildWavePath(points: Array<{ x: number; y: number }>, interpolation: LoadInterpolation): string {
  if (points.length === 0) return "";

  const [first, ...rest] = points;
  const start = `M ${first.x},${first.y}`;

  if (interpolation === "step") {
    return rest.reduce((path, point) => `${path} H ${point.x} V ${point.y}`, start);
  }

  if (interpolation === "smooth") {
    return rest.reduce((path, point, index) => {
      const previous = points[index];
      const controlOffset = (point.x - previous.x) / 2;
      const controlA = `${previous.x + controlOffset},${previous.y}`;
      const controlB = `${point.x - controlOffset},${point.y}`;
      return `${path} C ${controlA} ${controlB} ${point.x},${point.y}`;
    }, start);
  }

  return rest.reduce((path, point) => `${path} L ${point.x},${point.y}`, start);
}

function snapMs(ms: number): number {
  return Math.round(ms / 100) * 100;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Number.isFinite(value) ? value : min));
}
