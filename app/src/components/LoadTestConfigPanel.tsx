import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { HelpCircle, Pencil, Plus, Trash2 } from "lucide-react";
import type { LoadInterpolation, LoadPoint, LoadRunConfig, WaveLoadConfig } from "@/types/load-test";
import { isWaveLoadConfig } from "@/types/load-test";
import type { Pipeline } from "@/types/pipeline";
import type { ProjectEnvGroup } from "@/types/project";

interface LoadTestConfigPanelProps {
  pipeline: Pipeline;
  onStart: (config: LoadRunConfig, selectedBaseUrlKey?: string) => void;
  onConfigChange?: (config: LoadRunConfig, selectedBaseUrlKey?: string) => void;
  lastAvgLatencyMs?: number;
  initialConfig?: LoadRunConfig | null;
  envGroups?: ProjectEnvGroup[];
  selectedEnvGroupSlug?: string | null;
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
  value, onChange, min, max, step, suffix,
}: {
  value: number; onChange: (v: number) => void;
  min: number; max: number; step: number; suffix?: string;
}) {
  const { t } = useTranslation();
  const [manualMode, setManualMode] = useState(false);
  const [manualValue, setManualValue] = useState(String(value));

  const commitManual = () => {
    const parsed = parseInt(manualValue, 10);
    if (!isNaN(parsed) && parsed >= 0) {
      onChange(parsed);
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
          autoFocus
          min={0}
        />
        {suffix && <span className="text-[10px] text-muted-foreground">{suffix}</span>}
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <Slider
        value={[Math.min(value, max)]}
        onValueChange={([v]) => onChange(v)}
        min={min}
        max={max}
        step={step}
        className="flex-1"
      />
      <button
        type="button"
        onClick={() => { setManualValue(String(value)); setManualMode(true); }}
        className="text-muted-foreground hover:text-foreground transition-colors shrink-0"
        title={t("loadTest.configureManually")}
      >
        <Pencil className="h-3 w-3" />
      </button>
    </div>
  );
}

export function LoadTestConfigPanel({ pipeline, onStart, onConfigChange, lastAvgLatencyMs, initialConfig, envGroups = [], selectedEnvGroupSlug }: LoadTestConfigPanelProps) {
  const { t } = useTranslation();
  const initialWave = isWaveLoadConfig(initialConfig) ? initialConfig : defaultWaveConfig();
  const [points, setPoints] = useState<LoadPoint[]>(initialWave.points);
  const [interpolation, setInterpolation] = useState<LoadInterpolation>(initialWave.interpolation);
  const [maxInFlight, setMaxInFlight] = useState(initialWave.maxInFlight ?? 200);
  const [gracePeriodMs, setGracePeriodMs] = useState(initialWave.gracePeriodMs ?? 30_000);
  const [selectedEnv, setSelectedEnv] = useState<string | undefined>(undefined);

  const selectedEnvGroup = envGroups.find((group) => group.slug === selectedEnvGroupSlug);
  const sortedPoints = normalizeWavePoints(points);
  const durationMs = sortedPoints.at(-1)?.atMs ?? 0;
  const maxMs = Math.max(durationMs, 1);
  const waveConfig: WaveLoadConfig = {
    points: sortedPoints,
    interpolation,
    maxInFlight,
    gracePeriodMs,
  };

  useEffect(() => {
    onConfigChange?.(waveConfig, selectedEnv);
  }, [points, interpolation, maxInFlight, gracePeriodMs, selectedEnv, onConfigChange]);

  const setPoint = (index: number, patch: Partial<LoadPoint>) => {
    setPoints((current) =>
      normalizeWavePoints(current.map((point, currentIndex) =>
        currentIndex === index ? { ...point, ...patch } : point
      )),
    );
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
        <div className="space-y-2">
          {sortedPoints.map((point, index) => (
            <div key={`${point.atMs}-${index}`} className="grid grid-cols-[1fr_1fr_auto] gap-2">
              <Input
                type="number"
                min={0}
                value={point.atMs}
                onChange={(event) => setPoint(index, { atMs: Math.max(0, Number(event.target.value)) })}
                className="h-8 text-xs"
                aria-label={t("loadTest.pointTimeMs")}
              />
              <Input
                type="number"
                min={0}
                max={100}
                value={point.intensity}
                onChange={(event) => setPoint(index, { intensity: Math.min(100, Math.max(0, Number(event.target.value))) })}
                className="h-8 text-xs"
                aria-label={t("loadTest.pointIntensity")}
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                disabled={sortedPoints.length <= 2}
                onClick={() => setPoints((current) => current.filter((_, currentIndex) => currentIndex !== index))}
                aria-label={t("loadTest.removePoint")}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          ))}
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="w-full"
          onClick={() => {
            const last = sortedPoints.at(-1) ?? { atMs: 0, intensity: 30 };
            setPoints([...sortedPoints, { atMs: last.atMs + 60_000, intensity: last.intensity }]);
          }}
        >
          <Plus className="mr-2 h-4 w-4" />
          {t("loadTest.addPoint")}
        </Button>
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
          <div className="flex items-center gap-1.5">
            <Label className="text-xs font-medium">{t("loadTest.maxInFlight")}</Label>
            <HelpPopover text={t("loadTest.maxInFlight.help")} />
          </div>
          <span className="text-xs font-bold text-primary">{maxInFlight.toLocaleString()}</span>
        </div>
        <SliderWithManual value={maxInFlight} onChange={setMaxInFlight} min={1} max={5000} step={10} />
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <Label className="text-xs font-medium">{t("loadTest.gracePeriod")}</Label>
          <span className="text-xs font-bold text-primary">{Math.round(gracePeriodMs / 1000)}s</span>
        </div>
        <SliderWithManual value={Math.round(gracePeriodMs / 1000)} onChange={(seconds) => setGracePeriodMs(seconds * 1000)} min={0} max={120} step={1} suffix="s" />
      </div>

      <div className="rounded-md border border-border/60 p-3 text-primary">
        <svg viewBox="0 0 100 40" className="h-24 w-full" role="img" aria-label={t("loadTest.wavePreview")}>
          <polyline
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            points={sortedPoints
              .map((point) => {
                const x = (point.atMs / maxMs) * 100;
                const y = 40 - (point.intensity / 100) * 40;
                return `${x},${y}`;
              })
              .join(" ")}
          />
        </svg>
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

function defaultWaveConfig(): WaveLoadConfig {
  return {
    points: [
      { atMs: 0, intensity: 10 },
      { atMs: 120_000, intensity: 80 },
    ],
    interpolation: "smooth",
    maxInFlight: 200,
    gracePeriodMs: 30_000,
  };
}

function normalizeWavePoints(points: LoadPoint[]): LoadPoint[] {
  return [...points].sort((a, b) => a.atMs - b.atMs);
}
