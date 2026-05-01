import type { OpenAPISpec, Pipeline } from "./pipeline";

export interface ProjectSpec {
  id: string;
  slug: string;
  name: string;
  spec: OpenAPISpec;
  url?: string;
  sync: boolean;
  servers: Record<string, string>;
  specMd5?: string;
}

export interface ProjectEnvEntry {
  name: string;
  url: string;
  description?: string | null;
}

export interface ProjectEnvGroup {
  id: string;
  projectId: string;
  slug: string;
  name: string;
  entries: ProjectEnvEntry[];
  createdAt: string;
  updatedAt: string;
}

/**
 * Generate a slug from a spec name.
 */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[-_]+|[-_]+$/g, "") || "spec";
}

export interface Project {
  id: string;
  name: string;
  description?: string;
  createdAt: string;
  updatedAt: string;
  /** @deprecated Use specs[] instead. Kept for backward compatibility — returns merged routes from all specs. */
  spec?: OpenAPISpec;
  specs: ProjectSpec[];
  envGroups: ProjectEnvGroup[];
  pipelines: Pipeline[];
}

/**
 * Helper: build a merged OpenAPISpec from multiple ProjectSpecs.
 * Merges routes from all specs into a single virtual spec for backward compatibility.
 */
export function getMergedSpec(specs: ProjectSpec[]): OpenAPISpec | undefined {
  if (specs.length === 0) return undefined;
  if (specs.length === 1) return specs[0].spec;

  const allRoutes = specs.flatMap((s) => s.spec.routes);
  const first = specs[0].spec;
  return {
    title: specs.map((s) => s.name).join(" + "),
    version: first.version,
    routes: allRoutes,
    raw: first.raw, // raw is not merged — use individual specs for raw access
  };
}

/**
 * Migrate a legacy project (single spec) to multi-spec format.
 */
export function migrateProjectSpecs(project: Project): Project {
  if (project.specs && project.specs.length > 0) return project;
  if (!project.spec) return { ...project, specs: [], envGroups: project.envGroups ?? [] };

  const legacySpec: ProjectSpec = {
    id: "default",
    slug: slugify(project.spec.title || "default"),
    name: project.spec.title || "Default Spec",
    spec: project.spec,
    sync: false,
    servers: {},
  };

  return {
    ...project,
    specs: [legacySpec],
    envGroups: project.envGroups ?? [],
  };
}
