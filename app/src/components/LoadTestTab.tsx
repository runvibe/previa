import { useRef, useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadTestConfigPanel } from "./LoadTestConfigPanel";
import { LoadTestResultsPanel } from "./LoadTestResultsPanel";
import { LoadTestRunHistoryItem } from "./LoadTestRunHistoryItem";
import { RunHistoryPanel } from "./RunHistoryPanel";
import { useLoadTestHistoryStore } from "@/stores/useLoadTestHistoryStore";
import { getRuns } from "@/lib/execution-store";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useIsMobile } from "@/hooks/use-mobile";
import { AlertTriangle, History, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { getTestHistoryCollapsed, setTestHistoryCollapsed } from "@/lib/ui-preferences";
import type { Pipeline } from "@/types/pipeline";
import { isWaveLoadConfig, type LoadRunConfig, type LoadTestState } from "@/types/load-test";
import type { ProjectEnvGroup, ProjectSpec } from "@/types/project";

interface LoadTestTabProps {
  pipeline: Pipeline;
  projectId: string;
  pipelineIndex: number;
  onStateChange?: (state: LoadTestState) => void;
  onResetRef?: React.MutableRefObject<(() => void) | null>;
  onCancelRef?: React.MutableRefObject<(() => void) | null>;
  onStartRef?: React.MutableRefObject<(() => void) | null>;
  executionBackendUrl?: string;
  specs?: ProjectSpec[];
  envGroups?: ProjectEnvGroup[];
  selectedEnvGroupSlug?: string | null;
}

export function LoadTestTab({ pipeline, projectId, pipelineIndex, onStateChange, onResetRef, onCancelRef, onStartRef, executionBackendUrl, specs, envGroups = [], selectedEnvGroupSlug = null }: LoadTestTabProps) {
  const { t } = useTranslation();
  const store = useLoadTestHistoryStore();
  const isMobile = useIsMobile();
  const pendingConfigRef = useRef<{ config: LoadRunConfig; selectedBaseUrlKey?: string } | null>(null);
  const [historyCollapsed, setHistoryCollapsedState] = useState(() => getTestHistoryCollapsed("loadtest"));

  const setHistoryCollapsed = useCallback((collapsed: boolean) => {
    setTestHistoryCollapsed("loadtest", collapsed);
    setHistoryCollapsedState(collapsed);
  }, []);

  const { state, metrics, config, nodesInfo, runs, activeRunId, viewingHistoricRun, liveState } = store;

  // Notify parent of real test state changes (use ref to avoid infinite loops)
  const onStateChangeRef = useRef(onStateChange);
  onStateChangeRef.current = onStateChange;

  useEffect(() => {
    onStateChangeRef.current?.(liveState);
  }, [liveState]);

  // Wire up refs for header buttons
  useEffect(() => {
    if (onResetRef) onResetRef.current = () => store.resetTest();
    if (onCancelRef) onCancelRef.current = () => store.cancelTest();
    if (onStartRef) onStartRef.current = () => {
      if (pendingConfigRef.current) {
        store.runTest(
          pipeline,
          pipelineIndex,
          projectId,
          pendingConfigRef.current.config,
          executionBackendUrl,
          pendingConfigRef.current.selectedBaseUrlKey,
          specs,
          envGroups,
          selectedEnvGroupSlug
        );
      }
    };
  });

  // Load history on mount / pipeline change
  useEffect(() => {
    store.loadHistory(projectId, pipelineIndex, executionBackendUrl);
  }, [projectId, pipelineIndex, executionBackendUrl]);


  const handleStart = useCallback((cfg: LoadRunConfig, selectedBaseUrlKey?: string) => {
    store.runTest(pipeline, pipelineIndex, projectId, cfg, executionBackendUrl, selectedBaseUrlKey, specs, envGroups, selectedEnvGroupSlug);
  }, [pipeline, pipelineIndex, projectId, executionBackendUrl, specs, envGroups, selectedEnvGroupSlug]);

  const handleClearHistory = useCallback(async () => {
    await store.clearHistory(projectId, pipelineIndex, executionBackendUrl);
  }, [projectId, pipelineIndex, executionBackendUrl]);

  const historyPanel = (
    <RunHistoryPanel 
      title={t("history.title")} 
      onClear={handleClearHistory} 
      isEmpty={runs.length === 0}
      onCollapse={() => setHistoryCollapsed(true)}
      collapsed={false}
      collapseDirection={isMobile ? "bottom" : "side"}
      collapseOnHeaderClick={isMobile}
    >
      {runs.map((run) => (
        <LoadTestRunHistoryItem
          key={run.id}
          run={run}
          isActive={activeRunId === run.id}
          onClick={() => {
            if (run.state === "running") {
              if (liveState === "running") {
                store.backToLive();
              } else if (run.executionId && executionBackendUrl) {
                store.reconnectExecution(run.executionId, run.projectId, executionBackendUrl);
              }
            } else {
              store.selectHistoricRun(run, executionBackendUrl);
            }
          }}
        />
      ))}
    </RunHistoryPanel>
  );

  const collapsedHistoryAside = runs.length > 0 ? (
    <div
      className={cn(
        "shrink-0 border-l border-border/50 flex flex-col transition-[width] duration-300 ease-in-out overflow-hidden",
        "w-8",
      )}
    >
      <div className="flex flex-col items-center pt-2">
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={() => setHistoryCollapsed(false)}
          title="Show history"
        >
          <History className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  ) : null;

  const mobileHistoryPanel = runs.length > 0 ? (
    <div
      data-testid="mobile-load-test-history"
      className={cn(
        "shrink-0 border-t border-border/50 transition-[max-height] duration-300 ease-in-out overflow-hidden",
        historyCollapsed ? "max-h-10 cursor-pointer" : "max-h-[200px]",
      )}
      onClick={historyCollapsed ? () => setHistoryCollapsed(false) : undefined}
      role={historyCollapsed ? "button" : undefined}
      tabIndex={historyCollapsed ? 0 : undefined}
      onKeyDown={historyCollapsed ? (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          setHistoryCollapsed(false);
        }
      } : undefined}
    >
      {historyCollapsed ? (
        <div className="flex h-10 items-center justify-center">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={() => setHistoryCollapsed(false)}
            title="Show history"
          >
            <History className="h-4 w-4" />
          </Button>
        </div>
      ) : (
        <div className="h-[200px]">
          {historyPanel}
        </div>
      )}
    </div>
  ) : null;

  const lastAvgLatencyMs = runs.length > 0 ? runs[0].metrics.avgLatency : undefined;

  if (state === "idle" && !viewingHistoricRun) {
    const configContent = (
      <div
        data-testid="load-test-config-scroll"
        className="h-full min-h-0 flex-1 overflow-y-auto overflow-x-hidden p-4"
      >
        <div className="mx-auto w-full max-w-xl space-y-4">
          <LoadTestConfigPanel
            pipeline={pipeline}
            onStart={handleStart}
            onConfigChange={(cfg, envKey) => {
              pendingConfigRef.current = { config: cfg, selectedBaseUrlKey: envKey };
            }}
            lastAvgLatencyMs={lastAvgLatencyMs}
            initialConfig={config}
            envGroups={envGroups}
            selectedEnvGroupSlug={selectedEnvGroupSlug}
            runnerCount={nodesInfo?.nodesUsed}
          />
        </div>
      </div>
    );

    if (!isMobile && runs.length > 0) {
      if (historyCollapsed) {
        return (
          <div className="flex min-h-0 flex-1 overflow-hidden">
            <div className="min-h-0 flex-1 overflow-hidden">
              {configContent}
            </div>
            {collapsedHistoryAside}
          </div>
        );
      }

      return (
        <ResizablePanelGroup direction="horizontal" className="min-h-0 flex-1 overflow-hidden">
          <ResizablePanel defaultSize={75} minSize={40} className="min-h-0 overflow-hidden">
            {configContent}
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel defaultSize={25} minSize={12} className="flex flex-col">
            {historyPanel}
          </ResizablePanel>
        </ResizablePanelGroup>
      );
    }

    if (isMobile && runs.length > 0) {
      return (
        <div className="flex h-full min-h-0 flex-1 w-full flex-col overflow-hidden">
          {configContent}
          {mobileHistoryPanel}
        </div>
      );
    }

    return <div className="flex h-full min-h-0 flex-1 w-full overflow-hidden">{configContent}</div>;
  }

  const resultsContent = (
    <ScrollArea className="h-full p-4">
      <div className="w-full max-w-3xl mx-auto space-y-4">
        <LoadTestResultsPanel
          metrics={metrics}
          state={state}
          totalRequests={config && !isWaveLoadConfig(config) ? config.totalRequests : 0}
          config={config}
          nodesInfo={nodesInfo}
        />
      </div>
    </ScrollArea>
  );

  if (isMobile) {
    return (
      <div className="flex flex-1 flex-col overflow-hidden">
        {resultsContent}
        {runs.length > 0 && (
          mobileHistoryPanel
        )}
      </div>
    );
  }

  if (historyCollapsed && runs.length > 0) {
    return (
      <div className="flex min-h-0 flex-1 overflow-hidden">
        <div className="min-h-0 flex-1 overflow-hidden">
          {resultsContent}
        </div>
        {collapsedHistoryAside}
      </div>
    );
  }

  return (
    <ResizablePanelGroup direction="horizontal" className="flex-1 overflow-hidden" key={runs.length > 0 ? "with-history" : "no-history"}>
      <ResizablePanel defaultSize={runs.length > 0 ? 75 : 100} minSize={40}>
        {resultsContent}
      </ResizablePanel>
      {runs.length > 0 && (
        <>
          <ResizableHandle />
          <ResizablePanel defaultSize={25} minSize={12} className="flex flex-col">
            {historyPanel}
          </ResizablePanel>
        </>
      )}
    </ResizablePanelGroup>
  );
}
