import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { RunnerNavButton } from "@/components/AppShell";

describe("RunnerNavButton", () => {
  it("shows an alert dot when no runners are available", () => {
    const onClick = vi.fn();

    render(<RunnerNavButton hasUnavailableRunners onClick={onClick} />);

    expect(screen.getByLabelText("Runners indisponíveis")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Gerenciar runners" }));
    expect(onClick).toHaveBeenCalledOnce();
  });
});
