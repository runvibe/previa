import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ProjectEnvGroupsPanel } from "@/components/ProjectEnvGroupsPanel";
import type { ProjectEnvGroup } from "@/types/project";

function makeEnvGroup(index: number, entriesCount = 1): ProjectEnvGroup {
  return {
    id: `env-${index}`,
    projectId: "project-1",
    name: `Env ${index}`,
    slug: `env-${index}`,
    entries: Array.from({ length: entriesCount }, (_, entryIndex) => ({
      name: `api-${entryIndex}`,
      url: `https://api-${index}-${entryIndex}.example.com`,
      description: null,
    })),
    createdAt: "2026-05-01T00:00:00Z",
    updatedAt: "2026-05-01T00:00:00Z",
  };
}

describe("ProjectEnvGroupsPanel", () => {
  it("limits the sidebar env groups list height and allows scrolling", () => {
    render(
      <ProjectEnvGroupsPanel
        envGroups={[1, 2, 3, 4].map(makeEnvGroup)}
        onCreate={vi.fn()}
        onUpdate={vi.fn()}
        onDelete={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Env groups list")).toHaveClass("max-h-36", "overflow-y-auto");
  });

  it("uses a floating action bar for each env group", () => {
    render(
      <ProjectEnvGroupsPanel
        envGroups={[makeEnvGroup(1)]}
        onCreate={vi.fn()}
        onUpdate={vi.fn()}
        onDelete={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Env 1 actions")).toHaveClass(
      "glass",
      "absolute",
      "right-2",
      "opacity-0",
      "group-hover:opacity-100",
    );
  });

  it("does not show the env entry count in sidebar items", () => {
    render(
      <ProjectEnvGroupsPanel
        envGroups={[makeEnvGroup(9, 2)]}
        onCreate={vi.fn()}
        onUpdate={vi.fn()}
        onDelete={vi.fn()}
      />,
    );

    expect(screen.queryByText("2")).not.toBeInTheDocument();
  });
});
