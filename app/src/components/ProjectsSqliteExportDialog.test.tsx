import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ProjectsSqliteExportDialog } from "@/components/ProjectsSqliteExportDialog";
import type { Project } from "@/types/project";

const exportProjectsSqliteMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/project-io", () => ({
  exportProjectsSqlite: exportProjectsSqliteMock,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, number>) => {
      const translations: Record<string, string> = {
        "common.cancel": "Cancel",
        "common.export": "Export",
        "common.selectAll": "Select all",
        "export.exporting": "Exporting...",
        "export.includeHistory": "Include execution history",
        "export.sqlite.description": "Choose which stacks will be exported to a SQLite file.",
        "export.sqlite.error": "Error exporting projects.",
        "export.sqlite.selectedCount": `${params?.count ?? 0} of ${params?.total ?? 0} selected`,
        "export.sqlite.success": "Projects exported successfully!",
        "export.sqlite.title": "Export projects",
      };
      return translations[key] ?? key;
    },
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

const projects: Project[] = [
  {
    id: "project-a",
    name: "Alpha",
    createdAt: "2026-04-30T00:00:00Z",
    updatedAt: "2026-04-30T00:00:00Z",
    specs: [],
    envGroups: [],
    pipelines: [],
  },
  {
    id: "project-b",
    name: "Beta",
    createdAt: "2026-04-30T00:00:00Z",
    updatedAt: "2026-04-30T00:00:00Z",
    specs: [],
    envGroups: [],
    pipelines: [],
  },
];

describe("ProjectsSqliteExportDialog", () => {
  beforeEach(() => {
    exportProjectsSqliteMock.mockReset();
    exportProjectsSqliteMock.mockResolvedValue(undefined);
  });

  it("exports a partial selection after selecting all and deselecting one project", async () => {
    render(
      <ProjectsSqliteExportDialog
        projects={projects}
        open
        onOpenChange={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByLabelText("Select all"));
    fireEvent.click(screen.getByLabelText("Beta"));
    fireEvent.click(screen.getByRole("button", { name: "Export" }));

    await waitFor(() => {
      expect(exportProjectsSqliteMock).toHaveBeenCalledWith(["project-a"], false, false);
    });
  });

  it("exports all projects when every project remains selected", async () => {
    render(
      <ProjectsSqliteExportDialog
        projects={projects}
        open
        onOpenChange={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByLabelText("Select all"));
    fireEvent.click(screen.getByLabelText("Include execution history"));
    fireEvent.click(screen.getByRole("button", { name: "Export" }));

    await waitFor(() => {
      expect(exportProjectsSqliteMock).toHaveBeenCalledWith(["project-a", "project-b"], true, true);
    });
  });
});
