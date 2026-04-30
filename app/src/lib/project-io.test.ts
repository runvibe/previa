import { beforeEach, describe, expect, it, vi } from "vitest";
import { importProjectFile, isSqliteProjectImportFile } from "@/lib/project-io";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";

describe("project-io imports", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useOrchestratorStore.setState({
      contexts: [{ id: "local", name: "local", url: "http://127.0.0.1:5588" }],
      activeContextId: "local",
      activeContext: { id: "local", name: "local", url: "http://127.0.0.1:5588" },
      url: "http://127.0.0.1:5588",
      info: null,
    });
  });

  it("detects sqlite project import files", () => {
    expect(isSqliteProjectImportFile(new File([""], "projects.sqlite3"))).toBe(true);
    expect(isSqliteProjectImportFile(new File([""], "projects.sqlite"))).toBe(true);
    expect(isSqliteProjectImportFile(new File([""], "projects.db"))).toBe(true);
    expect(isSqliteProjectImportFile(new File(["{}"], "project.json", { type: "application/json" }))).toBe(false);
  });

  it("imports sqlite files as binary payloads", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 201,
      json: async () => ({
        includeHistory: true,
        projectsImported: 2,
        projects: [],
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const file = new File([new Uint8Array([1, 2, 3])], "projects.sqlite3", {
      type: "application/vnd.sqlite3",
    });

    await expect(importProjectFile(file)).resolves.toEqual({ projectsImported: 2 });
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:5588/api/v1/projects/import?includeHistory=true",
      expect.objectContaining({
        method: "POST",
        headers: { "Content-Type": "application/vnd.sqlite3" },
        body: expect.any(ArrayBuffer),
      }),
    );
  });

  it("keeps json project imports on the existing json endpoint path", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 201,
      json: async () => ({
        id: "imported-project",
        name: "Imported Project",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const file = new File([
      JSON.stringify({
        format: "previa.project.export.v1",
        exportedAt: "2026-04-30T00:00:00Z",
        historyIncluded: false,
        project: {
          id: "project-a",
          name: "Project A",
          description: null,
          specs: [],
          pipelines: [],
        },
      }),
    ], "project.json", { type: "application/json" });

    await expect(importProjectFile(file)).resolves.toEqual({ projectsImported: 1 });
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:5588/api/v1/projects/import",
      expect.objectContaining({
        method: "POST",
        headers: { "Content-Type": "application/json" },
      }),
    );
  });
});
