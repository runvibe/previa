import type { Project } from "@/types/project";
import { getApiUrl } from "@/stores/useOrchestratorStore";
import {
  exportProjectRemote,
  exportProjectsSqliteRemote,
  importProjectsSqliteRemote,
  importProjectRemote,
  type ProjectExportEnvelope,
} from "@/lib/api-client";

function downloadJson(payload: unknown, fileName: string) {
  const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
  downloadBlob(blob, fileName);
}

function downloadBlob(blob: Blob, fileName: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  a.click();
  URL.revokeObjectURL(url);
}

function requireApi(): string {
  const apiUrl = getApiUrl();
  if (!apiUrl) throw new Error("Backend não conectado. Configure a URL do orquestrador para exportar/importar projetos.");
  return apiUrl;
}

export async function exportProject(project: Project, includeHistory: boolean): Promise<void> {
  const apiUrl = requireApi();
  const fileName = `${project.name.toLowerCase().replace(/\s+/g, "-")}.previa.json`;
  const envelope = await exportProjectRemote(apiUrl, project.id, includeHistory);
  downloadJson(envelope, fileName);
}

export async function exportProjectsSqlite(
  projectIds: string[],
  all: boolean,
  includeHistory: boolean,
): Promise<void> {
  if (!all && projectIds.length === 0) {
    throw new Error("Selecione ao menos um projeto para exportar.");
  }

  const apiUrl = requireApi();
  const blob = await exportProjectsSqliteRemote(apiUrl, {
    all,
    projectIds: all ? [] : projectIds,
    includeHistory,
  });
  downloadBlob(
    new Blob([blob], { type: "application/vnd.sqlite3" }),
    "previa-projects.sqlite3",
  );
}

export async function importProject(fileContent: string): Promise<Project> {
  const apiUrl = requireApi();

  let parsed: ProjectExportEnvelope & {
    history?: unknown[];
    loadTestHistory?: unknown[];
  };
  try {
    parsed = JSON.parse(fileContent) as ProjectExportEnvelope & {
      history?: unknown[];
      loadTestHistory?: unknown[];
    };
  } catch {
    throw new Error("Arquivo JSON inválido.");
  }

  const hasHistory = !!(parsed.history && parsed.history.length > 0)
    || !!(parsed.loadTestHistory && parsed.loadTestHistory.length > 0);

  const result = await importProjectRemote(
    apiUrl,
    parsed as ProjectExportEnvelope,
    hasHistory,
  );

  return {
    id: result.id,
    name: result.name,
    description: parsed.project?.description,
    tags: Array.isArray(parsed.project?.tags)
      ? parsed.project.tags.filter((tag): tag is string => typeof tag === "string")
      : [],
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    specs: parsed.project?.specs || [],
    envGroups: parsed.project?.envGroups || [],
    pipelines: parsed.project?.pipelines || [],
  };
}

export interface ProjectImportFileResult {
  projectsImported: number;
}

export function isSqliteProjectImportFile(file: File): boolean {
  const name = file.name.toLowerCase();
  return (
    name.endsWith(".sqlite")
    || name.endsWith(".sqlite3")
    || name.endsWith(".db")
    || file.type === "application/vnd.sqlite3"
    || file.type === "application/x-sqlite3"
  );
}

function readFileText(file: File): Promise<string> {
  if (typeof file.text === "function") {
    return file.text();
  }

  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error ?? new Error("Falha ao ler o arquivo."));
    reader.readAsText(file);
  });
}

function readFileArrayBuffer(file: File): Promise<ArrayBuffer> {
  if (typeof file.arrayBuffer === "function") {
    return file.arrayBuffer();
  }

  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (reader.result instanceof ArrayBuffer) {
        resolve(reader.result);
        return;
      }
      reject(new Error("Falha ao ler o arquivo SQLite."));
    };
    reader.onerror = () => reject(reader.error ?? new Error("Falha ao ler o arquivo SQLite."));
    reader.readAsArrayBuffer(file);
  });
}

export async function importProjectFile(file: File): Promise<ProjectImportFileResult> {
  if (!isSqliteProjectImportFile(file)) {
    const text = await readFileText(file);
    await importProject(text);
    return { projectsImported: 1 };
  }

  const apiUrl = requireApi();
  const bytes = await readFileArrayBuffer(file);
  const response = await importProjectsSqliteRemote(apiUrl, bytes, true);
  return { projectsImported: response.projectsImported };
}
