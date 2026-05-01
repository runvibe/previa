import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Tabs } from "@/components/ui/tabs";
import { TestModeSidebar } from "@/components/TestModeSidebar";

describe("TestModeSidebar", () => {
  it("renders the test mode selector as an internal sidebar", () => {
    render(
      <Tabs defaultValue="integration">
        <TestModeSidebar />
      </Tabs>,
    );

    expect(screen.getByLabelText("Test modes")).toHaveClass("border-r");
    expect(screen.getByRole("tab", { name: /End-to-End Test/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Load Test/i })).toBeInTheDocument();
  });
});
