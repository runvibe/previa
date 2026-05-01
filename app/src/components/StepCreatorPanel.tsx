import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AlertTriangle, Plus, X, ShieldCheck, Clock, RotateCcw } from "lucide-react";
import { MethodBadge } from "@/components/MethodBadge";
import { RequestSection } from "@/components/RequestItem";
import { MonacoInput } from "@/components/MonacoInput";
import { validatePipelineContract, type ContractWarning } from "@/lib/pipeline-contract-validator";
import type { PipelineStep, OpenAPISpec, OpenAPIRoute, OpenAPIParameter, StepAssertion } from "@/types/pipeline";
import type { ProjectEnvGroup, ProjectSpec } from "@/types/project";
import type { TemplateValidationContext, ResponseFieldInfo } from "@/lib/template-validator";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE"] as const;
const BODY_METHODS = ["POST", "PUT", "PATCH"];
const ASSERT_OPERATORS = [
  { value: "equals", label: "equals" },
  { value: "not_equals", label: "not equals" },
  { value: "contains", label: "contains" },
  { value: "exists", label: "exists" },
  { value: "not_exists", label: "not exists" },
  { value: "gt", label: "greater than" },
  { value: "lt", label: "less than" },
] as const;

interface StepCreatorPanelProps {
  spec?: OpenAPISpec;
  specs?: ProjectSpec[];
  envGroups?: ProjectEnvGroup[];
  onAdd: (step: PipelineStep) => void;
  onCancel: () => void;
  initialStep?: PipelineStep;
  existingStepIds?: string[];
  stepResponseFields?: Record<string, ResponseFieldInfo[]>;
  /** Called on every field change in edit mode (bidirectional sync) */
  onChange?: (step: PipelineStep) => void;
  /** External step data from JSON/YAML editor (bidirectional sync) */
  externalStep?: PipelineStep;
}

function generateStepId(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_|_$/g, "") || "new_step";
}

function buildUrlWithPathParams(basePath: string, pathParams: Record<string, string>, specSlug?: string, envKey?: string, envGroupEntry?: string): string {
  const prefix = envGroupEntry
    ? `{{envs.current.${envGroupEntry}}}`
    : specSlug && envKey ? `{{specs.${specSlug}.url.${envKey}}}` : `{{specs.slug.url.env}}`;
  let url = `${prefix}${basePath}`;
  for (const [name, value] of Object.entries(pathParams)) {
    if (value) {
      url = url.replace(`{${name}}`, value);
    }
  }
  return url;
}

interface BodySchemaParam {
  name: string;
  required: boolean;
  type?: string;
  description?: string;
  enum?: string[];
  format?: string;
  pattern?: string;
}

function resolveRefFromRaw(raw: Record<string, unknown>, ref: string): unknown {
  const path = ref.replace(/^#\//, "").split("/");
  let current: unknown = raw;
  for (const segment of path) {
    if (current == null || typeof current !== "object") return undefined;
    current = (current as Record<string, unknown>)[segment];
  }
  return current;
}

function resolveSchemaRuntime(raw: Record<string, unknown>, schema: unknown): Record<string, unknown> | undefined {
  if (!schema || typeof schema !== "object") return undefined;
  const s = schema as Record<string, unknown>;
  if ("$ref" in s && typeof s.$ref === "string") {
    const resolved = resolveRefFromRaw(raw, s.$ref);
    if (resolved && typeof resolved === "object") return resolved as Record<string, unknown>;
    return undefined;
  }
  return s;
}

function getBodySchemaParams(route: OpenAPIRoute, specRaw?: Record<string, unknown>): { params: BodySchemaParam[]; allowAdditional: boolean } {
  if (!route.requestBody?.content) return { params: [], allowAdditional: false };
  
  const contentEntry = route.requestBody.content["application/json"] 
    ?? Object.values(route.requestBody.content)[0];
  
  let schema = contentEntry?.schema as Record<string, unknown> | undefined;
  if ((!schema || Object.keys(schema).length === 0) && specRaw) {
    const rawPaths = specRaw.paths as Record<string, Record<string, unknown>> | undefined;
    if (rawPaths) {
      const pathItem = rawPaths[route.path];
      if (pathItem) {
        const operation = pathItem[route.method.toLowerCase()] as Record<string, unknown> | undefined;
        const rawBody = operation?.requestBody as Record<string, unknown> | undefined;
        let resolvedBody = rawBody;
        if (rawBody && "$ref" in rawBody && typeof rawBody.$ref === "string") {
          resolvedBody = resolveRefFromRaw(specRaw, rawBody.$ref) as Record<string, unknown> | undefined;
        }
        const rawContent = resolvedBody?.content as Record<string, Record<string, unknown>> | undefined;
        const rawMediaObj = rawContent?.["application/json"] ?? (rawContent ? Object.values(rawContent)[0] : undefined);
        const rawSchema = rawMediaObj?.schema;
        schema = resolveSchemaRuntime(specRaw, rawSchema);
      }
    }
  }
  
  if (!schema) return { params: [], allowAdditional: false };

  console.log("[ADDPROP-DEBUG] schema keys:", Object.keys(schema), "additionalProperties:", JSON.stringify(schema.additionalProperties), "full schema:", JSON.stringify(schema).slice(0, 500));

  const allowAdditional = !!schema.additionalProperties;

  const props = schema.properties as Record<string, Record<string, unknown>> | undefined;
  if (!props) return { params: [], allowAdditional };
  const requiredFields = (schema.required as string[]) ?? [];
  const params = Object.entries(props).map(([name, propSchema]) => ({
    name,
    required: requiredFields.includes(name),
    type: (propSchema?.type as string) ?? undefined,
    description: (propSchema?.description as string) ?? undefined,
    enum: Array.isArray(propSchema?.enum) ? (propSchema.enum as unknown[]).map(String) : undefined,
    format: (propSchema?.format as string) ?? undefined,
    pattern: (propSchema?.pattern as string) ?? undefined,
  }));
  return { params, allowAdditional };
}

export function StepCreatorPanel({ spec, specs, envGroups = [], onAdd, onCancel, initialStep, existingStepIds, stepResponseFields, onChange, externalStep }: StepCreatorPanelProps) {
  const { t } = useTranslation();
  const [stepId, setStepId] = useState(initialStep?.id ?? "");
  const [idManuallyEdited, setIdManuallyEdited] = useState(!!initialStep?.id);
  const [name, setName] = useState(initialStep?.name ?? "");
  const [description, setDescription] = useState(initialStep?.description ?? "");
  const [method, setMethod] = useState<PipelineStep["method"]>(initialStep?.method ?? "GET");
  const [url, setUrl] = useState(initialStep?.url ?? "");
  const [headers, setHeaders] = useState<{ key: string; value: string }[]>(
    initialStep?.headers ? Object.entries(initialStep.headers).map(([key, value]) => ({ key, value })) : []
  );
  const [body, setBody] = useState(initialStep?.body ? JSON.stringify(initialStep.body, null, 2) : "");
  const [showRouteSuggestions, setShowRouteSuggestions] = useState(false);
  const initialRoute = useMemo(() => {
    if (initialStep?.operationId && spec) {
      return spec.routes.find(r => r.operationId === initialStep.operationId) ?? null;
    }
    return null;
  }, []);
  const [selectedRoute, setSelectedRoute] = useState<OpenAPIRoute | null>(initialRoute);
  const [pathParamValues, setPathParamValues] = useState<Record<string, string>>({});
  const [headerParamValues, setHeaderParamValues] = useState<Record<string, string>>({});
  const [bodyParamValues, setBodyParamValues] = useState<Record<string, string>>(() => {
    if (initialStep?.body && typeof initialStep.body === "object") {
      const entries: Record<string, string> = {};
      for (const [k, v] of Object.entries(initialStep.body)) {
        entries[k] = typeof v === "string" ? v : JSON.stringify(v);
      }
      return entries;
    }
    return {};
  });
  const [customBodyParams, setCustomBodyParams] = useState<{ key: string; value: string }[]>(() => {
    // When editing without a selected route, populate body as custom params
    if (initialStep?.body && typeof initialStep.body === "object") {
      return Object.entries(initialStep.body).map(([key, v]) => ({
        key,
        value: typeof v === "string" ? v : JSON.stringify(v),
      }));
    }
    return [];
  });
  const [asserts, setAsserts] = useState<StepAssertion[]>(initialStep?.asserts ?? []);
  const [delay, setDelay] = useState<number>(initialStep?.delay ?? 0);
  const [retry, setRetry] = useState<number>(initialStep?.retry ?? 0);

  // Extract path params from route parameters OR from path pattern like {id}
  const pathParams = useMemo((): OpenAPIParameter[] => {
    if (!selectedRoute) return [];
    const fromSpec = selectedRoute.parameters?.filter((p) => p.in === "path") ?? [];
    // Also extract from path pattern for params not already in spec
    const pathPattern = selectedRoute.path;
    const matches = pathPattern.match(/\{(\w+)\}/g) ?? [];
    const specNames = new Set(fromSpec.map((p) => p.name));
    const fromPath = matches
      .map((m) => m.slice(1, -1))
      .filter((name) => !specNames.has(name))
      .map((name): OpenAPIParameter => ({ name, in: "path", required: true }));
    return [...fromSpec, ...fromPath];
  }, [selectedRoute]);

  const headerParams = useMemo(() =>
    selectedRoute?.parameters?.filter((p) => p.in === "header") ?? [], [selectedRoute]);

  const queryParams = useMemo(() =>
    selectedRoute?.parameters?.filter((p) => p.in === "query") ?? [], [selectedRoute]);

  const bodySchemaResult = useMemo(() =>
    selectedRoute ? getBodySchemaParams(selectedRoute, spec?.raw) : { params: [], allowAdditional: false }, [selectedRoute, spec]);
  const bodySchemaParams = bodySchemaResult.params;
  const allowAdditionalBody = bodySchemaResult.allowAdditional;

  const validationContext = useMemo((): TemplateValidationContext => ({
    availableStepIds: (existingStepIds ?? []).filter(id => id !== stepId),
    currentStepId: stepId,
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
  }), [existingStepIds, stepId, stepResponseFields, specs, envGroups]);

  const canonicalize = useCallback((value: unknown): unknown => {
    if (value === null || value === undefined) return value;
    if (Array.isArray(value)) return value.map(canonicalize);
    if (typeof value === "object") {
      const sorted: Record<string, unknown> = {};
      for (const key of Object.keys(value as Record<string, unknown>).sort()) {
        const item = (value as Record<string, unknown>)[key];
        if (item !== undefined) {
          sorted[key] = canonicalize(item);
        }
      }
      return sorted;
    }
    return value;
  }, []);

  const serializeStep = useCallback((step: PipelineStep): string => {
    return JSON.stringify(canonicalize(step));
  }, [canonicalize]);

  const getChangedStepFields = useCallback((prevStep: PipelineStep | null | undefined, nextStep: PipelineStep): string[] => {
    if (!prevStep) return ["__initial__"];
    const fields: Array<keyof PipelineStep> = [
      "id",
      "name",
      "description",
      "method",
      "url",
      "headers",
      "body",
      "operationId",
      "asserts",
      "delay",
      "retry",
    ];

    return fields.filter((field) => {
      return JSON.stringify(canonicalize(prevStep[field])) !== JSON.stringify(canonicalize(nextStep[field]));
    });
  }, [canonicalize]);

  const currentStep = useMemo((): PipelineStep => {
    // Build body from bodyParamValues/customBodyParams, or raw body text
    let parsedBody: Record<string, unknown> | undefined;
    if (bodySchemaParams.length > 0 || customBodyParams.length > 0) {
      const obj: Record<string, unknown> = {};
      for (const [key, value] of Object.entries(bodyParamValues)) {
        if (value.trim()) obj[key] = value;
      }
      for (const cp of customBodyParams) {
        if (cp.key.trim()) obj[cp.key.trim()] = cp.value;
      }
      if (Object.keys(obj).length > 0) parsedBody = obj;
    } else if (BODY_METHODS.includes(method) && body.trim()) {
      try {
        parsedBody = JSON.parse(body);
      } catch {
        parsedBody = undefined;
      }
    }
    // Merge manual headers with route header params
    const headersObj: Record<string, string> = {};
    for (const h of headers) {
      if (h.key.trim()) headersObj[h.key.trim()] = h.value;
    }
    for (const [key, value] of Object.entries(headerParamValues)) {
      if (value.trim()) headersObj[key] = value;
    }
    return {
      id: stepId || generateStepId(name),
      name: name || "new_step",
      description,
      method,
      url,
      headers: headersObj,
      body: parsedBody,
      operationId: selectedRoute?.operationId ?? initialStep?.operationId,
      asserts: asserts.length > 0 ? asserts : undefined,
      delay: delay > 0 ? delay : undefined,
      retry: retry > 0 ? retry : undefined,
    };
  }, [name, description, method, url, headers, body, headerParamValues, bodyParamValues, customBodyParams, selectedRoute, bodySchemaParams, stepId, asserts, delay, retry, initialStep]);

  // --- Bidirectional sync: visual → JSON/YAML ---
  const isEditing = !!initialStep;
  const currentStepSerialized = useMemo(() => serializeStep(currentStep), [currentStep, serializeStep]);
  const lastEmittedRef = useRef<string>(initialStep ? serializeStep(initialStep) : "");
  const lastComparedStepRef = useRef<PipelineStep | null>(initialStep ?? null);
  const skipNextVisualSyncRef = useRef<boolean>(isEditing);

  useEffect(() => {
    if (!isEditing || !onChange) return;

    if (skipNextVisualSyncRef.current) {
      const changedFields = getChangedStepFields(lastComparedStepRef.current, currentStep);
      if (changedFields.length > 0 && changedFields[0] !== "__initial__") {
        console.info("[unsaved-debug] visual-sync skipped (baseline update)", { changedFields });
      }
      skipNextVisualSyncRef.current = false;
      lastComparedStepRef.current = currentStep;
      lastEmittedRef.current = currentStepSerialized;
      return;
    }

    if (currentStepSerialized !== lastEmittedRef.current) {
      const changedFields = getChangedStepFields(lastComparedStepRef.current, currentStep);
      console.info("[unsaved-debug] visual-sync emitted", { changedFields });
      lastComparedStepRef.current = currentStep;
      lastEmittedRef.current = currentStepSerialized;
      onChange(currentStep);
    }
  }, [currentStep, currentStepSerialized, getChangedStepFields, isEditing, onChange]);

  // --- Bidirectional sync: JSON/YAML → visual ---
  useEffect(() => {
    if (!externalStep || !isEditing) return;
    const externalSerialized = serializeStep(externalStep);
    if (externalSerialized === lastEmittedRef.current) return;

    const changedFields = getChangedStepFields(lastComparedStepRef.current, externalStep);
    console.info("[unsaved-debug] external-sync applied", { changedFields });

    // Prevent parent dirty state when local state update is driven by external sync
    skipNextVisualSyncRef.current = true;
    lastComparedStepRef.current = externalStep;
    lastEmittedRef.current = externalSerialized;

    setStepId(externalStep.id ?? "");
    setName(externalStep.name ?? "");
    setDescription(externalStep.description ?? "");
    setMethod(externalStep.method ?? "GET");
    setUrl(externalStep.url ?? "");
    setHeaders(
      externalStep.headers
        ? Object.entries(externalStep.headers).map(([key, value]) => ({ key, value }))
        : []
    );
    setBody(externalStep.body ? JSON.stringify(externalStep.body, null, 2) : "");
    setAsserts(externalStep.asserts ?? []);
    setDelay(externalStep.delay ?? 0);
    setRetry(externalStep.retry ?? 0);
    if (externalStep.body && typeof externalStep.body === "object") {
      const entries: Record<string, string> = {};
      for (const [k, v] of Object.entries(externalStep.body)) {
        entries[k] = typeof v === "string" ? v : JSON.stringify(v);
      }
      setBodyParamValues(entries);
      if (!selectedRoute) {
        setCustomBodyParams(
          Object.entries(externalStep.body).map(([key, v]) => ({
            key,
            value: typeof v === "string" ? v : JSON.stringify(v),
          }))
        );
      }
    }
  }, [externalStep, getChangedStepFields, isEditing, selectedRoute, serializeStep]);

  const warnings = useMemo((): ContractWarning[] => {
    if (!spec || !url) return [];
    const tempPipeline = {
      name: "temp",
      description: "",
      steps: [currentStep],
    };
    if (specs && specs.length > 0) {
      return validatePipelineContract(tempPipeline, specs);
    }
    return validatePipelineContract(tempPipeline, spec);
  }, [spec, currentStep, url]);

  const warningsByType = useMemo(() => {
    const map: Record<string, ContractWarning[]> = {};
    for (const w of warnings) {
      (map[w.type] ??= []).push(w);
    }
    return map;
  }, [warnings]);

  const handleSelectRoute = useCallback((route: OpenAPIRoute, specSlug?: string, specServers?: Record<string, string>) => {
    setSelectedRoute(route);
    const firstEnv = specServers ? Object.keys(specServers)[0] : undefined;
    const firstEnvGroupEntry = envGroups[0]?.entries[0]?.name;
    setUrl(buildUrlWithPathParams(route.path, {}, specSlug, firstEnv, firstEnvGroupEntry));
    setMethod(route.method.toUpperCase() as PipelineStep["method"]);
    if (route.summary || route.operationId) setName(route.summary || route.operationId || "");
    if (route.description) setDescription(route.description);
    setShowRouteSuggestions(false);
    setPathParamValues({});
    setHeaderParamValues({});
    setBodyParamValues({});
    setCustomBodyParams([]);
  }, [envGroups]);

  const handleClearRoute = useCallback(() => {
    setSelectedRoute(null);
    setPathParamValues({});
    setHeaderParamValues({});
    setCustomBodyParams([]);
  }, []);

  const handlePathParamChange = useCallback((paramName: string, value: string) => {
    setPathParamValues((prev) => {
      const next = { ...prev, [paramName]: value };
      // Rebuild URL - extract current prefix from url
      if (selectedRoute) {
        const prefixMatch = url.match(/^\{\{envs\.[^}]+\}\}/) ?? url.match(/^\{\{specs\.[^}]+\}\}/) ?? url.match(/^\{\{url\.[^}]+\}\}/);
        const prefix = prefixMatch ? prefixMatch[0] : (envGroups[0]?.entries[0]?.name ? `{{envs.current.${envGroups[0].entries[0].name}}}` : "{{specs.slug.url.env}}");
        let newUrl = `${prefix}${selectedRoute.path}`;
        for (const [n, v] of Object.entries(next)) {
          if (v) newUrl = newUrl.replace(`{${n}}`, v);
        }
        setUrl(newUrl);
      }
      return next;
    });
  }, [selectedRoute, url, envGroups]);

  const handleHeaderParamChange = useCallback((paramName: string, value: string) => {
    setHeaderParamValues((prev) => ({ ...prev, [paramName]: value }));
  }, []);

  const addHeader = () => setHeaders((h) => [...h, { key: "", value: "" }]);
  const removeHeader = (idx: number) => setHeaders((h) => h.filter((_, i) => i !== idx));
  const updateHeader = (idx: number, field: "key" | "value", val: string) =>
    setHeaders((h) => h.map((item, i) => (i === idx ? { ...item, [field]: val } : item)));

  const canAdd = name.trim() && url.trim();

  const handleAdd = () => {
    if (!canAdd) return;
    onAdd(currentStep);
  };

  return (
    <div className="space-y-5 p-4">
      {/* Route suggestions from spec */}
      {/* Route suggestions - supports multiple specs */}
      {(() => {
        const allRoutes: { route: OpenAPIRoute; specName?: string; specRaw?: Record<string, unknown> }[] = [];
        if (specs && specs.length > 0) {
          for (const s of specs) {
            for (const route of s.spec.routes) {
              allRoutes.push({ route, specName: s.name, specRaw: s.spec.raw });
            }
          }
        } else if (spec && spec.routes.length > 0) {
          for (const route of spec.routes) {
            allRoutes.push({ route });
          }
        }
        if (allRoutes.length === 0) return null;
        const hasMultipleSpecs = specs && specs.length > 1;

        return (
          <div>
            {selectedRoute ? (
              <div className="flex items-center gap-2 rounded-md border border-primary/30 bg-primary/5 px-3 py-2">
                <MethodBadge method={selectedRoute.method} />
                <span className="flex-1 truncate font-mono text-xs">{selectedRoute.path}</span>
                <Button variant="ghost" size="icon" className="h-6 w-6 shrink-0" onClick={handleClearRoute}>
                  <X className="h-3.5 w-3.5" />
                </Button>
              </div>
            ) : (
              <Button
                variant="outline"
                size="sm"
                className="w-full gap-1.5"
                onClick={() => setShowRouteSuggestions((v) => !v)}
              >
                <Plus className="h-3.5 w-3.5" />
                {showRouteSuggestions ? t("stepCreator.hideRoutes") : t("stepCreator.selectRoute")}
              </Button>
            )}
            {showRouteSuggestions && !selectedRoute && (
              <div className="mt-2 max-h-48 space-y-1 overflow-y-auto rounded-md border p-2">
                {hasMultipleSpecs ? (
                  // Group by spec
                  specs!.map((s) => (
                    <div key={s.id}>
                      <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                        {s.name}
                      </div>
                      {s.spec.routes.map((route, i) => (
                        <button
                          key={`${s.id}-${route.method}-${route.path}-${i}`}
                          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent transition-colors"
                          onClick={() => handleSelectRoute(route, s.slug, s.servers)}
                        >
                          <MethodBadge method={route.method} />
                          <span className="font-mono text-xs">{route.path}</span>
                          {route.summary && (
                            <span className="ml-auto truncate text-xs text-muted-foreground">
                              {route.summary}
                            </span>
                          )}
                        </button>
                      ))}
                    </div>
                  ))
                ) : (
                  allRoutes.map(({ route }, i) => (
                    <button
                      key={`${route.method}-${route.path}-${i}`}
                      className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent transition-colors"
                      onClick={() => handleSelectRoute(route, specs?.[0]?.slug, specs?.[0]?.servers)}
                    >
                      <MethodBadge method={route.method} />
                      <span className="font-mono text-xs">{route.path}</span>
                      {route.summary && (
                        <span className="ml-auto truncate text-xs text-muted-foreground">
                          {route.summary}
                        </span>
                      )}
                    </button>
                  ))
                )}
              </div>
            )}
          </div>
        );
      })()}

      {/* ID */}
      <div className="space-y-1.5">
        <Label htmlFor="step-id">ID</Label>
        <Input
          id="step-id"
          placeholder="ex: create_user"
          value={stepId}
          onChange={(e) => {
            setStepId(e.target.value);
            setIdManuallyEdited(true);
          }}
          className="font-mono text-sm"
        />
      </div>

      {/* Name */}
      <div className="space-y-1.5">
        <Label htmlFor="step-name">Name</Label>
        <Input
          id="step-name"
          placeholder="ex: Criar Usuário"
          value={name}
          onChange={(e) => {
            const newName = e.target.value;
            setName(newName);
            if (!idManuallyEdited) {
              setStepId(generateStepId(newName));
            }
          }}
        />
      </div>

      {/* Description */}
      <div className="space-y-1.5">
        <Label htmlFor="step-desc">Description</Label>
        <Input
          id="step-desc"
          placeholder="Descrição do step"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
      </div>

      {/* Method & URL — only when no route selected */}
      {!selectedRoute && (
        <>
          <div className="space-y-1.5">
            <Label htmlFor="step-method">Method</Label>
            <Select value={method} onValueChange={(v) => setMethod(v as PipelineStep["method"])}>
              <SelectTrigger id="step-method">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {METHODS.map((m) => (
                  <SelectItem key={m} value={m}>{m}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="step-url">URL</Label>
            <MonacoInput
              placeholder="{{url.spec.env}}/endpoint"
              value={url}
              onChange={setUrl}
              className="h-9"
              validationContext={validationContext}
            />
          </div>
        </>
      )}

      {/* === Path Parameters === */}
      {selectedRoute && pathParams.length > 0 && (
        <RequestSection
          title="Path Parameters"
          items={pathParams.map((p) => ({
            name: p.name,
            value: pathParamValues[p.name] ?? "",
            required: p.required,
          }))}
          onChange={handlePathParamChange}
          validationContext={validationContext}
        />
      )}

      {/* === Query Parameters === */}
      {selectedRoute && queryParams.length > 0 && (
        <RequestSection
          title="Query Parameters"
          items={queryParams.map((p) => ({
            name: p.name,
            value: "",
            required: p.required,
          }))}
          onChange={() => {}}
          validationContext={validationContext}
        />
      )}

      {/* === Header Parameters (from spec) === */}
      {selectedRoute && headerParams.length > 0 && (
        <RequestSection
          title="Header Parameters"
          items={headerParams.map((p) => ({
            name: p.name,
            value: headerParamValues[p.name] ?? "",
            required: p.required,
          }))}
          onChange={handleHeaderParamChange}
          validationContext={validationContext}
        />
      )}

      {/* === Body Parameters (unified) === */}
      {(selectedRoute ? (bodySchemaParams.length > 0 || allowAdditionalBody) : (BODY_METHODS.includes(method) && (customBodyParams.length > 0 || initialStep?.body))) && (() => {
        const fixedCount = selectedRoute ? bodySchemaParams.length : 0;
        return (
          <RequestSection
            title="Body Parameters"
            items={[
              ...(selectedRoute ? bodySchemaParams.map((p) => ({
                name: p.name,
                value: bodyParamValues[p.name] ?? "",
                required: p.required,
                type: p.type,
                description: p.description,
                enum: p.enum,
                format: p.format,
                pattern: p.pattern,
              })) : []),
              ...customBodyParams.map((cp) => ({
                name: cp.key,
                value: cp.value,
              })),
            ]}
            onChange={(name, value, index) => {
              if (index !== undefined && index >= fixedCount) {
                const ci = index - fixedCount;
                setCustomBodyParams((prev) => prev.map((cp, i) => i === ci ? { ...cp, value } : cp));
              } else {
                setBodyParamValues((prev) => ({ ...prev, [name]: value }));
              }
            }}
            onNameChange={(oldName, newName, index) => {
              if (index !== undefined && index >= fixedCount) {
                const ci = index - fixedCount;
                setCustomBodyParams((prev) => prev.map((cp, i) => i === ci ? { ...cp, key: newName } : cp));
              }
            }}
            onRemove={(name, index) => {
              if (index !== undefined && index >= fixedCount) {
                const ci = index - fixedCount;
                setCustomBodyParams((prev) => prev.filter((_, i) => i !== ci));
              }
            }}
            onAdd={() => setCustomBodyParams((prev) => [...prev, { key: "", value: "" }])}
            validationContext={validationContext}
          />
        );
      })()}

      {/* === Headers (manual) === */}
      <RequestSection
        title="Headers"
        items={headers.map((h) => ({
          name: h.key,
          value: h.value,
        }))}
        onChange={(name, value) => {
          const idx = headers.findIndex((h) => h.key === name);
          if (idx >= 0) updateHeader(idx, "value", value);
        }}
        onNameChange={(oldName, newName) => {
          const idx = headers.findIndex((h) => h.key === oldName);
          if (idx >= 0) updateHeader(idx, "key", newName);
        }}
        onRemove={(name) => {
          const idx = headers.findIndex((h) => h.key === name);
          if (idx >= 0) removeHeader(idx);
        }}
        onAdd={addHeader}
        validationContext={validationContext}
      />
      <InlineWarnings warnings={warningsByType["missing_required_header"]} />

      {/* === Assertions === */}
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <Label className="flex items-center gap-1.5">
            <ShieldCheck className="h-3.5 w-3.5" />
            Assertions
          </Label>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 gap-1 text-xs"
            onClick={() => setAsserts((prev) => [...prev, { field: "", operator: "equals", expected: "" }])}
          >
            <Plus className="h-3 w-3" /> Add
          </Button>
        </div>
        {asserts.map((a, i) => (
          <div key={i} className="flex items-center gap-1.5">
            <Input
              placeholder="status, body.id, header.x"
              value={a.field}
              onChange={(e) => setAsserts((prev) => prev.map((item, j) => j === i ? { ...item, field: e.target.value } : item))}
              className="flex-1 h-8 text-xs font-mono"
            />
            <Select
              value={a.operator}
              onValueChange={(v) => setAsserts((prev) => prev.map((item, j) => j === i ? { ...item, operator: v as StepAssertion["operator"] } : item))}
            >
              <SelectTrigger className="w-[120px] h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {ASSERT_OPERATORS.map((op) => (
                  <SelectItem key={op.value} value={op.value}>{op.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            {a.operator !== "exists" && a.operator !== "not_exists" && (
              <MonacoInput
                placeholder="expected (ex: {{steps.id.body.field}})"
                value={a.expected ?? ""}
                onChange={(v) => setAsserts((prev) => prev.map((item, j) => j === i ? { ...item, expected: v } : item))}
                className="flex-1 h-8 text-xs"
                validationContext={validationContext}
              />
            )}
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7 shrink-0"
              onClick={() => setAsserts((prev) => prev.filter((_, j) => j !== i))}
            >
              <X className="h-3 w-3" />
            </Button>
          </div>
        ))}
        {asserts.length === 0 && (
          <p className="text-xs text-muted-foreground">{t("stepCreator.noAssertions")}</p>
        )}
      </div>

      {/* === Delay & Retry === */}
      <div className="space-y-2">
        <Label className="flex items-center gap-1.5">
          <Clock className="h-3.5 w-3.5" />
          Execution Settings
        </Label>
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1">
            <Label htmlFor="step-delay" className="text-xs text-muted-foreground">Delay (ms)</Label>
            <Input
              id="step-delay"
              type="number"
              min={0}
              max={300000}
              value={delay}
              onChange={(e) => setDelay(Math.min(300000, Math.max(0, parseInt(e.target.value) || 0)))}
              className="h-8 text-xs"
              placeholder="0"
            />
            <p className="text-[10px] text-muted-foreground">Max 5 min (300000ms)</p>
          </div>
          <div className="space-y-1">
            <Label htmlFor="step-retry" className="text-xs text-muted-foreground flex items-center gap-1">
              <RotateCcw className="h-3 w-3" /> Retries
            </Label>
            <Input
              id="step-retry"
              type="number"
              min={0}
              max={10}
              value={retry}
              onChange={(e) => setRetry(Math.min(10, Math.max(0, parseInt(e.target.value) || 0)))}
              className="h-8 text-xs"
              placeholder="0"
            />
            <p className="text-[10px] text-muted-foreground">Max 10 attempts</p>
          </div>
        </div>
      </div>

      {/* Add button — only show for new steps (edit mode auto-syncs) */}
      {!isEditing && (
        <Button className="w-full" disabled={!canAdd} onClick={handleAdd}>
          {t("stepCreator.addStep")}
        </Button>
      )}
    </div>
  );
}

/* --- Sub-components --- */

function InlineWarnings({ warnings }: { warnings?: ContractWarning[] }) {
  if (!warnings || warnings.length === 0) return null;
  return (
    <div className="space-y-0.5">
      {warnings.map((w, i) => (
        <div key={i} className="flex items-start gap-1.5 text-xs text-warning">
          <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
          <span>{w.message}</span>
        </div>
      ))}
    </div>
  );
}
