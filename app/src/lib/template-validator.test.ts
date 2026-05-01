import { describe, expect, it } from "vitest";

import { validateInterpolations } from "@/lib/template-validator";

describe("template env group validation", () => {
  it("accepts current env group references when a selected group has the entry", () => {
    const diagnostics = validateInterpolations("{{envs.current.api}}/health", {
      selectedEnvGroupSlug: "hml",
      availableEnvGroups: [
        { slug: "hml", entries: ["api", "auth"] },
      ],
    });

    expect(diagnostics).toEqual([]);
  });

  it("warns when current env group references are used without a selection", () => {
    const diagnostics = validateInterpolations("{{envs.current.api}}/health", {
      availableEnvGroups: [
        { slug: "hml", entries: ["api"] },
      ],
    });

    expect(diagnostics).toEqual([
      expect.objectContaining({
        severity: "warning",
        message: expect.stringContaining("Selecione um env group"),
      }),
    ]);
  });

  it("warns for unknown explicit env group entries", () => {
    const diagnostics = validateInterpolations("{{envs.hml.payments}}/health", {
      availableEnvGroups: [
        { slug: "hml", entries: ["api", "auth"] },
      ],
    });

    expect(diagnostics).toEqual([
      expect.objectContaining({
        severity: "warning",
        message: expect.stringContaining("Entrada 'payments'"),
      }),
    ]);
  });
});
