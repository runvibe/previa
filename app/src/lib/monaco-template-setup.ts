/**
 * Centralized Monaco template language registration and theme setup.
 * Shared between MonacoCodeEditor and MonacoInput.
 */

import { helperDocs } from "./template-helpers";
import type { TemplateValidationContext } from "./template-validator";

type MonacoInstance = any;

let registered = false;

/**
 * Enhanced Monarch tokenizer for template-input language.
 * Tokenizes {{...}} expressions with separate tokens for brackets, namespaces, dots, and paths.
 */
const templateTokenizer = {
  tokenizer: {
    root: [
      [/\{\{/, "template-bracket", "@templateExpr"],
      [/./, ""],
    ],
    templateExpr: [
      [/\}\}/, "template-bracket", "@pop"],
      [/specs/, "template-namespace"],
      [/steps|helpers/, "template-namespace"],
      [/\./, "template-dot"],
      [/[a-zA-Z_][a-zA-Z0-9_-]*/, "template-path"],
      [/\s+/, "template-space"],
      [/./, "template-path"],
    ],
  },
};

/**
 * Monarch rules injected into JSON/YAML tokenizers to highlight {{...}} inside strings.
 */
const templateStringRules = [
  [/\{\{/, { token: "template-bracket", next: "@templateExpr" }],
];

const templateExprState = [
  [/\}\}/, { token: "template-bracket", next: "@pop" }],
  [/specs/, "template-namespace"],
  [/steps|helpers/, "template-namespace"],
  [/\./, "template-dot"],
  [/[a-zA-Z_][a-zA-Z0-9_-]*/, "template-path"],
  [/\s+/, "template-space"],
  [/./, "template-path"],
];

const lightThemeRules = [
  { token: "template-bracket", foreground: "9333EA", fontStyle: "bold" },
  { token: "template-namespace", foreground: "2563EB", fontStyle: "bold" },
  { token: "template-dot", foreground: "64748B" },
  { token: "template-path", foreground: "059669" },
  { token: "template-space", foreground: "64748B" },
  // Keep old token for backwards compat
  { token: "template-variable", foreground: "2563EB", fontStyle: "bold" },
];

const darkThemeRules = [
  { token: "template-bracket", foreground: "C084FC", fontStyle: "bold" },
  { token: "template-namespace", foreground: "60A5FA", fontStyle: "bold" },
  { token: "template-dot", foreground: "94A3B8" },
  { token: "template-path", foreground: "34D399" },
  { token: "template-space", foreground: "94A3B8" },
  { token: "template-variable", foreground: "60A5FA", fontStyle: "bold" },
];

/**
 * Register template-input language and define transparent themes.
 * Safe to call multiple times — only registers once.
 */
export function setupMonacoTemplateLanguage(monaco: MonacoInstance, isDark: boolean): void {
  if (!registered) {
    // Register template-input language
    if (!monaco.languages.getLanguages().some((l: any) => l.id === "template-input")) {
      monaco.languages.register({ id: "template-input" });
      monaco.languages.setMonarchTokensProvider("template-input", templateTokenizer as any);
    }

    // Define themes
    try {
      monaco.editor.defineTheme("transparent-light", {
        base: "vs",
        inherit: true,
        rules: lightThemeRules,
        colors: {
          "editor.background": "#00000000",
          "editor.lineHighlightBackground": "#00000008",
          "editorGutter.background": "#00000000",
          "minimap.background": "#00000000",
          "editorOverviewRuler.border": "#00000000",
          "editorCursor.foreground": "#1e293b",
        },
      });
      monaco.editor.defineTheme("transparent-dark", {
        base: "vs-dark",
        inherit: true,
        rules: darkThemeRules,
        colors: {
          "editor.background": "#00000000",
          "editor.lineHighlightBackground": "#ffffff08",
          "editorGutter.background": "#00000000",
          "minimap.background": "#00000000",
          "editorOverviewRuler.border": "#00000000",
          "editorCursor.foreground": "#e2e8f0",
        },
      });
    } catch {
      // themes already defined
    }

    registered = true;
  }

  monaco.editor.setTheme(isDark ? "transparent-dark" : "transparent-light");
}

/**
 * Apply the correct theme based on dark mode state.
 */
export function applyMonacoTheme(monaco: MonacoInstance, isDark: boolean): void {
  monaco.editor.setTheme(isDark ? "transparent-dark" : "transparent-light");
}

/**
 * Register a CompletionItemProvider for template expressions.
 * Works for template-input, json, and yaml languages.
 * Returns an IDisposable to clean up on unmount.
 */
export function registerTemplateCompletions(
  monaco: MonacoInstance,
  getContext: () => TemplateValidationContext | undefined,
  languages: string[] = ["template-input"]
): { dispose: () => void } {
  const providerConfig = {
    triggerCharacters: [".", "{"],
    provideCompletionItems(model: any, position: any) {
      const lineContent = model.getLineContent(position.lineNumber);
      const textBeforeCursor = lineContent.substring(0, position.column - 1);

      // Find last unclosed {{
      const openIdx = textBeforeCursor.lastIndexOf("{{");
      if (openIdx === -1) return { suggestions: [] };
      const afterOpen = textBeforeCursor.substring(openIdx + 2);
      if (afterOpen.includes("}}")) return { suggestions: [] };

      const expr = afterOpen.trim();
      const segments = expr ? expr.split(".") : [];

      // Calculate replace range manually instead of relying on getWordUntilPosition
      const lastSegment = segments.length > 0 ? segments[segments.length - 1] : "";
      const replaceRange = {
        startLineNumber: position.lineNumber,
        startColumn: position.column - lastSegment.length,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      };

      const makeSuggestion = (label: string, kind: number, detail: string, extra?: Record<string, any>) => ({
        label,
        kind,
        detail,
        insertText: label,
        range: replaceRange,
        filterText: label,
        sortText: `0_${label}`,
        ...extra,
      });

      // No segments or typing first namespace
      if (segments.length <= 1) {
        return {
          suggestions: [
            makeSuggestion("envs", monaco.languages.CompletionItemKind.Module, "URLs por env group (envs.current.<entrada>)"),
            makeSuggestion("specs", monaco.languages.CompletionItemKind.Module, "URL do servidor (specs.<slug>.url.<env>)"),
            makeSuggestion("steps", monaco.languages.CompletionItemKind.Module, "Dados de steps anteriores"),
            makeSuggestion("helpers", monaco.languages.CompletionItemKind.Module, "Funções de dados fake"),
          ],
        };
      }

      const root = segments[0];

      // envs.<group>
      if (root === "envs" && segments.length === 2) {
        const ctx = getContext();
        const envGroups = ctx?.availableEnvGroups ?? [];
        const suggestions = [
          makeSuggestion("current", monaco.languages.CompletionItemKind.Variable, "Env group selecionado na execução"),
          ...envGroups.map((group) =>
            makeSuggestion(group.slug, monaco.languages.CompletionItemKind.Variable, `Env group: ${group.slug}`)
          ),
        ];
        return { suggestions };
      }

      // envs.<group>.<entry>
      if (root === "envs" && segments.length === 3) {
        const ctx = getContext();
        const requestedSlug = segments[1];
        const resolvedSlug = requestedSlug === "current" ? ctx?.selectedEnvGroupSlug : requestedSlug;
        const envGroup = ctx?.availableEnvGroups?.find((group) => group.slug === resolvedSlug);
        return {
          suggestions: (envGroup?.entries ?? []).map((entry) =>
            makeSuggestion(entry, monaco.languages.CompletionItemKind.Constant, `Entrada: ${entry}`)
          ),
        };
      }

      // specs.<slug>
      if (root === "specs" && segments.length === 2) {
        const ctx = getContext();
        const specs = ctx?.availableSpecs ?? [];
        return {
          suggestions: specs.map((s) =>
            makeSuggestion(s.slug, monaco.languages.CompletionItemKind.Variable, `Spec: ${s.slug}`)
          ),
        };
      }

      // specs.<slug>.url
      if (root === "specs" && segments.length === 3) {
        return {
          suggestions: [
            makeSuggestion("url", monaco.languages.CompletionItemKind.Property, "URLs do servidor"),
          ],
        };
      }

      // specs.<slug>.url.<env>
      if (root === "specs" && segments.length === 4 && segments[2] === "url") {
        const ctx = getContext();
        const slug = segments[1];
        const specInfo = ctx?.availableSpecs?.find((s) => s.slug === slug);
        const envs = specInfo?.envs ?? [];
        return {
          suggestions: envs.map((env) =>
            makeSuggestion(env, monaco.languages.CompletionItemKind.Constant, `Ambiente: ${env}`)
          ),
        };
      }

      // helpers.<name>
      if (root === "helpers" && segments.length === 2) {
        return {
          suggestions: helperDocs.map((h) => {
            const name = h.name.split(" ")[0];
            return makeSuggestion(name, monaco.languages.CompletionItemKind.Function, h.description, {
              documentation: `Ex: ${h.example}`,
            });
          }),
        };
      }

      // steps.<id>
      if (root === "steps" && segments.length === 2) {
        const ctx = getContext();
        const ids = ctx?.availableStepIds ?? [];
        return {
          suggestions: ids.map((id) =>
            makeSuggestion(id, monaco.languages.CompletionItemKind.Variable, `Step: ${id}`)
          ),
        };
      }

      // steps.<id>.<field>
      if (root === "steps" && segments.length === 3) {
        const fields = ["status", "body", "headers"];
        return {
          suggestions: fields.map((f) =>
            makeSuggestion(f, monaco.languages.CompletionItemKind.Property, `Campo de resposta: ${f}`)
          ),
        };
      }

      // steps.<id>.body.<field> — suggest response body fields from spec
      if (root === "steps" && segments.length === 4 && segments[2] === "body") {
        const ctx = getContext();
        const stepId = segments[1];
        const fields = ctx?.stepResponseFields?.[stepId] ?? [];
        if (fields.length > 0) {
          return {
            suggestions: fields.map((f) =>
              makeSuggestion(f.name, monaco.languages.CompletionItemKind.Field,
                f.type ? `${f.type}${f.description ? ' — ' + f.description : ''}` : f.description ?? ''
              )
            ),
          };
        }
      }

      return { suggestions: [] };
    },
  };

  const disposables = languages.map((lang) =>
    monaco.languages.registerCompletionItemProvider(lang, providerConfig)
  );

  return {
    dispose: () => disposables.forEach((d: { dispose: () => void }) => d.dispose()),
  };
}
