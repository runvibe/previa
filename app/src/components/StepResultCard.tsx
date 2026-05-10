import { lazy, Suspense, useState, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Badge } from "@/components/ui/badge";
import { MethodBadge } from "@/components/MethodBadge";
import { STATUS_BORDER } from "@/lib/constants";
import { ChevronDown, Clock, CheckCircle2, XCircle, Circle as CircleIcon, ShieldCheck, ShieldX, RotateCcw, Timer, Sparkles, Copy, Eye, GripVertical, Play } from "lucide-react";
import { DotsLoader } from "@/components/DotsLoader";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger, TooltipProvider } from "@/components/ui/tooltip";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from "@/components/ui/resizable";
import type { PipelineStep, StepExecutionResult } from "@/types/pipeline";
import { toast } from "sonner";

const MonacoCodeEditor = lazy(() => import("@/components/MonacoCodeEditor"));

function calcHeight(value: string) {
  const lines = value.split("\n").length;
  return `${Math.max(lines * 19, 300)}px`;
}

function ReadOnlyMonaco({ value, fillHeight = false }: { value: string; fillHeight?: boolean }) {
  const [isDark, setIsDark] = useState(document.documentElement.classList.contains("dark"));
  useEffect(() => {
    const obs = new MutationObserver(() => setIsDark(document.documentElement.classList.contains("dark")));
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => obs.disconnect();
  }, []);
  const h = fillHeight ? "100%" : calcHeight(value);
  return (
    <Suspense fallback={<pre className={`overflow-auto bg-muted p-2 text-xs ${fillHeight ? "h-full" : ""}`}>{value}</pre>}>
      <div className={`overflow-hidden ${fillHeight ? "h-full" : ""}`}>
        <MonacoCodeEditor
          value={value}
          readOnly
          showHeader={false}
          showValidation={false}
          showLineNumbers
          isDark={isDark}
          height={h}
          className={fillHeight ? "h-full" : ""}
        />
      </div>
    </Suspense>
  );
}

const STATUS_ICON: Record<string, React.ReactNode> = {
  pending: <CircleIcon className="h-4 w-4 text-muted-foreground" />,
  running: <DotsLoader className="text-primary" />,
  success: <CheckCircle2 className="h-4 w-4 text-success" />,
  error: <XCircle className="h-4 w-4 text-destructive" />,
};

interface StepResultCardProps {
  step: PipelineStep;
  result?: StepExecutionResult;
  shouldCountdown?: boolean;
  onAnalyzeWithAI?: (step: PipelineStep, result: StepExecutionResult) => void;
  onGoToCode?: (stepId: string) => void;
  onRerunFromStep?: (stepId: string) => void;
  canRerunFromStep?: boolean;
  variant?: "list" | "grid";
}

function formatDisplayValue(value: unknown): string {
  if (value === null) return "null";
  if (value === undefined) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function buildCurl(method: string, request: StepExecutionResult["request"]): string {
  if (!request) return "";
  const parts: string[] = [`curl -X ${method.toUpperCase()}`];
  if (request.headers && typeof request.headers === "object") {
    for (const [key, val] of Object.entries(request.headers)) {
      parts.push(`  -H '${key}: ${String(val).replace(/'/g, "'\\''")}'`);
    }
  }
  if (request.body) {
    const bodyStr = typeof request.body === "string" ? request.body : JSON.stringify(request.body);
    parts.push(`  -d '${bodyStr.replace(/'/g, "'\\''")}'`);
  }
  parts.push(`  '${request.url}'`);
  return parts.join(" \\\n");
}

function DelayCountdown({ delayMs }: { delayMs: number }) {
  const [remaining, setRemaining] = useState(delayMs);

  useEffect(() => {
    setRemaining(delayMs);
    const start = Date.now();
    const interval = setInterval(() => {
      const elapsed = Date.now() - start;
      const left = Math.max(0, delayMs - elapsed);
      setRemaining(left);
      if (left <= 0) clearInterval(interval);
    }, 50);
    return () => clearInterval(interval);
  }, [delayMs]);

  const seconds = (remaining / 1000).toFixed(1);

  return (
    <span className="flex items-center gap-0.5 text-[10px] text-primary font-medium tabular-nums">
      <Timer className="h-3 w-3 animate-pulse" /> {seconds}s
    </span>
  );
}

function ElapsedTimer({ startedAt }: { startedAt?: number }) {
  const [elapsed, setElapsed] = useState(0);
  const origin = startedAt ?? Date.now();

  useEffect(() => {
    setElapsed(Date.now() - origin);
    const interval = setInterval(() => {
      setElapsed(Date.now() - origin);
    }, 50);
    return () => clearInterval(interval);
  }, [origin]);

  return (
    <span className="flex items-center gap-1 text-xs text-muted-foreground tabular-nums">
      <Clock className="h-3 w-3 animate-pulse" /> {elapsed}ms
    </span>
  );
}

/* ── Shared helpers ── */

function getStatusBgStyle(status: string, isDelaying?: boolean): React.CSSProperties {
  if (isDelaying) return { background: "hsl(var(--warning) / 0.12)" };
  if (status === "success") return { background: "hsl(142 71% 45% / 0.12)" };
  if (status === "error") return { background: "hsl(0 84% 60% / 0.12)" };
  if (status === "running") return { background: "hsl(var(--primary) / 0.12)" };
  return {};
}

function AssertSummaryBadge({ assertResults }: { assertResults: any[] }) {
  if (assertResults.length === 0) return null;
  const passed = assertResults.filter((r: any) => r?.passed === true).length;
  const total = assertResults.length;
  const allPassed = passed === total;
  return (
    <Badge variant={allPassed ? "secondary" : "destructive"} className="text-[10px] px-1.5 py-0">
      {allPassed ? <ShieldCheck className="h-3 w-3 mr-0.5" /> : <ShieldX className="h-3 w-3 mr-0.5" />}
      {passed}/{total}
    </Badge>
  );
}

function TimingInfo({ step, result, status, shouldCountdown, currentAttempt, totalAttempts, t }: {
  step: PipelineStep; result?: StepExecutionResult; status: string;
  shouldCountdown?: boolean; currentAttempt?: number; totalAttempts: number; t: any;
}) {
  return (
    <>
      {(currentAttempt != null && (currentAttempt > 1 || status === "running")) && (
        <Badge
          variant={status === "running" ? "default" : "outline"}
          className={`text-[10px] px-1.5 py-0 gap-0.5 ${status === "running" ? "animate-pulse" : ""}`}
        >
          <RotateCcw className="h-3 w-3" />
          {t("stepResult.attempt", { current: currentAttempt, total: totalAttempts })}
        </Badge>
      )}
      {result?.duration !== undefined ? (
        <span className="flex items-center gap-1 text-xs text-muted-foreground tabular-nums">
          <Clock className="h-3 w-3" /> {result.duration}ms
        </span>
      ) : status === "running" ? (
        <ElapsedTimer startedAt={result?.startedAt} />
      ) : step.delay && step.delay > 0 && (status === "pending" && shouldCountdown) ? (
        <DelayCountdown delayMs={step.delay} />
      ) : step.delay && step.delay > 0 ? (
        <span className="flex items-center gap-0.5 text-[10px] text-muted-foreground">
          <Timer className="h-3 w-3" /> {t("stepResult.delay", { ms: step.delay })}
        </span>
      ) : null}
    </>
  );
}

function CurlButton({ step, request }: { step: PipelineStep; request: StepExecutionResult["request"] }) {
  if (!request) return null;
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-2 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
            onClick={(e) => {
              e.stopPropagation();
              const curl = buildCurl(step.method, request);
              navigator.clipboard.writeText(curl);
              toast.success("cURL copied!");
            }}
          >
            <Copy className="h-3.5 w-3.5" />
            cURL
          </Button>
        </TooltipTrigger>
        <TooltipContent side="top" className="text-xs">Copy as cURL</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

function RerunFromStepButton({
  stepId,
  disabled,
  onRerunFromStep,
}: {
  stepId: string;
  disabled?: boolean;
  onRerunFromStep?: (stepId: string) => void;
}) {
  if (!onRerunFromStep) return null;
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-[16px] w-[16px] shrink-0 bg-transparent p-0 text-white shadow-none hover:bg-transparent hover:text-white"
            disabled={disabled}
            aria-label="Rerun from here"
            onClick={(e) => {
              e.stopPropagation();
              e.preventDefault();
              onRerunFromStep(stepId);
            }}
          >
            <Play className="h-3 w-3 fill-current" />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="top" className="text-xs">Rerun from here</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

function DetailContent({ step, result, assertResults, t }: {
  step: PipelineStep; result?: StepExecutionResult; assertResults: any[]; t: any;
}) {
  const hasAsserts = assertResults.length > 0;
  const displayUrl = result?.request?.url || step.url;
  const containerRef = useRef<HTMLDivElement>(null);
  const [isNarrow, setIsNarrow] = useState(false);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const obs = new ResizeObserver(([entry]) => {
      setIsNarrow(entry.contentRect.width < 800);
    });
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  return (
    <div ref={containerRef} className={`animate-fade-in h-full flex ${isNarrow ? "flex-col" : "flex-row"} min-h-[800px]`}>
      {/* ── Left: Request + Response ── */}
      <div className="flex-1 flex flex-col min-h-0 min-w-0">
        <div className="flex items-center gap-2 px-4 py-2 shrink-0">
          <p className="text-xs font-semibold">{t("stepResult.requestUrl")}</p>
          {result?.request && <CurlButton step={step} request={result.request} />}
        </div>
        <p className="font-mono text-xs text-muted-foreground break-all px-4 pb-2 shrink-0">{displayUrl}</p>

        {result?.request ? (
          <ResizablePanelGroup direction="vertical" className="flex-1 min-h-0 overflow-hidden mb-3">
            <ResizablePanel defaultSize={33} minSize={10}>
              <div className="h-full flex flex-col">
                <div className="px-2 py-1.5 bg-muted border-b text-xs font-semibold">
                  {t("stepResult.requestHeaders")}
                </div>
                <div className="flex-1 min-h-0 overflow-hidden">
                  <ReadOnlyMonaco value={JSON.stringify(result.request.headers, null, 2)} fillHeight />
                </div>
              </div>
            </ResizablePanel>
            <ResizableHandle withHandle className="bg-white/20 hover:bg-white/40 dark:bg-white/10 dark:hover:bg-white/20" />

            {result.request.body && (
              <>
                <ResizablePanel defaultSize={33} minSize={10}>
                  <div className="h-full flex flex-col">
                    <div className="px-2 py-1.5 bg-muted border-y text-xs font-semibold">
                      {t("stepResult.requestBody")}
                    </div>
                    <div className="flex-1 min-h-0 overflow-hidden">
                      <ReadOnlyMonaco value={typeof result.request.body === "string" ? result.request.body : JSON.stringify(result.request.body, null, 2)} fillHeight />
                    </div>
                  </div>
                </ResizablePanel>
                <ResizableHandle withHandle className="bg-white/20 hover:bg-white/40 dark:bg-white/10 dark:hover:bg-white/20" />
              </>
            )}

            {result.response && (
              <ResizablePanel defaultSize={result.request.body ? 33 : 67} minSize={10}>
                <div className="h-full flex flex-col">
                  <div className="px-2 py-1.5 bg-muted border-y text-xs font-semibold">
                    {t("stepResult.response", { status: result.response.status, statusText: result.response.statusText })}
                  </div>
                  <div className="flex-1 min-h-0 overflow-hidden">
                    <ReadOnlyMonaco value={JSON.stringify(result.response.body, null, 2)} fillHeight />
                  </div>
                </div>
              </ResizablePanel>
            )}
          </ResizablePanelGroup>
        ) : (
          <div className="flex-1 flex items-center justify-center text-xs text-muted-foreground">
            {t("stepResult.notExecutedYet", "Step not executed yet")}
          </div>
        )}
      </div>

      {/* ── Right: Assertions ── */}
      {hasAsserts && (
        <>
          <div className={`${isNarrow ? "w-full" : "w-72"} shrink-0 flex flex-col min-h-0 overflow-auto p-4`}>
            <p className="text-xs font-semibold mb-2">{t("stepResult.assertions")}</p>
            <div className="space-y-1.5">
              {assertResults.map((ar: any, ai) => (
                <div key={ai} className={`flex items-start gap-2 rounded-lg px-2.5 py-2 text-xs ${ar?.passed ? "bg-success/10" : "bg-destructive/10"}`}>
                  {ar?.passed ? <CheckCircle2 className="h-3.5 w-3.5 text-success shrink-0 mt-0.5" /> : <XCircle className="h-3.5 w-3.5 text-destructive shrink-0 mt-0.5" />}
                  <div className="flex flex-col gap-0.5 min-w-0">
                    <span className="font-mono break-all">{ar?.assertion?.field ?? "unknown"}</span>
                    <span className="text-muted-foreground">{ar?.assertion?.operator ?? "equals"}</span>
                    {ar?.assertion?.expected !== undefined && (
                      <span className="font-medium break-all">{formatDisplayValue(ar.assertion.expected)}</span>
                    )}
                    {!ar?.passed && ar?.actual !== undefined && (
                      <span className="text-muted-foreground">{t("stepResult.actual")} <span className="text-foreground break-all">{formatDisplayValue(ar.actual)}</span></span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

/* ── Grid variant ── */
function GridCard({ step, result, shouldCountdown, onAnalyzeWithAI, onGoToCode, onRerunFromStep, canRerunFromStep, status, assertResults, currentAttempt, totalAttempts, t }: {
  step: PipelineStep; result?: StepExecutionResult; shouldCountdown?: boolean;
  onAnalyzeWithAI?: (step: PipelineStep, result: StepExecutionResult) => void;
  onGoToCode?: (stepId: string) => void;
  onRerunFromStep?: (stepId: string) => void;
  canRerunFromStep?: boolean;
  status: string; assertResults: any[]; currentAttempt?: number; totalAttempts: number; t: any;
}) {
  const [detailOpen, setDetailOpen] = useState(false);

  return (
    <>
      <Card
        className={`h-full flex flex-col border-l-4 ${STATUS_BORDER[status]} hover:shadow-md transition-all duration-200`}
        style={getStatusBgStyle(status, status === "pending" && shouldCountdown && (step.delay ?? 0) > 0)}
      >
        <CardHeader className="p-2 pb-1 space-y-0.5">
          <div className="flex items-center gap-1.5">
            {STATUS_ICON[status]}
            <MethodBadge method={step.method} />
            <CardTitle className="text-xs truncate flex-1">{step.name}</CardTitle>
            {onGoToCode && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button variant="ghost" size="sm" className="h-5 px-2 shrink-0 text-[10px]" onClick={() => onGoToCode(step.id)}>
                      Code
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent side="top">Go to code</TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
            <RerunFromStepButton
              stepId={step.id}
              disabled={canRerunFromStep === false}
              onRerunFromStep={onRerunFromStep}
            />
            {result && result.status !== "pending" && result.status !== "running" && onAnalyzeWithAI && (
              <TooltipProvider delayDuration={300}>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      size="icon" variant="ghost"
                      className="h-5 w-5 rounded-md text-muted-foreground hover:text-primary hover:bg-primary/10 transition-colors"
                      onClick={() => onAnalyzeWithAI(step, result)}
                    >
                      <Sparkles className="h-3 w-3" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent side="top" className="text-xs">{t("stepResult.analyzeWithAI", "Analyze with AI")}</TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
        </CardHeader>

        <CardContent className="px-3 pb-2 pt-0 flex-1 flex flex-col gap-1 min-h-0">
          {step.description && (
            <p className="text-[11px] text-muted-foreground line-clamp-1">{step.description}</p>
          )}
          {result?.error && (
            <p className="text-[11px] text-destructive line-clamp-2">{result.error}</p>
          )}
          <span className="flex-1" />
          <div className="flex items-center gap-1.5 flex-wrap border-t border-border/50 pt-1.5 mt-auto">
            <AssertSummaryBadge assertResults={assertResults} />
            <TimingInfo step={step} result={result} status={status} shouldCountdown={shouldCountdown} currentAttempt={currentAttempt} totalAttempts={totalAttempts} t={t} />
            <span className="flex-1" />
            <CurlButton step={step} request={result?.request} />
            {result?.request && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-5 px-1.5 gap-1 text-[10px]"
                      onClick={() => setDetailOpen(true)}
                    >
                      <Eye className="h-3 w-3" />
                      Details
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent side="top" className="text-xs">View full request/response</TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
        </CardContent>
      </Card>

      <Dialog open={detailOpen} onOpenChange={setDetailOpen}>
        <DialogContent className="max-w-[920px] h-[80vh] flex flex-col overflow-hidden">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              {STATUS_ICON[status]}
              <MethodBadge method={step.method} />
              <span>{step.name}</span>
            </DialogTitle>
            {step.description && (
              <p className="text-sm text-muted-foreground">{step.description}</p>
            )}
          </DialogHeader>
          <div className="flex-1 min-h-0 overflow-hidden">
            <DetailContent step={step} result={result} assertResults={assertResults} t={t} />
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

/* ── List variant (original collapsible) ── */
function ListCard({ step, result, shouldCountdown, onAnalyzeWithAI, onGoToCode, onRerunFromStep, canRerunFromStep, status, assertResults, currentAttempt, totalAttempts, t }: {
  step: PipelineStep; result?: StepExecutionResult; shouldCountdown?: boolean;
  onAnalyzeWithAI?: (step: PipelineStep, result: StepExecutionResult) => void;
  onGoToCode?: (stepId: string) => void;
  onRerunFromStep?: (stepId: string) => void;
  canRerunFromStep?: boolean;
  status: string; assertResults: any[]; currentAttempt?: number; totalAttempts: number; t: any;
}) {
  return (
    <Collapsible className="h-full">
      <Card className={`relative h-full flex flex-col border-l-4 ${STATUS_BORDER[status]} hover:shadow-md transition-all duration-200`} style={getStatusBgStyle(status, status === "pending" && shouldCountdown && (step.delay ?? 0) > 0)}>
        <CollapsibleTrigger asChild>
          <div className="w-full cursor-pointer text-left">
            <CardHeader className="p-3 pb-2 space-y-1">
              <div className="flex items-center gap-2">
                {STATUS_ICON[status]}
                <MethodBadge method={step.method} />
                <span className="flex-1" />
                {onGoToCode && (
                  <TooltipProvider>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button variant="ghost" size="sm" className="h-6 px-2 shrink-0 text-[10px]" onClick={(e) => { e.stopPropagation(); e.preventDefault(); onGoToCode(step.id); }}>
                          Code
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent side="top">Go to code</TooltipContent>
                    </Tooltip>
                  </TooltipProvider>
                )}
                {result && result.status !== "pending" && result.status !== "running" && onAnalyzeWithAI && (
                  <TooltipProvider delayDuration={300}>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          size="icon" variant="ghost"
                          className="h-6 w-6 rounded-md text-muted-foreground hover:text-primary hover:bg-primary/10 transition-colors"
                          onClick={(e) => { e.stopPropagation(); onAnalyzeWithAI(step, result); }}
                        >
                          <Sparkles className="h-3.5 w-3.5" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent side="top" className="text-xs">{t("stepResult.analyzeWithAI", "Analyze with AI")}</TooltipContent>
                    </Tooltip>
                  </TooltipProvider>
                )}
                <RerunFromStepButton
                  stepId={step.id}
                  disabled={canRerunFromStep === false}
                  onRerunFromStep={onRerunFromStep}
                />
                <ChevronDown className="h-4 w-4 text-muted-foreground transition-transform shrink-0" />
              </div>
              <CardTitle className="text-sm truncate pl-6">{step.name}</CardTitle>
              <div className="flex items-center gap-2 flex-wrap pl-6">
                <AssertSummaryBadge assertResults={assertResults} />
                <TimingInfo step={step} result={result} status={status} shouldCountdown={shouldCountdown} currentAttempt={currentAttempt} totalAttempts={totalAttempts} t={t} />
              </div>
            </CardHeader>
            <CardContent className="px-4 pb-3 pt-0">
              <p className="text-xs text-muted-foreground">{step.description}</p>
              <p className="mt-1 font-mono text-xs text-muted-foreground">{step.url}</p>
              {result?.error && (
                <p className="mt-1 text-xs text-destructive">{result.error}</p>
              )}
            </CardContent>
          </div>
        </CollapsibleTrigger>

        <CollapsibleContent>
          <DetailContent step={step} result={result} assertResults={assertResults} t={t} />
        </CollapsibleContent>
      </Card>
    </Collapsible>
  );
}

/* ── Main export ── */
export function StepResultCard({ step, result, shouldCountdown, onAnalyzeWithAI, onGoToCode, onRerunFromStep, canRerunFromStep, variant = "list" }: StepResultCardProps) {
  const { t } = useTranslation();
  const status = result?.status || "pending";
  const configuredTotalAttempts = (step.retry ?? 0) + 1;
  const currentAttempt = result?.attempts ?? (status === "running" && configuredTotalAttempts > 1 ? 1 : undefined);
  const totalAttempts = result?.maxAttempts ?? configuredTotalAttempts;
  const assertResults = Array.isArray(result?.assertResults)
    ? result.assertResults.filter((item) => !!item && typeof item === "object")
    : [];

  const common = { step, result, shouldCountdown, onAnalyzeWithAI, onGoToCode, onRerunFromStep, canRerunFromStep, status, assertResults, currentAttempt, totalAttempts, t };

  if (variant === "grid") {
    return <GridCard {...common} />;
  }
  return <ListCard {...common} />;
}
