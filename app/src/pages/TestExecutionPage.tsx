import { useState, useCallback, useEffect, useRef, useMemo } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import * as apiClient from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

import { Play, Plus, Workflow, FileCode2, FileText, PlayCircle, Menu, Zap, RotateCcw, Server, Square, X, Sparkles, ArrowDown, ListChecks, List, MousePointerClick, PanelRightClose, PanelRightOpen, History, LayoutGrid, ListOrdered } from "lucide-react";
import { useStepAutoScroll, useStepVisibility } from "@/hooks/useStepAutoScroll";
import { Checkbox } from "@/components/ui/checkbox";
import { getPipelineOrder, savePipelineOrder, applyOrder } from "@/lib/pipeline-order";
import { getTestHistoryCollapsed, getTestModeSidebarCollapsed, setTestHistoryCollapsed, setTestModeSidebarCollapsed as saveTestModeSidebarCollapsed } from "@/lib/ui-preferences";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";
import { DotsLoader } from "@/components/DotsLoader";
import { Tabs, TabsContent } from "@/components/ui/tabs";
import { LoadTestTab } from "@/components/LoadTestTab";
import { TestModeSidebar } from "@/components/TestModeSidebar";
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from "@/components/ui/resizable";
import { StepFlowGraph } from "@/components/StepFlowGraph";
import type { StepFlowGraphItem } from "@/components/StepFlowGraph";
import { StepFlowList } from "@/components/StepFlowList";
import { useStepViewStore } from "@/stores/useStepViewStore";
import { useExperimentalFeaturesEnabled } from "@/stores/useExperimentalFeaturesStore";
import { useExecutionHistoryStore } from "@/stores/useExecutionHistoryStore";
import { useLoadTestHistoryStore } from "@/stores/useLoadTestHistoryStore";
import type { ExecutionRun } from "@/lib/execution-store";
import type { Pipeline, OpenAPISpec } from "@/types/pipeline";
import type { ProjectEnvGroup, ProjectSpec } from "@/types/project";

import { EmptyState } from "@/components/EmptyState";
import { SectionHeader } from "@/components/SectionHeader";
import { PipelineListItem } from "@/components/PipelineListItem";
import { StepResultCard } from "@/components/StepResultCard";
import { RunHistoryItem } from "@/components/RunHistoryItem";
import { RunHistoryPanel } from "@/components/RunHistoryPanel";
import { BatchControls } from "@/components/BatchControls";
import { PipelineMiniChart } from "@/components/PipelineMiniChart";
import { useIsMobile } from "@/hooks/use-mobile";
import { ConfirmDeleteDialog } from "@/components/ConfirmDeleteDialog";
import { ProjectEnvGroupsPanel } from "@/components/ProjectEnvGroupsPanel";

interface TestExecutionPageProps {
  pipelines: Pipeline[];
  spec?: OpenAPISpec;
  specs?: ProjectSpec[];
  envGroups?: ProjectEnvGroup[];
  projectId: string;
  onDeletePipeline: (index: number) => void;
  onCreatePipeline: () => void;
  onCreateAIPipeline?: () => void;
  onEditPipeline?: (index: number) => void;
  onDuplicatePipeline?: (index: number) => void;
  onImportSpec?: (content: string) => void;
  onEditSpec?: (specId?: string) => void;
  onDeleteSpec?: (specId: string) => void;
  onCreateEnvGroup?: (data: apiClient.ProjectEnvGroupUpsertRequest) => Promise<ProjectEnvGroup | null>;
  onUpdateEnvGroup?: (id: string, data: apiClient.ProjectEnvGroupUpsertRequest) => Promise<void>;
  onDeleteEnvGroup?: (id: string) => Promise<void>;
  selectedPipelineId?: string;
  initialSelectedIndex?: number;
  onSelectPipeline?: (index: number) => void;
  initialTab?: "integration" | "loadtest";
  onTabChange?: (tab: "integration" | "loadtest") => void;
  executionBackendUrl?: string;
  autoRunPipelineId?: string | null;
  autoSelectTab?: "integration" | "loadtest" | null;
  onAnalyzeStepWithAI?: (step: import("@/types/pipeline").PipelineStep, result: import("@/types/pipeline").StepExecutionResult) => void;
}

type BatchState = "idle" | "running" | "paused";

/* ── Sidebar content (shared between desktop panel & mobile sheet) ── */
function SidebarContent({
  spec, specs, envGroups, pipelines, selectedIndex, pipelineStatuses, running, isBatchActive, batchState,
  batchProgress, batchTotal, onEditSpec, onDeleteSpec, onCreatePipeline, onCreateAIPipeline, onSelect, onEdit, onDuplicate, onDelete,
  onCreateEnvGroup, onUpdateEnvGroup, onDeleteEnvGroup,
  handleRunAll, handleBatchPause, handleBatchResume, handleBatchCancel, executionBackendUrl,
  dragTargetIndex, onDragStart, onDragOver, onDrop,
  selectedForBatch, onToggleBatchCheck, onToggleAllBatchCheck, showBatchCheckboxes,
  queuePipelines, pipelineNames, experimentalFeaturesEnabled,
}: {
  spec?: OpenAPISpec; specs?: ProjectSpec[]; envGroups?: ProjectEnvGroup[]; pipelines: Pipeline[]; selectedIndex: number | null;
  pipelineStatuses: Record<number, "success" | "error" | "running" | "queued">; running: boolean;
  isBatchActive: boolean; batchState: BatchState;
  batchProgress: number; batchTotal: number;
  onEditSpec?: (specId?: string) => void; onDeleteSpec?: (specId: string) => void;
  onCreateEnvGroup?: (data: apiClient.ProjectEnvGroupUpsertRequest) => Promise<ProjectEnvGroup | null>;
  onUpdateEnvGroup?: (id: string, data: apiClient.ProjectEnvGroupUpsertRequest) => Promise<void>;
  onDeleteEnvGroup?: (id: string) => Promise<void>;
  onCreatePipeline: () => void; onCreateAIPipeline?: () => void;
  onSelect: (i: number, event?: React.MouseEvent) => void; onEdit?: (i: number) => void;
  onDuplicate?: (i: number) => void; onDelete: (i: number) => void;
  handleRunAll: () => void; handleBatchPause: () => void;
  handleBatchResume: () => void; handleBatchCancel: () => void;
  executionBackendUrl?: string;
  dragTargetIndex: number | null;
  onDragStart: (i: number) => void;
  onDragOver: (i: number) => void;
  onDrop: (i: number) => void;
  selectedForBatch: Set<number>;
  onToggleBatchCheck: (i: number) => void;
  onToggleAllBatchCheck: () => void;
  showBatchCheckboxes: boolean;
  queuePipelines?: apiClient.E2eQueuePipelineRecord[];
  pipelineNames?: Record<string, string>;
  experimentalFeaturesEnabled: boolean;
}) {
  const { t } = useTranslation();
  const [deleteTarget, setDeleteTarget] = useState<{ type: "pipeline" | "spec"; idOrIndex: string | number; name: string } | null>(null);
  const hasSpecs = specs && specs.length > 0;
  const hasAnySpec = hasSpecs || !!spec;
  const hasVisibleSpec = experimentalFeaturesEnabled && hasAnySpec;
  return (
    <>
      {experimentalFeaturesEnabled && (
        <div className="border-border/50 px-4 py-3">
          <SectionHeader title="API Specs">
            {onEditSpec && (
              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => onEditSpec()} title={t("testExecution.addSpec")}>
                <Plus className="h-3.5 w-3.5" />
              </Button>
            )}
          </SectionHeader>
          {hasSpecs ? (
            <div className="mt-1 space-y-1">
              {specs!.map((s) => (
                <div key={s.id} className="group flex items-center gap-1.5 text-xs rounded-md px-1.5 py-1 hover:bg-accent/50 transition-colors">
                  <button
                    className="flex-1 truncate text-left text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
                    onClick={() => onEditSpec?.(s.id)}
                  >
                    {s.name}
                  </button>
                  <Badge variant="secondary" className="text-[10px] px-1 py-0 h-4 shrink-0">
                    {s.spec?.routes?.length ?? 0}
                  </Badge>
                  {onDeleteSpec && (
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-5 w-5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
                      onClick={() => setDeleteTarget({ type: "spec", idOrIndex: s.id, name: s.name })}
                      title={t("testExecution.removeSpec")}
                    >
                      <X className="h-3 w-3" />
                    </Button>
                  )}
                </div>
              ))}
            </div>
          ) : spec ? (
            <button className="mt-1 truncate text-xs text-muted-foreground hover:text-foreground transition-colors cursor-pointer text-left w-full" onClick={() => onEditSpec?.()}>
              {spec.title} v{spec.version}
            </button>
          ) : (
            <p className="mt-1 text-xs text-muted-foreground">{t("testExecution.noSpecImported")}</p>
          )}
        </div>
      )}

      {onCreateEnvGroup && onUpdateEnvGroup && onDeleteEnvGroup && (
        <ProjectEnvGroupsPanel
          envGroups={envGroups ?? []}
          onCreate={onCreateEnvGroup}
          onUpdate={onUpdateEnvGroup}
          onDelete={onDeleteEnvGroup}
        />
      )}

      <div className="border-border/50 px-4 py-3">
        <SectionHeader title="Pipelines">
          {pipelines.length > 0 && !isBatchActive && (
            <div className="flex items-center gap-0.5">
              <Button
                variant={showBatchCheckboxes ? "secondary" : "ghost"}
                size="icon"
                className="h-7 w-7"
                onClick={onToggleAllBatchCheck}
                title={showBatchCheckboxes ? t("common.deselectAll") : t("common.selectAll")}
              >
                <ListChecks className="h-3.5 w-3.5" />
              </Button>
              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={handleRunAll} title={!executionBackendUrl ? t("testExecution.configureServerUrl") : t("testExecution.runAll")} disabled={running || !executionBackendUrl || (showBatchCheckboxes && selectedForBatch.size === 0)}>
                <PlayCircle className="h-4 w-4" />
              </Button>
            </div>
          )}
          {experimentalFeaturesEnabled && onCreateAIPipeline && (
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onCreateAIPipeline} title={hasAnySpec ? t("testExecution.createWithAI") : t("testExecution.importSpecFirst")} disabled={!hasAnySpec}>
              <Sparkles className="h-3.5 w-3.5" />
            </Button>
          )}
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onCreatePipeline} title={t("testExecution.newPipeline")}>
            <Plus className="h-4 w-4" />
          </Button>
        </SectionHeader>

        {isBatchActive && (
          <BatchControls
            batchState={batchState as "running" | "paused"}
            progress={batchProgress}
            total={batchTotal}
            onPause={handleBatchPause}
            onResume={handleBatchResume}
            onCancel={handleBatchCancel}
            queuePipelines={queuePipelines}
            pipelineNames={pipelineNames}
          />
        )}
      </div>

      <ScrollArea className="flex-1">
        <div className="space-y-0">
          {!hasVisibleSpec && pipelines.length === 0 && (
            <div className="p-4 text-center">
              <p className="text-sm text-muted-foreground">
                {experimentalFeaturesEnabled ? t("testExecution.importSpecToCreate") : t("testExecution.noPipelineCreated")}
              </p>
            </div>
          )}
          {hasVisibleSpec && pipelines.length === 0 && (
            <p className="p-4 text-center text-sm text-muted-foreground">{t("testExecution.noPipelineCreated")}</p>
          )}
          {pipelines.map((p, i) => (
            <PipelineListItem
              key={p.id || `${p.name}-${i}`}
              pipeline={p}
              index={i}
              isSelected={selectedIndex === i}
              status={pipelineStatuses[i]}
              onSelect={onSelect}
              onEdit={onEdit}
              onDuplicate={onDuplicate}
              onDelete={(i) => setDeleteTarget({ type: "pipeline", idOrIndex: i, name: pipelines[i]?.name ?? "" })}
              onDragStart={onDragStart}
              onDragOver={onDragOver}
              onDrop={onDrop}
              isDragTarget={dragTargetIndex === i}
              isChecked={selectedForBatch.has(i)}
              onToggleCheck={showBatchCheckboxes ? onToggleBatchCheck : undefined}
            />
          ))}
        </div>
      </ScrollArea>

      <ConfirmDeleteDialog
        open={!!deleteTarget}
        onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
        itemName={deleteTarget?.name ?? ""}
        itemType={deleteTarget?.type === "spec" ? "spec" : "pipeline"}
        onConfirm={() => {
          if (!deleteTarget) return;
          if (deleteTarget.type === "pipeline") {
            onDelete(deleteTarget.idOrIndex as number);
          } else if (onDeleteSpec) {
            onDeleteSpec(deleteTarget.idOrIndex as string);
          }
          setDeleteTarget(null);
        }}
      />
    </>
  );
}

export default function TestExecutionPage({ pipelines, spec, specs, envGroups = [], projectId, onDeletePipeline, onCreatePipeline, onCreateAIPipeline, onEditPipeline, onDuplicatePipeline, onImportSpec, onEditSpec, onDeleteSpec, onCreateEnvGroup, onUpdateEnvGroup, onDeleteEnvGroup, selectedPipelineId, initialSelectedIndex, onSelectPipeline, initialTab, onTabChange, executionBackendUrl, autoRunPipelineId, autoSelectTab, onAnalyzeStepWithAI }: TestExecutionPageProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const stepViewMode = useStepViewStore((s) => s.mode);
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const stepScrollContainerRef = useRef<HTMLDivElement>(null!);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [historyCollapsed, setHistoryCollapsedState] = useState(() => getTestHistoryCollapsed("integration"));
  const setHistoryCollapsed = useCallback((collapsed: boolean) => {
    setTestHistoryCollapsed("integration", collapsed);
    setHistoryCollapsedState(collapsed);
  }, []);
  const [testModeSidebarCollapsed, setTestModeSidebarCollapsedState] = useState(() => getTestModeSidebarCollapsed());
  const setTestModeSidebarCollapsed = useCallback((collapsed: boolean) => {
    saveTestModeSidebarCollapsed(collapsed);
    setTestModeSidebarCollapsedState(collapsed);
  }, []);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(
    initialSelectedIndex ?? (pipelines.length > 0 ? 0 : null)
  );

  const handleGoToCode = useCallback((stepId: string) => {
    if (selectedIndex !== null && onEditPipeline) {
      // Pass stepId as query param so the editor can scroll to it
      const url = new URL(window.location.href);
      url.searchParams.set("stepId", stepId);
      window.history.replaceState(null, "", url.toString());
      onEditPipeline(selectedIndex);
    }
  }, [selectedIndex, onEditPipeline]);

  // Read execution state from store
  const results = useExecutionHistoryStore((state) => state.results);
  const running = useExecutionHistoryStore((state) => state.running);
  const runHistory = useExecutionHistoryStore((state) => state.runs);
  const activeRunId = useExecutionHistoryStore((state) => state.activeRunId);
  const executionNode = useExecutionHistoryStore((state) => state.executionNode);
  const pipelineStatuses = useExecutionHistoryStore((state) => state.latestStatuses);
  const setLatestExecutionStatuses = useExecutionHistoryStore((state) => state.setLatestStatuses);
  const clearPipelineStatus = useExecutionHistoryStore((state) => state.clearPipelineStatus);
  const checkPipelineRuntime = useExecutionHistoryStore((state) => state.checkPipelineRuntime);
  const loadExecutionHistory = useExecutionHistoryStore((state) => state.loadHistory);
  const fetchAllLatestStatuses = useExecutionHistoryStore((state) => state.fetchAllLatestStatuses);
  const runExecutionTest = useExecutionHistoryStore((state) => state.runTest);
  const rerunExecutionFromStep = useExecutionHistoryStore((state) => state.rerunFromStep);
  const disconnectExecutionController = useExecutionHistoryStore((state) => state.disconnectController);
  const clearExecutionResults = useExecutionHistoryStore((state) => state.clearResults);
  const setExecutionRuns = useExecutionHistoryStore((state) => state.setRuns);
  const selectExecutionRun = useExecutionHistoryStore((state) => state.selectRun);
  const deleteLocalExecutionRunsForPipeline = useExecutionHistoryStore((state) => state.deleteLocalRunsForPipeline);

  const [chartRefreshKey, setChartRefreshKey] = useState(0);
  const [activeTab, setActiveTab] = useState<string>(initialTab ?? "integration");
  const [loadTestState, setLoadTestState] = useState<string>("idle");
  const loadTestResetRef = useRef<(() => void) | null>(null);
  const loadTestCancelRef = useRef<(() => void) | null>(null);
  const loadTestStartRef = useRef<(() => void) | null>(null);
  const [selectedEnvGroupSlug, setSelectedEnvGroupSlug] = useState<string | null>(envGroups[0]?.slug ?? null);

  useEffect(() => {
    if (envGroups.length === 0) {
      setSelectedEnvGroupSlug(null);
      return;
    }
    setSelectedEnvGroupSlug((current) =>
      current && envGroups.some((group) => group.slug === current) ? current : envGroups[0].slug
    );
  }, [envGroups]);

  const runtimeEnvGroups = useMemo(() => apiClient.projectEnvGroupsToRuntime(envGroups), [envGroups]);
  const effectiveSelectedEnvGroupSlug = selectedEnvGroupSlug ?? envGroups[0]?.slug ?? null;

  const handleLoadTestStateChange = useCallback((s: string) => {
    setLoadTestState(s);
  }, []);

  const [batchState, setBatchState] = useState<BatchState>("idle");
  const [batchQueue, setBatchQueue] = useState<number[]>([]);
  const [batchProgress, setBatchProgress] = useState(0);
  const [batchTotal, setBatchTotal] = useState(0);
  const batchStateRef = useRef<BatchState>("idle");

  // Server-side queue state
  const [queueId, setQueueId] = useState<string | null>(null);
  const [queuePipelines, setQueuePipelines] = useState<apiClient.E2eQueuePipelineRecord[]>([]);
  const queueStreamRef = useRef<apiClient.E2eQueueStreamController | null>(null);
  const queueFallbackPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const queueReconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const queueActiveRef = useRef<{ id: string | null; terminal: boolean }>({ id: null, terminal: false });
  const selectedQueueStateRef = useRef<{ pipelineId: string | null; status: string | null }>({
    pipelineId: null,
    status: null,
  });

  // Pipeline selection for batch execution
  const [showBatchCheckboxes, setShowBatchCheckboxes] = useState(false);
  const [selectedForBatch, setSelectedForBatch] = useState<Set<number>>(() =>
    new Set(pipelines.map((_, i) => i))
  );

  // Sync selectedForBatch when pipelines change
  useEffect(() => {
    setSelectedForBatch(new Set(pipelines.map((_, i) => i)));
  }, [pipelines.length]);

  useEffect(() => { batchStateRef.current = batchState; }, [batchState]);

  // Pipeline ordering (per context)
  const activeContextId = useOrchestratorStore((s) => s.activeContext?.id);
  const orderedPipelines = useMemo(() => {
    if (!activeContextId) return pipelines;
    const order = getPipelineOrder(activeContextId);
    return applyOrder(pipelines, order);
  }, [pipelines, activeContextId]);

  const syncQueueSelection = useCallback((items: apiClient.E2eQueuePipelineRecord[]) => {
    const runningQueueItem = items.find((item) => item.status === "running");
    if (!runningQueueItem?.id) return false;

    const runningIndex = orderedPipelines.findIndex((pipeline) => pipeline.id === runningQueueItem.id);
    if (runningIndex < 0 || runningIndex === selectedIndex) return false;

    const originalIndex = pipelines.findIndex((pipeline) => pipeline.id === runningQueueItem.id);

    console.log("[DEBUG][syncQueueSelection] following running queue pipeline", {
      queuePipelineId: runningQueueItem.id,
      runningIndex,
      selectedIndex,
      originalIndex,
    });

    setSelectedIndex(runningIndex);
    onSelectPipeline?.(originalIndex >= 0 ? originalIndex : runningIndex);
    return true;
  }, [onSelectPipeline, orderedPipelines, pipelines, selectedIndex]);

  const syncSelectedQueuePipeline = useCallback(async (items: apiClient.E2eQueuePipelineRecord[]) => {
    if (!executionBackendUrl || activeTab !== "integration" || selectedIndex === null) return;

    const selectedPipeline = orderedPipelines[selectedIndex];
    if (!selectedPipeline?.id) return;

    const selectedQueueRecord = items.find((item) => item.id === selectedPipeline.id);
    if (!selectedQueueRecord) return;

    const previous = selectedQueueStateRef.current;
    const statusChanged =
      previous.pipelineId !== selectedPipeline.id
      || previous.status !== selectedQueueRecord.status;

    selectedQueueStateRef.current = {
      pipelineId: selectedPipeline.id,
      status: selectedQueueRecord.status,
    };

    if (!statusChanged) {
      return;
    }

    console.log("[DEBUG][syncSelectedQueuePipeline] queue state for selected pipeline", {
      selectedIndex,
      pipelineId: selectedPipeline.id,
      pipelineName: selectedPipeline.name,
      queueStatus: selectedQueueRecord.status,
    });

    if (selectedQueueRecord.status === "running") {
      await checkPipelineRuntime(
        projectId,
        selectedPipeline.id,
        selectedIndex,
        executionBackendUrl,
        selectedPipeline,
      );
      return;
    }

    if (
      selectedQueueRecord.status === "completed"
      || selectedQueueRecord.status === "failed"
      || selectedQueueRecord.status === "cancelled"
    ) {
      await loadExecutionHistory(projectId, selectedIndex, executionBackendUrl, selectedPipeline);
    }
  }, [executionBackendUrl, activeTab, selectedIndex, orderedPipelines, checkPipelineRuntime, loadExecutionHistory, projectId]);

  const clearQueueTransport = useCallback(() => {
    queueStreamRef.current?.disconnect();
    queueStreamRef.current = null;

    if (queueFallbackPollRef.current) {
      clearInterval(queueFallbackPollRef.current);
      queueFallbackPollRef.current = null;
    }

    if (queueReconnectTimerRef.current) {
      clearTimeout(queueReconnectTimerRef.current);
      queueReconnectTimerRef.current = null;
    }
  }, []);

  const handleQueueSnapshot = useCallback(async (record: apiClient.E2eQueueRecord) => {
    queueActiveRef.current = {
      id: record.id,
      terminal: record.status === "completed" || record.status === "failed" || record.status === "cancelled",
    };

    setQueueId(record.id);
    setQueuePipelines(record.pipelines);
    if (!queueActiveRef.current.terminal) {
      const switchedSelection = syncQueueSelection(record.pipelines);
      if (switchedSelection) {
        return;
      }
    }
    await syncSelectedQueuePipeline(record.pipelines);

    const completed = record.pipelines.filter((p) => p.status === "completed" || p.status === "failed" || p.status === "cancelled").length;
    setBatchProgress(completed);
    setBatchTotal(record.pipelines.length);
    setBatchState(record.status === "pending" || record.status === "running" ? "running" : "idle");

    if (!queueActiveRef.current.terminal) return;

    clearQueueTransport();
    setLatestExecutionStatuses((() => {
      const nextStatuses = { ...pipelineStatuses } as Record<number, "success" | "error" | "running" | "queued">;

      for (const qp of record.pipelines) {
        const pipelineIdx = orderedPipelines.findIndex((pipeline) => pipeline.id === qp.id);
        if (pipelineIdx < 0) continue;

        if (qp.status === "completed") {
          nextStatuses[pipelineIdx] = "success";
        } else if (qp.status === "failed") {
          nextStatuses[pipelineIdx] = "error";
        } else {
          delete nextStatuses[pipelineIdx];
        }
      }

      return nextStatuses;
    })());
    setBatchTotal(0);
    setQueueId(null);
    setQueuePipelines([]);

    if (selectedIndex !== null) {
      const selectedPipeline = orderedPipelines[selectedIndex];
      if (selectedPipeline) {
        await loadExecutionHistory(projectId, selectedIndex, executionBackendUrl, selectedPipeline);
      }
    }
    setChartRefreshKey((prev) => prev + 1);

    if (record.status === "completed") {
      toast.success(t("batch.completed", "Fila de execução concluída"));
    } else if (record.status === "failed") {
      toast.error(t("batch.failed", "Fila de execução com falhas"));
    } else {
      toast.info(t("batch.cancelled", "Fila de execução cancelada"));
    }
  }, [clearQueueTransport, executionBackendUrl, loadExecutionHistory, orderedPipelines, pipelineStatuses, projectId, selectedIndex, setLatestExecutionStatuses, syncQueueSelection, syncSelectedQueuePipeline, t]);

  const connectQueueStream = useCallback((queueRecordId: string) => {
    if (!executionBackendUrl) return;

    clearQueueTransport();
    queueActiveRef.current = { id: queueRecordId, terminal: false };

    queueStreamRef.current = apiClient.connectE2eQueue(executionBackendUrl, projectId, queueRecordId, {
      onSnapshot: (record) => {
        void handleQueueSnapshot(record);
      },
      onError: (error) => {
        console.error("[DEBUG][queueSse] stream error", { queueId: queueRecordId, error });

        if (queueActiveRef.current.id !== queueRecordId || queueActiveRef.current.terminal) return;

        if (!queueFallbackPollRef.current) {
          queueFallbackPollRef.current = setInterval(async () => {
            try {
              const record = await apiClient.getE2eQueue(executionBackendUrl, projectId, queueRecordId);
              await handleQueueSnapshot(record);

              if (queueActiveRef.current.terminal) {
                if (queueFallbackPollRef.current) {
                  clearInterval(queueFallbackPollRef.current);
                  queueFallbackPollRef.current = null;
                }
                return;
              }

              if (!queueStreamRef.current && !queueReconnectTimerRef.current) {
                queueReconnectTimerRef.current = setTimeout(() => {
                  queueReconnectTimerRef.current = null;
                  connectQueueStream(queueRecordId);
                }, 1500);
              }
            } catch (pollError) {
              console.error("[DEBUG][queueSse] fallback poll error", { queueId: queueRecordId, error: pollError });
            }
          }, 2000);
        }
      },
      onClose: () => {
        queueStreamRef.current = null;

        if (queueActiveRef.current.id !== queueRecordId || queueActiveRef.current.terminal) return;

        if (!queueReconnectTimerRef.current) {
          queueReconnectTimerRef.current = setTimeout(() => {
            queueReconnectTimerRef.current = null;
            connectQueueStream(queueRecordId);
          }, 1500);
        }
      },
    });
  }, [clearQueueTransport, executionBackendUrl, handleQueueSnapshot, projectId]);

  useEffect(() => {
    if (initialTab) {
      setActiveTab(initialTab);
    }
  }, [initialTab]);

  useEffect(() => {
    selectedQueueStateRef.current = { pipelineId: null, status: null };
  }, [selectedIndex, activeTab, projectId]);

  const { showGoToButton, goToRunningStep } = useStepAutoScroll(stepScrollContainerRef, running, results);

  // Load history when pipeline changes + poll every 1s (tab-aware)
  const loadLoadTestHistory = useLoadTestHistoryStore((state) => state.loadHistory);
  const disconnectLoadTestController = useLoadTestHistoryStore((state) => state.disconnectController);
  const integrationSelectionRef = useRef<string | null>(null);
  useEffect(() => {
    if (selectedIndex === null || selectedIndex >= orderedPipelines.length) {
      console.log("[DEBUG][TestExecutionPage.useEffect] no selected pipeline or out of bounds", {
        selectedIndex,
        orderedPipelinesLength: orderedPipelines.length,
        activeTab,
      });
      disconnectExecutionController();
      clearExecutionResults();
      setExecutionRuns([]);
      integrationSelectionRef.current = null;
      return;
    }

    const pipeline = orderedPipelines[selectedIndex];
    console.log("[DEBUG][TestExecutionPage.useEffect] selected pipeline changed", {
      selectedIndex,
      pipelineId: pipeline?.id,
      pipelineName: pipeline?.name,
      activeTab,
      hasBackendUrl: !!executionBackendUrl,
    });

    if (activeTab === "integration") {
      const nextSelectionKey = `${projectId}:${pipeline.id ?? selectedIndex}:integration`;
      const shouldDisconnectExecution =
        integrationSelectionRef.current !== null && integrationSelectionRef.current !== nextSelectionKey;

      if (shouldDisconnectExecution) {
        disconnectExecutionController();
      }
      integrationSelectionRef.current = nextSelectionKey;

      let cancelled = false;
      (async () => {
        // First: check pipeline runtime for active execution recovery
        console.log("[DEBUG][useEffect] pipeline runtime check", {
          pipelineId: pipeline?.id,
          pipelineName: pipeline?.name,
          hasBackendUrl: !!executionBackendUrl,
          selectedIndex,
        });
        if (executionBackendUrl && pipeline?.id) {
          await checkPipelineRuntime(projectId, pipeline.id, selectedIndex, executionBackendUrl, pipeline);
        }
        if (cancelled) return;
        // Then load history for sidebar list
        console.log("[DEBUG][TestExecutionPage.useEffect] loading integration history after runtime check", {
          selectedIndex,
          pipelineId: pipeline?.id,
          pipelineName: pipeline?.name,
        });
        loadExecutionHistory(projectId, selectedIndex, executionBackendUrl, pipeline);
      })();
      // Polling desativado temporariamente
      return () => {
        cancelled = true;
        console.log("[DEBUG][TestExecutionPage.useEffect] cleaning integration effect", {
          selectedIndex,
          pipelineId: pipeline?.id,
          pipelineName: pipeline?.name,
        });
      };
    } else {
      if (integrationSelectionRef.current !== null) {
        disconnectExecutionController();
        integrationSelectionRef.current = null;
      }
      // loadtest tab
      console.log("[DEBUG][TestExecutionPage.useEffect] loading load-test history", {
        selectedIndex,
        pipelineId: pipeline?.id,
        pipelineName: pipeline?.name,
      });
      loadLoadTestHistory(projectId, selectedIndex, executionBackendUrl);
      // Polling desativado temporariamente
      return () => {
        console.log("[DEBUG][TestExecutionPage.useEffect] cleaning load-test effect", {
          selectedIndex,
          pipelineId: pipeline?.id,
          pipelineName: pipeline?.name,
        });
        disconnectLoadTestController();
      };
    }
  }, [selectedIndex, projectId, orderedPipelines, executionBackendUrl, activeTab, disconnectExecutionController, clearExecutionResults, setExecutionRuns, checkPipelineRuntime, loadExecutionHistory, loadLoadTestHistory, disconnectLoadTestController]);

  useEffect(() => {
    if (selectedPipelineId) {
      const nextIndex = orderedPipelines.findIndex((pipeline) => pipeline.id === selectedPipelineId);
      if (nextIndex >= 0) {
        setSelectedIndex(nextIndex);
        return;
      }
    }

    if (initialSelectedIndex !== undefined && initialSelectedIndex < pipelines.length) {
      const fallbackPipeline = pipelines[initialSelectedIndex];
      if (fallbackPipeline?.id) {
        const nextIndex = orderedPipelines.findIndex((pipeline) => pipeline.id === fallbackPipeline.id);
        setSelectedIndex(nextIndex >= 0 ? nextIndex : initialSelectedIndex);
        return;
      }

      setSelectedIndex(initialSelectedIndex);
      return;
    }

    if (orderedPipelines.length === 0) {
      setSelectedIndex(null);
    }
  }, [selectedPipelineId, initialSelectedIndex, pipelines, orderedPipelines]);

  // Fetch latest statuses for all pipelines on mount
  useEffect(() => {
    if (orderedPipelines.length > 0 && executionBackendUrl) {
      fetchAllLatestStatuses(projectId, orderedPipelines.length, executionBackendUrl);
    }
  }, [projectId, orderedPipelines.length, executionBackendUrl, fetchAllLatestStatuses]);

  // Notify parent to update URL when auto-selecting first pipeline on mount
  useEffect(() => {
    if (initialSelectedIndex === undefined && pipelines.length > 0) {
      onSelectPipeline?.(0);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Auto-run pipeline when triggered by AI assistant
  const autoRunProcessedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!autoRunPipelineId || autoRunProcessedRef.current === autoRunPipelineId) return;
    const idx = pipelines.findIndex((p) => p.id === autoRunPipelineId);
    if (idx < 0) return;
    autoRunProcessedRef.current = autoRunPipelineId;
    setSelectedIndex(idx);
    if (autoSelectTab) setActiveTab(autoSelectTab);

    // Delay to let state settle before triggering execution
    setTimeout(() => {
      if (autoSelectTab === "loadtest") {
        loadTestStartRef.current?.();
      } else {
        executeSinglePipeline(idx);
      }
    }, 300);
  }, [autoRunPipelineId, autoSelectTab, pipelines]);

  // Auto-select tab from AI assistant (without auto-run)
  useEffect(() => {
    if (autoSelectTab && !autoRunPipelineId) {
      setActiveTab(autoSelectTab);
    }
  }, [autoSelectTab]);

  const [dragSourceIndex, setDragSourceIndex] = useState<number | null>(null);
  const [dragTargetIndex, setDragTargetIndex] = useState<number | null>(null);

  const handleDragStart = useCallback((index: number) => {
    setDragSourceIndex(index);
  }, []);

  const handleDragOver = useCallback((index: number) => {
    setDragTargetIndex(index);
  }, []);

  const handleDrop = useCallback((targetIndex: number) => {
    if (dragSourceIndex === null || dragSourceIndex === targetIndex) {
      setDragSourceIndex(null);
      setDragTargetIndex(null);
      return;
    }
    const newOrder = [...orderedPipelines];
    const [moved] = newOrder.splice(dragSourceIndex, 1);
    newOrder.splice(targetIndex, 0, moved);
    if (activeContextId) {
      savePipelineOrder(activeContextId, newOrder.map((p) => p.id));
    }
    // Update selected index to follow the moved item
    if (selectedIndex === dragSourceIndex) {
      setSelectedIndex(targetIndex);
      onSelectPipeline?.(targetIndex);
    } else if (selectedIndex !== null) {
      // Find where the currently selected pipeline ended up
      const selectedPipelineId = orderedPipelines[selectedIndex]?.id;
      const newIdx = newOrder.findIndex((p) => p.id === selectedPipelineId);
      if (newIdx >= 0 && newIdx !== selectedIndex) {
        setSelectedIndex(newIdx);
        onSelectPipeline?.(newIdx);
      }
    }
    setDragSourceIndex(null);
    setDragTargetIndex(null);
  }, [dragSourceIndex, orderedPipelines, activeContextId, selectedIndex, onSelectPipeline]);

  const selectedPipeline = selectedIndex !== null && selectedIndex < orderedPipelines.length ? orderedPipelines[selectedIndex] : null;

  // Error step navigation
  const failedStepIds = useMemo(() => {
    if (!selectedPipeline) return [];
    return selectedPipeline.steps
      .filter((step) => results[step.id]?.status === "error")
      .map((step) => step.id);
  }, [selectedPipeline, results]);

  const anyErrorVisible = useStepVisibility(stepScrollContainerRef, failedStepIds);
  const [errorNavIndex, setErrorNavIndex] = useState(0);
  useEffect(() => setErrorNavIndex(0), [failedStepIds.length]);

  const scrollToNextError = useCallback(() => {
    if (failedStepIds.length === 0) return;
    const idx = errorNavIndex % failedStepIds.length;
    const stepId = failedStepIds[idx];
    const el = document.querySelector(`[data-step-id="${stepId}"]`);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
      el.classList.add("ring-2", "ring-destructive/50", "rounded-xl");
      setTimeout(() => el.classList.remove("ring-2", "ring-destructive/50", "rounded-xl"), 1500);
    }
    setErrorNavIndex(idx + 1);
  }, [failedStepIds, errorNavIndex]);

  const executeSinglePipeline = useCallback(async (index: number): Promise<"success" | "error"> => {
    const pipeline = orderedPipelines[index];
    if (!pipeline) return "error";
    console.log("[DEBUG][executeSinglePipeline] pipeline to execute", {
      pipelineId: pipeline.id,
      pipelineName: pipeline.name,
      firstStepAsserts: pipeline.steps[0]?.asserts,
      timestamp: Date.now(),
    });
    const prevIndex = selectedIndex;
    setSelectedIndex(index);
    // Only navigate if index actually changed — avoids unnecessary doLoad cascade
    if (index !== prevIndex) {
      onSelectPipeline?.(index);
    }

    const status = await runExecutionTest(pipeline, index, projectId, executionBackendUrl, specs, envGroups, effectiveSelectedEnvGroupSlug);
    setChartRefreshKey(prev => prev + 1);
    return status;
  }, [orderedPipelines, projectId, executionBackendUrl, onSelectPipeline, runExecutionTest, specs, envGroups, effectiveSelectedEnvGroupSlug, selectedIndex]);

  const handleRun = useCallback(async () => {
    if (selectedIndex === null) return;
    await executeSinglePipeline(selectedIndex);
  }, [selectedIndex, executeSinglePipeline]);

  const handleRerunFromStep = useCallback(async (stepId: string) => {
    if (!selectedPipeline || selectedIndex === null) return;
    if (!executionBackendUrl) {
      toast.error(t("testExecution.configureServerUrl"));
      return;
    }
    await rerunExecutionFromStep(
      selectedPipeline,
      selectedIndex,
      projectId,
      stepId,
      executionBackendUrl,
      specs,
      envGroups,
      effectiveSelectedEnvGroupSlug,
    );
    setChartRefreshKey((prev) => prev + 1);
  }, [selectedPipeline, selectedIndex, executionBackendUrl, t, rerunExecutionFromStep, projectId, specs, envGroups, effectiveSelectedEnvGroupSlug]);

  // Recover active queue on mount
  useEffect(() => {
    if (!executionBackendUrl || batchState !== "idle") return;
    let cancelled = false;
    (async () => {
      try {
        const activeQueue = await apiClient.getCurrentE2eQueue(executionBackendUrl, projectId);
        if (cancelled || !activeQueue) return;
        if (activeQueue.status === "pending" || activeQueue.status === "running") {
          await handleQueueSnapshot(activeQueue);
          connectQueueStream(activeQueue.id);
        }
      } catch (err) {
        console.error("Failed to check active queue:", err);
      }
    })();
    return () => { cancelled = true; };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [executionBackendUrl, projectId]);

  const handleRunAll = useCallback(async () => {
    if (!executionBackendUrl) {
      toast.error(t("testExecution.configureServerUrl"));
      return;
    }

    // Determine which pipelines to run
    let pipelinesToRun: Pipeline[];
    if (showBatchCheckboxes) {
      const indices = orderedPipelines.map((_, i) => i).filter(i => selectedForBatch.has(i));
      if (indices.length === 0) return;
      pipelinesToRun = indices.map(i => orderedPipelines[i]).filter(Boolean);
    } else {
      pipelinesToRun = [...orderedPipelines];
    }

    const pipelineIds = pipelinesToRun.map(p => p.id).filter((id): id is string => !!id);
    if (pipelineIds.length === 0) return;

    setShowBatchCheckboxes(false);
    setBatchProgress(0);
    setBatchTotal(pipelineIds.length);
    setBatchState("running");
    setQueuePipelines([]);

    try {
      const runtimeSpecs = specs?.map(s => ({ slug: s.slug, servers: s.servers }));
      const queueRecord = await apiClient.createE2eQueue(executionBackendUrl, projectId, {
        pipelineIds,
        specs: runtimeSpecs,
        envGroups: runtimeEnvGroups,
        selectedEnvGroupSlug: effectiveSelectedEnvGroupSlug,
      });
      await handleQueueSnapshot(queueRecord);
      connectQueueStream(queueRecord.id);
    } catch (err) {
      console.error("Failed to create queue:", err);
      toast.error(t("batch.createError", "Erro ao criar fila de execução"));
      setBatchState("idle");
      setBatchTotal(0);
    }
  }, [orderedPipelines, selectedForBatch, showBatchCheckboxes, executionBackendUrl, projectId, specs, runtimeEnvGroups, effectiveSelectedEnvGroupSlug, selectedIndex, t]);

  const handleToggleBatchCheck = useCallback((index: number) => {
    setSelectedForBatch(prev => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index); else next.add(index);
      // If none left, hide checkboxes
      if (next.size === 0) setShowBatchCheckboxes(false);
      return next;
    });
  }, []);

  const handleToggleAllBatchCheck = useCallback(() => {
    if (showBatchCheckboxes) {
      setShowBatchCheckboxes(false);
      setSelectedForBatch(new Set(orderedPipelines.map((_, i) => i)));
    } else {
      setShowBatchCheckboxes(true);
      setSelectedForBatch(new Set(orderedPipelines.map((_, i) => i)));
    }
  }, [showBatchCheckboxes, orderedPipelines]);

  const handleBatchPause = useCallback(() => {}, []);
  const handleBatchResume = useCallback(() => {}, []);
  const handleBatchCancel = useCallback(async () => {
    // Cancel server-side queue
    if (queueId && executionBackendUrl) {
      try {
        await apiClient.deleteE2eQueue(executionBackendUrl, projectId, queueId);
      } catch (err) {
        console.error("Failed to cancel queue:", err);
      }
    }
    clearQueueTransport();
    setBatchState("idle");
    setBatchQueue([]);
    setBatchProgress(0);
    setBatchTotal(0);
    setQueueId(null);
    setQueuePipelines([]);
    for (const qp of queuePipelines) {
      const pipelineIdx = orderedPipelines.findIndex((pipeline) => pipeline.id === qp.id);
      if (pipelineIdx >= 0 && (qp.status === "pending" || qp.status === "running" || qp.status === "cancelled")) {
        clearPipelineStatus(pipelineIdx);
      }
    }
  }, [clearPipelineStatus, clearQueueTransport, queueId, executionBackendUrl, projectId, queuePipelines, orderedPipelines]);

  // Cleanup streams on unmount
  useEffect(() => {
    return () => {
      clearQueueTransport();
      disconnectExecutionController();
      disconnectLoadTestController();
    };
  }, [clearQueueTransport, disconnectExecutionController, disconnectLoadTestController]);

  const handleSelectRun = (run: ExecutionRun) => {
    selectExecutionRun(run, executionBackendUrl);
  };

  const isBatchActive = batchState !== "idle";

  const handleSelectPipeline = (i: number, event?: React.MouseEvent) => {
    if (event && (event.ctrlKey || event.metaKey)) {
      // Ctrl+click: enter selection mode with only this item
      if (!showBatchCheckboxes) {
        setShowBatchCheckboxes(true);
        setSelectedForBatch(new Set([i]));
      } else {
        // Already in selection mode — toggle this item
        setSelectedForBatch(prev => {
          const next = new Set(prev);
          if (next.has(i)) next.delete(i); else next.add(i);
          if (next.size === 0) setShowBatchCheckboxes(false);
          return next;
        });
      }
      return;
    }
    setSelectedIndex(i);
    const selectedPipelineId = orderedPipelines[i]?.id;
    const originalIndex = selectedPipelineId
      ? pipelines.findIndex((pipeline) => pipeline.id === selectedPipelineId)
      : i;
    onSelectPipeline?.(originalIndex >= 0 ? originalIndex : i);
    if (isMobile) setSidebarOpen(false);
  };

  const [confirmClearOpen, setConfirmClearOpen] = useState(false);

  const handleClearHistory = useCallback(async () => {
    if (selectedIndex === null) return;
    try {
      if (executionBackendUrl) {
        await apiClient.deleteIntegrationHistory(executionBackendUrl, projectId, { pipelineIndex: selectedIndex });
        setExecutionRuns([]);
      } else {
        await deleteLocalExecutionRunsForPipeline(projectId, selectedIndex);
      }
      clearExecutionResults();
      clearPipelineStatus(selectedIndex);
    } catch (e) {
      console.error("Failed to clear history:", e);
      toast.error(t("testExecution.clearHistoryError"));
    }
  }, [selectedIndex, projectId, executionBackendUrl, t, deleteLocalExecutionRunsForPipeline, clearExecutionResults, clearPipelineStatus, setExecutionRuns]);

  const pipelineNamesMap = useMemo(() => {
    const map: Record<string, string> = {};
    for (const p of orderedPipelines) {
      if (p.id) map[p.id] = p.name;
    }
    return map;
  }, [orderedPipelines]);

  const effectivePipelineStatuses = useMemo(() => {
    const merged = { ...pipelineStatuses } as Record<number, "success" | "error" | "running" | "queued">;

    for (const qp of queuePipelines) {
      const pipelineIdx = orderedPipelines.findIndex((pipeline) => pipeline.id === qp.id);
      if (pipelineIdx < 0) continue;

      if (qp.status === "completed") {
        merged[pipelineIdx] = "success";
      } else if (qp.status === "failed") {
        merged[pipelineIdx] = "error";
      } else if (qp.status === "running") {
        merged[pipelineIdx] = "running";
      } else if (qp.status === "pending") {
        merged[pipelineIdx] = "queued";
      } else {
        delete merged[pipelineIdx];
      }
    }

    return merged;
  }, [pipelineStatuses, queuePipelines, orderedPipelines]);

  const sidebarProps = {
    spec, specs, envGroups, pipelines: orderedPipelines, selectedIndex, pipelineStatuses: effectivePipelineStatuses, running, isBatchActive, batchState,
    batchProgress, batchTotal, onEditSpec, onDeleteSpec, onCreatePipeline, onCreateAIPipeline,
    onCreateEnvGroup, onUpdateEnvGroup, onDeleteEnvGroup,
    onSelect: handleSelectPipeline, onEdit: onEditPipeline, onDuplicate: onDuplicatePipeline, onDelete: onDeletePipeline,
    handleRunAll, handleBatchPause, handleBatchResume, handleBatchCancel, executionBackendUrl,
    dragTargetIndex, onDragStart: handleDragStart, onDragOver: handleDragOver, onDrop: handleDrop,
    selectedForBatch, onToggleBatchCheck: handleToggleBatchCheck, onToggleAllBatchCheck: handleToggleAllBatchCheck,
    showBatchCheckboxes,
    queuePipelines,
    pipelineNames: pipelineNamesMap,
    experimentalFeaturesEnabled,
  };

  /* ── Main content (right panel) ── */
  const mainContent = (
    <>
      {pipelines.length === 0 ? (
        <EmptyState
          icon={Workflow}
          title={t("testExecution.noPipeline.title")}
          description={t("testExecution.noPipeline.description")}
          action={{ label: t("testExecution.noPipeline.action"), icon: <Plus className="h-4 w-4" />, onClick: onCreatePipeline }}
        />
      ) : selectedPipeline ? (
        <Tabs defaultValue="integration" value={activeTab} onValueChange={(v) => {
          setActiveTab(v);
          if (selectedIndex !== null) {
            onSelectPipeline?.(selectedIndex);
          }
          setTimeout(() => onTabChange?.(v as "integration" | "loadtest"), 0);
        }} className="flex flex-1 flex-col overflow-hidden">
          <div className="glass px-3 sm:px-4 py-3">
            <div className="flex items-center gap-3 border-border/50">
              <div className="min-w-0 flex-1">
                <h2 className="font-semibold truncate">{selectedPipeline.name}</h2>
                <p className="text-xs text-muted-foreground truncate">{selectedPipeline.description}</p>
              </div>
              {activeTab === "integration" && executionNode && (
                <span className="flex items-center gap-1.5 text-xs text-muted-foreground rounded-md px-2 py-1 shrink-0">
                  <Server className="h-3 w-3" />
                  <span className="font-mono">{executionNode}</span>
                </span>
              )}
              {activeTab === "integration" && (
                <Button
                  variant="outline"
                  size="icon"
                  className="h-8 w-8 shrink-0"
                  onClick={() => useStepViewStore.getState().setMode(stepViewMode === "graph" ? "list" : "graph")}
                  title={stepViewMode === "graph" ? "List view" : "Grid view"}
                >
                  {stepViewMode === "graph" ? <ListOrdered className="h-3.5 w-3.5" /> : <LayoutGrid className="h-3.5 w-3.5" />}
                </Button>
              )}
              {envGroups.length > 0 && (
                <Select value={effectiveSelectedEnvGroupSlug ?? undefined} onValueChange={(value) => setSelectedEnvGroupSlug(value)}>
                  <SelectTrigger className="h-8 w-[140px] shrink-0 text-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent
                    viewportClassName="!h-auto max-h-[6.5rem] overflow-y-auto"
                    viewportStyle={{ height: "auto", maxHeight: "6.5rem", overflowY: "auto" }}
                  >
                    {envGroups.map((group) => (
                      <SelectItem key={group.id} value={group.slug}>
                        {group.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
              {activeTab === "integration" ? (
                <Button onClick={handleRun} disabled={running || isBatchActive || !executionBackendUrl} size="sm" className="shrink-0" title={!executionBackendUrl ? t("testExecution.configureServerUrlSettings") : undefined}>
                  {running ? (
                    <><DotsLoader /> Running…</>
                  ) : (
                    <><Play className="h-3 w-3" /> Run</>
                  )}
                </Button>
              ) : loadTestState === "running" ? (
                <Button onClick={() => loadTestCancelRef.current?.()} variant="destructive" size="sm" className="shrink-0">
                  <Square className="h-3 w-3" /> Stop
                </Button>
              ) : loadTestState !== "idle" ? (
                <Button onClick={() => loadTestResetRef.current?.()} variant="outline" size="sm" className="shrink-0">
                  <RotateCcw className="h-3 w-3" /> New Test
                </Button>
              ) : (
                <Button onClick={() => loadTestStartRef.current?.()} size="sm" className="shrink-0" disabled={!executionBackendUrl} title={!executionBackendUrl ? t("testExecution.configureServerUrlSettings") : undefined}>
                  <Zap className="h-3 w-3" /> Start
                </Button>
              )}
            </div>
          </div>

          <div className={cn("flex flex-1 min-h-0 overflow-hidden", isMobile ? "flex-col" : "flex-row")}>
            <TestModeSidebar
              compact={isMobile}
              collapsed={!isMobile && testModeSidebarCollapsed}
              onCollapsedChange={setTestModeSidebarCollapsed}
            />
            <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
              <TabsContent value="integration" className="flex-1 flex flex-col overflow-hidden mt-0 !animate-none min-w-0">
                {selectedPipeline.steps.length === 0 ? (
                  <EmptyState
                    icon={Plus}
                    title={t("testExecution.noSteps.title")}
                    description={t("testExecution.noSteps.integration.description")}
                    action={onEditPipeline ? {
                      label: t("testExecution.noSteps.action"),
                      icon: <FileCode2 className="h-4 w-4" />,
                      onClick: () => onEditPipeline(selectedIndex!),
                    } : undefined}
                  />
                ) : (
                  <>
                    <div>
                      {selectedIndex !== null && !isMobile && (
                        <PipelineMiniChart
                          projectId={projectId}
                          pipelineIndex={selectedIndex}
                          refreshKey={chartRefreshKey}
                          executionBackendUrl={executionBackendUrl}
                        />
                      )}
                    </div>

                    {isMobile ? (
                      <div className="relative flex-1 overflow-auto">
                        <div className="p-3">
                          <StepFlowList
                            items={selectedPipeline.steps.map((step, idx) => {
                              const prevStep = idx > 0 ? selectedPipeline.steps[idx - 1] : null;
                              const prevStatus = prevStep ? results[prevStep.id]?.status : null;
                              const shouldCountdown = step.delay && step.delay > 0 && (
                                idx === 0 ? running : (prevStatus === "success" || prevStatus === "error")
                              ) && (results[step.id]?.status === "pending" || results[step.id]?.status === "running" || !results[step.id]);
                              return {
                                key: step.id,
                                content: <div data-step-id={step.id}><StepResultCard step={step} result={results[step.id]} shouldCountdown={!!shouldCountdown} onAnalyzeWithAI={experimentalFeaturesEnabled ? onAnalyzeStepWithAI : undefined} onGoToCode={onEditPipeline ? handleGoToCode : undefined} onRerunFromStep={handleRerunFromStep} canRerunFromStep={!running && !isBatchActive && !!executionBackendUrl} /></div>,
                              };
                            })}
                          />
                          {runHistory.length > 0 && (
                            <div
                              data-testid="mobile-integration-history"
                              className={cn(
                                "mt-6 border-t border-border/50 transition-[max-height] duration-300 ease-in-out overflow-hidden",
                                historyCollapsed ? "max-h-10 cursor-pointer" : "max-h-[250px]"
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
                                <div className="h-[250px]">
                                  <RunHistoryPanel
                                    onClear={() => setConfirmClearOpen(true)}
                                    isEmpty={runHistory.length === 0}
                                    onCollapse={() => setHistoryCollapsed(true)}
                                    collapsed={false}
                                    collapseDirection="bottom"
                                    collapseOnHeaderClick
                                  >
                                    {runHistory.map((run) => (
                                      <RunHistoryItem key={run.id} run={run} isActive={activeRunId === run.id} onClick={() => handleSelectRun(run)} />
                                    ))}
                                  </RunHistoryPanel>
                                </div>
                              )}
                            </div>
                          )}
                        </div>
                        {failedStepIds.length > 0 && !anyErrorVisible ? (
                          <Button
                            size="sm"
                            variant="destructive"
                            className="fixed bottom-4 left-0 right-0 mx-auto w-fit z-50 gap-1.5 shadow-lg"
                            onClick={scrollToNextError}
                          >
                            <ArrowDown className="h-3.5 w-3.5" />
                            Next error ({(errorNavIndex % failedStepIds.length) + 1}/{failedStepIds.length})
                          </Button>
                        ) : showGoToButton ? (
                          <Button
                            size="sm"
                            variant="default"
                            className="fixed bottom-4 left-0 right-0 mx-auto w-fit z-50 gap-1.5 shadow-lg animate-fade-in bg-primary text-primary-foreground hover:bg-primary/90"
                            onClick={goToRunningStep}
                          >
                            <MousePointerClick className="h-3.5 w-3.5" />
                            {t("testExecution.goToRunning", "Go to running step")}
                          </Button>
                        ) : null}
                      </div>
                    ) : (
                      <div className="relative flex-1 overflow-hidden flex min-w-0">
                        <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
                          <div ref={stepScrollContainerRef} className="h-full overflow-auto p-4 min-w-0">
                            {stepViewMode === "graph" ? (
                              <StepFlowGraph
                                items={selectedPipeline.steps.map((step, idx) => {
                                  const prevStep = idx > 0 ? selectedPipeline.steps[idx - 1] : null;
                                  const prevStatus = prevStep ? results[prevStep.id]?.status : null;
                                  const shouldCountdown = step.delay && step.delay > 0 && (
                                    idx === 0 ? running : (prevStatus === "success" || prevStatus === "error")
                                  ) && (results[step.id]?.status === "pending" || results[step.id]?.status === "running" || !results[step.id]);
                                  return {
                                    key: step.id,
                                    status: results[step.id]?.status,
                                    content: <div data-step-id={step.id} className="h-full"><StepResultCard variant="grid" step={step} result={results[step.id]} shouldCountdown={!!shouldCountdown} onAnalyzeWithAI={experimentalFeaturesEnabled ? onAnalyzeStepWithAI : undefined} onGoToCode={onEditPipeline ? handleGoToCode : undefined} onRerunFromStep={handleRerunFromStep} canRerunFromStep={!running && !isBatchActive && !!executionBackendUrl} /></div>,
                                  };
                                })}
                              />
                            ) : (
                              <StepFlowList
                                items={selectedPipeline.steps.map((step, idx) => {
                                  const prevStep = idx > 0 ? selectedPipeline.steps[idx - 1] : null;
                                  const prevStatus = prevStep ? results[prevStep.id]?.status : null;
                                  const shouldCountdown = step.delay && step.delay > 0 && (
                                    idx === 0 ? running : (prevStatus === "success" || prevStatus === "error")
                                  ) && (results[step.id]?.status === "pending" || results[step.id]?.status === "running" || !results[step.id]);
                                  return {
                                    key: step.id,
                                    content: <div data-step-id={step.id}><StepResultCard step={step} result={results[step.id]} shouldCountdown={!!shouldCountdown} onAnalyzeWithAI={experimentalFeaturesEnabled ? onAnalyzeStepWithAI : undefined} onGoToCode={onEditPipeline ? handleGoToCode : undefined} onRerunFromStep={handleRerunFromStep} canRerunFromStep={!running && !isBatchActive && !!executionBackendUrl} /></div>,
                                  };
                                })}
                              />
                            )}
                          </div>
                        </div>

                        {runHistory.length > 0 && (
                          <div
                            className={cn(
                              "shrink-0 border-l border-border/50 flex flex-col transition-[width] duration-300 ease-in-out overflow-hidden",
                              historyCollapsed ? "w-8" : "w-[260px]"
                            )}
                          >
                            {historyCollapsed ? (
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
                            ) : (
                              <div className="flex-1 min-h-0 min-w-[260px]">
                                <RunHistoryPanel
                                  onClear={() => setConfirmClearOpen(true)}
                                  isEmpty={runHistory.length === 0}
                                  onCollapse={() => setHistoryCollapsed(true)}
                                  collapsed={false}
                                >
                                  {runHistory.map((run) => (
                                    <RunHistoryItem key={run.id} run={run} isActive={activeRunId === run.id} onClick={() => handleSelectRun(run)} />
                                  ))}
                                </RunHistoryPanel>
                              </div>
                            )}
                          </div>
                        )}
                        {failedStepIds.length > 0 && !anyErrorVisible ? (
                          <Button
                            size="sm"
                            variant="destructive"
                            className="fixed bottom-4 left-0 right-0 mx-auto w-fit z-50 gap-1.5 shadow-lg"
                            onClick={scrollToNextError}
                          >
                            <ArrowDown className="h-3.5 w-3.5" />
                            Next error ({(errorNavIndex % failedStepIds.length) + 1}/{failedStepIds.length})
                          </Button>
                        ) : showGoToButton ? (
                          <Button
                            size="sm"
                            variant="default"
                            className="fixed bottom-4 left-0 right-0 mx-auto w-fit z-50 gap-1.5 shadow-lg animate-fade-in bg-primary text-primary-foreground hover:bg-primary/90"
                            onClick={goToRunningStep}
                          >
                            <MousePointerClick className="h-3.5 w-3.5" />
                            {t("testExecution.goToRunning", "Go to running step")}
                          </Button>
                        ) : null}
                      </div>
                    )}
                  </>
                )}
              </TabsContent>

              <TabsContent value="loadtest" className="flex-1 flex flex-col overflow-hidden mt-0 !animate-none">
                {selectedPipeline.steps.length === 0 ? (
                  <EmptyState
                    icon={Plus}
                    title={t("testExecution.noSteps.title")}
                    description={t("testExecution.noSteps.loadtest.description")}
                    action={onEditPipeline ? {
                      label: t("testExecution.noSteps.action"),
                      icon: <FileCode2 className="h-4 w-4" />,
                      onClick: () => onEditPipeline(selectedIndex!),
                    } : undefined}
                  />
                ) : (
                  <>
                    <LoadTestTab
                      pipeline={selectedPipeline}
                      projectId={projectId}
                      pipelineIndex={selectedIndex!}
                      executionBackendUrl={executionBackendUrl}
                      specs={specs}
                      envGroups={envGroups}
                      selectedEnvGroupSlug={effectiveSelectedEnvGroupSlug}
                      onStateChange={handleLoadTestStateChange}
                      onResetRef={loadTestResetRef}
                      onCancelRef={loadTestCancelRef}
                      onStartRef={loadTestStartRef}
                    />
                  </>
                )}
              </TabsContent>
            </div>
          </div>
        </Tabs>
      ) : (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          {t("testExecution.selectPipeline")}
        </div>
      )}
    </>
  );

  /* ── Mobile layout ── */
  if (isMobile) {
    return (
      <div className="flex flex-1 flex-col overflow-hidden">
        <div className="flex items-center gap-2 border-border/50 px-3 py-2 bg-card/40 ">
          <Sheet open={sidebarOpen} onOpenChange={setSidebarOpen}>
            <SheetTrigger asChild>
              <Button variant="ghost" size="icon" className="h-8 w-8">
                <Menu className="h-4 w-4" />
              </Button>
            </SheetTrigger>
            <SheetContent side="left" className="w-screen max-w-none p-0 flex flex-col sm:max-w-none">
              <SheetHeader className="px-4 py-3 border-border/50">
                <SheetTitle className="text-sm">{t("testExecution.navigation")}</SheetTitle>
              </SheetHeader>
              <SidebarContent {...sidebarProps} />
            </SheetContent>
          </Sheet>
          <span className="text-sm font-medium truncate">
            {selectedPipeline?.name || t("testExecution.selectPipeline")}
          </span>
        </div>
        <div className="flex flex-1 flex-col overflow-hidden">
          {mainContent}
        </div>
      </div>
    );
  }

  /* ── Desktop layout ── */
  return (
    <>
      <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 overflow-hidden">
        <ResizablePanel defaultSize={20} minSize={15} className="glass flex min-h-0 flex-col overflow-hidden">
          <SidebarContent {...sidebarProps} />
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize={80} minSize={30} className="flex min-h-0 min-w-0 flex-col overflow-hidden">
          {mainContent}
        </ResizablePanel>
      </ResizablePanelGroup>
      <ConfirmDialog
        open={confirmClearOpen}
        onOpenChange={setConfirmClearOpen}
        onConfirm={handleClearHistory}
        title={t("history.clearTitle", "Clear History")}
        description={t("history.clearDescription", "Are you sure you want to delete all run history for this pipeline? This action cannot be undone.")}
        confirmLabel={t("common.delete", "Delete")}
        variant="destructive"
      />
    </>
  );
}
