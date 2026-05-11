import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ProjectSettingsDialog } from "@/components/ProjectSettingsDialog";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    i18n: { language: "en", changeLanguage: vi.fn() },
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}));

describe("ProjectSettingsDialog", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("shows experimental features enabled by default with AI Assistant settings visible", () => {
    render(<ProjectSettingsDialog />);

    fireEvent.click(screen.getByTitle("settings.tooltip"));

    expect(screen.getByText("Experimental Features")).toBeInTheDocument();
    expect(screen.getByText("AI Assistant")).toBeInTheDocument();
  });

  it("hides AI Assistant settings when experimental features are disabled", () => {
    localStorage.setItem("previa-experimental-features-enabled", "false");

    render(<ProjectSettingsDialog />);

    fireEvent.click(screen.getByTitle("settings.tooltip"));

    expect(screen.getByText("Experimental Features")).toBeInTheDocument();
    expect(screen.queryByText("AI Assistant")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("settings.openai.label")).not.toBeInTheDocument();
  });
});
