import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LoadTestConfigPanel } from "@/components/LoadTestConfigPanel";
import type { WaveLoadConfig } from "@/types/load-test";
import type { Pipeline } from "@/types/pipeline";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const labels: Record<string, string> = {
        "loadTest.wavePoints": "Wave points",
        "loadTest.wavePoints.help": "Timeline points for the load wave.",
        "loadTest.wavePoints.hint": "Each row defines when the wave changes.",
        "loadTest.duration": "Duration",
        "loadTest.duration.help": "Total wave duration.",
        "loadTest.durationCustom": "Custom",
        "loadTest.pointTimeMs": "Time in milliseconds",
        "loadTest.pointIntensity": "Intensity percent",
        "loadTest.pointTimeColumn": "Time (ms)",
        "loadTest.pointIntensityColumn": "Intensity (%)",
        "loadTest.addPoint": "Add point",
        "loadTest.removePoint": "Remove point",
        "loadTest.interpolation": "Interpolation",
        "loadTest.interpolationSmooth": "Smooth",
        "loadTest.interpolationLinear": "Linear",
        "loadTest.interpolationStep": "Step",
        "loadTest.runnerMaxRps": "RPS limit per runner",
        "loadTest.gracePeriod": "Grace period",
        "loadTest.wavePreview": "Wave editor",
        "loadTest.previewIntensityAxis": "Intensity (%)",
        "loadTest.previewTimeAxis": "Time (ms)",
        "loadTest.pointMaxRequests": "Max. requests per point",
        "loadTest.selectedPoint": "Selected point",
        "loadTest.configureManually": "Configure manually",
        "loadTest.estimatedTime": "Estimated time",
      };
      return labels[key] ?? key;
    },
  }),
}));

const pipeline: Pipeline = {
  id: "pipeline-1",
  name: "Pipeline",
  description: "Pipeline",
  steps: [
    {
      id: "step-1",
      name: "Step",
      description: "Step",
      headers: {},
      method: "GET",
      url: "https://example.com",
    },
  ],
};

function renderPanel(onConfigChange = vi.fn(), initialConfig?: WaveLoadConfig, runnerCount?: number) {
  render(
    <LoadTestConfigPanel
      pipeline={pipeline}
      onStart={vi.fn()}
      onConfigChange={onConfigChange}
      initialConfig={initialConfig}
      runnerCount={runnerCount}
    />,
  );
  return onConfigChange;
}

function mockGraphBounds() {
  const graph = screen.getByTestId("wave-editor-graph");
  Object.defineProperty(graph, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      width: 400,
      height: 200,
      right: 400,
      bottom: 200,
      toJSON: () => {},
    }),
  });
  return graph;
}

describe("LoadTestConfigPanel", () => {
  it("uses duration as the wave timeline width", () => {
    renderPanel();

    fireEvent.change(screen.getByLabelText("Duration"), { target: { value: "300000" } });

    expect(screen.getAllByText("300,000 ms (300s)").length).toBeGreaterThan(0);
  });

  it("uses duration presets and only shows the duration input for custom values", async () => {
    const onConfigChange = renderPanel(vi.fn(), {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 60_000, intensity: 80 },
      ],
      interpolation: "smooth",
      gracePeriodMs: 30_000,
    });

    expect(screen.getByRole("button", { name: "1m" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "10m" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "30m" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Duration")).not.toBeInTheDocument();
    expect(screen.getAllByText("60,000 ms (60s)").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "Custom" }));
    expect(screen.getByLabelText("Duration")).toHaveValue(60_000);

    fireEvent.click(screen.getByRole("button", { name: "10m" }));
    expect(screen.queryByLabelText("Duration")).not.toBeInTheDocument();

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as WaveLoadConfig;
      expect(latest.points.at(-1)?.atMs).toBe(600_000);
    });
  });

  it("places interpolation immediately before the wave editor", () => {
    renderPanel();

    const interpolationSelect = screen.getByRole("combobox");
    const graph = screen.getByTestId("wave-editor-graph");

    expect(interpolationSelect.compareDocumentPosition(graph)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  });

  it("keeps the wave graph and selected point controls in one bordered card", () => {
    renderPanel();

    const card = screen.getByTestId("wave-point-editor-card");

    expect(card).toHaveClass("border");
    expect(card).toContainElement(screen.getByTestId("wave-editor-graph"));
    expect(card).toContainElement(screen.getByText("Selected point"));
  });

  it("does not expose or emit max in flight from the wave config", async () => {
    const onConfigChange = renderPanel(vi.fn(), {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 120_000, intensity: 80 },
      ],
      interpolation: "smooth",
      maxInFlight: 5000,
      gracePeriodMs: 30_000,
    } as WaveLoadConfig & { maxInFlight: number });

    expect(screen.queryByText("Max in flight")).not.toBeInTheDocument();

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as Record<string, unknown>;
      expect(latest).not.toHaveProperty("maxInFlight");
    });
  });

  it("defaults runner max RPS to 600 and supports manual editing", async () => {
    const onConfigChange = renderPanel();

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as WaveLoadConfig;
      expect(latest.runnerMaxRps).toBe(600);
    });

    expect(screen.getByText("600 RPS")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Configure manually RPS limit per runner" }));
    fireEvent.change(screen.getByLabelText("RPS limit per runner"), { target: { value: "750" } });
    fireEvent.blur(screen.getByLabelText("RPS limit per runner"));

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as WaveLoadConfig;
      expect(latest.runnerMaxRps).toBe(750);
    });
  });

  it("shows planned request values below each wave point", () => {
    renderPanel(vi.fn(), {
      points: [
        { atMs: 0, intensity: 50 },
        { atMs: 2_000, intensity: 50 },
      ],
      interpolation: "linear",
      runnerMaxRps: 600,
      gracePeriodMs: 30_000,
    }, 3);

    expect(screen.getAllByText("900")).toHaveLength(2);
    expect(screen.getByText("Max. requests per point")).toBeInTheDocument();
    expect(screen.getByTestId("wave-point-value-strip")).toHaveTextContent("900");
    expect(screen.getByTestId("wave-point-marker-value-0")).toHaveClass("translate-x-0");
    expect(screen.getByTestId("wave-point-marker-value-1")).toHaveClass("-translate-x-full");
    expect(screen.getByTestId("wave-editor-graph").querySelectorAll("text")).toHaveLength(0);
    expect(screen.queryByText("900 req")).not.toBeInTheDocument();
  });

  it("only renders planned request values for configured wave points", () => {
    renderPanel(vi.fn(), {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 60_000, intensity: 50 },
        { atMs: 120_000, intensity: 80 },
      ],
      interpolation: "smooth",
      runnerMaxRps: 600,
      gracePeriodMs: 30_000,
    }, 3);

    expect(screen.queryAllByTestId(/^wave-second-marker-/)).toHaveLength(0);
    expect(screen.getAllByTestId(/^wave-point-marker-value-/)).toHaveLength(3);
    expect(screen.getByTestId("wave-point-marker-value-0")).toHaveTextContent("180");
    expect(screen.getByTestId("wave-point-marker-value-1")).toHaveTextContent("900");
    expect(screen.getByTestId("wave-point-marker-value-2")).toHaveTextContent("1440");
    expect(screen.getByTestId("wave-point-marker-value-1")).toHaveClass("-translate-x-1/2");
    expect(screen.queryAllByText(/ req$/)).toHaveLength(0);
  });

  it("renders round first and last wave point markers on the graph limits without horizontal letterboxing", () => {
    renderPanel(vi.fn(), {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 120_000, intensity: 80 },
      ],
      interpolation: "smooth",
      runnerMaxRps: 600,
      gracePeriodMs: 30_000,
    });

    expect(screen.getByTestId("wave-editor-graph")).toHaveAttribute("preserveAspectRatio", "none");
    expect(screen.getByTestId("wave-editor-graph").querySelectorAll("circle")).toHaveLength(0);
    expect(screen.getByTestId("wave-point-0")).toHaveClass("rounded-full");
    expect(screen.getByTestId("wave-point-0")).toHaveStyle({ left: "0%", top: "90%", width: "18px", height: "18px" });
    expect(screen.getByTestId("wave-point-1")).toHaveStyle({ left: "100%", top: "20%", width: "14px", height: "14px" });
  });

  it("clamps runner max RPS manual values between 1 and 1000", async () => {
    const onConfigChange = renderPanel(vi.fn(), {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 120_000, intensity: 80 },
      ],
      interpolation: "smooth",
      runnerMaxRps: 500,
      gracePeriodMs: 30_000,
    });

    expect(screen.getByText("500 RPS")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Configure manually RPS limit per runner" }));
    fireEvent.change(screen.getByLabelText("RPS limit per runner"), { target: { value: "1200" } });
    fireEvent.blur(screen.getByLabelText("RPS limit per runner"));

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as WaveLoadConfig;
      expect(latest.runnerMaxRps).toBe(1000);
    });

    fireEvent.click(screen.getByRole("button", { name: "Configure manually RPS limit per runner" }));
    fireEvent.change(screen.getByLabelText("RPS limit per runner"), { target: { value: "0" } });
    fireEvent.blur(screen.getByLabelText("RPS limit per runner"));

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as WaveLoadConfig;
      expect(latest.runnerMaxRps).toBe(1);
    });
  });

  it("creates points with one graph click and drags existing points", async () => {
    const onConfigChange = renderPanel();
    const graph = mockGraphBounds();

    fireEvent.click(graph, { clientX: 200, clientY: 50 });

    expect(screen.getByText("Selected point")).toBeInTheDocument();
    expect(screen.getByDisplayValue("60000")).toBeInTheDocument();
    expect(screen.getByDisplayValue("75")).toBeInTheDocument();

    const createdPoint = screen.getByTestId("wave-point-1");
    fireEvent.click(createdPoint, { clientX: 200, clientY: 50 });

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as WaveLoadConfig;
      expect(latest.points).toHaveLength(3);
    });

    const point = screen.getByTestId("wave-point-1");
    fireEvent.mouseDown(point, { clientX: 200, clientY: 50 });
    fireEvent.mouseMove(graph, { clientX: 300, clientY: 100 });
    fireEvent.mouseUp(graph, { clientX: 300, clientY: 100 });

    await waitFor(() => {
      const latest = onConfigChange.mock.calls.at(-1)?.[0] as WaveLoadConfig;
      expect(latest.points).toContainEqual({ atMs: 90_000, intensity: 50 });
    });
  });

  it("renders a different graph line shape for each interpolation", () => {
    const config: WaveLoadConfig = {
      points: [
        { atMs: 0, intensity: 10 },
        { atMs: 60_000, intensity: 80 },
        { atMs: 120_000, intensity: 30 },
      ],
      interpolation: "smooth",
      gracePeriodMs: 30_000,
    };

    renderPanel(vi.fn(), config);

    const path = screen.getByTestId("wave-editor-path");
    expect(path).toHaveAttribute("d", expect.stringContaining("C"));

    cleanup();
    renderPanel(vi.fn(), { ...config, interpolation: "step" });

    const stepPath = screen.getByTestId("wave-editor-path");
    expect(stepPath).toHaveAttribute("d", expect.stringContaining("H"));
    expect(stepPath).toHaveAttribute("d", expect.stringContaining("V"));
    expect(stepPath).not.toHaveAttribute("d", expect.stringContaining("C"));

    cleanup();
    renderPanel(vi.fn(), { ...config, interpolation: "linear" });

    const linearPath = screen.getByTestId("wave-editor-path");
    expect(linearPath).toHaveAttribute("d", expect.stringContaining("L"));
    expect(linearPath).not.toHaveAttribute("d", expect.stringContaining("C"));
    expect(linearPath).not.toHaveAttribute("d", expect.stringContaining("H"));
  });
});
