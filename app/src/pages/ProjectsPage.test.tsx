import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import ProjectsPage from "@/pages/ProjectsPage";
import type { Project } from "@/types/project";

const useAppHeaderMock = vi.hoisted(() => vi.fn());
const exportProjectsSqliteMock = vi.hoisted(() => vi.fn());
const importProjectFileMock = vi.hoisted(() => vi.fn());
const toastSuccessMock = vi.hoisted(() => vi.fn());
const toastErrorMock = vi.hoisted(() => vi.fn());

const project: Project = {
  id: "project-1",
  name: "Stack 1",
  createdAt: "2026-04-30T00:00:00.000Z",
  updatedAt: "2026-04-30T00:00:00.000Z",
  specs: [],
  envGroups: [],
  pipelines: [],
};

const projectStoreMock = vi.hoisted(() => ({
  projects: [] as Project[],
  loading: false,
  loadProjects: vi.fn(),
  createProject: vi.fn(),
  updateProject: vi.fn(),
  deleteProject: vi.fn(),
  duplicateProject: vi.fn(),
}));

const useOrchestratorStoreMock = vi.hoisted(() => {
  const state = {
    url: "http://127.0.0.1:5588",
    fetchInfo: vi.fn(),
  };
  const store = vi.fn((selector: (value: typeof state) => unknown) => selector(state));
  return Object.assign(store, {
    getState: vi.fn(() => state),
  });
});

vi.mock("@/components/AppShell", () => ({
  useAppHeader: useAppHeaderMock,
}));

vi.mock("@/lib/project-io", () => ({
  exportProjectsSqlite: exportProjectsSqliteMock,
  importProjectFile: importProjectFileMock,
}));

vi.mock("@/stores/useProjectStore", () => ({
  useProjectStore: () => projectStoreMock,
}));

vi.mock("@/stores/useOrchestratorStore", () => ({
  useOrchestratorStore: useOrchestratorStoreMock,
}));

vi.mock("sonner", () => ({
  toast: {
    success: toastSuccessMock,
    error: toastErrorMock,
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    i18n: { language: "en" },
    t: (key: string, params?: Record<string, number | string>) => {
      const translations: Record<string, string> = {
        "common.delete": "Delete",
        "common.duplicate": "Duplicate",
        "common.export": "Export",
        "common.import": "Import",
        "common.open": "Open",
        "common.rename": "Rename",
        "common.cancel": "Cancel",
        "dashboard.title": "Dashboard",
        "export.sqlite.error": "Error exporting projects.",
        "export.sqlite.success": "Projects exported successfully!",
        "projects.defaultName": `Stack ${params?.number ?? ""}`,
        "projects.duplicated": "Project duplicated!",
        "projects.empty.button": "Create First Stack",
        "projects.empty.description": "Create your first stack.",
        "projects.empty.title": "No stacks yet",
        "projects.filters.clear": "Clear filters",
        "projects.filters.noResults.description": "Try another search or remove a tag filter.",
        "projects.filters.noResults.title": "No stacks match",
        "projects.filters.searchPlaceholder": "Search title or description",
        "projects.pagination.next": "Next",
        "projects.pagination.pageLabel": `Page ${params?.page ?? ""} of ${params?.total ?? ""}`,
        "projects.pagination.previous": "Previous",
        "projects.pagination.summary": `${params?.start ?? ""}-${params?.end ?? ""} of ${params?.total ?? ""} stacks`,
        "projects.importError": "Error importing project.",
        "projects.imported": "Project imported!",
        "projects.loading": "Loading...",
        "projects.new": "New Stack",
        "projects.open": "Open Stack",
        "projects.renamed": "Project renamed!",
        "projects.subtitle": "Manage your API stacks and pipelines",
        "projects.tags.add": "Add tag",
        "projects.tags.edit": "Edit tags",
        "projects.tags.inputLabel": "Tag name",
        "projects.tags.save": "Save tags",
        "projects.tags.title": "Edit stack tags",
        "projects.tags.updated": "Stack tags updated.",
        "projects.title": "My Stacks",
        "projects.deleteConfirm.description": `Delete ${params?.name ?? ""}?`,
        "projects.deleteConfirm.title": "Delete stack?",
        "onboarding.agent.title": "Give your agent a QA runtime",
        "onboarding.agent.description": "Create a stack, import an OpenAPI spec, and let your agent run real workflows instead of guessing.",
        "onboarding.agent.create": "Create stack",
        "onboarding.agent.import": "Import stack",
        "onboarding.agent.docs": "Open docs",
      };
      return translations[key] ?? key;
    },
  }),
}));

function renderPage() {
  return render(
    <MemoryRouter>
      <ProjectsPage />
    </MemoryRouter>,
  );
}

async function openProjectMenu() {
  const menuButton = screen.getByRole("button", { name: "Stack 1 actions" });
  fireEvent.pointerDown(menuButton);
  fireEvent.keyDown(menuButton, { key: "Enter" });
}

describe("ProjectsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    projectStoreMock.projects = [project];
    projectStoreMock.loading = false;
    projectStoreMock.createProject.mockResolvedValue(project);
    projectStoreMock.updateProject.mockResolvedValue(project);
    projectStoreMock.deleteProject.mockResolvedValue(undefined);
    projectStoreMock.duplicateProject.mockResolvedValue({
      ...project,
      id: "project-copy",
      name: "Stack 1 (cópia)",
    });
    exportProjectsSqliteMock.mockResolvedValue(undefined);
  });

  it("exports a stack card as a SQLite project export", async () => {
    renderPage();

    await openProjectMenu();
    fireEvent.click(await screen.findByRole("menuitem", { name: "Export" }));

    await waitFor(() => {
      expect(exportProjectsSqliteMock).toHaveBeenCalledWith(["project-1"], false, false);
    });
    expect(toastSuccessMock).toHaveBeenCalledWith("Projects exported successfully!");
  });

  it("refreshes projects after duplicating from the stack card", async () => {
    renderPage();

    await waitFor(() => expect(projectStoreMock.loadProjects).toHaveBeenCalled());
    projectStoreMock.loadProjects.mockClear();

    await openProjectMenu();
    fireEvent.click(await screen.findByRole("menuitem", { name: "Duplicate" }));

    await waitFor(() => {
      expect(projectStoreMock.duplicateProject).toHaveBeenCalledWith("project-1");
    });
    expect(projectStoreMock.loadProjects).toHaveBeenCalledTimes(1);
    expect(toastSuccessMock).toHaveBeenCalledWith("Project duplicated!");
  });

  it("saves edited stack tags through the project store", async () => {
    projectStoreMock.projects = [{ ...project, tags: ["billing"] }];
    renderPage();

    await openProjectMenu();
    fireEvent.click(await screen.findByRole("menuitem", { name: "Edit tags" }));

    fireEvent.change(await screen.findByLabelText("Tag name"), { target: { value: "critical" } });
    fireEvent.click(screen.getByRole("button", { name: "Add tag" }));
    fireEvent.click(screen.getByRole("button", { name: "Save tags" }));

    await waitFor(() => {
      expect(projectStoreMock.updateProject).toHaveBeenCalledWith("project-1", {
        tags: ["billing", "critical"],
      });
    });
  });

  it("filters stacks by title and description search", () => {
    projectStoreMock.projects = [
      { ...project, id: "project-1", name: "Payments", description: "Checkout flows" },
      { ...project, id: "project-2", name: "Orders", description: "Fulfillment APIs" },
    ];

    renderPage();

    fireEvent.change(screen.getByPlaceholderText("Search title or description"), {
      target: { value: "checkout" },
    });

    expect(screen.getByText("Payments")).toBeInTheDocument();
    expect(screen.queryByText("Orders")).not.toBeInTheDocument();
  });

  it("filters stacks by selected tag", () => {
    projectStoreMock.projects = [
      { ...project, id: "project-1", name: "Payments", tags: ["billing"] },
      { ...project, id: "project-2", name: "Orders", tags: ["fulfillment"] },
    ];

    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "billing" }));

    expect(screen.getByText("Payments")).toBeInTheDocument();
    expect(screen.queryByText("Orders")).not.toBeInTheDocument();
  });

  it("keeps the stacks page scrollable inside the app shell", () => {
    renderPage();

    expect(screen.getByRole("main")).toHaveClass("min-h-0", "overflow-y-auto");
  });

  it("paginates stacks with a maximum of ten cards per page", () => {
    projectStoreMock.projects = Array.from({ length: 11 }, (_, index) => ({
      ...project,
      id: `project-${index + 1}`,
      name: `Stack ${index + 1}`,
    }));

    renderPage();

    expect(screen.getByText("Stack 1")).toBeInTheDocument();
    expect(screen.getByText("Stack 10")).toBeInTheDocument();
    expect(screen.queryByText("Stack 11")).not.toBeInTheDocument();
    expect(screen.getByText("1-10 of 11 stacks")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));

    expect(screen.queryByText("Stack 1")).not.toBeInTheDocument();
    expect(screen.getByText("Stack 11")).toBeInTheDocument();
    expect(screen.getByText("11-11 of 11 stacks")).toBeInTheDocument();
  });

  it("shows agent-runtime onboarding when there are no stacks", () => {
    projectStoreMock.projects = [];

    renderPage();

    expect(screen.getByText("Give your agent a QA runtime")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create stack" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import stack" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open docs" })).toHaveAttribute(
      "href",
      "https://github.com/runvibe/previa/tree/main/docs/previa",
    );
  });
});
