import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { useParams, useNavigate, useLocation } from "react-router-dom";
import { cn } from "@/lib/utils";
import { useIsMobile } from "@/hooks/use-mobile";
import { useTranslation } from "react-i18next";
import { useAppHeader } from "@/components/AppShell";
import { ProjectSettingsDialog } from "@/components/ProjectSettingsDialog";
import PipelineCreatorPage from "@/pages/PipelineCreatorPage";
import RouteEditorPage from "@/pages/RouteEditorPage";
import TestExecutionPage from "@/pages/TestExecutionPage";
import DashboardPage from "@/pages/DashboardPage";
import { AIPipelineChat, type AIChatRef } from "@/components/AIPipelineChat";
import { SplitPaneLayout } from "@/components/SplitPaneLayout";
import { useProjectStore } from "@/stores/useProjectStore";
import { useOrchestratorStore, getApiUrl } from "@/stores/useOrchestratorStore";
import { useThemeStore } from "@/stores/useThemeStore";
import { useExecutionHistoryStore } from "@/stores/useExecutionHistoryStore";
import { useChatPositionStore } from "@/stores/useChatPositionStore";
import { useOpenAIKeyStore } from "@/stores/useOpenAIKeyStore";
import { useExperimentalFeaturesEnabled } from "@/stores/useExperimentalFeaturesStore";

import * as api from "@/lib/api-client";
import { generateUUID } from "@/lib/uuid";
import type { Pipeline } from "@/types/pipeline";
import { type Project, getMergedSpec } from "@/types/project";
import { toast } from "sonner";
import { parseOpenAPISpec } from "@/lib/openapi-parser";
import { MessageSquare, Monitor } from "lucide-react";
import { Button } from "@/components/ui/button";

let _skipNextLoad = false;

export default function ProjectFlowPage() {
  const { t } = useTranslation();
  const { id, pipelineId, specId } = useParams<{ id: string; pipelineId?: string; specId?: string }>();
  const navigate = useNavigate();
  const location = useLocation();
  const isDashboardRoute = location.pathname.includes("/dashboard");
  const isPipelineEditorRoute = location.pathname.endsWith("/editor") && (!!pipelineId || location.pathname.includes("/pipeline/new/"));
  const isSpecEditorRoute = (location.pathname.endsWith("/editor") || location.pathname.endsWith("/try-it")) && (!!specId || location.pathname.includes("/specs/new/"));
  const initialTab: "integration" | "loadtest" = location.pathname.endsWith("/load-test") ? "loadtest" : "integration";

  const view = isPipelineEditorRoute ? "create-pipeline" : isSpecEditorRoute ? "routes" : "execute";

  const { currentProject: project, setCurrentProject, updateProject: storeUpdateProject, loadProject, saveProjectSpec, saveProjectPipelines, addSpec, updateSpec: storeUpdateSpec, removeSpec, createEnvGroup, updateEnvGroup, deleteEnvGroup } = useProjectStore();
  const orchUrl = useOrchestratorStore((s) => s.url);
  const isDark = useThemeStore((s) => s.theme === "dark");
  const deleteLocalRunsForPipeline = useExecutionHistoryStore((s) => s.deleteLocalRunsForPipeline);
  const chatPosition = useChatPositionStore((s) => s.position);
  const chatCollapsed = useChatPositionStore((s) => s.collapsed);
  const toggleChatCollapsed = useChatPositionStore((s) => s.toggleCollapsed);
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const hasApiKey = useOpenAIKeyStore((s) => !!s.apiKey?.trim());
  const aiAssistantAvailable = experimentalFeaturesEnabled && hasApiKey;
  const isMobile = useIsMobile();
  const backendUrl = orchUrl || undefined;

  const chatRef = useRef<AIChatRef>(null);

  const [aiAutoRunId, setAiAutoRunId] = useState<string | null>(null);
  const [aiAutoTab, setAiAutoTab] = useState<"integration" | "loadtest" | null>(null);
  const [newPipelineTemplate, setNewPipelineTemplate] = useState<Pipeline | null>(null);
  const [editingPipelineIndex, setEditingPipelineIndex] = useState<number | null>(null);
  const [mobileView, setMobileView] = useState<"app" | "chat">("app");

  const [chatMounted, setChatMounted] = useState(!chatCollapsed && aiAssistantAvailable);
  const [isClosing, setIsClosing] = useState(false);
  const [isOpening, setIsOpening] = useState(false);

  useEffect(() => {
    if (!chatCollapsed && aiAssistantAvailable) {
      setChatMounted(true);
      setIsOpening(true);
      requestAnimationFrame(() => {
        requestAnimationFrame(() => setIsOpening(false));
      });
    } else if (chatMounted) {
      setIsClosing(true);
      const timer = setTimeout(() => {
        setChatMounted(false);
        setIsClosing(false);
      }, 300);
      return () => clearTimeout(timer);
    }
  }, [aiAssistantAvailable, chatCollapsed, chatMounted]);

  useEffect(() => {
    if (!id) { navigate("/"); return; }

    if (_skipNextLoad) {
      _skipNextLoad = false;
      return;
    }

    const doLoad = async () => {
      const loaded = await loadProject(id);
      if (!loaded) { navigate("/"); return; }
      setCurrentProject(loaded);
    };

    doLoad();
  }, [id, location.key, navigate, loadProject, setCurrentProject, storeUpdateProject, saveProjectPipelines]);

  const basePath = `/projects/${id}`;
  const selectedIndex = project && pipelineId
    ? project.pipelines.findIndex(p => p.id === pipelineId)
    : null;
  const selectedPipelineName = selectedIndex !== null && selectedIndex >= 0
    ? project?.pipelines[selectedIndex]?.name
    : undefined;

  const handleBackToProjects = useCallback(() => {
    navigate("/");
  }, [navigate]);

  const handleStackDashboardOpen = useCallback(() => {
    if (!id) return;
    navigate(`${basePath}/dashboard`);
  }, [basePath, id, navigate]);

  const headerActions = useMemo(() => <ProjectSettingsDialog />, []);

  useAppHeader({
    projectName: project?.name,
    pipelineName: selectedPipelineName,
    onBackToProjects: handleBackToProjects,
    onDashboard: id ? handleStackDashboardOpen : undefined,
    isDashboardActive: isDashboardRoute,
    headerActions,
  });

  const syncProjectPipelinesInMemory = useCallback((updatedPipelines: Pipeline[], updatedAt = new Date().toISOString()) => {
    if (!project) return;
    const updatedProject = { ...project, pipelines: updatedPipelines, updatedAt };
    setCurrentProject(updatedProject);
    useProjectStore.setState((state) => ({
      projects: state.projects.map((p) => (p.id === project.id ? updatedProject : p)),
    }));
  }, [project, setCurrentProject]);

  useEffect(() => {
    if (selectedIndex != null && selectedIndex >= 0 && project?.pipelines[selectedIndex]) {
      const tab = initialTab === "loadtest" ? "load" : "e2e";
      chatRef.current?.setActiveContext(project.pipelines[selectedIndex], tab);
    }
  }, [selectedIndex, initialTab, project?.pipelines]);

  const updateProject = useCallback((updates: Partial<Project>) => {
    if (!project || !id) return;
    const updated = { ...project, ...updates, updatedAt: new Date().toISOString() };
    setCurrentProject(updated);
    storeUpdateProject(id, updates);
  }, [project, id, setCurrentProject, storeUpdateProject]);

  const handleSaveAndRun = useCallback(async (pipeline: Pipeline) => {
    if (!project) return;
    const apiUrl = getApiUrl();
    const candidateId = pipeline.id || newPipelineTemplate?.id || pipelineId || generateUUID();
    const isUpdate = project.pipelines.some(p => p.id === candidateId)
      || (newPipelineTemplate?.id === candidateId);

    let canonicalId = candidateId;

    if (apiUrl) {
      try {
        if (isUpdate) {
          try {
            await api.upsertPipeline(apiUrl, project.id, candidateId, pipeline);
          } catch (putErr: any) {
            const status = putErr?.response?.status ?? putErr?.status;
            if (status === 404) {
              console.warn("PUT 404 fallback → POST");
              const created = await api.createPipeline(apiUrl, project.id, { ...pipeline, id: undefined });
              canonicalId = created.id ?? canonicalId;
            } else {
              throw putErr;
            }
          }
        } else {
          const created = await api.createPipeline(apiUrl, project.id, { ...pipeline, id: candidateId });
          canonicalId = created.id ?? candidateId;
        }
      } catch (err) {
        console.warn("Failed to save pipeline on backend:", err);
        toast.error(t("projectFlow.backendSaveError"));
        throw err;
      }
    }

    const pipelineWithId = { ...pipeline, id: canonicalId };

    let updatedPipelines: Pipeline[];
    const existingIndex = project.pipelines.findIndex(p => p.id === candidateId || p.id === canonicalId);
    if (existingIndex >= 0) {
      updatedPipelines = [...project.pipelines];
      updatedPipelines[existingIndex] = pipelineWithId;
    } else {
      updatedPipelines = [...project.pipelines, pipelineWithId];
    }

    syncProjectPipelinesInMemory(updatedPipelines);

    _skipNextLoad = true;
    navigate(`${basePath}/pipeline/${canonicalId}/integration-test`);
    setEditingPipelineIndex(null);
  }, [basePath, navigate, project, newPipelineTemplate?.id, pipelineId, syncProjectPipelinesInMemory, t]);

  const handleDeletePipeline = async (index: number) => {
    if (!project) return;
    const deletedPipeline = project.pipelines[index];
    const updatedPipelines = project.pipelines.filter((_, i) => i !== index);

    const apiUrl = getApiUrl();
    if (apiUrl && deletedPipeline?.id) {
      try { await api.deletePipeline(apiUrl, project.id, deletedPipeline.id); }
      catch (err) { console.warn("Failed to delete pipeline from backend:", err); }
      syncProjectPipelinesInMemory(updatedPipelines);
    } else {
      const { updateProject: localUpdate } = await import("@/lib/project-db");
      await localUpdate(project.id, { pipelines: updatedPipelines });
      syncProjectPipelinesInMemory(updatedPipelines);
    }

    if (!apiUrl) {
      deleteLocalRunsForPipeline(project.id, index).catch(console.error);
    }
    if (selectedIndex === index) { _skipNextLoad = true; navigate(basePath); }
  };

  const handleCreatePipeline = async () => {
    if (!project) return;
    const apiUrl = getApiUrl();

    if (apiUrl) {
      try {
        const emptyPipeline: Pipeline = { name: "new_pipeline", description: "", steps: [] };
        const created = await api.createPipeline(apiUrl, project.id, emptyPipeline);
        syncProjectPipelinesInMemory([...project.pipelines, created]);
        setNewPipelineTemplate(created);
        setEditingPipelineIndex(null);
        navigate(`${basePath}/pipeline/${created.id}/editor`);
        return;
      } catch (err) { console.warn("Failed to create empty pipeline on backend:", err); toast.error(t("projectFlow.backendCreateError")); }
    }

    const localId = generateUUID();
    const defaultPipeline: Pipeline = { id: localId, name: "new_pipeline", description: "", steps: [] };
    const updatedPipelines = [...project.pipelines, defaultPipeline];

    const { updateProject: localUpdate } = await import("@/lib/project-db");
    await localUpdate(project.id, { pipelines: updatedPipelines });
    syncProjectPipelinesInMemory(updatedPipelines);

    setNewPipelineTemplate(null);
    setEditingPipelineIndex(updatedPipelines.length - 1);
    _skipNextLoad = true;
    navigate(`${basePath}/pipeline/${localId}/editor`);
  };

  const handleDuplicatePipeline = async (index: number) => {
    if (!project) return;
    const original = project.pipelines[index];
    if (!original) return;
    const localId = generateUUID();
    let duplicated: Pipeline = { ...JSON.parse(JSON.stringify(original)), id: localId, name: `${original.name}_copy` };

    const apiUrl = getApiUrl();
    if (apiUrl) {
      try {
        const created = await api.createPipeline(apiUrl, project.id, duplicated);
        duplicated = { ...duplicated, ...created, id: created.id ?? duplicated.id };
      }
      catch (err) { console.warn("Failed to duplicate pipeline on backend:", err); }
      syncProjectPipelinesInMemory([...project.pipelines, duplicated]);
    } else {
      const updatedPipelines = [...project.pipelines, duplicated];
      const { updateProject: localUpdate } = await import("@/lib/project-db");
      await localUpdate(project.id, { pipelines: updatedPipelines });
      syncProjectPipelinesInMemory(updatedPipelines);
    }
    toast.success(t("pipeline.duplicated"));
  };

  const handleEditPipeline = async (index: number) => {
    if (!project) return;
    const p = project.pipelines[index];
    setNewPipelineTemplate(null);
    setEditingPipelineIndex(index);
    const pid = p?.id ?? generateUUID();
    if (!p?.id) {
      const updatedPipelines = [...project.pipelines];
      updatedPipelines[index] = { ...p, id: pid };
      if (getApiUrl()) {
        syncProjectPipelinesInMemory(updatedPipelines, project.updatedAt);
      } else {
        const { updateProject: localUpdate } = await import("@/lib/project-db");
        await localUpdate(project.id, { pipelines: updatedPipelines });
        syncProjectPipelinesInMemory(updatedPipelines, project.updatedAt);
      }
    }
    const qs = window.location.search;
    navigate(`${basePath}/pipeline/${pid}/editor${qs}`);
  };

  const handleSelectPipeline = async (index: number) => {
    if (!project) return;
    const p = project.pipelines[index];
    const pid = p?.id ?? generateUUID();
    if (!p?.id) {
      const updatedPipelines = [...project.pipelines];
      updatedPipelines[index] = { ...p, id: pid };
      if (getApiUrl()) {
        syncProjectPipelinesInMemory(updatedPipelines, project.updatedAt);
      } else {
        const { updateProject: localUpdate } = await import("@/lib/project-db");
        await localUpdate(project.id, { pipelines: updatedPipelines });
        syncProjectPipelinesInMemory(updatedPipelines, project.updatedAt);
      }
    }
    const tabSuffix = location.pathname.endsWith("/load-test") ? "/load-test" : "/integration-test";
    navigate(`${basePath}/pipeline/${pid}${tabSuffix}`);
  };

  const handleTabChange = (tab: "integration" | "loadtest") => {
    if (!project || selectedIndex === null || selectedIndex < 0) return;
    const p = project.pipelines[selectedIndex];
    if (!p?.id) return;
    const suffix = tab === "loadtest" ? "/load-test" : "/integration-test";
    navigate(`${basePath}/pipeline/${p.id}${suffix}`, { replace: true });
  };

  const handleImportSpec = async (content: string) => {
    try {
      const spec = parseOpenAPISpec(content);
      await addSpec(project.id, spec);
    } catch (e) { console.error("Failed to parse OpenAPI spec:", e); }
  };

  const handleConfirmSpec = async (spec: import("@/types/pipeline").OpenAPISpec, slug: string, servers: Record<string, string>, url?: string, sync?: boolean) => {
    let savedSpecId: string | undefined;
    if (specId && specId !== "new") {
      await storeUpdateSpec(project.id, specId, spec, url, sync ?? false, slug, servers);
      savedSpecId = specId;
    } else {
      const result = await addSpec(project.id, spec, url, sync ?? false, slug, servers);
      savedSpecId = result?.id;
    }

    if (savedSpecId && url && sync) {
      const { useSpecSyncStore } = await import("@/stores/useSpecSyncStore");
      const updatedProject = useProjectStore.getState().currentProject;
      const savedSpec = updatedProject?.specs.find((s) => s.id === savedSpecId);
      const md5 = savedSpec?.specMd5;
      if (md5) useSpecSyncStore.getState().enableSync(project.id, savedSpecId, url, md5);
    } else if (savedSpecId && !sync) {
      const { useSpecSyncStore } = await import("@/stores/useSpecSyncStore");
      useSpecSyncStore.getState().disableSync(project.id, savedSpecId);
    }

    navigate(basePath);
  };

  const handleOpenRouteEditor = (editSpecId?: string) => {
    if (editSpecId) navigate(`${basePath}/specs/${editSpecId}/editor`);
    else navigate(`${basePath}/specs/new/editor`);
  };

  const handleDeleteSpec = async (deleteSpecId: string) => {
    if (!project) return;
    await removeSpec(project.id, deleteSpecId);
  };

  const handleAICreatePipeline = useCallback(() => {
    if (!experimentalFeaturesEnabled) return;
    chatRef.current?.sendCommand(
      "Analyze the available OpenAPI specs and suggest E2E test scenarios I should create. Use the suggest_scenarios tool to present the suggestions organized by resource/entity.",
      "Generate Pipelines"
    );
  }, [experimentalFeaturesEnabled]);

  if (!project) {
    return (
      <main className="flex h-full min-h-0 flex-1 items-center justify-center overflow-hidden bg-background">
        <p className="text-sm text-muted-foreground">{t("projectFlow.loadingProject")}</p>
      </main>
    );
  }

  const leftContent = (() => {
    if (isDashboardRoute) {
      return (
        <DashboardPage
          projectId={project.id}
          pipelines={project.pipelines}
          initialPipelineId={pipelineId}
          onBack={() => navigate(pipelineId ? `${basePath}/pipeline/${pipelineId}/integration-test` : basePath)}
          executionBackendUrl={backendUrl}
        />
      );
    }

    if (view === "create-pipeline") {
      return (
        <PipelineCreatorPage
          onSaveAndRun={handleSaveAndRun}
          isDark={isDark}
          onCancel={() => { setEditingPipelineIndex(null); navigate(basePath); }}
          spec={getMergedSpec(project.specs)}
          specs={project.specs}
          envGroups={project.envGroups}
          initialPipeline={pipelineId ? project.pipelines.find(p => p.id === pipelineId) ?? newPipelineTemplate ?? undefined : newPipelineTemplate ?? undefined}
        />
      );
    }

    if (view === "routes") {
      const editingSpec = specId && specId !== "new" ? project.specs.find(s => s.id === specId) : undefined;
      return (
        <RouteEditorPage
          spec={editingSpec?.spec}
          specId={specId !== "new" ? specId : undefined}
          projectId={project.id}
          initialSlug={editingSpec?.slug}
          initialServers={editingSpec?.servers}
          initialUrl={editingSpec?.url}
          initialSync={editingSpec?.sync}
          initialSpecMd5={editingSpec?.specMd5}
          onConfirm={handleConfirmSpec}
          onCancel={() => navigate(basePath)}
          isDark={isDark}
        />
      );
    }

    return (
      <TestExecutionPage
        pipelines={project.pipelines}
        spec={project.spec}
        specs={project.specs}
        envGroups={project.envGroups}
        projectId={project.id}
        onDeletePipeline={handleDeletePipeline}
        onCreatePipeline={handleCreatePipeline}
        onCreateAIPipeline={experimentalFeaturesEnabled ? handleAICreatePipeline : undefined}
        onEditPipeline={handleEditPipeline}
        onDuplicatePipeline={handleDuplicatePipeline}
        onImportSpec={handleImportSpec}
        onEditSpec={handleOpenRouteEditor}
        onDeleteSpec={handleDeleteSpec}
        onCreateEnvGroup={(data) => createEnvGroup(project.id, data)}
        onUpdateEnvGroup={(envGroupId, data) => updateEnvGroup(project.id, envGroupId, data)}
        onDeleteEnvGroup={(envGroupId) => deleteEnvGroup(project.id, envGroupId)}
        selectedPipelineId={pipelineId}
        initialSelectedIndex={selectedIndex !== null && selectedIndex >= 0 ? selectedIndex : undefined}
        onSelectPipeline={handleSelectPipeline}
        initialTab={initialTab}
        onTabChange={handleTabChange}
        executionBackendUrl={backendUrl}
        autoRunPipelineId={aiAutoRunId}
        autoSelectTab={aiAutoTab}
        onAnalyzeStepWithAI={experimentalFeaturesEnabled ? (step, result) => {
          const summary = JSON.stringify({
            step: { id: step.id, name: step.name, method: step.method, url: step.url },
            result: {
              status: result.status,
              duration: result.duration,
              error: result.error,
              request: result.request,
              response: result.response,
              assertResults: result.assertResults,
            },
          }, null, 2);
          chatRef.current?.sendCommand(
            `Analyze the following step execution result and provide insights about what happened, potential issues, and suggestions for improvement:\n\n\`\`\`json\n${summary}\n\`\`\``,
            `Analyze Step: ${step.name}`
          );
        } : undefined}
      />
    );
  })();

  const chatPanel = experimentalFeaturesEnabled ? (
    <AIPipelineChat
      ref={chatRef}
      projectId={project.id}
      specs={project.specs}
      envGroups={project.envGroups}
      pipelines={project.pipelines}
    />
  ) : null;

  return isMobile ? (
    <>
      <main className="flex h-full min-h-0 flex-1 overflow-hidden">
        {mobileView === "chat" && aiAssistantAvailable ? chatPanel : leftContent}
      </main>
      {aiAssistantAvailable && (
        <div className="glass h-12 border-t border-border flex shrink-0 z-50">
          <button
            onClick={() => setMobileView("app")}
            className={cn(
              "flex-1 flex items-center justify-center gap-1.5 text-sm font-medium transition-colors",
              mobileView === "app" ? "text-primary" : "text-muted-foreground"
            )}
          >
            <Monitor className="h-4 w-4" />
            App
          </button>
          <button
            onClick={() => setMobileView("chat")}
            className={cn(
              "flex-1 flex items-center justify-center gap-1.5 text-sm font-medium transition-colors",
              mobileView === "chat" ? "text-primary" : "text-muted-foreground"
            )}
          >
            <MessageSquare className="h-4 w-4" />
            Chat
          </button>
        </div>
      )}
    </>
  ) : (
    <main className={cn("relative flex h-full min-h-0 flex-1 overflow-hidden", chatPosition === "left" && "flex-row-reverse")}>
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden transition-all duration-300 ease-out">
        {leftContent}
      </div>

      {aiAssistantAvailable && chatMounted && (
        <div
          className={cn(
            "flex h-full min-h-0 shrink-0 overflow-hidden border-border transition-[width] duration-300 ease-out",
            (isClosing || isOpening) ? "w-0" : "w-[350px]"
          )}
        >
          <div className="flex h-full min-h-0 w-[350px]">
            {chatPanel}
          </div>
        </div>
      )}

      {aiAssistantAvailable && chatCollapsed && !chatMounted && (
        <Button
          onClick={toggleChatCollapsed}
          size="icon"
          className={cn(
            "fixed bottom-6 z-50 h-12 w-12 rounded-full shadow-lg animate-scale-in",
            chatPosition === "right" ? "right-6" : "left-6"
          )}
          title="Open chat"
        >
          <MessageSquare className="h-5 w-5" />
        </Button>
      )}
    </main>
  );
}
