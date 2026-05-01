import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Input } from "./input";

describe("Input", () => {
  it("keeps visible fill on matching card backgrounds without borders", () => {
    render(<Input aria-label="API URL" />);

    const input = screen.getByLabelText("API URL");

    expect(input).not.toHaveClass("border");
    expect(input).not.toHaveClass("border-input");
    expect(input).toHaveClass("bg-background/60");
  });
});
