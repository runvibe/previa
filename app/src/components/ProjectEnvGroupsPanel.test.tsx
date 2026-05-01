import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ProjectEnvGroupsPanel } from "@/components/ProjectEnvGroupsPanel";
import type { ProjectEnvGroup } from "@/types/project";

function makeEnvGroup(index: number): ProjectEnvGroup {
  return {
    id: `env-${index}`,
    projectId: "project-1",
    name: `Env ${index}`,
    slug: `env-${index}`,
    entries: [{ name: "api", url: `https://api-${index}.example.com`, description: null }],
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
});
