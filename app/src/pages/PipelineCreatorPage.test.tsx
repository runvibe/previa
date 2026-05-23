import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

import PipelineCreatorPage from "@/pages/PipelineCreatorPage";
import type { Pipeline } from "@/types/pipeline";

vi.mock("@/components/editors", () => ({
  PipelineEditor: ({ value }: { value: string }) => (
    <pre data-testid="pipeline-editor-value">{value}</pre>
  ),
}));

vi.mock("@/components/SplitPaneLayout", () => ({
  SplitPaneLayout: ({ leftPanel, rightPanel }: { leftPanel: React.ReactNode; rightPanel: React.ReactNode }) => (
    <div>
      <section>{leftPanel}</section>
      <section>{rightPanel}</section>
    </div>
  ),
}));

vi.mock("@/components/PreviewLayout", () => ({
  PreviewLayout: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

vi.mock("@/components/StepFlowGraph", () => ({
  StepFlowGraph: () => <div data-testid="step-flow-graph" />,
}));

vi.mock("@/components/UnsavedChangesDialog", () => ({
  UnsavedChangesDialog: () => null,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

function renderCreator(initialPipeline?: Pipeline) {
  return render(
    <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
      <PipelineCreatorPage
        onSaveAndRun={vi.fn()}
        isDark={false}
        initialPipeline={initialPipeline}
      />
    </MemoryRouter>,
  );
}

describe("PipelineCreatorPage", () => {
  it("fills the editor when the initial pipeline arrives after route render", () => {
    const pipeline: Pipeline = {
      id: "pipe-1",
      name: "GET qrud_open users",
      description: "Chama o endpoint aberto do qrud_open",
      steps: [
        {
          id: "get_qrud_open_users",
          name: "GET qrud_open users",
          description: "Chama o endpoint aberto do qrud_open",
          method: "GET",
          url: "http://gateway.sdx.autob/v1/qrud-open/users",
          headers: {},
          asserts: [{ field: "status", operator: "equals", expected: "200" }],
        },
      ],
    };

    const { rerender } = renderCreator();

    expect(screen.getByTestId("pipeline-editor-value")).toHaveTextContent("");

    rerender(
      <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
        <PipelineCreatorPage
          onSaveAndRun={vi.fn()}
          isDark={false}
          initialPipeline={pipeline}
        />
      </MemoryRouter>,
    );

    expect(screen.getByTestId("pipeline-editor-value")).toHaveTextContent("GET qrud_open users");
    expect(screen.getByTestId("pipeline-editor-value")).toHaveTextContent("get_qrud_open_users");
  });
});
