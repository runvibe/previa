import { render, screen, waitFor } from "@testing-library/react";
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
    Object.defineProperty(AbortSignal, "timeout", {
      configurable: true,
      value: vi.fn(() => new AbortController().signal),
    });
    window.localStorage.clear();
    useOrchestratorStore.setState({
      contexts: [],
      activeContextId: null,
      activeContext: null,
      url: null,
      info: null,
    });
  });

  it("auto-registers the current origin before checking localhost 5588", async () => {
    const origin = window.location.origin.replace(/\/+$/, "");
    const fetchMock = vi.fn(async (url: string) => {
      if (url === `${origin}/health`) return { ok: true };
      if (url === `${origin}/info`) {
        return {
          ok: true,
          json: async () => ({
            context: "same-origin",
            totalRunners: 1,
            activeRunners: 1,
          }),
        };
      }
      if (url === "http://localhost:5588/health") return { ok: true };
      if (url === "http://localhost:5588/info") {
        return {
          ok: true,
          json: async () => ({
            context: "local",
            totalRunners: 1,
            activeRunners: 1,
          }),
        };
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    render(<ContextSwitcher />);

    await waitFor(() => {
      expect(useOrchestratorStore.getState().activeContext).toMatchObject({
        name: "same-origin",
        url: origin,
      });
    });

    const urls = fetchMock.mock.calls.map(([url]) => url);
    expect(urls[0]).toBe(`${origin}/health`);
    expect(urls[1]).toBe(`${origin}/info`);
    expect(urls).toContain("http://localhost:5588/health");
  });

  it("keeps localhost 5588 behind the confirmation prompt", async () => {
    const origin = window.location.origin.replace(/\/+$/, "");
    const fetchMock = vi.fn(async (url: string) => {
      if (url === `${origin}/health`) return { ok: false };
      if (url === "http://localhost:5588/health") return { ok: true };
      if (url === "http://localhost:5588/info") {
        return {
          ok: true,
          json: async () => ({
            context: "local",
            totalRunners: 1,
            activeRunners: 1,
          }),
        };
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    render(<ContextSwitcher />);

    const prompt = await screen.findByText("Contexto local encontrado");
    const promptCard = prompt.closest(".z-\\[9999\\]");
    expect(promptCard).toBeTruthy();
    expect(promptCard).toHaveClass("fixed");
    expect(promptCard).toHaveClass("pointer-events-auto");
    expect(promptCard?.parentElement).toBe(document.body);
    expect(useOrchestratorStore.getState().contexts).toEqual([]);
  });
});
