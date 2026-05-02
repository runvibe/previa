import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppShell, RunnerNavButton } from "@/components/AppShell";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";

const toastErrorMock = vi.hoisted(() => vi.fn());
const toastDismissMock = vi.hoisted(() => vi.fn());

vi.mock("@/components/OnboardingModal", () => ({
  OnboardingModal: () => null,
}));

vi.mock("@/components/EventsPanel", () => ({
  EventsPanel: () => <div data-testid="events-panel" />,
}));

vi.mock("@/components/InstallAppButton", () => ({
  InstallAppButton: () => <button type="button">Install</button>,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastErrorMock,
    dismiss: toastDismissMock,
  },
}));

describe("RunnerNavButton", () => {
  it("shows an alert dot when no runners are available", () => {
    const onClick = vi.fn();

    render(<RunnerNavButton hasUnavailableRunners onClick={onClick} />);

    expect(screen.getByLabelText("Runners indisponíveis")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Gerenciar runners" }));
    expect(onClick).toHaveBeenCalledOnce();
  });
});

describe("AppShell", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllEnvs();
    toastErrorMock.mockReset();
    toastDismissMock.mockReset();
    useOrchestratorStore.setState({
      contexts: [{ id: "current", name: "current", url: window.location.origin }],
      activeContextId: "current",
      activeContext: { id: "current", name: "current", url: window.location.origin },
      url: window.location.origin,
      info: null,
    });
  });

  it("loads orchestrator info when mounted on any route", async () => {
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === `${window.location.origin}/info`) {
        return {
          ok: true,
          json: async () => ({
            context: "default",
            totalRunners: 3,
            activeRunners: 1,
          }),
        };
      }
      if (url === `${window.location.origin}/openapi.json`) {
        return {
          ok: true,
          json: async () => ({ info: { version: "test" } }),
        };
      }
      throw new Error(`Unexpected request: ${url}`);
    }));

    render(
      <MemoryRouter initialEntries={["/projects/project-a/pipeline/pipeline-a/load-test"]}>
        <AppShell />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(useOrchestratorStore.getState().info).toMatchObject({
        context: "default",
        totalRunners: 3,
        activeRunners: 1,
      });
    });
    expect(toastDismissMock).toHaveBeenCalledWith("previa-api-offline");
    expect(toastErrorMock).not.toHaveBeenCalled();
  });

  it("shows a toast with the service url when the api is offline", async () => {
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === `${window.location.origin}/info`) {
        throw new Error("offline");
      }
      if (url === `${window.location.origin}/openapi.json`) {
        return {
          ok: true,
          json: async () => ({ info: { version: "test" } }),
        };
      }
      throw new Error(`Unexpected request: ${url}`);
    }));

    render(
      <MemoryRouter initialEntries={["/projects/project-a/pipeline/pipeline-a/load-test"]}>
        <AppShell />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalledWith("Sem conexão com o servidor", {
        id: "previa-api-offline",
        description: `URL do serviço: ${window.location.origin}`,
      });
    });
    expect(useOrchestratorStore.getState().info).toBeNull();
  });
});
