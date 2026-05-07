/**
 * IndexedDB persistence for Projects and Pipelines.
 * Two object stores mirroring the backend contract:
 *   - "projects": id, name, description, createdAt, updatedAt, specJson, executionBackendUrl
 *   - "pipelines": id, projectId, position, name, description, createdAt, updatedAt, pipelineJson
 */
import type { Project, ProjectEnvGroup, ProjectSpec } from "@/types/project";
import { migrateProjectSpecs } from "@/types/project";
import type { Pipeline, OpenAPISpec } from "@/types/pipeline";
import { generateUUID } from "@/lib/uuid";

const DB_NAME = "previa-db-v1";
const DB_VERSION = 3; // bump from v2 to v3 to persist project env groups

// ─── helpers ───────────────────────────────────────────────────────

interface ProjectRow {
  id: string;
  name: string;
  description: string | null;
  createdAt: string;
  updatedAt: string;
  createdAtMs: number;
  updatedAtMs: number;
  specJson: string | null;
  tagsJson?: string | null;
  envGroupsJson?: string | null;
}

interface PipelineRow {
  id: string;
  projectId: string;
  position: number;
  name: string;
  description: string | null;
  createdAt: string;
  updatedAt: string;
  pipelineJson: string; // full Pipeline object serialised
}

function toProjectRow(p: Project): ProjectRow {
  // Serialize specs array into specJson for storage
  const specsData = p.specs && p.specs.length > 0 ? p.specs : undefined;
  const legacySpec = p.spec;
  // Prefer specs array; fallback to legacy spec
  const specJson = specsData
    ? JSON.stringify({ __multi: true, specs: specsData })
    : legacySpec ? JSON.stringify(legacySpec) : null;

  return {
    id: p.id,
    name: p.name,
    description: p.description ?? null,
    createdAt: p.createdAt,
    updatedAt: p.updatedAt,
    createdAtMs: new Date(p.createdAt).getTime(),
    updatedAtMs: new Date(p.updatedAt).getTime(),
    specJson,
    tagsJson: JSON.stringify(p.tags ?? []),
    envGroupsJson: JSON.stringify(p.envGroups ?? []),
  };
}

function fromProjectRow(row: ProjectRow, pipelines: Pipeline[]): Project {
  let specs: ProjectSpec[] = [];
  let envGroups: ProjectEnvGroup[] = [];
  let tags: string[] = [];
  let legacySpec: OpenAPISpec | undefined;

  if (row.specJson) {
    try {
      const parsed = JSON.parse(row.specJson);
      if (parsed && parsed.__multi && Array.isArray(parsed.specs)) {
        specs = parsed.specs as ProjectSpec[];
      } else {
        // Legacy single spec
        legacySpec = parsed as OpenAPISpec;
      }
    } catch { /* ignore parse errors */ }
  }

  if (row.envGroupsJson) {
    try {
      const parsed = JSON.parse(row.envGroupsJson);
      envGroups = Array.isArray(parsed) ? parsed as ProjectEnvGroup[] : [];
    } catch { /* ignore parse errors */ }
  }

  if (row.tagsJson) {
    try {
      const parsed = JSON.parse(row.tagsJson);
      tags = Array.isArray(parsed)
        ? parsed.filter((tag): tag is string => typeof tag === "string")
        : [];
    } catch { /* ignore parse errors */ }
  }

  const project: Project = {
    id: row.id,
    name: row.name,
    description: row.description ?? undefined,
    tags,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
    spec: legacySpec,
    specs,
    pipelines,
    envGroups,
  };

  return migrateProjectSpecs(project);
}

function toPipelineRows(projectId: string, pipelines: Pipeline[]): PipelineRow[] {
  const now = new Date().toISOString();
  return pipelines.map((p, i) => ({
    id: p.id || generateUUID(),
    projectId,
    position: i,
    name: p.name,
    description: p.description || null,
    createdAt: now,
    updatedAt: p.updatedAt || now,
    pipelineJson: JSON.stringify(p),
  }));
}

function fromPipelineRow(row: PipelineRow): Pipeline {
  const pipeline = JSON.parse(row.pipelineJson) as Pipeline;
  pipeline.updatedAt = row.updatedAt;
  return pipeline;
}

// ─── DB open ───────────────────────────────────────────────────────

function openDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);

    req.onupgradeneeded = (event) => {
      const db = req.result;
      const oldVersion = event.oldVersion;

      // v1 stores (executions) already exist if oldVersion >= 1
      if (oldVersion < 1) {
        // Create execution store (from execution-store.ts original)
        const runsStore = db.createObjectStore("runs", { keyPath: "id", autoIncrement: true });
        runsStore.createIndex("project_pipeline", ["projectId", "pipelineIndex"], { unique: false });
        runsStore.createIndex("projectId", "projectId", { unique: false });
      }

      if (oldVersion < 2) {
        // Projects store
        const projStore = db.createObjectStore("projects", { keyPath: "id" });
        projStore.createIndex("updatedAtMs", "updatedAtMs", { unique: false });

        // Pipelines store
        const pipStore = db.createObjectStore("pipelines", { keyPath: "id" });
        pipStore.createIndex("projectId", "projectId", { unique: false });
        pipStore.createIndex("project_position", ["projectId", "position"], { unique: false });
      }
    };

    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

// ─── CRUD ──────────────────────────────────────────────────────────

export async function getProjects(): Promise<Project[]> {
  const db = await openDB();
  const tx = db.transaction(["projects", "pipelines"], "readonly");

  const projectRows: ProjectRow[] = await new Promise((res, rej) => {
    const r = tx.objectStore("projects").index("updatedAtMs").getAll();
    r.onsuccess = () => res(r.result as ProjectRow[]);
    r.onerror = () => rej(r.error);
  });

  const allPipelineRows: PipelineRow[] = await new Promise((res, rej) => {
    const r = tx.objectStore("pipelines").getAll();
    r.onsuccess = () => res(r.result as PipelineRow[]);
    r.onerror = () => rej(r.error);
  });

  // Group pipelines by projectId
  const pipelineMap = new Map<string, PipelineRow[]>();
  for (const row of allPipelineRows) {
    const arr = pipelineMap.get(row.projectId) || [];
    arr.push(row);
    pipelineMap.set(row.projectId, arr);
  }

  return projectRows
    .sort((a, b) => b.updatedAtMs - a.updatedAtMs)
    .map((row) => {
      const pRows = (pipelineMap.get(row.id) || []).sort((a, b) => a.position - b.position);
      return fromProjectRow(row, pRows.map(fromPipelineRow));
    });
}

export async function getProject(id: string): Promise<Project | null> {
  const db = await openDB();
  const tx = db.transaction(["projects", "pipelines"], "readonly");

  const row: ProjectRow | undefined = await new Promise((res, rej) => {
    const r = tx.objectStore("projects").get(id);
    r.onsuccess = () => res(r.result as ProjectRow | undefined);
    r.onerror = () => rej(r.error);
  });

  if (!row) return null;

  const pipelineRows: PipelineRow[] = await new Promise((res, rej) => {
    const r = tx.objectStore("pipelines").index("projectId").getAll(id);
    r.onsuccess = () => res(r.result as PipelineRow[]);
    r.onerror = () => rej(r.error);
  });

  pipelineRows.sort((a, b) => a.position - b.position);
  return fromProjectRow(row, pipelineRows.map(fromPipelineRow));
}

export async function saveProject(project: Project): Promise<void> {
  const db = await openDB();
  const tx = db.transaction(["projects", "pipelines"], "readwrite");
  const projectStore = tx.objectStore("projects");
  const pipelineStore = tx.objectStore("pipelines");

  // Upsert project row
  projectStore.put(toProjectRow(project));

  // Delete existing pipelines for this project
  const existingPipelines: PipelineRow[] = await new Promise((res, rej) => {
    const r = pipelineStore.index("projectId").getAll(project.id);
    r.onsuccess = () => res(r.result as PipelineRow[]);
    r.onerror = () => rej(r.error);
  });
  for (const ep of existingPipelines) {
    pipelineStore.delete(ep.id);
  }

  // Insert new pipelines
  const rows = toPipelineRows(project.id, project.pipelines);
  for (const row of rows) {
    pipelineStore.put(row);
  }

  return new Promise((res, rej) => {
    tx.oncomplete = () => res();
    tx.onerror = () => rej(tx.error);
  });
}

export async function deleteProject(id: string): Promise<void> {
  const db = await openDB();
  const tx = db.transaction(["projects", "pipelines"], "readwrite");
  const projectStore = tx.objectStore("projects");
  const pipelineStore = tx.objectStore("pipelines");

  projectStore.delete(id);

  // Delete associated pipelines
  const pipelines: PipelineRow[] = await new Promise((res, rej) => {
    const r = pipelineStore.index("projectId").getAll(id);
    r.onsuccess = () => res(r.result as PipelineRow[]);
    r.onerror = () => rej(r.error);
  });
  for (const p of pipelines) {
    pipelineStore.delete(p.id);
  }

  return new Promise((res, rej) => {
    tx.oncomplete = () => res();
    tx.onerror = () => rej(tx.error);
  });
}

export async function updateProject(id: string, updates: Partial<Omit<Project, "id">>): Promise<Project | null> {
  const existing = await getProject(id);
  if (!existing) return null;

  const updated: Project = {
    ...existing,
    ...updates,
    updatedAt: new Date().toISOString(),
  };

  await saveProject(updated);
  return updated;
}

export async function duplicateProject(id: string): Promise<Project | null> {
  const project = await getProject(id);
  if (!project) return null;

  const now = new Date().toISOString();
  const newProject: Project = {
    ...project,
    id: generateUUID(),
    name: `${project.name} (cópia)`,
    tags: [...(project.tags ?? [])],
    createdAt: now,
    updatedAt: now,
  };
  newProject.envGroups = (project.envGroups ?? []).map((group) => ({
    ...group,
    id: generateUUID(),
    projectId: newProject.id,
    createdAt: now,
    updatedAt: now,
  }));

  await saveProject(newProject);
  return newProject;
}

// ─── Migration from localStorage ──────────────────────────────────

const LS_KEY = "api-pipeline-studio:projects";
const LS_MIGRATED_KEY = "api-pipeline-studio:idb-migrated";

export async function migrateFromLocalStorage(): Promise<void> {
  // Only migrate once
  if (localStorage.getItem(LS_MIGRATED_KEY)) return;

  const raw = localStorage.getItem(LS_KEY);
  if (!raw) {
    localStorage.setItem(LS_MIGRATED_KEY, "true");
    return;
  }

  try {
    const projects: Project[] = JSON.parse(raw);
    for (const project of projects) {
      await saveProject(project);
    }
    localStorage.setItem(LS_MIGRATED_KEY, "true");
    console.log(`[project-db] Migrated ${projects.length} projects from localStorage to IndexedDB`);
  } catch (err) {
    console.error("[project-db] Migration from localStorage failed:", err);
  }
}
