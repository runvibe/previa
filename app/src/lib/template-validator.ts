import { templateHelpers } from "./template-helpers";

export interface ResponseFieldInfo {
  name: string;
  type?: string;
  description?: string;
}

export interface TemplateValidationContext {
  /** IDs dos steps disponíveis (anteriores ao step atual) */
  availableStepIds?: string[];
  /** ID do step atual (para não referenciar a si mesmo) */
  currentStepId?: string;
  /** Campos de response body por step ID (extraídos do OpenAPI spec) */
  stepResponseFields?: Record<string, ResponseFieldInfo[]>;
  /** Slugs de specs disponíveis com suas envs */
  availableSpecs?: Array<{ slug: string; envs: string[] }>;
  /** Env groups disponíveis com seus nomes de entradas */
  availableEnvGroups?: Array<{ slug: string; entries: string[] }>;
  /** Env group selecionado para resolver envs.current.* */
  selectedEnvGroupSlug?: string | null;
}

export interface TemplateDiagnostic {
  message: string;
  offset: number;
  length: number;
  severity: "error" | "warning";
}

const VALID_NAMESPACES = ["steps", "helpers", "specs", "envs", "url"];
const helperNames = new Set(Object.keys(templateHelpers));

/**
 * Validates all `{{...}}` interpolation expressions in a text string.
 */
export function validateInterpolations(
  text: string,
  context?: TemplateValidationContext
): TemplateDiagnostic[] {
  const diagnostics: TemplateDiagnostic[] = [];
  const regex = /\{\{([^}]*)\}\}/g;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(text)) !== null) {
    const fullMatch = match[0];
    const inner = match[1].trim();
    const offset = match.index;
    const length = fullMatch.length;

    if (!inner) {
      diagnostics.push({
        message: "Expressão vazia. Use: specs.<spec>.url.<env>, helpers.<nome> ou steps.<id>.<campo>",
        offset,
        length,
        severity: "error",
      });
      continue;
    }

    const segments = inner.split(".");
    const root = segments[0];


    if (!VALID_NAMESPACES.includes(root)) {
      diagnostics.push({
        message: `Namespace desconhecido: '${root}'. Válidos: ${VALID_NAMESPACES.filter(n => n !== "url").join(", ")}`,
        offset,
        length,
        severity: "error",
      });
      continue;
    }

    // Legacy url.<slug>.<env> — warn to migrate
    if (root === "url") {
      diagnostics.push({
        message: "Formato antigo. Use {{specs.<slug>.url.<env>}} em vez de {{url.<slug>.<env>}}.",
        offset,
        length,
        severity: "warning",
      });
      continue;
    }

    if (root === "specs") {
      if (segments.length < 4 || segments[2] !== "url") {
        diagnostics.push({
          message: "specs requer formato specs.<slug>.url.<env>. Ex: {{specs.auth-api.url.hml}}",
          offset,
          length,
          severity: "error",
        });
        continue;
      }
      const slug = segments[1];
      const env = segments[3];
      if (!slug || !env) {
        diagnostics.push({
          message: "specs requer formato specs.<slug>.url.<env>. Ex: {{specs.auth-api.url.hml}}",
          offset,
          length,
          severity: "error",
        });
        continue;
      }
      if (context?.availableSpecs) {
        const specInfo = context.availableSpecs.find((s) => s.slug === slug);
        if (!specInfo) {
          diagnostics.push({
            message: `Spec '${slug}' não encontrada. Disponíveis: ${context.availableSpecs.map((s) => s.slug).join(", ") || "(nenhuma)"}`,
            offset,
            length,
            severity: "warning",
          });
        } else if (!specInfo.envs.includes(env)) {
          diagnostics.push({
            message: `Ambiente '${env}' não encontrado na spec '${slug}'. Disponíveis: ${specInfo.envs.join(", ") || "(nenhum)"}`,
            offset,
            length,
            severity: "warning",
          });
        }
      }
      continue;
    }

    if (root === "envs") {
      if (segments.length < 3) {
        diagnostics.push({
          message: "envs requer formato envs.<group>.<entrada> ou envs.current.<entrada>. Ex: {{envs.current.api}}",
          offset,
          length,
          severity: "error",
        });
        continue;
      }

      const groupSlug = segments[1];
      const entryName = segments[2];
      if (!groupSlug || !entryName) {
        diagnostics.push({
          message: "envs requer formato envs.<group>.<entrada> ou envs.current.<entrada>. Ex: {{envs.hml.api}}",
          offset,
          length,
          severity: "error",
        });
        continue;
      }

      if (groupSlug === "current" && !context?.selectedEnvGroupSlug) {
        diagnostics.push({
          message: "Selecione um env group para validar {{envs.current.*}}.",
          offset,
          length,
          severity: "warning",
        });
        continue;
      }

      if (context?.availableEnvGroups) {
        const resolvedSlug = groupSlug === "current" ? context.selectedEnvGroupSlug : groupSlug;
        const groupInfo = context.availableEnvGroups.find((group) => group.slug === resolvedSlug);
        if (!groupInfo) {
          diagnostics.push({
            message: `Env group '${resolvedSlug || groupSlug}' não encontrado. Disponíveis: ${context.availableEnvGroups.map((group) => group.slug).join(", ") || "(nenhum)"}`,
            offset,
            length,
            severity: "warning",
          });
        } else if (!groupInfo.entries.includes(entryName)) {
          diagnostics.push({
            message: `Entrada '${entryName}' não encontrada no env group '${groupInfo.slug}'. Disponíveis: ${groupInfo.entries.join(", ") || "(nenhuma)"}`,
            offset,
            length,
            severity: "warning",
          });
        }
      }
      continue;
    }

    if (root === "helpers") {
      if (segments.length < 2 || !segments[1]) {
        diagnostics.push({
          message: "helpers requer um nome. Ex: {{helpers.uuid}}, {{helpers.email}}",
          offset,
          length,
          severity: "error",
        });
        continue;
      }
      const helperPart = segments[1];
      const helperName = helperPart.split(/\s+/)[0];
      if (!helperNames.has(helperName)) {
        diagnostics.push({
          message: `Helper desconhecido: '${helperName}'. Disponíveis: ${Array.from(helperNames).slice(0, 10).join(", ")}...`,
          offset,
          length,
          severity: "error",
        });
      }
      continue;
    }

    if (root === "steps") {
      if (segments.length < 3) {
        diagnostics.push({
          message: "steps requer step_id e campo. Ex: {{steps.login.status}}",
          offset,
          length,
          severity: "error",
        });
        continue;
      }
      const stepId = segments[1];
      if (!stepId) {
        diagnostics.push({
          message: "ID do step está vazio em {{steps.<id>.<campo>}}",
          offset,
          length,
          severity: "error",
        });
        continue;
      }

      if (context?.currentStepId && stepId === context.currentStepId) {
        diagnostics.push({
          message: `Step '${stepId}' referencia a si mesmo`,
          offset,
          length,
          severity: "warning",
        });
      } else if (context?.availableStepIds && !context.availableStepIds.includes(stepId)) {
        diagnostics.push({
          message: `Step '${stepId}' não encontrado ou não precede o step atual`,
          offset,
          length,
          severity: "warning",
        });
      }
      continue;
    }
  }

  return diagnostics;
}
