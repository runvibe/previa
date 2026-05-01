import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Zap, AlertTriangle, HelpCircle, Pencil } from "lucide-react";
import type { LoadTestConfig } from "@/types/load-test";
import type { Pipeline } from "@/types/pipeline";
import type { ProjectEnvGroup } from "@/types/project";

interface LoadTestConfigPanelProps {
  pipeline: Pipeline;
  onStart: (config: LoadTestConfig, selectedBaseUrlKey?: string) => void;
  onConfigChange?: (config: LoadTestConfig, selectedBaseUrlKey?: string) => void;
  lastAvgLatencyMs?: number;
  initialConfig?: LoadTestConfig | null;
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
  const [totalRequests, setTotalRequests] = useState(initialConfig?.totalRequests ?? 50);
  const [concurrency, setConcurrency] = useState(initialConfig?.concurrency ?? 10);
  const [rampUpSeconds, setRampUpSeconds] = useState(initialConfig?.rampUpSeconds ?? 5);
  const [selectedEnv, setSelectedEnv] = useState<string | undefined>(undefined);

  const hasMultipleEnvs = false;
  const selectedEnvGroup = envGroups.find((group) => group.slug === selectedEnvGroupSlug);
  const avgLatencySec = (lastAvgLatencyMs ?? 300) / 1000;
  const batches = Math.ceil(totalRequests / concurrency);
  const estimatedTime = (batches * avgLatencySec) + rampUpSeconds;

  useEffect(() => {
    onConfigChange?.({ totalRequests, concurrency, rampUpSeconds }, selectedEnv);
  }, [totalRequests, concurrency, rampUpSeconds, selectedEnv, onConfigChange]);

  return (
    <div className="space-y-6 p-1">
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1.5">
            <Label className="text-xs font-medium">{t("loadTest.totalRequests")}</Label>
            <HelpPopover text={t("loadTest.totalRequests.help")} />
          </div>
          <span className="text-xs font-bold text-primary">{totalRequests.toLocaleString()}</span>
        </div>
        <SliderWithManual value={totalRequests} onChange={setTotalRequests} min={1} max={100000} step={100} />
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1.5">
            <Label className="text-xs font-medium">{t("loadTest.concurrency")}</Label>
            <HelpPopover text={t("loadTest.concurrency.help")} />
          </div>
          <span className="text-xs font-bold text-primary">{concurrency.toLocaleString()}</span>
        </div>
        <SliderWithManual value={concurrency} onChange={setConcurrency} min={1} max={100} step={1} />
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1.5">
            <Label className="text-xs font-medium">{t("loadTest.rampUp")}</Label>
            <HelpPopover text={t("loadTest.rampUp.help")} />
          </div>
          <span className="text-xs font-bold text-primary">{rampUpSeconds}s</span>
        </div>
        <SliderWithManual value={rampUpSeconds} onChange={setRampUpSeconds} min={0} max={60} step={1} suffix="s" />
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
          ~{estimatedTime < 60 ? `${Math.ceil(estimatedTime)}s` : `${Math.ceil(estimatedTime / 60)}min`}
        </p>
      </div>
    </div>
  );
}
