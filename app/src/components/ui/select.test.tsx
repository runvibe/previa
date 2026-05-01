import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Select, SelectContent, SelectItem } from "@/components/ui/select";

describe("SelectContent", () => {
  it("allows constraining the scrollable viewport", () => {
    render(
      <Select defaultOpen>
        <SelectContent
          viewportClassName="!h-auto max-h-[6.5rem] overflow-y-auto"
          viewportStyle={{ height: "auto", maxHeight: "6.5rem", overflowY: "auto" }}
        >
          <SelectItem value="sandbox">Sandbox</SelectItem>
          <SelectItem value="staging">Staging</SelectItem>
          <SelectItem value="production">Production</SelectItem>
          <SelectItem value="qa">QA</SelectItem>
        </SelectContent>
      </Select>,
    );

    const viewport = screen.getByRole("option", { name: "Sandbox" }).closest("[data-radix-select-viewport]");

    expect(viewport).toHaveClass("max-h-[6.5rem]", "!h-auto", "overflow-y-auto");
    expect(viewport).toHaveStyle({
      height: "auto",
      maxHeight: "6.5rem",
      overflowY: "auto",
    });
  });
});
