import { useRef, useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadTestConfigPanel } from "./LoadTestConfigPanel";
import { LoadTestResultsPanel } from "./LoadTestResultsPanel";
import { LoadTestRunHistoryItem } from "./LoadTestRunHistoryItem";
import { RunHistoryPanel } from "./RunHistoryPanel";
import { useLoadTestHistoryStore } from "@/stores/useLoadTestHistoryStore";
import { getRuns } from "@/lib/execution-store";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useIsMobile } from "@/hooks/use-mobile";
import { AlertTriangle, X } from "lucide-react";
import type { Pipeline } from "@/types/pipeline";
import type { LoadTestConfig, LoadTestState } from "@/types/load-test";
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
  const pendingConfigRef = useRef<{ config: LoadTestConfig; selectedBaseUrlKey?: string } | null>(null);

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


  const handleStart = useCallback((cfg: LoadTestConfig, selectedBaseUrlKey?: string) => {
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

  const lastAvgLatencyMs = runs.length > 0 ? runs[0].metrics.avgLatency : undefined;

  if (state === "idle" && !viewingHistoricRun) {
    const configContent = (
      <div className="flex-1 flex items-start justify-center overflow-auto p-4">
        <div className="w-full max-w-xl space-y-4">
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
          />
        </div>
      </div>
    );

    if (!isMobile && runs.length > 0) {
      return (
        <ResizablePanelGroup direction="horizontal" className="flex-1 overflow-hidden">
          <ResizablePanel defaultSize={75} minSize={40}>
            {configContent}
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel defaultSize={25} minSize={12} className="flex flex-col">
            {historyPanel}
          </ResizablePanel>
        </ResizablePanelGroup>
      );
    }

    return <div className="flex flex-1 w-full overflow-hidden">{configContent}</div>;
  }

  const resultsContent = (
    <ScrollArea className="h-full p-4">
      <div className="w-full max-w-3xl mx-auto space-y-4">
        <LoadTestResultsPanel
          metrics={metrics}
          state={state}
          totalRequests={config?.totalRequests ?? 0}
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
          <div className="border-border/50 max-h-[200px]">
            {historyPanel}
          </div>
        )}
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
