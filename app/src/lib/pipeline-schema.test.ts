import { describe, expect, it } from "vitest";

import { PipelineStepSchema } from "@/lib/pipeline-schema";

describe("PipelineStepSchema response extractions", () => {
  it("preserves every extraction field", () => {
    const parsed = PipelineStepSchema.parse({
      id: "email",
      name: "Read e-mail",
      description: "",
      headers: {},
      method: "GET",
      url: "https://example.test/message",
      extracts: [{
        name: "code",
        field: "body.HTML",
        regex: "<strong>([0-9]{6})</strong>",
        group: 1,
        required: true,
      }],
    });

    expect((parsed as Record<string, unknown>).extracts).toEqual([{
      name: "code",
      field: "body.HTML",
      regex: "<strong>([0-9]{6})</strong>",
      group: 1,
      required: true,
    }]);
  });
});
