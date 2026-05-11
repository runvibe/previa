import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppHeader } from "@/components/AppHeader";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";

vi.mock("@/components/EventsPanel", () => ({
  EventsPanel: () => <div data-testid="events-panel" />,
}));

describe("AppHeader", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
    useOrchestratorStore.setState({
      contexts: [],
      activeContextId: null,
      activeContext: null,
      url: null,
      info: null,
    });
  });

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

  it("does not show an api version subtitle when no context is active", () => {
    render(
      <MemoryRouter>
        <AppHeader onBackToProjects={vi.fn()} />
      </MemoryRouter>,
    );

    expect(screen.queryByText("alpha")).not.toBeInTheDocument();
  });

  it("shows the active context api version under the logo", async () => {
    const activeContext = { id: "local", name: "local", url: "http://127.0.0.1:5798" };
    useOrchestratorStore.setState({
      contexts: [activeContext],
      activeContextId: activeContext.id,
      activeContext,
      url: activeContext.url,
      info: null,
    });
    const fetchMock = vi.fn(async (url: string) => {
      if (url === "http://127.0.0.1:5798/openapi.json") {
        return {
          ok: true,
          json: async () => ({ info: { version: "1.0.0-alpha.21" } }),
        };
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    render(
      <MemoryRouter>
        <AppHeader onBackToProjects={vi.fn()} />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("1.0.0-alpha.21")).toBeInTheDocument();
    });
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:5798/openapi.json",
      expect.any(Object),
    );
  });
});
