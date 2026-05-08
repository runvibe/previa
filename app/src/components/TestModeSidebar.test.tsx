import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Tabs } from "@/components/ui/tabs";
import { TestModeSidebar } from "@/components/TestModeSidebar";

describe("TestModeSidebar", () => {
  it("renders the test mode selector as an internal sidebar", () => {
    render(
      <Tabs defaultValue="integration">
        <TestModeSidebar collapsed={false} onCollapsedChange={vi.fn()} />
      </Tabs>,
    );

    expect(screen.getByLabelText("Test modes")).toHaveClass("border-r");
    expect(screen.getByRole("tablist")).toHaveClass("bg-transparent");
    expect(screen.getByRole("tab", { name: /End-to-End Test/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Load Test/i })).not.toHaveClass("hover:bg-accent/60");
  });

  it("requests collapse from the sidebar toggle", () => {
    const onCollapsedChange = vi.fn();

    render(
      <Tabs defaultValue="integration">
        <TestModeSidebar collapsed={false} onCollapsedChange={onCollapsedChange} />
      </Tabs>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Collapse test mode sidebar" }));

    expect(onCollapsedChange).toHaveBeenCalledWith(true);
  });

  it("renders only icon navigation when collapsed", () => {
    render(
      <Tabs defaultValue="loadtest">
        <TestModeSidebar collapsed onCollapsedChange={vi.fn()} />
      </Tabs>,
    );

    expect(screen.getByLabelText("Test modes")).toHaveClass("w-14");
    expect(screen.getByRole("tab", { name: "End-to-End Test" })).toHaveAttribute("title", "End-to-End Test");
    expect(screen.getByRole("tab", { name: "Load Test" })).toHaveAttribute("title", "Load Test");
    expect(screen.queryByText("End-to-End Test")).not.toBeInTheDocument();
    expect(screen.queryByText("Load Test")).not.toBeInTheDocument();
  });

  it("keeps compact navigation expanded without a collapse toggle", () => {
    render(
      <Tabs defaultValue="loadtest">
        <TestModeSidebar compact collapsed onCollapsedChange={vi.fn()} />
      </Tabs>,
    );

    expect(screen.getByLabelText("Test modes")).toHaveClass("border-b");
    expect(screen.getByRole("tab", { name: "End-to-End Test" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Load Test" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Collapse test mode sidebar" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Expand test mode sidebar" })).not.toBeInTheDocument();
  });

  it("shows button names as tooltips when collapsed", async () => {
    render(
      <Tabs defaultValue="integration">
        <TestModeSidebar collapsed onCollapsedChange={vi.fn()} />
      </Tabs>,
    );

    fireEvent.mouseEnter(screen.getByRole("tab", { name: "Load Test" }));

    expect(await screen.findByText("Load Test")).toBeInTheDocument();
  });

  it("renders collapsed tooltips with a solid high-layer background", async () => {
    render(
      <Tabs defaultValue="integration">
        <TestModeSidebar collapsed onCollapsedChange={vi.fn()} />
      </Tabs>,
    );

    fireEvent.mouseEnter(screen.getByRole("tab", { name: "Load Test" }));

    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).not.toHaveClass("glass", "bg-popover", "backdrop-blur-xl");
    expect(tooltip).toHaveClass("bg-[hsl(var(--popover))]", "z-[2147483647]");
    expect(tooltip.parentElement).toBe(document.body);
  });

  it("can be removed from layout when collapsed", () => {
    render(
      <Tabs defaultValue="integration">
        <TestModeSidebar collapsed hideWhenCollapsed onCollapsedChange={vi.fn()} />
      </Tabs>,
    );

    expect(screen.queryByLabelText("Test modes")).not.toBeInTheDocument();
  });
});
