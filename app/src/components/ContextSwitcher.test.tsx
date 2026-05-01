import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ContextSwitcher } from "@/components/ContextSwitcher";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

describe("ContextSwitcher", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
    useOrchestratorStore.setState({
      contexts: [{ id: "current", name: "current", url: window.location.origin }],
      activeContextId: "current",
      activeContext: { id: "current", name: "current", url: window.location.origin },
      url: window.location.origin,
      info: {
        context: "default",
        totalRunners: 2,
        activeRunners: 1,
      },
    });
  });

  it("renders the resolved api endpoint as a read-only indicator", () => {
    render(<ContextSwitcher />);

    expect(screen.getByText("default")).toBeInTheDocument();
    expect(screen.getByText(window.location.origin)).toBeInTheDocument();
    expect(screen.queryByText("Contexto local encontrado")).not.toBeInTheDocument();
    expect(screen.queryByText("Add context")).not.toBeInTheDocument();
  });

  it("shows a disconnected state when orchestrator info is unavailable", () => {
    useOrchestratorStore.setState({ info: null });

    render(<ContextSwitcher />);

    expect(screen.getByText("Backend indisponível")).toBeInTheDocument();
    expect(screen.getByText(window.location.origin)).toBeInTheDocument();
  });
});
