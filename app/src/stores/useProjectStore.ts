import { create } from "zustand";
import type { Project, ProjectEnvGroup, ProjectSpec } from "@/types/project";
import { getMergedSpec, migrateProjectSpecs } from "@/types/project";
import { generateUUID } from "@/lib/uuid";
import type { OpenAPISpec, Pipeline } from "@/types/pipeline";
import {
  getProjects,
  getProject,
  saveProject as persistProject,
  deleteProject as removeProject,
  duplicateProject as dupProject,
  updateProject as patchProject,
  migrateFromLocalStorage,
} from "@/lib/project-db";
import { getApiUrl } from "@/stores/useOrchestratorStore";
import * as api from "@/lib/api-client";
import { toast } from "sonner";
import i18n from "@/i18n";

interface ProjectState {
  projects: Project[];
  currentProject: Project | null;
  loading: boolean;
  isRemote: boolean;

  loadProjects: () => Promise<void>;
  loadProject: (id: string) => Promise<Project | null>;
  createProject: (data: Partial<Project>) => Promise<Project>;
  updateProject: (id: string, updates: Partial<Omit<Project, "id">>) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  duplicateProject: (id: string) => Promise<Project | null>;
  setCurrentProject: (project: Project | null) => void;
  /** @deprecated Use addSpec/updateSpec/removeSpec instead */
  saveProjectSpec: (projectId: string, spec: OpenAPISpec) => Promise<void>;
  saveProjectPipelines: (projectId: string, pipelines: Pipeline[]) => Promise<void>;
  migrateIfNeeded: () => Promise<void>;
  addSpec: (projectId: string, spec: OpenAPISpec, url?: string, sync?: boolean, slug?: string, servers?: Record<string, string>) => Promise<ProjectSpec | null>;
  updateSpec: (projectId: string, specId: string, spec: OpenAPISpec, url?: string, sync?: boolean, slug?: string, servers?: Record<string, string>) => Promise<void>;
  removeSpec: (projectId: string, specId: string) => Promise<void>;
  createEnvGroup: (projectId: string, data: api.ProjectEnvGroupUpsertRequest) => Promise<ProjectEnvGroup | null>;
  updateEnvGroup: (projectId: string, envGroupId: string, data: api.ProjectEnvGroupUpsertRequest) => Promise<void>;
  deleteEnvGroup: (projectId: string, envGroupId: string) => Promise<void>;
  createPipeline: (projectId: string, pipeline: Pipeline) => Promise<Pipeline>;
}

function updateEnvGroupsInProject(project: Project, envGroups: ProjectEnvGroup[]): Project {
  return {
    ...project,
    envGroups,
    updatedAt: new Date().toISOString(),
  };
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  projects: [],
  currentProject: null,
  loading: false,
  isRemote: !!getApiUrl(),

  migrateIfNeeded: async () => {
    await migrateFromLocalStorage();
  },

  loadProjects: async () => {
    const apiUrl = getApiUrl();
    const hasCache = get().projects.length > 0;
    set({ loading: !hasCache, isRemote: !!apiUrl });

    if (apiUrl) {
      try {
        const remote = await api.listProjects(apiUrl);
        set({ projects: remote, loading: false });
        return;
      } catch (err) {
        console.error("Failed to load projects from backend:", err);
        toast.error(i18n.t("store.loadProjectsError"));
        set({ projects: [], loading: false });
        return;
      }
    }

    // Local fallback
    await migrateFromLocalStorage();
    const projects = await getProjects();
    set({ projects, loading: false });
  },

  loadProject: async (id: string) => {
    const apiUrl = getApiUrl();
    set({ isRemote: !!apiUrl });

    if (apiUrl) {
      try {
        console.log("[DEBUG][store] loadProject START", { id, timestamp: Date.now() });
        const remote = await api.getProject(apiUrl, id);
        console.log("[DEBUG][store] loadProject END", { id, timestamp: Date.now() });

        // Parse specs if they are raw objects (best effort)
        try {
          const { parseOpenAPISpec } = await import("@/lib/openapi-parser");
          for (const ps of remote.specs) {
            if (ps.spec && !(ps.spec as any).routes) {
              try {
                ps.spec = parseOpenAPISpec(JSON.stringify(ps.spec));
              } catch {
                // Keep raw spec if parser fails.
              }
            }
          }
        } catch (err) {
          console.warn("Failed to import OpenAPI parser, keeping raw specs:", err);
        }

        // Update legacy merged spec (best effort)
        try {
          remote.spec = getMergedSpec(remote.specs);
        } catch (err) {
          console.warn("Failed to merge specs for project:", err);
        }

        console.log("[DEBUG][loadProject] remote loaded", {
          firstPipelineAsserts: remote.pipelines[0]?.steps[0]?.asserts,
          pipelineCount: remote.pipelines.length,
          timestamp: Date.now(),
        });
        set({ currentProject: remote });
        return remote;
      } catch (err) {
        console.warn("Failed to load project from backend:", err);
        toast.error(i18n.t("store.loadProjectError"));
        set({ currentProject: null });
        return null;
      }
    }

    const project = await getProject(id);
    console.log("[DEBUG][loadProject] local loaded", {
      firstPipelineAsserts: project?.pipelines[0]?.steps[0]?.asserts,
      pipelineCount: project?.pipelines.length,
      timestamp: Date.now(),
    });
    if (project) {
      set({ currentProject: project });
    }
    return project;
  },

  createProject: async (data: Partial<Project>) => {
    const apiUrl = getApiUrl();
    set({ isRemote: !!apiUrl });

    if (apiUrl) {
      try {
        // 1. Create project (metadata only)
        const record = await api.createProject(apiUrl, {
          name: data.name || "Novo Projeto",
          description: data.description ?? null,
          tags: data.tags ?? [],
          spec: data.spec?.raw ? (data.spec as unknown as Record<string, unknown>) : null,
          
        });

        const projectId = record.id;

        // 2. Create pipelines individually
        const createdPipelines: Pipeline[] = [];
        if (data.pipelines?.length) {
          for (const p of data.pipelines) {
            try {
              const created = await api.createPipeline(apiUrl, projectId, p);
              createdPipelines.push(created);
            } catch (err) {
              console.warn("Failed to create pipeline:", err);
            }
          }
        }

        // 3. Create spec if provided
        let spec: OpenAPISpec | undefined = data.spec;
        if (data.spec?.raw) {
          try {
            await api.createSpec(apiUrl, projectId, {
              spec: data.spec.raw as unknown as Record<string, unknown>,
            });
          } catch (err) {
            console.warn("Failed to create spec:", err);
          }
        }

        const createdEnvGroups: ProjectEnvGroup[] = [];
        if (data.envGroups?.length) {
          for (const group of data.envGroups) {
            try {
              const created = await api.createProjectEnvGroup(apiUrl, projectId, {
                slug: group.slug,
                name: group.name,
                entries: group.entries,
              });
              createdEnvGroups.push(created);
            } catch (err) {
              console.warn("Failed to create env group:", err);
            }
          }
        }

        const project: Project = {
          id: projectId,
          name: record.name,
          description: record.description ?? undefined,
          tags: record.tags ?? data.tags ?? [],
          createdAt: record.createdAt,
          updatedAt: record.updatedAt,
          pipelines: createdPipelines,
          spec,
          specs: data.specs || [],
          envGroups: createdEnvGroups,
        };

        set((state) => ({ projects: [...state.projects, project] }));
        return project;
      } catch (err) {
        console.error("Failed to create project on backend:", err);
        toast.error(i18n.t("store.createProjectError"));
        throw err;
      }
    }

    // Local
    const now = new Date().toISOString();
    const newProject: Project = {
      id: generateUUID(),
      name: data.name || "Novo Projeto",
      description: data.description,
      tags: data.tags ?? [],
      createdAt: now,
      updatedAt: now,
      pipelines: data.pipelines || [],
      spec: data.spec,
      specs: data.specs || [],
      envGroups: data.envGroups || [],
    };
    await persistProject(newProject);
    set((state) => ({ projects: [...state.projects, newProject] }));
    return newProject;
  },

  updateProject: async (id, updates) => {
    const apiUrl = getApiUrl();

    if (apiUrl) {
      const current = get().projects.find((p) => p.id === id) ?? (get().currentProject?.id === id ? get().currentProject : null);
      const updated = current ? { ...current, ...updates, updatedAt: new Date().toISOString() } : null;

      if (updated) {
        set((state) => ({
          projects: state.projects.map((p) => (p.id === id ? updated : p)),
          currentProject: state.currentProject?.id === id ? updated : state.currentProject,
        }));
      }

      if (updated) {
        api.upsertProject(apiUrl, id, {
          name: updated.name,
          description: updated.description ?? null,
          tags: updated.tags ?? [],
          
        }).catch((err) => {
          console.warn("Failed to sync project to backend:", err);
          toast.error(i18n.t("store.syncProjectError"));
        });
      }
      return;
    }

    const updated = await patchProject(id, updates);
    if (updated) {
      set((state) => ({
        projects: state.projects.map((p) => (p.id === id ? updated : p)),
        currentProject: state.currentProject?.id === id ? updated : state.currentProject,
      }));
    }
  },

  deleteProject: async (id) => {
    const apiUrl = getApiUrl();

    if (apiUrl) {
      try {
        await api.deleteProject(apiUrl, id);
      } catch (err) {
        console.warn("Failed to delete from backend:", err);
        toast.error(i18n.t("store.deleteProjectError"));
      }
      set((state) => ({
        projects: state.projects.filter((p) => p.id !== id),
        currentProject: state.currentProject?.id === id ? null : state.currentProject,
      }));
      return;
    }

    await removeProject(id);
    set((state) => ({
      projects: state.projects.filter((p) => p.id !== id),
      currentProject: state.currentProject?.id === id ? null : state.currentProject,
    }));
  },

  duplicateProject: async (id) => {
    const apiUrl = getApiUrl();

    if (apiUrl) {
      try {
        const source = await api.getProject(apiUrl, id);

        // 1. Create project metadata
        const record = await api.createProject(apiUrl, {
          name: `${source.name} (cópia)`,
          description: source.description ?? null,
          tags: source.tags ?? [],
          spec: source.spec?.raw ? (source.spec as unknown as Record<string, unknown>) : null,
          
        });

        const projectId = record.id;

        // 2. Create pipelines individually
        const createdPipelines: Pipeline[] = [];
        for (const p of source.pipelines) {
          try {
            const created = await api.createPipeline(apiUrl, projectId, p);
            createdPipelines.push(created);
          } catch (err) {
            console.warn("Failed to duplicate pipeline:", err);
          }
        }

        // 3. Create specs
        const createdSpecs: ProjectSpec[] = [];
        for (const spec of source.specs ?? []) {
          try {
            const created = await api.createSpec(apiUrl, projectId, {
              spec: (spec.spec.raw ?? spec.spec) as unknown as Record<string, unknown>,
              sync: spec.sync,
              url: spec.url ?? null,
              slug: spec.slug,
              servers: spec.servers,
            });
            createdSpecs.push({
              id: created.id,
              slug: created.slug ?? spec.slug,
              name: spec.name,
              spec: created.spec as unknown as OpenAPISpec,
              url: created.url ?? undefined,
              sync: created.sync,
              servers: created.servers ?? {},
              specMd5: (created as { specMd5?: string }).specMd5,
            });
          } catch (err) {
            console.warn("Failed to duplicate spec:", err);
          }
        }

        if (createdSpecs.length === 0 && source.spec?.raw) {
          try {
            const created = await api.createSpec(apiUrl, projectId, {
              spec: source.spec.raw as unknown as Record<string, unknown>,
            });
            createdSpecs.push({
              id: created.id,
              slug: created.slug ?? "default",
              name: source.spec.title || "Default Spec",
              spec: created.spec as unknown as OpenAPISpec,
              url: created.url ?? undefined,
              sync: created.sync,
              servers: created.servers ?? {},
              specMd5: (created as { specMd5?: string }).specMd5,
            });
          } catch (err) {
            console.warn("Failed to duplicate legacy spec:", err);
          }
        }

        const createdEnvGroups: ProjectEnvGroup[] = [];
        for (const group of source.envGroups ?? []) {
          try {
            const created = await api.createProjectEnvGroup(apiUrl, projectId, {
              slug: group.slug,
              name: group.name,
              entries: group.entries,
            });
            createdEnvGroups.push(created);
          } catch (err) {
            console.warn("Failed to duplicate env group:", err);
          }
        }

        const project: Project = {
          id: projectId,
          name: record.name,
          description: record.description ?? undefined,
          tags: record.tags ?? source.tags ?? [],
          createdAt: record.createdAt,
          updatedAt: record.updatedAt,
          pipelines: createdPipelines,
          spec: getMergedSpec(createdSpecs),
          specs: createdSpecs,
          envGroups: createdEnvGroups,
        };

        set((state) => ({ projects: [...state.projects, project] }));
        return project;
      } catch (err) {
        console.error("Failed to duplicate project on backend:", err);
        toast.error(i18n.t("store.duplicateProjectError"));
        return null;
      }
    }

    const newProject = await dupProject(id);
    if (newProject) {
      set((state) => ({ projects: [...state.projects, newProject] }));
    }
    return newProject ?? null;
  },

  setCurrentProject: (project) => {
    set({ currentProject: project });
  },

  saveProjectSpec: async (projectId, spec) => {
    const apiUrl = getApiUrl();

    if (!apiUrl) {
      await patchProject(projectId, { spec });
    }
    set((state) => ({
      projects: state.projects.map((p) => (p.id === projectId ? { ...p, spec, updatedAt: new Date().toISOString() } : p)),
      currentProject: state.currentProject?.id === projectId ? { ...state.currentProject, spec, updatedAt: new Date().toISOString() } : state.currentProject,
    }));

    // Sync to dedicated spec endpoint
    if (apiUrl && spec?.raw) {
      try {
        console.log("[DEBUG][store] saveProjectSpec POST START", { projectId, timestamp: Date.now() });
        const rawSpec = spec.raw as unknown as Record<string, unknown>;
        await api.createSpec(apiUrl, projectId, { spec: rawSpec });
        console.log("[DEBUG][store] saveProjectSpec POST END", { projectId, timestamp: Date.now() });
      } catch (err) {
        console.warn("Failed to sync spec to backend:", err);
        toast.error(i18n.t("store.saveSpecError"));
      }
    }
  },

  saveProjectPipelines: async (projectId, pipelines) => {
    const apiUrl = getApiUrl();

    if (!apiUrl) {
      await patchProject(projectId, { pipelines });
    }
    set((state) => ({
      projects: state.projects.map((p) => (p.id === projectId ? { ...p, pipelines, updatedAt: new Date().toISOString() } : p)),
      currentProject: state.currentProject?.id === projectId ? { ...state.currentProject, pipelines, updatedAt: new Date().toISOString() } : state.currentProject,
    }));

    // Sync pipelines individually to backend
    if (apiUrl) {
      try {
        const localIds = new Set(pipelines.map((p) => p.id).filter(Boolean));

        // 1. POST all pipelines FIRST
        console.log("[DEBUG][store] saveProjectPipelines UPSERT all START", { count: pipelines.length, timestamp: Date.now() });
        for (const p of pipelines) {
          try {
            if (p.id) {
              console.log("[DEBUG][store] PUT pipeline", { id: p.id, name: p.name, timestamp: Date.now() });
              await api.upsertPipeline(apiUrl, projectId, p.id, p);
            } else {
              console.log("[DEBUG][store] POST pipeline (new)", { name: p.name, timestamp: Date.now() });
              await api.createPipeline(apiUrl, projectId, p);
            }
          } catch (err) {
            console.warn("Failed to sync pipeline:", p.name, err);
            toast.error(i18n.t("store.syncPipelineError"));
          }
        }
        console.log("[DEBUG][store] saveProjectPipelines UPSERT all END", { timestamp: Date.now() });

        // 2. THEN fetch remote list to find what to delete
        console.log("[DEBUG][store] saveProjectPipelines GET listPipelines START", { timestamp: Date.now() });
        const remotePipelines = await api.listPipelines(apiUrl, projectId);
        console.log("[DEBUG][store] saveProjectPipelines GET listPipelines END", { remoteCount: remotePipelines.length, timestamp: Date.now() });

        // Delete pipelines that were removed locally
        for (const rp of remotePipelines) {
          if (rp.id && !localIds.has(rp.id)) {
            try {
              console.log("[DEBUG][store] saveProjectPipelines DELETE START", { pipelineId: rp.id, timestamp: Date.now() });
              await api.deletePipeline(apiUrl, projectId, rp.id);
              console.log("[DEBUG][store] saveProjectPipelines DELETE END", { pipelineId: rp.id, timestamp: Date.now() });
            } catch (err) {
              console.warn("Failed to delete remote pipeline:", rp.id, err);
            }
          }
        }
      } catch (err) {
        console.warn("Failed to sync pipelines to backend:", err);
      }
    }
  },

  addSpec: async (projectId, spec, url, sync = false, slugParam?, serversParam?) => {
    const apiUrl = getApiUrl();
    const specName = spec.title || url || "New Spec";
    const { slugify } = await import("@/types/project");
    const finalSlug = slugParam || slugify(specName);
    const finalServers = serversParam || {};

    if (apiUrl) {
      try {
        const rawSpec = spec.raw as unknown as Record<string, unknown>;
        const record = await api.createSpec(apiUrl, projectId, { spec: rawSpec, url: url ?? null, sync, slug: finalSlug, servers: finalServers });
        const name = (record.spec as any)?.info?.title || specName;
        const newSpec: ProjectSpec = {
          id: record.id,
          slug: (record as any).slug || finalSlug,
          name,
          spec,
          url: record.url ?? undefined,
          sync: record.sync,
          servers: (record as any).servers || finalServers,
        };

        const current = get().currentProject;
        if (current?.id === projectId) {
          const updatedSpecs = [...current.specs, newSpec];
          const merged = getMergedSpec(updatedSpecs);
          const updated = { ...current, specs: updatedSpecs, spec: merged, updatedAt: new Date().toISOString() };
          set((state) => ({
            currentProject: updated,
            projects: state.projects.map((p) => (
              p.id === projectId ? { ...p, specs: updatedSpecs, spec: merged, updatedAt: updated.updatedAt } : p
            )),
          }));
        }
        return newSpec;
      } catch (err) {
        console.warn("Failed to add spec:", err);
        toast.error(i18n.t("store.addSpecError"));
        return null;
      }
    }

    // Local mode
    const newSpec: ProjectSpec = {
      id: generateUUID(),
      slug: finalSlug,
      name: specName,
      spec,
      url,
      sync,
      servers: finalServers,
    };
    const current = get().currentProject;
    if (current?.id === projectId) {
      const updatedSpecs = [...current.specs, newSpec];
      const merged = getMergedSpec(updatedSpecs);
      const updated = { ...current, specs: updatedSpecs, spec: merged, updatedAt: new Date().toISOString() };
      await persistProject(updated);
      set((state) => ({
        currentProject: updated,
        projects: state.projects.map((p) => (
          p.id === projectId ? { ...p, specs: updatedSpecs, spec: merged, updatedAt: updated.updatedAt } : p
        )),
      }));
    }
    return newSpec;
  },

  updateSpec: async (projectId, specId, spec, url, sync = false, slugParam?, serversParam?) => {
    const apiUrl = getApiUrl();

    if (apiUrl) {
      try {
        const rawSpec = spec.raw as unknown as Record<string, unknown>;
        await api.upsertSpec(apiUrl, projectId, specId, {
          spec: rawSpec,
          url: url ?? null,
          sync,
          slug: slugParam ?? null,
          servers: serversParam ?? null,
        });
      } catch (err) {
        console.warn("Failed to update spec:", err);
        toast.error(i18n.t("store.updateSpecError"));
      }
    }

    const current = get().currentProject;
    if (current?.id === projectId) {
      const updatedSpecs = current.specs.map((s) =>
        s.id === specId ? {
          ...s,
          spec,
          name: spec.title || s.name,
          url,
          sync,
          ...(slugParam !== undefined ? { slug: slugParam } : {}),
          ...(serversParam !== undefined ? { servers: serversParam } : {}),
        } : s
      );
      const merged = getMergedSpec(updatedSpecs);
      const updated = { ...current, specs: updatedSpecs, spec: merged, updatedAt: new Date().toISOString() };
      if (!apiUrl) {
        await persistProject(updated);
      }
      set((state) => ({
        currentProject: updated,
        projects: state.projects.map((p) => (
          p.id === projectId ? { ...p, specs: updatedSpecs, spec: merged, updatedAt: updated.updatedAt } : p
        )),
      }));
    }
  },

  removeSpec: async (projectId, specId) => {
    const apiUrl = getApiUrl();

    if (apiUrl) {
      try {
        await api.deleteSpec(apiUrl, projectId, specId);
      } catch (err) {
        console.warn("Failed to delete spec:", err);
        toast.error(i18n.t("store.addSpecError"));
      }
    }

    const current = get().currentProject;
    if (current?.id === projectId) {
      const updatedSpecs = current.specs.filter((s) => s.id !== specId);
      const merged = getMergedSpec(updatedSpecs);
      const updated = { ...current, specs: updatedSpecs, spec: merged, updatedAt: new Date().toISOString() };
      if (!apiUrl) {
        await persistProject(updated);
      }
      set((state) => ({
        currentProject: updated,
        projects: state.projects.map((p) => (
          p.id === projectId ? { ...p, specs: updatedSpecs, spec: merged, updatedAt: updated.updatedAt } : p
        )),
      }));
    }
  },

  createEnvGroup: async (projectId, data) => {
    const apiUrl = getApiUrl();
    let envGroup: ProjectEnvGroup;

    if (apiUrl) {
      try {
        envGroup = await api.createProjectEnvGroup(apiUrl, projectId, data);
      } catch (err) {
        console.warn("Failed to create env group:", err);
        toast.error("Erro ao criar env group");
        return null;
      }
    } else {
      const now = new Date().toISOString();
      envGroup = {
        id: generateUUID(),
        projectId,
        slug: data.slug,
        name: data.name,
        entries: data.entries,
        createdAt: now,
        updatedAt: now,
      };
    }

    const current = get().currentProject;
    if (current?.id === projectId) {
      const updated = updateEnvGroupsInProject(current, [...(current.envGroups ?? []), envGroup]);
      if (!apiUrl) {
        await persistProject(updated);
      }
      set((state) => ({
        currentProject: updated,
        projects: state.projects.map((p) => (p.id === projectId ? { ...p, envGroups: updated.envGroups, updatedAt: updated.updatedAt } : p)),
      }));
    }

    return envGroup;
  },

  updateEnvGroup: async (projectId, envGroupId, data) => {
    const apiUrl = getApiUrl();
    let envGroup: ProjectEnvGroup | null = null;

    if (apiUrl) {
      try {
        envGroup = await api.updateProjectEnvGroup(apiUrl, projectId, envGroupId, data);
      } catch (err) {
        console.warn("Failed to update env group:", err);
        toast.error("Erro ao atualizar env group");
        return;
      }
    }

    const current = get().currentProject;
    if (current?.id === projectId) {
      const now = new Date().toISOString();
      const updatedEnvGroups = (current.envGroups ?? []).map((group) =>
        group.id === envGroupId
          ? envGroup ?? { ...group, slug: data.slug, name: data.name, entries: data.entries, updatedAt: now }
          : group
      );
      const updated = updateEnvGroupsInProject(current, updatedEnvGroups);
      if (!apiUrl) {
        await persistProject(updated);
      }
      set((state) => ({
        currentProject: updated,
        projects: state.projects.map((p) => (p.id === projectId ? { ...p, envGroups: updated.envGroups, updatedAt: updated.updatedAt } : p)),
      }));
    }
  },

  deleteEnvGroup: async (projectId, envGroupId) => {
    const apiUrl = getApiUrl();

    if (apiUrl) {
      try {
        await api.deleteProjectEnvGroup(apiUrl, projectId, envGroupId);
      } catch (err) {
        console.warn("Failed to delete env group:", err);
        toast.error("Erro ao remover env group");
        return;
      }
    }

    const current = get().currentProject;
    if (current?.id === projectId) {
      const updated = updateEnvGroupsInProject(current, (current.envGroups ?? []).filter((group) => group.id !== envGroupId));
      if (!apiUrl) {
        await persistProject(updated);
      }
      set((state) => ({
        currentProject: updated,
        projects: state.projects.map((p) => (p.id === projectId ? { ...p, envGroups: updated.envGroups, updatedAt: updated.updatedAt } : p)),
      }));
    }
  },

  createPipeline: async (projectId, pipeline) => {
    const apiUrl = getApiUrl();
    let created: Pipeline;

    if (apiUrl) {
      // POST to backend — adopt canonical ID
      const record = await api.createPipeline(apiUrl, projectId, { ...pipeline, id: undefined });
      created = { ...pipeline, ...record, id: record.id ?? generateUUID() };
    } else {
      // Local-only: generate UUID
      created = { ...pipeline, id: generateUUID() };
    }

    // Update currentProject.pipelines
    const current = get().currentProject;
    if (current?.id === projectId) {
      const updatedPipelines = [...current.pipelines, created];
      const updated = { ...current, pipelines: updatedPipelines, updatedAt: new Date().toISOString() };
      if (!apiUrl) {
        await persistProject(updated);
      }
      set((state) => ({
        currentProject: updated,
        projects: state.projects.map((p) => (
          p.id === projectId ? { ...p, pipelines: updatedPipelines, updatedAt: updated.updatedAt } : p
        )),
      }));
    }

    return created;
  },
}));
