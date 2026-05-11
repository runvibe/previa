import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps, ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import TestExecutionPage from "@/pages/TestExecutionPage";
import type { Pipeline } from "@/types/pipeline";

let isMobile = false;

const executionStore = {
  results: {},
  running: false,
  runs: [],
  activeRunId: null,
  executionNode: null,
  latestStatuses: {},
  setLatestStatuses: vi.fn(),
  clearPipelineStatus: vi.fn(),
  checkPipelineRuntime: vi.fn().mockResolvedValue(undefined),
  loadHistory: vi.fn().mockResolvedValue([]),
  fetchAllLatestStatuses: vi.fn().mockResolvedValue({}),
  runTest: vi.fn(),
  disconnectController: vi.fn(),
  clearResults: vi.fn(),
  setRuns: vi.fn(),
  selectRun: vi.fn(),
  deleteLocalRunsForPipeline: vi.fn(),
};

const loadTestStore = {
  loadHistory: vi.fn().mockResolvedValue([]),
  disconnectController: vi.fn(),
};

vi.mock("@/hooks/use-mobile", () => ({
  useIsMobile: () => isMobile,
}));

vi.mock("@/components/LoadTestTab", () => ({
  LoadTestTab: () => <div data-testid="load-test-tab" />,
}));

vi.mock("@/stores/useExecutionHistoryStore", () => ({
  useExecutionHistoryStore: (selector: (state: typeof executionStore) => unknown) => selector(executionStore),
}));

vi.mock("@/stores/useLoadTestHistoryStore", () => ({
  useLoadTestHistoryStore: (selector: (state: typeof loadTestStore) => unknown) => selector(loadTestStore),
}));

vi.mock("@/stores/useOrchestratorStore", () => ({
  useOrchestratorStore: (selector: (state: { activeContext: { id: string } }) => unknown) =>
    selector({ activeContext: { id: "context-1" } }),
}));

const stepViewState = {
  mode: "list" as const,
  setMode: vi.fn(),
};

vi.mock("@/stores/useStepViewStore", () => {
  const useStepViewStore = (selector: (state: typeof stepViewState) => unknown) => selector(stepViewState);
  useStepViewStore.getState = () => stepViewState;
  return { useStepViewStore };
});

vi.mock("@/lib/api-client", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api-client")>("@/lib/api-client");
  return {
    ...actual,
    projectEnvGroupsToRuntime: vi.fn(() => []),
  };
});

vi.mock("react-i18next", () => ({
  Trans: ({ children }: { children?: ReactNode }) => <>{children}</>,
  useTranslation: () => ({
    i18n: { language: "en-US" },
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}));

const pipeline: Pipeline = {
  id: "pipeline-1",
  name: "Pipeline",
  description: "Pipeline description",
  steps: [
    {
      id: "step-1",
      name: "Step",
      description: "Step description",
      headers: {},
      method: "GET",
      url: "http://localhost/test",
    },
  ],
};

const spec = {
  id: "spec-1",
  slug: "example",
  name: "Example API",
  sync: false,
  servers: {},
  spec: {
    raw: {},
    title: "Example API",
    version: "1.0.0",
    routes: [{ method: "GET", path: "/users" }],
  },
};

const renderPage = (props: Partial<ComponentProps<typeof TestExecutionPage>> = {}) =>
  render(
    <TestExecutionPage
      pipelines={[pipeline]}
      projectId="project-1"
      onDeletePipeline={vi.fn()}
      onCreatePipeline={vi.fn()}
      onSelectPipeline={vi.fn()}
      initialTab="loadtest"
      executionBackendUrl="http://localhost:8080"
      {...props}
    />,
  );

describe("TestExecutionPage", () => {
  beforeEach(() => {
    isMobile = false;
    localStorage.clear();
    vi.clearAllMocks();
  });

  it("keeps test mode icon navigation visible when the desktop submenu is collapsed", async () => {
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Collapse test mode sidebar" }));

    await waitFor(() => {
      expect(screen.getByLabelText("Test modes")).toHaveClass("w-14");
    });
    expect(screen.getByRole("tab", { name: "End-to-End Test" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Load Test" })).toBeInTheDocument();
    expect(screen.queryByTitle("Open navbar")).not.toBeInTheDocument();
  });

  it("restores and persists the desktop test mode sidebar collapsed state", async () => {
    localStorage.setItem("api-pipeline-studio:test-mode-sidebar-collapsed", "true");

    renderPage();

    await waitFor(() => {
      expect(screen.getByLabelText("Test modes")).toHaveClass("w-14");
    });

    fireEvent.click(screen.getByRole("button", { name: "Expand test mode sidebar" }));

    await waitFor(() => {
      expect(screen.getByLabelText("Test modes")).toHaveClass("w-[184px]");
    });
    expect(localStorage.getItem("api-pipeline-studio:test-mode-sidebar-collapsed")).toBe("false");
  });

  it("hides API specs and AI creation when experimental features are disabled", () => {
    localStorage.setItem("previa-experimental-features-enabled", "false");

    renderPage({
      specs: [spec],
      onCreateAIPipeline: vi.fn(),
      onEditSpec: vi.fn(),
      onDeleteSpec: vi.fn(),
    });

    expect(screen.queryByText("API Specs")).not.toBeInTheDocument();
    expect(screen.queryByTitle("testExecution.createWithAI")).not.toBeInTheDocument();
    expect(screen.getByTitle("testExecution.newPipeline")).toBeInTheDocument();
  });
});
