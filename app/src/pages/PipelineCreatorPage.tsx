import { useState, useCallback, useMemo, useRef, useEffect, type MutableRefObject } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";
import type { ProjectEnvGroup, ProjectSpec } from "@/types/project";
import type { OnMount } from "@monaco-editor/react";
import jsYaml from "js-yaml";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

import { BookOpen, ArrowLeft, AlertTriangle, Plus, Code } from "lucide-react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { toDisplayText } from "@/lib/utils";
import { useEditorFormatStore } from "@/stores/useEditorFormatStore";
import type { MarkerInfo, FormatType } from "@/lib/pipeline-schema";
import type { Pipeline, PipelineStep, OpenAPISpec } from "@/types/pipeline";
import type { ResponseFieldInfo, TemplateValidationContext } from "@/lib/template-validator";
import type { GenericValidationResult } from "@/components/MonacoCodeEditor";
import { validatePipelineContract, type ContractWarning } from "@/lib/pipeline-contract-validator";
import PipelineDocsPanel from "@/components/PipelineDocsPanel";
import { PipelineEditor } from "@/components/editors";
import { SplitPaneLayout } from "@/components/SplitPaneLayout";
import { PreviewLayout } from "@/components/PreviewLayout";
import { MethodBadge } from "@/components/MethodBadge";
import { StepFlowGraph } from "@/components/StepFlowGraph";
import { StepCreatorPanel } from "@/components/StepCreatorPanel";
import { UnsavedChangesDialog } from "@/components/UnsavedChangesDialog";

interface PipelineCreatorPageProps {
  onSaveAndRun: (pipeline: Pipeline) => void;
  isDark: boolean;
  onCancel?: () => void;
  spec?: OpenAPISpec;
  specs?: ProjectSpec[];
  envGroups?: ProjectEnvGroup[];
  initialPipeline?: Pipeline;
}

export default function PipelineCreatorPage({ onSaveAndRun, isDark, onCancel, spec, specs, envGroups = [], initialPipeline }: PipelineCreatorPageProps) {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const [content, setContent] = useState(() => {
    if (initialPipeline) {
      const { id, ...payload } = initialPipeline;
      return JSON.stringify(payload, null, 2);
    }
    return "";
  });
  const { format, setFormat } = useEditorFormatStore();
  const [pipeline, setPipeline] = useState<Pipeline | null>(null);
  const [contractWarnings, setContractWarnings] = useState<ContractWarning[]>([]);
  const [showDocs, setShowDocs] = useState(false);
  const [showStepCreator, setShowStepCreator] = useState(false);
  const [editingStepIndex, setEditingStepIndex] = useState<number | null>(null);
  const [showUnsavedDialog, setShowUnsavedDialog] = useState(false);

  const savedContentRef = useRef<string>("");

  const canonicalize = useCallback((obj: unknown): unknown => {
    if (obj === null || obj === undefined) return obj;
    if (Array.isArray(obj)) return obj.map(canonicalize);
    if (typeof obj === "object") {
      const sorted: Record<string, unknown> = {};
      for (const key of Object.keys(obj as Record<string, unknown>).sort()) {
        const val = (obj as Record<string, unknown>)[key];
        if (val !== undefined) sorted[key] = canonicalize(val);
      }
      return sorted;
    }
    return obj;
  }, []);

  const normalizeContent = useCallback((c: string): string => {
    if (!c) return "";
    try {
      const trimmed = c.trim();
      if (!trimmed) return "";
      const parsed = trimmed.startsWith("{") || trimmed.startsWith("[")
        ? JSON.parse(trimmed)
        : jsYaml.load(trimmed);
      return JSON.stringify(canonicalize(parsed));
    } catch { return c.trim(); }
  }, [canonicalize]);

  // Initialize saved content ref — use normalizeContent for consistent comparison
  useEffect(() => {
    if (initialPipeline) {
      const { id, ...payload } = initialPipeline;
      savedContentRef.current = JSON.stringify(payload);
    }
  }, []); // only on mount

  // Keep savedContentRef in sync after initial content normalization
  const savedContentInitialized = useRef(false);
  useEffect(() => {
    if (!savedContentInitialized.current && content) {
      savedContentRef.current = normalizeContent(content);
      savedContentInitialized.current = true;
    }
  }, [content, normalizeContent]);

  const isDirty = useMemo(() => {
    return normalizeContent(content) !== savedContentRef.current;
  }, [content, normalizeContent]);

  const handleFormatChange = useCallback((newFormat: FormatType) => {
    setFormat(newFormat);
  }, [setFormat]);

  const handleEditorChange = useCallback((newValue: string) => {
    setContent(newValue);
  }, []);

  const handleValidation = useCallback((result: GenericValidationResult<Pipeline>) => {
    if (result.success) {
      setPipeline(result.data);
      if (specs && specs.length > 0) {
        setContractWarnings(validatePipelineContract(result.data, specs));
      } else if (spec) {
        setContractWarnings(validatePipelineContract(result.data, spec));
      } else {
        setContractWarnings([]);
      }
    } else {
      setPipeline(null);
      setContractWarnings([]);
    }
  }, [spec, specs]);

  const handleImport = (text: string) => {
    try {
      const parsed = jsYaml.load(text);
      const converted = format === "json"
        ? JSON.stringify(parsed, null, 2)
        : jsYaml.dump(parsed, { indent: 2, lineWidth: -1 });
      setContent(converted);
    } catch {
      setContent(text);
    }
  };

  const handleAddStep = useCallback((newStep: PipelineStep) => {
    let data: Record<string, unknown>;
    try {
      if (format === "json") {
        data = content.trim() ? JSON.parse(content) : {};
      } else {
        data = (content.trim() ? jsYaml.load(content) : {}) as Record<string, unknown>;
      }
    } catch {
      data = {};
    }

    if (!Array.isArray(data.steps)) {
      data.steps = [];
    }
    (data.steps as unknown[]).push(newStep);

    if (!data.name) data.name = "my_pipeline";
    if (!data.description) data.description = "";

    const serialized = format === "json"
      ? JSON.stringify(data, null, 2)
      : jsYaml.dump(data, { indent: 2, lineWidth: -1 });

    setContent(serialized);
    setShowStepCreator(false);
    setEditingStepIndex(null);
  }, [content, format]);

  const handleUpdateStep = useCallback((updatedStep: PipelineStep) => {
    if (editingStepIndex === null) return;
    let data: Record<string, unknown>;
    try {
      if (format === "json") {
        data = content.trim() ? JSON.parse(content) : {};
      } else {
        data = (content.trim() ? jsYaml.load(content) : {}) as Record<string, unknown>;
      }
    } catch {
      data = {};
    }

    if (!Array.isArray(data.steps)) return;
    (data.steps as unknown[])[editingStepIndex] = updatedStep;

    const serialized = format === "json"
      ? JSON.stringify(data, null, 2)
      : jsYaml.dump(data, { indent: 2, lineWidth: -1 });

    setContent(serialized);
    setShowStepCreator(false);
    setEditingStepIndex(null);
  }, [content, format, editingStepIndex]);

  const syncSourceRef = useRef<"visual" | null>(null);
  const editorInstanceRef = useRef<Parameters<OnMount>[0] | null>(null);

  const revealStepInEditor = useCallback((stepId: string) => {
    const editor = editorInstanceRef.current;
    if (!editor) return;
    const lines = content.split("\n");
    const escaped = stepId.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const jsonRegex = new RegExp(`"id"\\s*:\\s*"${escaped}"`);
    const yamlRegex = new RegExp(`id:\\s*${escaped}(\\s|$)`);
    const regex = format === "json" ? jsonRegex : yamlRegex;
    const lineIndex = lines.findIndex((l) => regex.test(l));
    if (lineIndex < 0) return;
    const lineNumber = lineIndex + 1;
    editor.revealLineInCenter(lineNumber);
    editor.setPosition({ lineNumber, column: 1 });
    editor.focus();
  }, [content, format]);

  // Auto-reveal step from query param (e.g. coming from execution page)
  const pendingStepIdRef = useRef<string | null>(searchParams.get("stepId"));
  useEffect(() => {
    const stepId = searchParams.get("stepId");
    if (stepId) {
      pendingStepIdRef.current = stepId;
      // Remove query param immediately
      setSearchParams((prev) => {
        const next = new URLSearchParams(prev);
        next.delete("stepId");
        return next;
      }, { replace: true });
    }
  }, [searchParams, setSearchParams]);

  useEffect(() => {
    if (!pendingStepIdRef.current || !editorInstanceRef.current) return;
    const stepId = pendingStepIdRef.current;
    // Small delay to ensure editor is fully mounted and content loaded
    const timer = setTimeout(() => {
      revealStepInEditor(stepId);
      pendingStepIdRef.current = null;
    }, 300);
    return () => clearTimeout(timer);
  }, [content, revealStepInEditor]);

  const handleStepChange = useCallback((updatedStep: PipelineStep) => {
    if (editingStepIndex === null) return;

    let data: Record<string, unknown>;
    try {
      if (format === "json") {
        data = content.trim() ? JSON.parse(content) : {};
      } else {
        data = (content.trim() ? jsYaml.load(content) : {}) as Record<string, unknown>;
      }
    } catch {
      data = {};
    }

    if (!Array.isArray(data.steps)) return;

    // Skip update if the step hasn't actually changed
    const currentStep = (data.steps as unknown[])[editingStepIndex];
    if (JSON.stringify(canonicalize(currentStep)) === JSON.stringify(canonicalize(updatedStep))) return;

    syncSourceRef.current = "visual";
    (data.steps as unknown[])[editingStepIndex] = updatedStep;

    const serialized = format === "json"
      ? JSON.stringify(data, null, 2)
      : jsYaml.dump(data, { indent: 2, lineWidth: -1 });

    setContent(serialized);
  }, [content, format, editingStepIndex]);

  const handlePipelineFieldChange = useCallback((field: "name" | "description", value: string) => {
    let data: Record<string, unknown>;
    try {
      if (format === "json") {
        data = content.trim() ? JSON.parse(content) : {};
      } else {
        data = (content.trim() ? jsYaml.load(content) : {}) as Record<string, unknown>;
      }
    } catch {
      data = {};
    }
    data[field] = value;
    const serialized = format === "json"
      ? JSON.stringify(data, null, 2)
      : jsYaml.dump(data, { indent: 2, lineWidth: -1 });
    setContent(serialized);
  }, [content, format]);

  const warningMarkers = useMemo((): MarkerInfo[] => {
    return contractWarnings.map((w) => {
      let field: string;
      switch (w.type) {
        case "invalid_method":
          field = "method";
          break;
        case "missing_required_header":
          field = "headers";
          break;
        default:
          field = "url";
      }
      return {
        path: ["steps", w.stepIndex, field],
        message: w.message,
      };
    });
  }, [contractWarnings]);

  const addStepCard = (
    <Card
      className="border-dashed border-2 cursor-pointer hover:border-primary/50 transition-colors"
      onClick={() => setShowStepCreator(true)}
    >
      <CardContent className="flex items-center justify-center py-8">
        <Plus className="h-6 w-6 text-muted-foreground" />
      </CardContent>
    </Card>
  );

  const docsButton = (
    <Button
      variant={showDocs ? "secondary" : "ghost"}
      size="sm"
      onClick={() => setShowDocs((v) => !v)}
      className="gap-1"
    >
      <BookOpen className="h-3.5 w-3.5" />
      Docs
    </Button>
  );

  const existingStepIds = useMemo(
    () => pipeline?.steps?.map(s => s.id) ?? [],
    [pipeline]
  );

  const stepResponseFields = useMemo((): Record<string, ResponseFieldInfo[]> => {
    if (!pipeline) return {};
    const allRoutes = specs && specs.length > 0
      ? specs.flatMap(s => s.spec.routes)
      : spec?.routes ?? [];
    if (allRoutes.length === 0) return {};

    const result: Record<string, ResponseFieldInfo[]> = {};
    for (const step of pipeline.steps) {
      if (!step.operationId) continue;
      const route = allRoutes.find(r => r.operationId === step.operationId);
      if (route?.responseFields) {
        result[step.id] = route.responseFields;
      }
    }
    return result;
  }, [pipeline, spec, specs]);

  const editorValidationContext = useMemo((): TemplateValidationContext => ({
    availableStepIds: existingStepIds,
    stepResponseFields,
    availableSpecs: (specs ?? []).map(s => ({
      slug: s.slug,
      envs: Object.keys(s.servers),
    })),
    availableEnvGroups: envGroups.map((group) => ({
      slug: group.slug,
      entries: group.entries.map((entry) => entry.name),
    })),
    selectedEnvGroupSlug: envGroups[0]?.slug ?? null,
  }), [existingStepIds, stepResponseFields, specs, envGroups]);

  const leftPanel = (
    <div className="flex h-full flex-col">
      <PipelineEditor
        value={content}
        onChange={handleEditorChange}
        format={format}
        onFormatChange={handleFormatChange}
        isDark={isDark}
        onValidation={handleValidation}
        onImport={handleImport}
        buttonBack={() => {
          if (isDirty) {
            setShowUnsavedDialog(true);
          } else {
            onCancel?.();
          }
        }}
        warningMarkers={warningMarkers}
        validationContext={editorValidationContext}
        editorInstanceRef={editorInstanceRef}
      />
    </div>
  );

  const isEditing = editingStepIndex !== null;
  const editingStep = useMemo((): PipelineStep | undefined => {
    if (editingStepIndex === null) return undefined;
    try {
      const data = format === "json"
        ? JSON.parse(content)
        : jsYaml.load(content) as Record<string, unknown>;
      const steps = (data as Record<string, unknown>)?.steps;
      if (Array.isArray(steps) && steps[editingStepIndex]) {
        return steps[editingStepIndex] as PipelineStep;
      }
    } catch { /* ignore */ }
    return pipeline?.steps[editingStepIndex];
  }, [editingStepIndex, content, format, pipeline]);

  const rightPanel = showStepCreator ? (
    <PreviewLayout
      title={isEditing ? t("pipelineCreator.editStep") : t("pipelineCreator.newStep")}
      subtitle={isEditing ? t("pipelineCreator.editStepSubtitle") : t("pipelineCreator.newStepSubtitle")}
      leftContent={
        <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => { setShowStepCreator(false); setEditingStepIndex(null); }}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
      }
    >
      <StepCreatorPanel
        key={isEditing ? `edit-${editingStepIndex}` : "new"}
        spec={spec}
        specs={specs}
        envGroups={envGroups}
        onAdd={isEditing ? handleUpdateStep : handleAddStep}
        onCancel={() => { setShowStepCreator(false); setEditingStepIndex(null); }}
        initialStep={editingStep}
        existingStepIds={existingStepIds}
        stepResponseFields={stepResponseFields}
        onChange={isEditing ? handleStepChange : undefined}
        externalStep={isEditing ? editingStep : undefined}
      />
    </PreviewLayout>
  ) : showDocs ? (
    <PreviewLayout
      title={t("pipelineCreator.docsTitle")}
      subtitle={t("pipelineCreator.docsSubtitle")}
      leftContent={
        <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => setShowDocs(false)}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
      }
    >
      <PipelineDocsPanel isDark={isDark} format={format} onFormatChange={setFormat} />
    </PreviewLayout>
  ) : (
    <PreviewLayout
      title={pipeline ? toDisplayText(pipeline.name) : "Pipeline"}
      subtitle={pipeline ? toDisplayText(pipeline.description) : t("pipelineCreator.typeToPrev", { format: format.toUpperCase() })}
      rightContent={docsButton}
      buttonContent={<>Save</>}
      onButtonClick={() => pipeline && onSaveAndRun(pipeline)}
      buttonDisabled={!pipeline}
    >
      <div className="flex flex-col flex-1 p-4 space-y-4 overflow-hidden min-w-0">
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="pipeline-name">Name</Label>
            <Input
              id="pipeline-name"
              value={pipeline?.name ?? ""}
              onChange={(e) => handlePipelineFieldChange("name", e.target.value)}
              placeholder="pipeline_name"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="pipeline-description">Description</Label>
            <Input
              id="pipeline-description"
              value={pipeline?.description ?? ""}
              onChange={(e) => handlePipelineFieldChange("description", e.target.value)}
              placeholder="Pipeline description"
            />
          </div>
        </div>

        {pipeline ? (
          <div className="flex-1 min-h-0 min-w-0 w-full">
          <StepFlowGraph
            itemHeight={130}
            items={[
              ...pipeline.steps.map((step, idx) => {
                const stepWarnings = contractWarnings.filter((w) => w.stepId === step.id);
                return {
                  key: step.id,
                  content: (
                    <Card
                      className={`h-full cursor-pointer hover:border-primary/50 transition-colors ${stepWarnings.length > 0 ? "border-amber-500/50" : ""}`}
                      onClick={() => {
                        setEditingStepIndex(idx);
                        setShowStepCreator(true);
                      }}
                    >
                      <CardHeader className="p-4 pb-2">
                        <div className="flex items-center justify-between gap-2">
                          <div className="flex items-center gap-2 flex-1 min-w-0">
                            <MethodBadge method={step.method} />
                            <CardTitle className="text-sm truncate">{step.name}</CardTitle>
                          </div>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6 shrink-0"
                            onClick={(e) => {
                              e.stopPropagation();
                              revealStepInEditor(step.id);
                            }}
                            title="Go to code"
                          >
                            <Code className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </CardHeader>
                      <CardContent className="px-4 pb-3 pt-0">
                        <p className="text-xs text-muted-foreground">{toDisplayText(step.description)}</p>
                        
                        {stepWarnings.length > 0 && (
                          <div className="mt-2 space-y-1">
                            {stepWarnings.map((w, i) => (
                              <div key={i} className="flex items-start gap-1.5 text-xs text-warning">
                                <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
                                <span>{w.message}</span>
                              </div>
                            ))}
                          </div>
                        )}
                      </CardContent>
                    </Card>
                  ),
                };
              }),
              { key: "__add_step", content: addStepCard },
            ]}
          />
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center gap-4 text-muted-foreground py-8">
            <p>{t("pipelineCreator.typeToPrev", { format: format.toUpperCase() })}</p>
            {addStepCard}
          </div>
        )}
      </div>
    </PreviewLayout>
  );

  return (
    <>
      <SplitPaneLayout
        leftPanel={leftPanel}
        rightPanel={rightPanel}
        leftDefaultSize={30}
        rightDefaultSize={70}
        leftMinSize={30}
        rightMinSize={30}
        autoSaveId="split-pipeline"
        withPadding={false}
        withBorder={false}
      />
      <UnsavedChangesDialog
        open={showUnsavedDialog}
        onOpenChange={setShowUnsavedDialog}
        onSave={() => {
          if (pipeline) onSaveAndRun(pipeline);
        }}
        onDiscard={() => onCancel?.()}
      />
    </>
  );
}
