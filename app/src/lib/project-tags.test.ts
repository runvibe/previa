import { describe, expect, it } from "vitest";

import {
  collectProjectTags,
  filterProjectsBySearchAndTags,
  normalizeProjectTags,
} from "@/lib/project-tags";
import type { Project } from "@/types/project";

const baseProject = (project: Partial<Project>): Project => ({
  id: project.id ?? "project-1",
  name: project.name ?? "Payments",
  description: project.description,
  createdAt: "2026-05-07T00:00:00.000Z",
  updatedAt: "2026-05-07T00:00:00.000Z",
  specs: [],
  envGroups: [],
  pipelines: [],
  tags: project.tags,
});

describe("project tags", () => {
  it("normalizes tags by trimming empties and deduplicating case-insensitively", () => {
    expect(normalizeProjectTags([" billing ", "", "Billing", "Critical"])).toEqual([
      "billing",
      "Critical",
    ]);
  });

  it("collects sorted unique tags from projects", () => {
    const projects = [
      baseProject({ tags: ["critical", "billing"] }),
      baseProject({ id: "project-2", tags: ["Billing", "qa"] }),
    ];

    expect(collectProjectTags(projects)).toEqual(["billing", "critical", "qa"]);
  });

  it("searches project title and description case-insensitively", () => {
    const projects = [
      baseProject({ name: "Payments", description: "Checkout stack" }),
      baseProject({ id: "project-2", name: "Orders", description: "Fulfillment flows" }),
    ];

    expect(filterProjectsBySearchAndTags(projects, "checkout", [])).toHaveLength(1);
    expect(filterProjectsBySearchAndTags(projects, "orders", [])[0].id).toBe("project-2");
  });

  it("filters projects that contain all selected tags", () => {
    const projects = [
      baseProject({ id: "project-1", tags: ["billing", "critical"] }),
      baseProject({ id: "project-2", tags: ["billing"] }),
      baseProject({ id: "project-3", tags: ["qa", "critical"] }),
    ];

    expect(
      filterProjectsBySearchAndTags(projects, "", ["billing", "critical"]).map((p) => p.id),
    ).toEqual(["project-1"]);
  });
});
