import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RunnersPage from "@/pages/RunnersPage";
import type { RunnerRecord } from "@/lib/api-client";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";

const useAppHeaderMock = vi.hoisted(() => vi.fn());
const toastSuccessMock = vi.hoisted(() => vi.fn());
const toastErrorMock = vi.hoisted(() => vi.fn());
const translateMock = vi.hoisted(() => (key: string, params?: Record<string, string | number>) => {
  const translations: Record<string, string> = {
    "common.cancel": "Cancel",
    "common.delete": "Delete",
    "common.loading": "Loading...",
    "common.save": "Save",
    "runners.actions": "Actions",
    "runners.add": "Add runner",
    "runners.addError": "Error adding runner.",
    "runners.addSuccess": "Runner added successfully!",
    "runners.addTitle": "Add runner",
    "runners.deleteConfirm.description": `Remove runner ${params?.endpoint ?? ""} from this context?`,
    "runners.deleteConfirm.title": "Remove runner?",
    "runners.deleteError": "Error removing runner.",
    "runners.deleteRunner": `Remove runner ${params?.endpoint ?? ""}`,
    "runners.deleteSuccess": "Runner removed successfully!",
    "runners.disableRunner": "Disable runner",
    "runners.disabled": "Disabled",
    "runners.empty.description": "Add a runner endpoint to make it available for executions.",
    "runners.empty.title": "No runners registered",
    "runners.enableRunner": "Enable runner",
    "runners.enabled": "Enabled",
    "runners.endpoint": "Endpoint",
    "runners.lastSeen": "Last seen",
    "runners.loadError": "Error loading runners.",
    "runners.name": "Name",
    "runners.nameFor": `Name for ${params?.endpoint ?? ""}`,
    "runners.namePlaceholder": "Optional name",
    "runners.noBackend": "Backend not connected. Select or configure an active context.",
    "runners.refresh": "Refresh",
    "runners.runtime": "Runtime",
    "runners.status": "Status",
    "runners.subtitle": `Manage runners for ${params?.context ?? ""}`,
    "runners.summary.enabled": "Enabled",
    "runners.summary.healthy": "Healthy",
    "runners.summary.total": "Total",
    "runners.title": "Runners",
    "runners.updateError": "Error updating runner.",
    "runners.updateSuccess": "Runner updated successfully!",
  };
  return translations[key] ?? key;
});

vi.mock("@/components/AppShell", () => ({
  useAppHeader: useAppHeaderMock,
}));

vi.mock("sonner", () => ({
  toast: {
    success: toastSuccessMock,
    error: toastErrorMock,
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: translateMock,
  }),
}));

const baseUrl = "http://127.0.0.1:5588";

const runnerA: RunnerRecord = {
  id: "runner-a",
  endpoint: "http://127.0.0.1:55880",
  name: "Local runner",
  source: "manual",
  enabled: true,
  healthStatus: "healthy",
  lastSeenAt: "2026-04-30T10:00:00Z",
  lastError: null,
  runtime: {
    pid: 123,
    memoryBytes: 1048576,
    virtualMemoryBytes: 2097152,
    cpuUsagePercent: 1.5,
  },
  createdAt: "2026-04-30T09:00:00Z",
  updatedAt: "2026-04-30T10:00:00Z",
};

const runnerB: RunnerRecord = {
  ...runnerA,
  id: "runner-b",
  endpoint: "http://127.0.0.1:55881",
  name: null,
  enabled: false,
  healthStatus: "unknown",
  lastSeenAt: null,
  runtime: null,
};

function renderPage() {
  return render(
    <MemoryRouter>
      <RunnersPage />
    </MemoryRouter>,
  );
}

function mockFetch(initialRunners: RunnerRecord[] = [runnerA]) {
  let runners = [...initialRunners];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";

    if (url === `${baseUrl}/info`) {
      return {
        ok: true,
        status: 200,
        json: async () => ({ context: "local", totalRunners: runners.length, activeRunners: runners.filter((runner) => runner.enabled).length }),
      };
    }

    if (url === `${baseUrl}/api/v1/runners` && method === "GET") {
      return {
        ok: true,
        status: 200,
        json: async () => runners,
      };
    }

    if (url === `${baseUrl}/api/v1/runners` && method === "POST") {
      const body = JSON.parse(String(init?.body));
      const created = {
        ...runnerB,
        id: "runner-created",
        endpoint: body.endpoint,
        name: body.name,
        enabled: body.enabled,
      };
      runners = [...runners, created];
      return {
        ok: true,
        status: 201,
        json: async () => created,
      };
    }

    if (url === `${baseUrl}/api/v1/runners/runner-a` && method === "PATCH") {
      const body = JSON.parse(String(init?.body));
      const updated = { ...runnerA, ...body };
      runners = runners.map((runner) => (runner.id === "runner-a" ? updated : runner));
      return {
        ok: true,
        status: 200,
        json: async () => updated,
      };
    }

    if (url === `${baseUrl}/api/v1/runners/runner-a` && method === "DELETE") {
      runners = runners.filter((runner) => runner.id !== "runner-a");
      return {
        ok: true,
        status: 204,
        json: async () => undefined,
      };
    }

    throw new Error(`Unexpected request: ${method} ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("RunnersPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    toastSuccessMock.mockReset();
    toastErrorMock.mockReset();
    useAppHeaderMock.mockReset();
    useOrchestratorStore.setState({
      contexts: [{ id: "local", name: "local", url: baseUrl }],
      activeContextId: "local",
      activeContext: { id: "local", name: "local", url: baseUrl },
      url: baseUrl,
      info: null,
    });
  });

  it("renders runners returned by the API", async () => {
    mockFetch();

    renderPage();

    expect(await screen.findByDisplayValue("Local runner")).toBeInTheDocument();
    expect(screen.getByText("http://127.0.0.1:55880")).toBeInTheDocument();
    expect(screen.getByText("healthy")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Status" })).toHaveClass("text-center");
    expect(screen.getByRole("columnheader", { name: "Actions" })).toHaveClass("text-center");
  });

  it("adds a runner and updates the list", async () => {
    const fetchMock = mockFetch([]);

    renderPage();

    fireEvent.change(screen.getByLabelText("Endpoint"), { target: { value: "http://127.0.0.1:55882" } });
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "New runner" } });
    fireEvent.click(screen.getByRole("button", { name: "Add runner" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        `${baseUrl}/api/v1/runners`,
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({
            endpoint: "http://127.0.0.1:55882",
            name: "New runner",
            enabled: true,
          }),
        }),
      );
    });
    expect(await screen.findByDisplayValue("New runner")).toBeInTheDocument();
  });

  it("auto-saves a runner name when editing finishes", async () => {
    const fetchMock = mockFetch();

    renderPage();

    const nameInput = await screen.findByLabelText("Name for http://127.0.0.1:55880");
    fireEvent.focus(nameInput);
    fireEvent.change(nameInput, { target: { value: "Renamed runner" } });
    fireEvent.blur(nameInput);

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        `${baseUrl}/api/v1/runners/runner-a`,
        expect.objectContaining({
          method: "PATCH",
          body: JSON.stringify({ name: "Renamed runner" }),
        }),
      );
    });
  });

  it("toggles runner enabled state", async () => {
    const fetchMock = mockFetch();

    renderPage();

    const toggle = await screen.findByRole("switch", { name: "Disable runner" });
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        `${baseUrl}/api/v1/runners/runner-a`,
        expect.objectContaining({
          method: "PATCH",
          body: JSON.stringify({ enabled: false }),
        }),
      );
    });
  });

  it("removes a runner after confirmation", async () => {
    const fetchMock = mockFetch();

    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "Remove runner http://127.0.0.1:55880" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(`${baseUrl}/api/v1/runners/runner-a`, { method: "DELETE" });
    });
  });
});
