import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ProjectTagsDialog } from "@/components/ProjectTagsDialog";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => ({
      "common.cancel": "Cancel",
      "projects.tags.add": "Add tag",
      "projects.tags.inputLabel": "Tag name",
      "projects.tags.save": "Save tags",
      "projects.tags.title": "Edit stack tags",
    }[key] ?? key),
  }),
}));

describe("ProjectTagsDialog", () => {
  it("adds removes deduplicates and saves tags", () => {
    const onSave = vi.fn();

    render(
      <ProjectTagsDialog
        open
        projectName="Payments"
        tags={["billing"]}
        onOpenChange={() => undefined}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByLabelText("Tag name"), { target: { value: "Critical" } });
    fireEvent.click(screen.getByRole("button", { name: "Add tag" }));
    fireEvent.change(screen.getByLabelText("Tag name"), { target: { value: "critical" } });
    fireEvent.click(screen.getByRole("button", { name: "Add tag" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove billing" }));
    fireEvent.click(screen.getByRole("button", { name: "Save tags" }));

    expect(onSave).toHaveBeenCalledWith(["Critical"]);
  });
});
