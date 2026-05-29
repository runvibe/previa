import { beforeEach, describe, expect, it, vi } from "vitest";

import { useProjectStore } from "@/stores/useProjectStore";
import type { Project } from "@/types/project";

const toastErrorMock = vi.hoisted(() => vi.fn());
const toastSuccessMock = vi.hoisted(() => vi.fn());

vi.mock("sonner", () => ({
  toast: {
    error: toastErrorMock,
    success: toastSuccessMock,
  },
}));

const baseUrl = "http://127.0.0.1:5588";
const apiUrl = `${baseUrl}/api/v1`;

const projectSummary: Project = {
  id: "project-1",
  name: "Stack 1",
  createdAt: "2026-04-30T00:00:00.000Z",
  updatedAt: "2026-04-30T00:00:00.000Z",
  specs: [],
  envGroups: [],
  pipelines: [],
};

function jsonResponse(body: unknown, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
    text: async () => JSON.stringify(body),
  };
}

describe("useProjectStore loadProjects", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    toastErrorMock.mockReset();
    toastSuccessMock.mockReset();
    vi.stubEnv("VITE_PREVIA_API_BASE_URL", baseUrl);
    useProjectStore.setState({
      projects: [],
      currentProject: null,
      loading: false,
      isRemote: true,
    });
  });

  it("surfaces structured backend errors when loading stacks", async () => {
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url === `${apiUrl}/projects` && method === "GET") {
        return jsonResponse({
          error: "service_unavailable",
          message: "runner registry unavailable",
        }, 503);
      }

      throw new Error(`Unexpected request: ${method} ${url}`);
    }));

    await useProjectStore.getState().loadProjects();

    expect(toastErrorMock).toHaveBeenCalledWith("Service unavailable: runner registry unavailable");
  });
});

describe("useProjectStore duplicateProject", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    toastErrorMock.mockReset();
    toastSuccessMock.mockReset();
    vi.stubEnv("VITE_PREVIA_API_BASE_URL", baseUrl);
    useProjectStore.setState({
      projects: [projectSummary],
      currentProject: null,
      loading: false,
      isRemote: true,
    });
  });

  it("loads the full remote project before duplicating a stack card project", async () => {
    const requests: Array<{ url: string; method: string; body?: string }> = [];
    const rawSpec = {
      openapi: "3.0.0",
      info: { title: "Users API", version: "1.0.0" },
      paths: {},
    };
    const pipeline = {
      id: "pipeline-1",
      name: "Users CRUD",
      description: "CRUD pipeline",
      steps: [
        {
          id: "step-1",
          name: "Create user",
          method: "POST",
          url: "http://gateway.sdx.autob/qrud-open/users",
          description: "",
          headers: {},
          body: { username: "ana", email: "ana@example.com" },
        },
      ],
    };
    const envGroup = {
      id: "env-1",
      projectId: "project-1",
      slug: "dev",
      name: "Development",
      entries: [{ name: "API", url: "http://localhost:3000" }],
      createdAt: "2026-04-30T00:00:00.000Z",
      updatedAt: "2026-04-30T00:00:00.000Z",
    };

    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      requests.push({ url, method, body: init?.body ? String(init.body) : undefined });

      if (url === `${apiUrl}/projects/project-1` && method === "GET") {
        return jsonResponse({
          id: "project-1",
          name: "Stack 1",
          description: null,
          createdAt: projectSummary.createdAt,
          updatedAt: projectSummary.updatedAt,
        });
      }

      if (url === `${apiUrl}/projects/project-1/pipelines` && method === "GET") {
        return jsonResponse([pipeline]);
      }

      if (url === `${apiUrl}/projects/project-1/specs` && method === "GET") {
        return jsonResponse([{
          id: "spec-1",
          projectId: "project-1",
          spec: rawSpec,
          sync: false,
          slug: "users-api",
          servers: {},
          createdAt: projectSummary.createdAt,
          updatedAt: projectSummary.updatedAt,
        }]);
      }

      if (url === `${apiUrl}/projects/project-1/env-groups` && method === "GET") {
        return jsonResponse([envGroup]);
      }

      if (url === `${apiUrl}/projects` && method === "POST") {
        return jsonResponse({
          id: "project-copy",
          name: "Stack 1 (cópia)",
          description: null,
          createdAt: projectSummary.createdAt,
          updatedAt: projectSummary.updatedAt,
        }, 201);
      }

      if (url === `${apiUrl}/projects/project-copy/pipelines` && method === "POST") {
        return jsonResponse({ ...pipeline, id: "pipeline-copy" }, 201);
      }

      if (url === `${apiUrl}/projects/project-copy/specs` && method === "POST") {
        return jsonResponse({
          id: "spec-copy",
          projectId: "project-copy",
          spec: rawSpec,
          sync: false,
          slug: "users-api",
          servers: {},
          createdAt: projectSummary.createdAt,
          updatedAt: projectSummary.updatedAt,
        }, 201);
      }

      if (url === `${apiUrl}/projects/project-copy/env-groups` && method === "POST") {
        return jsonResponse({ ...envGroup, id: "env-copy", projectId: "project-copy" }, 201);
      }

      throw new Error(`Unexpected request: ${method} ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    const duplicated = await useProjectStore.getState().duplicateProject("project-1");

    expect(duplicated?.id).toBe("project-copy");
    expect(requests).toEqual(expect.arrayContaining([
      expect.objectContaining({ url: `${apiUrl}/projects/project-1`, method: "GET" }),
      expect.objectContaining({ url: `${apiUrl}/projects/project-1/pipelines`, method: "GET" }),
      expect.objectContaining({ url: `${apiUrl}/projects/project-1/specs`, method: "GET" }),
      expect.objectContaining({ url: `${apiUrl}/projects/project-1/env-groups`, method: "GET" }),
      expect.objectContaining({ url: `${apiUrl}/projects/project-copy/pipelines`, method: "POST" }),
      expect.objectContaining({ url: `${apiUrl}/projects/project-copy/specs`, method: "POST" }),
      expect.objectContaining({ url: `${apiUrl}/projects/project-copy/env-groups`, method: "POST" }),
    ]));
    expect(fetchMock).toHaveBeenCalledTimes(8);
  });
});

describe("useProjectStore deleteProject", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    toastErrorMock.mockReset();
    toastSuccessMock.mockReset();
    vi.stubEnv("VITE_PREVIA_API_BASE_URL", baseUrl);
    useProjectStore.setState({
      projects: [projectSummary],
      currentProject: projectSummary,
      loading: false,
      isRemote: true,
    });
  });

  it("keeps a remote stack visible and reports missing permission when delete returns 403", async () => {
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url === `${apiUrl}/projects/project-1` && method === "DELETE") {
        return jsonResponse({ message: "forbidden" }, 403);
      }

      throw new Error(`Unexpected request: ${method} ${url}`);
    }));

    await expect(useProjectStore.getState().deleteProject("project-1")).rejects.toThrow("HTTP 403");

    expect(useProjectStore.getState().projects).toEqual([projectSummary]);
    expect(useProjectStore.getState().currentProject).toEqual(projectSummary);
    expect(toastErrorMock).toHaveBeenCalledWith("You do not have permission to perform this action.");
  });
});
