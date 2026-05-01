import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

import { AppHeader } from "@/components/AppHeader";

vi.mock("@/components/ContextSwitcher", () => ({
  ContextSwitcher: () => <div data-testid="context-switcher" />,
}));

vi.mock("@/components/EventsPanel", () => ({
  EventsPanel: () => <div data-testid="events-panel" />,
}));

describe("AppHeader", () => {
  it("exposes the stack dashboard from the stack context menu", async () => {
    const onDashboard = vi.fn();

    render(
      <MemoryRouter initialEntries={["/projects/project-1/pipeline/pipeline-1/integration-test"]}>
        <Routes>
          <Route
            path="/projects/:id/pipeline/:pipelineId/integration-test"
            element={
              <AppHeader
                projectName="Stack 1"
                pipelineName="Pipe 1"
                onBackToProjects={vi.fn()}
                onDashboard={onDashboard}
              />
            }
          />
        </Routes>
      </MemoryRouter>,
    );

    const stackMenuTrigger = screen.getByRole("button", { name: "Stack 1 actions" });
    fireEvent.keyDown(stackMenuTrigger, { key: "Enter" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Dashboard" }));

    expect(onDashboard).toHaveBeenCalledOnce();
  });
});
