import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { StepResultCard } from "@/components/StepResultCard";
import type { PipelineStep } from "@/types/pipeline";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

const step: PipelineStep = {
  id: "step-1",
  name: "Step 1",
  description: "A step",
  headers: {},
  method: "GET",
  url: "https://example.com",
  asserts: [],
};

describe("StepResultCard", () => {
  it("shows a rerun-from-step button and calls the handler", () => {
    const onRerunFromStep = vi.fn();

    render(
      <StepResultCard
        step={step}
        result={{ stepId: step.id, status: "success" }}
        onRerunFromStep={onRerunFromStep}
      />,
    );

    expect(screen.queryByText("Run here")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Rerun from here" }));

    expect(onRerunFromStep).toHaveBeenCalledWith(step.id);
  });
});
