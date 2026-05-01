import { useState, useRef, useEffect, useCallback, forwardRef, useImperativeHandle } from "react";
import i18n from "@/i18n";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Send, Sparkles, CornerDownLeft, Square, Paperclip, X, FileText, Plus, History, Trash2, Wrench, Check, AlertCircle, Wifi, WifiOff, RefreshCw, PanelLeftClose, PanelRightClose, Bug, Zap } from "lucide-react";
import { generateUUID as genUUID } from "@/lib/uuid";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useChatPositionStore } from "@/stores/useChatPositionStore";
import { DotsLoader } from "@/components/DotsLoader";
import { useOpenAIKeyStore } from "@/stores/useOpenAIKeyStore";
import { generateUUID } from "@/lib/uuid";
import type { Pipeline } from "@/types/pipeline";
import type { ProjectEnvGroup, ProjectSpec } from "@/types/project";
import ReactMarkdown from "react-markdown";
import { toast } from "sonner";
import { ScenarioSelector, type Scenario } from "./ScenarioSelector";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";
import {
  mcpConnect,
  mcpCallTool,
  mcpToolsToOpenAI,
  mcpResultToText,
  mcpGetPrompt,
  type McpTool,
} from "@/lib/mcp-client";
import { useProjectStore } from "@/stores/useProjectStore";
import {
  listConversations,
  saveConversation,
  deleteConversation,
  createNewConversation,
  deriveTitle,
  type ChatConversation,
  type ChatMessage,
} from "@/lib/chat-db";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useLocation, useParams } from "react-router-dom";

export interface AIChatRef {
  sendCommand: (content: string, displayTitle: string) => void;
  setActiveContext: (pipeline: Pipeline, testType: "e2e" | "load") => void;
}

interface AIPipelineChatProps {
  projectId: string;
  specs: ProjectSpec[];
  envGroups?: ProjectEnvGroup[];
  pipelines: Pipeline[];
}

type Attachment = { name: string; content: string };

type Msg = {
  role: "user" | "assistant" | "system";
  content: string;
  scenarios?: Scenario[];
  displayTitle?: string;
  attachments?: Attachment[];
};

// --- Detect MCP tools that mutate data (CRUD) and require a UI refresh ---
const MUTATION_PREFIXES = ["create", "update", "delete", "upsert", "insert", "remove", "add", "edit", "patch", "put"];
function isMutationTool(toolName: string): boolean {
  const lower = toolName.toLowerCase();
  return MUTATION_PREFIXES.some((p) => lower.includes(p));
}

// --- JSON repair for truncated tool call args ---

function tryRepairJson(raw: string): any | null {
  let attempt = raw.trim();
  for (let i = 0; i < 10; i++) {
    let braces = 0, brackets = 0;
    for (const ch of attempt) {
      if (ch === "{") braces++;
      else if (ch === "}") braces--;
      else if (ch === "[") brackets++;
      else if (ch === "]") brackets--;
    }
    if (braces === 0 && brackets === 0) break;
    const lastComplete = Math.max(attempt.lastIndexOf(","), attempt.lastIndexOf("}"), attempt.lastIndexOf("]"));
    if (lastComplete > 0 && (braces > 0 || brackets > 0)) {
      const ch = attempt[lastComplete];
      attempt = attempt.slice(0, lastComplete + (ch === "," ? 0 : 1));
    }
    while (brackets > 0) { attempt += "]"; brackets--; }
    while (braces > 0) { attempt += "}"; braces--; }
  }
  try {
    return JSON.parse(attempt);
  } catch {
    return null;
  }
}

// --- System prompt (simplified — tools come from MCP) ---

function getLanguageLabel(): string {
  const lang = i18n.language;
  if (lang.startsWith("pt")) return "Portuguese (Brazil)";
  return "English";
}

function buildSystemPrompt(projectId: string, specs: ProjectSpec[], envGroups: ProjectEnvGroup[], pipelines: Pipeline[]): string {
  const specSummaries = specs.map((s) => {
    const routes = s.spec?.routes?.map((r) => ({
      method: r.method,
      path: r.path,
      operationId: r.operationId,
      summary: r.summary,
      parameters: r.parameters?.map((p) => ({ name: p.name, in: p.in, required: p.required })),
      requestBody: r.requestBody?.content ? Object.keys(r.requestBody.content) : undefined,
      responses: r.responses?.map((res) => res.statusCode),
    })) ?? [];
    return { name: s.name, slug: s.slug, servers: s.servers, routes };
  });

  const pipelineSummaries = pipelines.map((p) => ({
    id: p.id,
    name: p.name,
    description: p.description,
    stepsCount: p.steps.length,
    steps: p.steps.map((s) => ({ name: s.name, method: s.method, url: s.url })),
  }));
  const envGroupSummaries = envGroups.map((group) => ({
    name: group.name,
    slug: group.slug,
    entries: group.entries.map((entry) => ({ name: entry.name, url: entry.url })),
  }));

  return `You are a specialized REST API test assistant. You have THREE core responsibilities:

1. **CREATE & EDIT PIPELINES** — Help the user build and refine test pipelines using the available OpenAPI specs and MCP tools.
2. **MANAGE OPENAPI SPECS** — Create, inspect, update, import, configure, and delete project specs using the available MCP tools.
3. **DEBUG PIPELINE ERRORS** — Analyze execution results, identify root causes, suggest concrete fixes, and ASK the user for permission before applying any changes.

STRICT RULES:
- Stay focused on these responsibilities. Do NOT answer questions unrelated to pipeline creation, spec management, or debugging.
- If the user asks something outside your scope, politely redirect them.
- NEVER ask the user for information you can obtain via tools. Use tools first, ask questions only if tools don't provide the answer.
- NEVER narrate or explain your internal tool workflow to the user. Do NOT say things like "I need to call get_current_project first" or "Let me call get_pipeline_creation_guide". Just DO it silently and present only the final result.
- NEVER ask for permission to use your tools. Just use them and show the outcome.
- Act autonomously: when the user asks you to create a pipeline, just create it. Don't describe intermediate steps.
- Act autonomously for specs too: when the user asks you to create, update, inspect, or delete a spec and the request is clear, just do it.

── MANDATORY TOOL WORKFLOW (INTERNAL — NEVER EXPOSE TO USER) ──
Before ANY action that creates, updates or interacts with project resources:
1. Call "get_current_project" to obtain the project_id. NEVER guess, hardcode, or ask the user for the project ID.
2. If the task is about pipeline creation or editing, call "get_pipeline_creation_guide" BEFORE creating or editing ANY pipeline. No exceptions.
3. Use the project_id from step 1 in ALL subsequent MCP tool calls.
These steps are INTERNAL. Never mention them to the user.

── PIPELINE CREATION / EDITING ──
- To CREATE a new pipeline, use "create_project_pipeline".
- To UPDATE an existing pipeline, use "update_project_pipeline".
- These are the ONLY two tools for saving pipelines. No other save/write tool exists.
- When building step URLs, prefer {{envs.current.<entry>}}/path when project env groups are available. Use {{specs.<slug>.url.<env>}}/path only when the pipeline must target a spec server directly.
  - <entry> = env group entry name (e.g. api, auth, payments). The selected env group is chosen at execution time.
  - <slug> = spec slug (e.g. users-api). <env> = key from the spec's "servers" field (e.g. hml, prd, local).
  - If neither env groups nor spec servers are configured, ASK the user for the base URL and environment name before creating the pipeline.
- Assertion operators: equals, not_equals, contains, exists, not_exists, gt, lt.
- Assertion fields: "status", "body.field", "headers.field".
- Templates: {{steps.STEP_ID.response.body.field}} to chain data between steps.

── OPENAPI SPEC MANAGEMENT ──
- Use the available MCP spec tools to create, inspect, update, configure, import, and delete specs.
- When the user refers to "this spec" or "the current spec", use "get_current_spec".
- Prefer tools such as "get_project_spec", "create_project_spec", "update_project_spec", and "delete_project_spec" when available.
- When updating a spec, preserve existing slug, sync, url, and servers unless the user asked to change them.
- If the user asks to update servers, source URL, or sync behavior, change those fields explicitly.
- If the user provides JSON or YAML content for a spec, use that content as the source of truth.
- If creating a new spec and required fields are missing, ask only for the minimum missing information.

── DEBUGGING & ERROR ANALYSIS ──
- When analyzing a step result with errors or failed assertions:
  1. Explain WHAT went wrong clearly and concisely.
  2. Explain WHY it likely failed (wrong status, missing field, bad assertion, incorrect URL, etc.).
  3. Propose a CONCRETE FIX (show exactly what should change in the pipeline step).
  4. ASK the user: "Shall I apply this fix to the pipeline?" — only proceed if they confirm.
- Use "get_current_pipeline" when the user refers to "this pipeline" or "the current pipeline".
- Use "get_current_spec" when the user refers to "this spec" or "the current spec".

── CONTEXT ──
Available OpenAPI specs:
${JSON.stringify(specSummaries, null, 2)}

Available env groups:
${JSON.stringify(envGroupSummaries, null, 2)}

Existing pipelines:
${JSON.stringify(pipelineSummaries, null, 2)}

Use all available MCP tools to accomplish your tasks. Use free text only for explanations, questions, and conversation.
Always identify pipelines by name in conversation and use the correct ID in tool calls.

── LANGUAGE ──
Always respond to the user in ${getLanguageLabel()}. All explanations, questions, and conversation must be in this language.`;
}

// --- Streaming with tool-use loop ---

interface ToolCallAccum {
  id: string;
  name: string;
  args: string;
}

async function streamOpenAISinglePass(
  apiKey: string,
  messages: Array<{ role: string; content: string | null; tool_call_id?: string; tool_calls?: any[] }>,
  tools: any[],
  onDelta: (text: string) => void,
  signal?: AbortSignal,
  model: string = "gpt-5.2",
): Promise<{ toolCalls: ToolCallAccum[]; textContent: string }> {
  const body: any = { model, messages, stream: true, max_completion_tokens: 16384 };
  if (tools.length > 0) body.tools = tools;

  const resp = await fetch("https://api.openai.com/v1/chat/completions", {
    method: "POST",
    headers: { Authorization: `Bearer ${apiKey}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });

  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`OpenAI error ${resp.status}: ${text}`);
  }

  const reader = resp.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const toolCallsMap: Map<number, ToolCallAccum> = new Map();
  let textContent = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    let newlineIndex: number;
    while ((newlineIndex = buffer.indexOf("\n")) !== -1) {
      let line = buffer.slice(0, newlineIndex);
      buffer = buffer.slice(newlineIndex + 1);
      if (line.endsWith("\r")) line = line.slice(0, -1);
      if (!line.startsWith("data: ")) continue;
      const jsonStr = line.slice(6).trim();
      if (jsonStr === "[DONE]") {
        return { toolCalls: Array.from(toolCallsMap.values()), textContent };
      }
      try {
        const parsed = JSON.parse(jsonStr);
        const delta = parsed.choices?.[0]?.delta;
        if (delta?.content) {
          textContent += delta.content;
          onDelta(delta.content);
        }
        if (delta?.tool_calls) {
          for (const tc of delta.tool_calls) {
            const idx = tc.index ?? 0;
            if (!toolCallsMap.has(idx)) {
              toolCallsMap.set(idx, { id: tc.id || `call_${idx}`, name: "", args: "" });
            }
            const entry = toolCallsMap.get(idx)!;
            if (tc.id) entry.id = tc.id;
            if (tc.function?.name) entry.name = tc.function.name;
            if (tc.function?.arguments) entry.args += tc.function.arguments;
          }
        }
      } catch {
        buffer = line + "\n" + buffer;
        break;
      }
    }
  }

  return { toolCalls: Array.from(toolCallsMap.values()), textContent };
}

// --- Main component ---

export const AIPipelineChat = forwardRef<AIChatRef, AIPipelineChatProps>(function AIPipelineChat(
  { projectId, specs, envGroups = [], pipelines },
  ref
) {
  const location = useLocation();
  const { specId } = useParams<{ specId?: string }>();
  const apiKey = useOpenAIKeyStore((s) => s.apiKey);
  const selectedModel = useOpenAIKeyStore((s) => s.model);
  const orchUrl = useOrchestratorStore((s) => s.url);
  const chatPosition = useChatPositionStore((s) => s.position);
  const toggleCollapsed = useChatPositionStore((s) => s.toggleCollapsed);
  const currentProject = useProjectStore((s) => s.currentProject);
  const makeWelcome = useCallback((): Msg => ({ role: "assistant", content: i18n.t("chat.welcome") }), []);
  const [messages, setMessages] = useState<Msg[]>(() => [makeWelcome()]);
  const [input, setInput] = useState("");
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [activeToolCalls, setActiveToolCalls] = useState<Array<{ name: string; status: "running" | "done" | "error" }>>([]);
  const [suggestions, setSuggestions] = useState<Array<{ title: string; message: string }>>([]);
  const [loadingSuggestions, setLoadingSuggestions] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);
  const abortRef = useRef<AbortController | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const systemPrompt = useRef(buildSystemPrompt(projectId, specs, pipelines));

  // --- Solve Now (auto-pilot) state ---
  const [solveNowMode, setSolveNowMode] = useState(false);
  const [solveIterationCount, setSolveIterationCount] = useState(0);
  const SOLVE_MAX_ITERATIONS = 10;
  const solveNowModeRef = useRef(false);
  const mcpPromptRef = useRef<string | null>(null);
  const activeSpecRef = useRef<{
    viewing: boolean;
    mode?: "editor" | "try-it" | "diff";
    isNew?: boolean;
    spec?: ProjectSpec | null;
  }>({ viewing: false });

  // Keep ref in sync with state
  useEffect(() => { solveNowModeRef.current = solveNowMode; }, [solveNowMode]);

  useEffect(() => {
    const pathname = location.pathname;
    const isSpecRoute = pathname.includes("/specs/");

    if (!isSpecRoute) {
      activeSpecRef.current = { viewing: false };
      return;
    }

    const mode = pathname.endsWith("/try-it")
      ? "try-it"
      : pathname.endsWith("/diff")
        ? "diff"
        : "editor";

    if (specId === "new" || pathname.includes("/specs/new/")) {
      activeSpecRef.current = { viewing: true, mode, isNew: true, spec: null };
      return;
    }

    const currentSpec = currentProject?.specs.find((spec) => spec.id === specId) ?? null;
    activeSpecRef.current = { viewing: true, mode, isNew: false, spec: currentSpec };
  }, [currentProject, location.pathname, specId]);

  // --- Client-side virtual tool definition (always available) ---
  const SUGGEST_SCENARIOS_TOOL = {
    type: "function" as const,
    function: {
      name: "suggest_scenarios",
      description: "Present interactive test scenario suggestions to the user. The scenarios will be displayed as selectable cards grouped by category. The user can pick which ones to generate as pipelines.",
      parameters: {
        type: "object",
        properties: {
          scenarios: {
            type: "array",
            items: {
              type: "object",
              properties: {
                id: { type: "string", description: "Unique ID for the scenario" },
                category: { type: "string", description: "Category/group name (e.g. entity name or resource)" },
                title: { type: "string", description: "Short scenario title" },
                description: { type: "string", description: "Brief description of what the test covers" },
              },
              required: ["id", "category", "title", "description"],
            },
          },
        },
        required: ["scenarios"],
      },
    },
  };

  const GET_CURRENT_PIPELINE_TOOL = {
    type: "function" as const,
    function: {
      name: "get_current_pipeline",
      description: "Returns the pipeline the user is currently viewing in the UI, including all steps, assertions, headers, and configuration. Use this to understand what the user is looking at right now.",
      parameters: {
        type: "object",
        properties: {},
        required: [],
      },
    },
  };

  const GET_CURRENT_PROJECT_TOOL = {
    type: "function" as const,
    function: {
      name: "get_current_project",
      description: "Returns the current project ID and name. You MUST call this before executing any MCP tool that requires a project_id parameter.",
      parameters: {
        type: "object",
        properties: {},
        required: [],
      },
    },
  };

  const GET_CURRENT_SPEC_TOOL = {
    type: "function" as const,
    function: {
      name: "get_current_spec",
      description: "Returns the OpenAPI spec the user is currently viewing in the UI, including metadata and spec contents when available. Use this when the user refers to the current spec.",
      parameters: {
        type: "object",
        properties: {},
        required: [],
      },
    },
  };

  // --- MCP tools ---
  const [mcpTools, setMcpTools] = useState<McpTool[]>([]);
  const [mcpOnlyTools, setMcpOnlyTools] = useState<any[]>([]);
  const openAITools = [...mcpOnlyTools, SUGGEST_SCENARIOS_TOOL, GET_CURRENT_PIPELINE_TOOL, GET_CURRENT_PROJECT_TOOL, GET_CURRENT_SPEC_TOOL];
  const [mcpStatus, setMcpStatus] = useState<"disconnected" | "connecting" | "connected">("disconnected");
  const mcpSessionRef = useRef<{ url: string; sessionId: string } | null>(null);

  const mcpUrl = orchUrl ? `${orchUrl.replace(/\/+$/, "")}/mcp` : null;

  const mcpCancelRef = useRef<() => void>(() => { });

  const startMcpDiscovery = useCallback(() => {
    if (!mcpUrl) {
      setMcpTools([]);
      setMcpOnlyTools([]);
      setMcpStatus("disconnected");
      mcpSessionRef.current = null;
      return;
    }
    // Cancel any previous discovery
    mcpCancelRef.current();
    let cancelled = false;
    mcpCancelRef.current = () => { cancelled = true; };
    let attempt = 0;
    const MAX_RETRIES = 5;
    const BASE_DELAY = 2000;

    setMcpStatus("connecting");

    const discover = () => {
      mcpConnect(mcpUrl)
        .then(async ({ session, tools }) => {
          if (cancelled) return;
          mcpSessionRef.current = session;
          setMcpTools(tools);
          setMcpOnlyTools(mcpToolsToOpenAI(tools));
          setMcpStatus("connected");

          // Fetch system prompt from MCP server
          try {
            const promptMessages = await mcpGetPrompt(session, "default");
            if (promptMessages && promptMessages.length > 0) {
              const promptText = promptMessages
                .filter((m) => m.content?.type === "text" && m.content?.text)
                .map((m) => m.content.text)
                .join("\n\n");
              if (promptText) {
                mcpPromptRef.current = promptText;
                systemPrompt.current = promptText;
                console.log("[MCP] Using server-side system prompt");
              }
            }
          } catch (err) {
            console.warn("[MCP] Failed to fetch prompt, using local fallback:", err);
          }
        })
        .catch((err) => {
          if (cancelled) return;
          attempt++;
          if (attempt <= MAX_RETRIES) {
            const delay = BASE_DELAY * Math.pow(2, attempt - 1);
            console.warn(`MCP discovery failed (attempt ${attempt}/${MAX_RETRIES}), retrying in ${delay}ms...`, err);
            setMcpStatus("connecting");
            setTimeout(() => { if (!cancelled) discover(); }, delay);
          } else {
            console.warn("MCP discovery failed after all retries:", err);
            mcpSessionRef.current = null;
            setMcpTools([]);
            setMcpOnlyTools([]);
            setMcpStatus("disconnected");
          }
        });
    };

    discover();
  }, [mcpUrl]);

  useEffect(() => {
    startMcpDiscovery();
    return () => { mcpCancelRef.current(); };
  }, [startMcpDiscovery]);

  // --- Conversation persistence ---
  const [conversationId, setConversationId] = useState<string>(() => generateUUID());
  const [historyList, setHistoryList] = useState<ChatConversation[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    const hasUserMsg = messages.some((m) => m.role === "user");
    if (!hasUserMsg) return;

    saveTimerRef.current = setTimeout(() => {
      const chatMessages: ChatMessage[] = messages.map((m) => ({
        role: m.role,
        content: m.content,
        displayTitle: m.displayTitle,
        attachments: m.attachments,
      }));
      const now = new Date().toISOString();
      const conv: ChatConversation = {
        id: conversationId,
        projectId,
        title: deriveTitle(chatMessages),
        messages: chatMessages,
        createdAt: now,
        updatedAt: now,
      };
      saveConversation(conv).catch(console.error);
    }, 800);

    return () => { if (saveTimerRef.current) clearTimeout(saveTimerRef.current); };
  }, [messages, conversationId, projectId]);

  const refreshHistory = useCallback(async () => {
    const list = await listConversations(projectId);
    setHistoryList(list);
  }, [projectId]);

  useEffect(() => {
    if (historyOpen) refreshHistory();
  }, [historyOpen, refreshHistory]);

  const handleNewConversation = useCallback(() => {
    // Abort any running stream
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
    setIsStreaming(false);
    setActiveToolCalls([]);
    setSolveNowMode(false);
    setSolveIterationCount(0);
    setConversationId(generateUUID());
    setMessages([makeWelcome()]);
    setInput("");
    setAttachments([]);
    setSuggestions([]);
  }, []);

  const handleLoadConversation = useCallback((conv: ChatConversation) => {
    setConversationId(conv.id);
    const restored: Msg[] = conv.messages.map((m) => ({
      role: m.role,
      content: m.content,
      displayTitle: m.displayTitle,
      attachments: m.attachments,
    }));
    setMessages(restored.length > 0 ? restored : [makeWelcome()]);
    setHistoryOpen(false);
  }, []);

  const handleDeleteConversation = useCallback(async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    await deleteConversation(id);
    if (id === conversationId) handleNewConversation();
    refreshHistory();
  }, [conversationId, handleNewConversation, refreshHistory]);

  useEffect(() => {
    if (mcpPromptRef.current) {
      // MCP prompt is the base; append dynamic context
      const specSummaries = specs.map((s) => {
        const routes = s.spec?.routes?.map((r) => ({
          method: r.method, path: r.path, operationId: r.operationId,
        })) ?? [];
        return { name: s.name, slug: s.slug, servers: s.servers, routes };
      });
      const pipelineSummaries = pipelines.map((p) => ({
        id: p.id, name: p.name, stepsCount: p.steps.length,
        steps: p.steps.map((s) => ({ name: s.name, method: s.method, url: s.url })),
      }));
      const envGroupSummaries = envGroups.map((group) => ({
        name: group.name,
        slug: group.slug,
        entries: group.entries.map((entry) => ({ name: entry.name, url: entry.url })),
      }));
      systemPrompt.current = `${mcpPromptRef.current}

── ADDITIONAL FRONTEND CONTRACT ──
You must also support OpenAPI spec CRUD and spec configuration tasks, not only pipelines.
When the user asks to create, inspect, update, configure, import, or delete specs, use the available MCP spec tools directly.
Use "get_current_spec" when the user refers to the current spec.

── CONTEXT ──
Available OpenAPI specs:
${JSON.stringify(specSummaries, null, 2)}

Available env groups:
${JSON.stringify(envGroupSummaries, null, 2)}

Existing pipelines:
${JSON.stringify(pipelineSummaries, null, 2)}

── LANGUAGE ──
Always respond to the user in ${getLanguageLabel()}. All explanations, questions, and conversation must be in this language.`;
    } else {
      systemPrompt.current = buildSystemPrompt(projectId, specs, envGroups, pipelines);
    }
  }, [projectId, specs, envGroups, pipelines]);

  useEffect(() => {
    const onLangChange = () => {
      setMessages((prev) => {
        if (prev.length === 1 && prev[0].role === "assistant") {
          return [makeWelcome()];
        }
        return prev;
      });
    };
    i18n.on("languageChanged", onLangChange);
    return () => { i18n.off("languageChanged", onLangChange); };
  }, [makeWelcome]);

  useEffect(() => {
    if (autoScrollRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, activeToolCalls]);

  // --- File handling ---

  const handleFileSelect = useCallback(async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files) return;
    const newAttachments: Attachment[] = [];
    for (const file of Array.from(files)) {
      if (file.size > 1024 * 1024) {
        toast.error(i18n.t("chat.fileExceedsLimit", { name: file.name }));
        continue;
      }
      try {
        const text = await file.text();
        newAttachments.push({ name: file.name, content: text });
      } catch {
        toast.error(i18n.t("chat.fileReadError", { name: file.name }));
      }
    }
    setAttachments((prev) => [...prev, ...newAttachments]);
    if (fileInputRef.current) fileInputRef.current.value = "";
  }, []);

  const removeAttachment = useCallback((index: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== index));
  }, []);

  // --- Fetch follow-up suggestions (background, non-blocking) ---

  const fetchSuggestions = useCallback(async (conversationMessages: Msg[]) => {
    if (!apiKey) return;
    setLoadingSuggestions(true);
    try {
      const lastMessages = conversationMessages.slice(-10); // limit context for speed
      const apiMessages = [
        { role: "system", content: systemPrompt.current },
        ...lastMessages
          .filter((m) => m.role !== "system" || m.displayTitle)
          .map((m) => ({ role: m.displayTitle ? "user" : m.role, content: m.content })),
         {
           role: "user",
           content: solveNowModeRef.current
             ? "Based on this conversation, suggest 2-4 short follow-up actions the user might want to take next. Focus on pipeline creation, pipeline debugging, or OpenAPI spec management tasks. Respond in the same language the user has been using. IMPORTANT: Set priority=1 on the single most impactful suggestion that will best advance solving the current problem. All other suggestions should have priority=0."
             : "Based on this conversation, suggest 2-4 short follow-up actions the user might want to take next. Focus on pipeline creation, pipeline debugging, or OpenAPI spec management tasks. Respond in the same language the user has been using.",
         },
       ];

       const body: any = {
         model: selectedModel,
         messages: apiMessages,
         max_completion_tokens: 512,
         tools: [
           {
             type: "function",
             function: {
               name: "suggest_followups",
               description: "Return 2-4 follow-up action suggestions for the user.",
               parameters: {
                 type: "object",
                 properties: {
                   suggestions: {
                     type: "array",
                     items: {
                       type: "object",
                       properties: {
                         title: { type: "string", description: "Short label (3-6 words)" },
                         message: { type: "string", description: "Full message to send to the assistant" },
                         priority: { type: "number", description: "1 = most relevant to solve the current issue, 0 = other option. Exactly one suggestion should be priority=1." },
                       },
                       required: ["title", "message", "priority"],
                     },
                   },
                 },
                 required: ["suggestions"],
               },
             },
           },
         ],
         tool_choice: { type: "function", function: { name: "suggest_followups" } },
       };

      const resp = await fetch("https://api.openai.com/v1/chat/completions", {
        method: "POST",
        headers: { Authorization: `Bearer ${apiKey}`, "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });

      if (!resp.ok) {
        const errText = await resp.text().catch(() => "");
        console.warn("Suggestions API error:", resp.status, errText);
        return;
      }

      const data = await resp.json();
      console.log("Suggestions API response:", JSON.stringify(data.choices?.[0]?.message, null, 2));
      const toolCall = data.choices?.[0]?.message?.tool_calls?.[0];
      if (toolCall?.function?.arguments) {
        try {
          const parsed = JSON.parse(toolCall.function.arguments);
          if (Array.isArray(parsed.suggestions)) {
            setSuggestions(parsed.suggestions.slice(0, 4));
          }
        } catch {
          const repaired = tryRepairJson(toolCall.function.arguments);
          if (repaired?.suggestions) setSuggestions(repaired.suggestions.slice(0, 4));
        }
      } else {
        console.warn("Suggestions: no tool_calls in response", data.choices?.[0]?.message);
      }
    } catch (err) {
      console.warn("Failed to fetch suggestions:", err);
    } finally {
      setLoadingSuggestions(false);
    }
  }, [apiKey, selectedModel]);

  // --- Tool-use loop: stream → collect tool_calls → call MCP → feed back → repeat ---

  const runToolUseLoop = useCallback(async (
    initialApiMessages: Array<{ role: string; content: string | null; tool_call_id?: string }>,
    onAssistantDelta: (chunk: string) => void,
    onScenariosDetected: (scenarios: Scenario[]) => void,
    signal: AbortSignal,
    onToolActivity: (calls: Array<{ name: string; status: "running" | "done" | "error" }>) => void,
  ) => {
    let apiMessages = [...initialApiMessages];

    while (true) {
      let fullAssistantText = "";

      const { toolCalls, textContent } = await streamOpenAISinglePass(
        apiKey!,
        apiMessages,
        openAITools,
        (chunk) => {
          fullAssistantText += chunk;
          onAssistantDelta(chunk);
        },
        signal,
        selectedModel,
      );

      // No tool calls → done
      if (toolCalls.length === 0) break;

      // Build assistant message with tool_calls for the conversation
      const assistantMsg: any = {
        role: "assistant",
        content: textContent || null,
        tool_calls: toolCalls.map((tc) => ({
          id: tc.id,
          type: "function",
          function: { name: tc.name, arguments: tc.args },
        })),
      };
      apiMessages.push(assistantMsg);

      // Show all tool calls as "running"
      const toolStatus: Array<{ name: string; status: "running" | "done" | "error" }> = toolCalls.map((tc) => ({ name: tc.name, status: "running" as const }));
      onToolActivity([...toolStatus]);

      // Execute each tool call against MCP (or handle client-side virtual tools)
      for (let i = 0; i < toolCalls.length; i++) {
        const tc = toolCalls[i];
        let parsedArgs: Record<string, unknown> = {};
        try {
          parsedArgs = JSON.parse(tc.args);
        } catch {
          const repaired = tryRepairJson(tc.args);
          if (repaired) parsedArgs = repaired;
        }

        // --- Client-side virtual tool: suggest_scenarios ---
        if (tc.name === "suggest_scenarios") {
          const argsScenarios = (parsedArgs as any).scenarios;
          if (Array.isArray(argsScenarios) && argsScenarios.length > 0) {
            onScenariosDetected(argsScenarios);
          }
          apiMessages.push({
            role: "tool",
            content: JSON.stringify({ ok: true, count: argsScenarios?.length ?? 0 }),
            tool_call_id: tc.id,
          });
          toolStatus[i] = { name: tc.name, status: "done" };
          onToolActivity([...toolStatus]);
          continue;
        }

        // --- Client-side virtual tool: get_current_pipeline ---
        if (tc.name === "get_current_pipeline") {
          const active = activePipelineRef.current;
          const result = active
            ? { viewing: true, testType: active.testType, pipeline: active.pipeline }
            : { viewing: false, message: "The user is not currently viewing any pipeline." };
          apiMessages.push({
            role: "tool",
            content: JSON.stringify(result, null, 2),
            tool_call_id: tc.id,
          });
          toolStatus[i] = { name: tc.name, status: "done" };
          onToolActivity([...toolStatus]);
          continue;
        }

        // --- Client-side virtual tool: get_current_project ---
        if (tc.name === "get_current_project") {
          const project = useProjectStore.getState().currentProject;
          const result = project
            ? { projectId: project.id, name: project.name }
            : { projectId, name: "Unknown" };
          apiMessages.push({
            role: "tool",
            content: JSON.stringify(result, null, 2),
            tool_call_id: tc.id,
          });
          toolStatus[i] = { name: tc.name, status: "done" };
          onToolActivity([...toolStatus]);
          continue;
        }

        // --- Client-side virtual tool: get_current_spec ---
        if (tc.name === "get_current_spec") {
          const activeSpec = activeSpecRef.current;
          const result = activeSpec.viewing
            ? activeSpec.isNew
              ? {
                  viewing: true,
                  mode: activeSpec.mode,
                  isNew: true,
                  message: "The user is creating a new spec and there is no saved spec yet.",
                }
              : {
                  viewing: true,
                  mode: activeSpec.mode,
                  isNew: false,
                  spec: activeSpec.spec
                    ? {
                        id: activeSpec.spec.id,
                        name: activeSpec.spec.name,
                        slug: activeSpec.spec.slug,
                        url: activeSpec.spec.url,
                        sync: activeSpec.spec.sync,
                        servers: activeSpec.spec.servers,
                        specMd5: activeSpec.spec.specMd5,
                        spec: activeSpec.spec.spec,
                      }
                    : null,
                }
            : { viewing: false, message: "The user is not currently viewing any spec." };
          apiMessages.push({
            role: "tool",
            content: JSON.stringify(result, null, 2),
            tool_call_id: tc.id,
          });
          toolStatus[i] = { name: tc.name, status: "done" };
          onToolActivity([...toolStatus]);
          continue;
        }

        if (!mcpUrl) {
          apiMessages.push({
            role: "tool",
            content: "Error: No backend configured (orchestrator URL not set).",
            tool_call_id: tc.id,
          });
          toolStatus[i] = { name: tc.name, status: "error" };
          onToolActivity([...toolStatus]);
          continue;
        }

        try {
          const result = await mcpCallTool(mcpSessionRef.current!, tc.name, parsedArgs);
          const resultText = mcpResultToText(result);

          apiMessages.push({
            role: "tool",
            content: resultText || "OK",
            tool_call_id: tc.id,
          });
          toolStatus[i] = { name: tc.name, status: result.isError ? "error" : "done" };
          onToolActivity([...toolStatus]);

          // Refresh project data after CRUD mutations
          if (!result.isError && isMutationTool(tc.name)) {
            useProjectStore.getState().loadProject(projectId).catch(() => { });
          }
        } catch (err: any) {
          apiMessages.push({
            role: "tool",
            content: `Error calling ${tc.name}: ${err.message}`,
            tool_call_id: tc.id,
          });
          toolStatus[i] = { name: tc.name, status: "error" };
          onToolActivity([...toolStatus]);
        }
      }

      // Clear tool indicators before next loop iteration
      onToolActivity([]);
      // Loop continues: OpenAI will process tool results and may call more tools or respond with text
    }
  }, [apiKey, openAITools, mcpUrl, selectedModel]);

  // --- Send message (internal) ---

  const sendMessageInternal = useCallback(async (text: string, currentMessages: Msg[]) => {
    if (!apiKey || isStreaming) return;
    if (!text.trim() && attachments.length === 0) return;
    autoScrollRef.current = true;

    let fullContent = text.trim();
    if (attachments.length > 0) {
      const fileBlocks = attachments.map((a) => `\n\n---\n📎 **${a.name}**\n\`\`\`\n${a.content}\n\`\`\``).join("");
      fullContent = fullContent ? fullContent + fileBlocks : `${i18n.t("chat.analyzeFiles")}${fileBlocks}`;
    }

    const userMsg: Msg = { role: "user", content: fullContent, attachments: attachments.length > 0 ? [...attachments] : undefined };
    const updatedMessages = [...currentMessages, userMsg];
    setMessages(updatedMessages);
    setInput("");
    setAttachments([]);
    setSuggestions([]);
    setIsStreaming(true);

    let assistantContent = "";
    const controller = new AbortController();
    abortRef.current = controller;

    const apiMessages = [
      { role: "system", content: systemPrompt.current },
      ...updatedMessages.map((m) => ({ role: m.displayTitle ? "user" : m.role, content: m.displayTitle ? m.content : m.content })),
    ];

    let scenariosForMessage: Scenario[] | undefined;

    try {
      await runToolUseLoop(
        apiMessages,
        (chunk) => {
          assistantContent += chunk;
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (last?.role === "assistant" && prev.length > updatedMessages.length) {
              return prev.map((m, i) => (i === prev.length - 1 ? { ...m, content: assistantContent, scenarios: scenariosForMessage } : m));
            }
            return [...prev, { role: "assistant", content: assistantContent, scenarios: scenariosForMessage }];
          });
        },
        (scenarios) => {
          scenariosForMessage = scenarios;
          // Update the current assistant message with scenarios
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (last?.role === "assistant") {
              return prev.map((m, i) => (i === prev.length - 1 ? { ...m, scenarios } : m));
            }
            return [...prev, { role: "assistant", content: assistantContent || i18n.t("chat.scenariosIntro"), scenarios }];
          });
        },
        controller.signal,
        (calls) => setActiveToolCalls(calls),
      );
    } catch (err: any) {
      if (err.name !== "AbortError") {
        const errorMsg = err.message || i18n.t("chat.openaiError");
        setMessages((prev) => [...prev, { role: "assistant", content: i18n.t("chat.errorPrefix", { error: errorMsg }) }]);
      }
    } finally {
      setIsStreaming(false);
      setActiveToolCalls([]);
      abortRef.current = null;
      // Fetch suggestions in background (use a microtask to read final messages)
      setTimeout(() => {
        setMessages((prev) => {
          fetchSuggestions(prev);
          return prev;
        });
      }, 100);
    }
  }, [apiKey, isStreaming, attachments, runToolUseLoop]);

  // --- Send command (for system-action messages triggered by buttons) ---

  const sendCommand = useCallback(async (content: string, displayTitle: string) => {
    if (!apiKey || isStreaming) return;

    const actionMsg: Msg = { role: "system", content, displayTitle };
    const updatedMessages = [...messages, actionMsg];
    setMessages(updatedMessages);
    setSuggestions([]);
    setIsStreaming(true);

    let assistantContent = "";
    const controller = new AbortController();
    abortRef.current = controller;

    const apiMessages = [
      { role: "system", content: systemPrompt.current },
      ...updatedMessages.map((m) => ({ role: m.displayTitle ? "user" as const : m.role, content: m.content })),
    ];

    let scenariosForMessage: Scenario[] | undefined;

    try {
      await runToolUseLoop(
        apiMessages,
        (chunk) => {
          assistantContent += chunk;
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (last?.role === "assistant" && prev.length > updatedMessages.length) {
              return prev.map((m, i) => (i === prev.length - 1 ? { ...m, content: assistantContent, scenarios: scenariosForMessage } : m));
            }
            return [...prev, { role: "assistant", content: assistantContent, scenarios: scenariosForMessage }];
          });
        },
        (scenarios) => {
          scenariosForMessage = scenarios;
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (last?.role === "assistant") {
              return prev.map((m, i) => (i === prev.length - 1 ? { ...m, scenarios } : m));
            }
            return [...prev, { role: "assistant", content: assistantContent || i18n.t("chat.scenariosIntro"), scenarios }];
          });
        },
        controller.signal,
        (calls) => setActiveToolCalls(calls),
      );
    } catch (err: any) {
      if (err.name !== "AbortError") {
        const errorMsg = err.message || i18n.t("chat.openaiError");
        setMessages((prev) => [...prev, { role: "assistant", content: i18n.t("chat.errorPrefix", { error: errorMsg }) }]);
      }
    } finally {
      setIsStreaming(false);
      setActiveToolCalls([]);
      abortRef.current = null;
      // Fetch suggestions in background
      setTimeout(() => {
        setMessages((prev) => {
          fetchSuggestions(prev);
          return prev;
        });
      }, 100);
    }
  }, [apiKey, messages, isStreaming, runToolUseLoop]);

  // --- Active context injection ---
  const lastContextKeyRef = useRef<string>("");
  const activePipelineRef = useRef<{ pipeline: Pipeline; testType: "e2e" | "load" } | null>(null);

  const setActiveContext = useCallback((pipeline: Pipeline, testType: "e2e" | "load") => {
    const contextKey = `${pipeline.id}::${testType}`;
    if (contextKey === lastContextKeyRef.current) return;
    lastContextKeyRef.current = contextKey;

    activePipelineRef.current = { pipeline, testType };

    const detail = {
      id: pipeline.id,
      name: pipeline.name,
      description: pipeline.description,
      steps: pipeline.steps.map((s) => ({
        id: s.id,
        name: s.name,
        method: s.method,
        url: s.url,
        headers: s.headers,
        body: s.body,
        asserts: s.asserts,
        delay: s.delay,
        retry: s.retry,
        operationId: s.operationId,
      })),
    };

    const content = `[CONTEXT UPDATE] The user is now viewing the following pipeline in ${testType} test mode:\n${JSON.stringify(detail, null, 2)}\nUse this context to answer questions about this pipeline without needing to call get_pipeline.`;

    setMessages((prev) => [...prev, { role: "system", content }]);
  }, []);

  useImperativeHandle(ref, () => ({ sendCommand, setActiveContext }), [sendCommand, setActiveContext]);

  // --- Public send message ---
  const sendMessage = useCallback(async (text: string) => {
    await sendMessageInternal(text, messages);
  }, [sendMessageInternal, messages]);

  // (handleGenerateFromScenarios and handleKeyDown moved after handleSolveNow)

  // --- Auto-pilot: when solve mode is on, auto-pick first suggestion ---

  useEffect(() => {
    if (
      solveNowModeRef.current &&
      suggestions.length > 0 &&
      !isStreaming &&
      solveIterationCount < SOLVE_MAX_ITERATIONS
    ) {
      const timer = setTimeout(() => {
        if (!solveNowModeRef.current) return; // may have been toggled off
        const bestSuggestion = suggestions.find((s: any) => s.priority === 1) || suggestions[0];
        const nextAction = bestSuggestion.message;
        setSolveIterationCount((prev) => prev + 1);
        setSuggestions([]);
        sendMessage(nextAction);
      }, 1500);
      return () => clearTimeout(timer);
    }
    if (solveNowModeRef.current && solveIterationCount >= SOLVE_MAX_ITERATIONS && !isStreaming) {
      toast.info("Auto-pilot reached iteration limit (10). Stopping.");
      setSolveNowMode(false);
    }
  }, [suggestions, isStreaming, solveIterationCount]);

  // --- Generate from selected scenarios ---

  const handleGenerateFromScenarios = useCallback((selected: Scenario[]) => {
    const list = selected.map((s) => `- [${s.category}] ${s.title}: ${s.description}`).join("\n");
    const msg = i18n.t("chat.generateFromScenarios", { list });
    sendMessage(msg);
  }, [sendMessage]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (solveNowMode) setSolveIterationCount(0); // reset on manual send
      sendMessage(input);
    }
  };

  return (
    <div className="glass relative flex h-full min-h-0 flex-col">

      {/* Header */}
      <div className="flex items-center justify-between px-4 h-14 shadow-xs">
        <div className="flex items-center gap-2">
          <Sparkles className="h-4 w-4 text-primary" />
          <h2 className="text-sm font-semibold">{i18n.t("chat.title")}</h2>
          {mcpStatus === "connected" && (
            <span className="inline-flex items-center gap-1 text-[10px] font-medium text-primary bg-primary/10 rounded-full px-2 py-0.5" title={`${mcpTools.length} tools available`}>
              <Wifi className="h-2.5 w-2.5" />
              {mcpTools.length} tools
            </span>
          )}
          {mcpStatus === "connecting" && (
            <span className="inline-flex items-center gap-1 text-[10px] font-medium text-muted-foreground rounded-full px-2 py-0.5 animate-pulse" title="Connecting to MCP server...">
              <RefreshCw className="h-2.5 w-2.5 animate-spin" />
              connecting
            </span>
          )}
          {mcpStatus === "disconnected" && orchUrl && (
            <button
              onClick={startMcpDiscovery}
              className="inline-flex items-center gap-1 text-[10px] font-medium text-destructive bg-destructive/10 hover:bg-destructive/20 rounded-full px-2 py-0.5 transition-colors cursor-pointer"
              title="Click to retry MCP connection"
            >
              <WifiOff className="h-2.5 w-2.5" />
              offline
              <RefreshCw className="h-2.5 w-2.5 ml-0.5" />
            </button>
          )}
        </div>
        <div className="flex items-center gap-1">
          <Dialog>
            <DialogTrigger asChild>
              <Button
                size="icon"
                variant="ghost"
                className="h-7 w-7 rounded-lg text-muted-foreground hover:text-foreground"
                title="Debug: view prompts & tools"
              >
                <Bug className="h-3.5 w-3.5" />
              </Button>
            </DialogTrigger>
            <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
              <DialogHeader>
                <DialogTitle>Chat Debug</DialogTitle>
              </DialogHeader>
              <Tabs defaultValue="system" className="flex-1 overflow-hidden flex flex-col">
                <TabsList className="w-full justify-start">
                  <TabsTrigger value="system">System Prompt</TabsTrigger>
                  <TabsTrigger value="messages">All Messages ({messages.length})</TabsTrigger>
                  <TabsTrigger value="tools">Tools ({openAITools.length})</TabsTrigger>
                </TabsList>
                <TabsContent value="system" className="flex-1 overflow-hidden mt-2">
                  <ScrollArea className="h-[55vh]">
                    <pre className="text-xs whitespace-pre-wrap break-words p-3 bg-muted/50 rounded-lg font-mono text-foreground">
                      {systemPrompt.current}
                    </pre>
                  </ScrollArea>
                </TabsContent>
                <TabsContent value="messages" className="flex-1 overflow-hidden mt-2">
                  <ScrollArea className="h-[55vh]">
                    <div className="space-y-2 p-1">
                      {messages.map((msg, i) => (
                        <div key={i} className="border border-border/40 rounded-lg p-2">
                          <div className="flex items-center gap-2 mb-1">
                            <span className={`text-[10px] font-bold uppercase px-1.5 py-0.5 rounded ${
                              msg.role === "system" ? "bg-warning/20 text-warning" :
                              msg.role === "user" ? "bg-primary/20 text-primary" :
                              "bg-success/20 text-success"
                            }`}>{msg.role}</span>
                            {msg.displayTitle && <span className="text-[10px] text-muted-foreground">({msg.displayTitle})</span>}
                          </div>
                          <pre className="text-[11px] whitespace-pre-wrap break-words font-mono text-foreground/80 max-h-40 overflow-auto">
                            {msg.content.length > 500 ? msg.content.slice(0, 500) + "…" : msg.content}
                          </pre>
                        </div>
                      ))}
                    </div>
                  </ScrollArea>
                </TabsContent>
                <TabsContent value="tools" className="flex-1 overflow-hidden mt-2">
                  <ScrollArea className="h-[55vh]">
                    <div className="space-y-2 p-1">
                      {openAITools.map((tool, i) => {
                        const fn = tool.function;
                        return (
                          <details key={i} className="border border-border/40 rounded-lg">
                            <summary className="px-3 py-2 cursor-pointer text-xs font-medium text-foreground hover:bg-muted/30 rounded-lg">
                              <span className="font-mono text-primary">{fn.name}</span>
                              <span className="ml-2 text-muted-foreground font-normal">{fn.description?.slice(0, 80)}{(fn.description?.length ?? 0) > 80 ? "…" : ""}</span>
                            </summary>
                            <pre className="text-[11px] whitespace-pre-wrap break-words font-mono p-3 bg-muted/30 text-foreground/80">
                              {JSON.stringify(fn.parameters, null, 2)}
                            </pre>
                          </details>
                        );
                      })}
                    </div>
                  </ScrollArea>
                </TabsContent>
              </Tabs>
            </DialogContent>
          </Dialog>
          <Button
            size="icon"
            variant="ghost"
            className="h-7 w-7 rounded-lg text-muted-foreground hover:text-foreground"
            onClick={toggleCollapsed}
            title="Collapse chat"
          >
            {chatPosition === "right" ? <PanelRightClose className="h-3.5 w-3.5" /> : <PanelLeftClose className="h-3.5 w-3.5" />}
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-7 w-7 rounded-lg text-muted-foreground hover:text-foreground"
            onClick={handleNewConversation}
            
            title={i18n.t("chat.newConversation")}
          >
            <Plus className="h-3.5 w-3.5" />
          </Button>
          <Popover open={historyOpen} onOpenChange={setHistoryOpen}>
            <PopoverTrigger asChild>
              <Button
                size="icon"
                variant="ghost"
                className="h-7 w-7 rounded-lg text-muted-foreground hover:text-foreground"
                disabled={isStreaming}
                title={i18n.t("chat.historyTitle")}
              >
                <History className="h-3.5 w-3.5" />
              </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-72 p-0 rounded-lg border border-border/60 shadow-md">
              <div className="px-3 py-2.5 border-border/50 bg-muted/30">
                <p className="text-xs font-semibold text-foreground">{i18n.t("chat.historyTitle")}</p>
              </div>
              <ScrollArea className="max-h-64">
                {historyList.length === 0 ? (
                  <p className="text-xs text-muted-foreground p-4 text-center">{i18n.t("chat.noHistory")}</p>
                ) : (
                  <div className="py-1 px-1">
                    {historyList.map((conv) => (
                      <button
                        key={conv.id}
                        onClick={() => handleLoadConversation(conv)}
                        className={`w-full flex items-center justify-between gap-2 px-2.5 py-2 text-left rounded-md transition-colors ${conv.id === conversationId
                          ? "bg-primary/10 text-primary"
                          : "hover:bg-accent/50 text-foreground"
                          }`}
                      >
                        <div className="min-w-0 flex-1">
                          <p className={`truncate text-xs font-medium ${conv.id === conversationId ? "text-primary" : "text-foreground"}`}>{conv.title}</p>
                          <p className="text-[10px] text-muted-foreground mt-0.5">
                            {new Date(conv.updatedAt).toLocaleDateString(i18n.language === "pt-BR" ? "pt-BR" : "en-US", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" })}
                          </p>
                        </div>
                        <Button
                          size="icon"
                          variant="ghost"
                          className="h-6 w-6 shrink-0 rounded-md text-muted-foreground hover:text-destructive hover:bg-destructive/10"
                          onClick={(e) => handleDeleteConversation(conv.id, e)}
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      </button>
                    ))}
                  </div>
                )}
              </ScrollArea>
            </PopoverContent>
          </Popover>
        </div>
      </div>

      {/* Messages */}
      <div
        ref={scrollRef}
        className="custom-scrollbar min-h-0 flex-1 overflow-y-auto p-4 space-y-4"
        onScroll={() => {
          const el = scrollRef.current;
          if (!el) return;
          const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
          autoScrollRef.current = atBottom;
        }}
      >
        {messages.map((msg, i) => {
          if (msg.displayTitle) {
            return (
              <div key={i} className="flex justify-center my-3">
                <div className="border border-border/40 rounded-full px-5 py-2 text-sm font-medium text-muted-foreground">
                  {msg.displayTitle}
                </div>
              </div>
            );
          }

          if (msg.role === "system") return null;

          return (
            <div key={i} className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}>
              <div
                className={`text-sm ${msg.role === "user"
                  ? "max-w-[85%] rounded-xl px-4 py-3 bg-primary text-primary-foreground"
                  : "w-full text-foreground"
                  }`}
              >
                {msg.role === "assistant" && msg.scenarios && msg.scenarios.length > 0 ? (
                  <div className="space-y-3">
                    {msg.content && (
                      <div className="prose prose-sm dark:prose-invert max-w-none [&>p]:mb-2">
                        <ReactMarkdown>{msg.content}</ReactMarkdown>
                      </div>
                    )}
                    <ScenarioSelector
                      scenarios={msg.scenarios}
                      onGenerate={handleGenerateFromScenarios}
                      disabled={isStreaming}
                    />
                  </div>
                ) : msg.role === "assistant" ? (
                  <div className="prose prose-sm dark:prose-invert max-w-none [&>p]:mb-2 [&>ul]:mb-2 [&>ol]:mb-2">
                    <ReactMarkdown>{msg.content}</ReactMarkdown>
                  </div>
                ) : (
                  <div>
                    <p className="whitespace-pre-wrap">{msg.content}</p>
                    {msg.attachments && msg.attachments.length > 0 && (
                      <div className="flex flex-wrap gap-1.5 mt-2">
                        {msg.attachments.map((a, ai) => (
                          <span key={ai} className="inline-flex items-center gap-1 rounded-md bg-primary/10 px-2 py-0.5 text-[11px] font-medium text-primary">
                            <FileText className="h-3 w-3" />
                            {a.name}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>
          );
        })}
        {/* (SolveNowCard removed — auto-pilot now uses normal chat flow) */}
        {activeToolCalls.length > 0 && (
          <div className="flex justify-start">
            <div className="rounded-xl px-3 py-2.5 border border-border/30 space-y-1.5 min-w-[180px]">
              {activeToolCalls.map((tc, idx) => (
                <div key={idx} className="flex items-center gap-2 text-xs text-muted-foreground">
                  {tc.status === "running" ? (
                    <DotsLoader className="text-primary" />
                  ) : tc.status === "done" ? (
                    <Check className="h-3 w-3 text-primary shrink-0" />
                  ) : (
                    <AlertCircle className="h-3 w-3 text-destructive shrink-0" />
                  )}
                  <Wrench className="h-3 w-3 shrink-0" />
                  <span className="font-mono truncate">{tc.name}</span>
                </div>
              ))}
            </div>
          </div>
        )}
        {isStreaming && activeToolCalls.length === 0 && messages[messages.length - 1]?.role !== "assistant" && (
          <div className="flex justify-start">
            <div className="rounded-xl px-4 py-3 border border-border/30">
              <DotsLoader />
            </div>
          </div>
        )}
      </div>

      {/* Auto-pilot banner */}
      {solveNowMode && isStreaming && (
        <div className="px-3 pt-2">
          <div className="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
            <Zap className="h-3.5 w-3.5 text-warning animate-pulse shrink-0" />
            <span className="text-xs font-medium text-foreground">
              Auto-pilot iteration {solveIterationCount}/{SOLVE_MAX_ITERATIONS} — running...
            </span>
            <Button
              size="sm"
              variant="ghost"
              className="ml-auto h-6 px-2 text-[10px] text-muted-foreground hover:text-destructive"
              onClick={() => {
                setSolveNowMode(false);
                setSolveIterationCount(0);
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}
      {solveNowMode && !isStreaming && suggestions.length > 0 && (
        <div className="px-3 pt-2">
          <div className="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
            <Zap className="h-3.5 w-3.5 text-warning animate-pulse shrink-0" />
            <span className="text-xs font-medium text-foreground">
              Auto-pilot {solveIterationCount + 1}/{SOLVE_MAX_ITERATIONS} — picking next action...
            </span>
            <Button
              size="sm"
              variant="ghost"
              className="ml-auto h-6 px-2 text-[10px] text-muted-foreground hover:text-destructive"
              onClick={() => {
                setSolveNowMode(false);
                setSolveIterationCount(0);
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}

      {/* Suggestion chips (hidden when auto-pilot is active) */}
      {suggestions.length > 0 && !isStreaming && !solveNowMode && (
        <div className="px-3 pt-2">
          <div className="flex gap-2 overflow-x-auto pb-1 custom-scrollbar">
            {suggestions.map((s, i) => (
              <button
                key={i}
                onClick={() => {
                  setSuggestions([]);
                  sendMessage(s.message);
                }}
                className="shrink-0 rounded-lg hover:bg-muted border border-border/40 px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:border-primary/30"
                title={s.message}
              >
                {s.title}
              </button>
            ))}
          </div>
        </div>
      )}
      {loadingSuggestions && !isStreaming && suggestions.length === 0 && (
        <div className="px-3 pt-2 flex gap-2">
          {[1, 2, 3].map((i) => (
            <div key={i} className="h-7 w-24 rounded-lg animate-pulse shrink-0" />
          ))}
        </div>
      )}

      {/* Input – AI Prompt Box */}
      <div className="p-3 pt-2">
        <div className="relative rounded-xl border border-border/60 transition-all duration-200" style={{ backgroundColor: "hsl(var(--card) / 0.8)" }} onFocusCapture={e => { e.currentTarget.style.backgroundColor = "hsl(var(--card) / 1)"; }} onBlurCapture={e => { if (!e.currentTarget.contains(e.relatedTarget)) e.currentTarget.style.backgroundColor = "hsl(var(--card) / 0.8)"; }}>
          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5 px-3 pt-2.5">
              {attachments.map((a, i) => (
                <span
                  key={i}
                  className="inline-flex items-center gap-1 rounded-md bg-primary/10 pl-2 pr-1 py-0.5 text-[11px] font-medium text-primary"
                >
                  <FileText className="h-3 w-3" />
                  {a.name}
                  <button
                    type="button"
                    onClick={() => removeAttachment(i)}
                    className="ml-0.5 rounded hover:bg-primary/20 p-0.5 transition-colors"
                  >
                    <X className="h-2.5 w-2.5" />
                  </button>
                </span>
              ))}
            </div>
          )}
          <Textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={i18n.t("chat.placeholder")}
            className="min-h-[48px] max-h-[140px] resize-none border-0 rounded-xl bg-transparent px-4 pt-3 pb-10 text-sm shadow-none focus-visible:ring-0 placeholder:text-muted-foreground/50"
            disabled={isStreaming || !apiKey}
          />
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept=".json,.yaml,.yml,.txt,.md,.csv,.xml,.html,.js,.ts,.tsx,.py,.go,.rs,.env,.toml"
            onChange={handleFileSelect}
            className="hidden"
          />
          <div className="absolute bottom-2 left-3 right-3 flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Button
                size="icon"
                variant="ghost"
                className="h-7 w-7 rounded-lg text-muted-foreground/50 hover:text-foreground"
                onClick={() => fileInputRef.current?.click()}
                disabled={isStreaming || !apiKey}
                title={i18n.t("chat.attachFile")}
              >
                <Paperclip className="h-3.5 w-3.5" />
              </Button>
              <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground/50">
                <CornerDownLeft className="h-3 w-3" />
                <span>{i18n.t("chat.enterToSend")}</span>
              </div>
            </div>
            <div className="flex items-center gap-1.5">
              {isStreaming && (
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-7 w-7 rounded-lg text-muted-foreground hover:text-destructive"
                  onClick={() => abortRef.current?.abort()}
                  title={i18n.t("chat.stopGenerating")}
                >
                  <Square className="h-3 w-3 fill-current" />
                </Button>
              )}
              {!isStreaming && apiKey && (
                <Button
                  size="icon"
                  variant="ghost"
                  className={`h-7 w-7 rounded-lg transition-colors ${solveNowMode ? "text-warning bg-warning/15 ring-1 ring-warning/30" : "text-muted-foreground/50 hover:text-warning hover:bg-warning/10"}`}
                  onClick={() => setSolveNowMode((prev) => !prev)}
                  title={solveNowMode ? "Solve Mode ON — click to disable" : "Enable Solve This Now ⚡"}
                >
                  <Zap className="h-3.5 w-3.5" />
                </Button>
              )}
              <Button
                size="icon"
                className="h-7 w-7 rounded-lg"
                onClick={() => {
                  if (solveNowMode) setSolveIterationCount(0);
                  sendMessage(input);
                }}
                disabled={isStreaming || (!input.trim() && attachments.length === 0) || !apiKey}
              >
                {isStreaming ? <DotsLoader /> : solveNowMode ? <Zap className="h-3.5 w-3.5" /> : <Send className="h-3.5 w-3.5" />}
              </Button>
            </div>
          </div>
        </div>
        {!apiKey && (
          <p className="text-xs text-destructive mt-2 px-1">
            {i18n.t("chat.apiKeyRequired")}
          </p>
        )}
      </div>
    </div>
  );
});
