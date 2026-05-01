import { beforeEach, describe, expect, it, vi } from "vitest";
import { exportProjectsSqlite, importProjectFile, isSqliteProjectImportFile } from "@/lib/project-io";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";

describe("project-io imports", () => {
  const originalCreateElement = document.createElement.bind(document);
  let anchors: HTMLAnchorElement[];

  beforeEach(() => {
    vi.restoreAllMocks();
    vi.stubEnv("VITE_PREVIA_API_BASE_URL", "http://127.0.0.1:5588");
    anchors = [];
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:previa-export"),
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: vi.fn(),
    });
    vi.spyOn(document, "createElement").mockImplementation((tagName) => {
      const element = originalCreateElement(tagName);
      if (tagName.toLowerCase() === "a") {
        anchors.push(element as HTMLAnchorElement);
        Object.defineProperty(element, "click", {
          configurable: true,
          value: vi.fn(),
        });
      }
      return element;
    });
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

  it("exports all projects as a sqlite download", async () => {
    const sqliteBlob = new Blob([new Uint8Array([1, 2, 3])], { type: "application/vnd.sqlite3" });
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      blob: async () => sqliteBlob,
    });
    vi.stubGlobal("fetch", fetchMock);

    await exportProjectsSqlite(["project-a", "project-b"], true, true);

    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:5588/api/v1/projects/export",
      expect.objectContaining({
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          all: true,
          projectIds: [],
          includeHistory: true,
        }),
      }),
    );
    expect(URL.createObjectURL).toHaveBeenCalledWith(expect.any(Blob));
    expect(anchors[0].download).toBe("previa-projects.sqlite3");
  });

  it("exports selected projects as a sqlite download", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      blob: async () => new Blob([new Uint8Array([1])], { type: "application/vnd.sqlite3" }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await exportProjectsSqlite(["project-a", "project-b"], false, false);

    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:5588/api/v1/projects/export",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          all: false,
          projectIds: ["project-a", "project-b"],
          includeHistory: false,
        }),
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
