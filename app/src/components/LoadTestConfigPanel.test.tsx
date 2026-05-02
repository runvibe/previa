import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
        "loadTest.maxInFlight": "Max in flight",
        "loadTest.maxInFlight.help": "Maximum concurrent executions.",
        "loadTest.gracePeriod": "Grace period",
        "loadTest.wavePreview": "Wave editor",
        "loadTest.previewIntensityAxis": "Intensity (%)",
        "loadTest.previewTimeAxis": "Time (ms)",
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

function renderPanel(onConfigChange = vi.fn()) {
  render(
    <LoadTestConfigPanel
      pipeline={pipeline}
      onStart={vi.fn()}
      onConfigChange={onConfigChange}
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
});
