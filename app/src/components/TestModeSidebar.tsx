import { useState } from "react";
import { createPortal } from "react-dom";
import { PanelLeftClose, PanelLeftOpen, Workflow, Zap } from "lucide-react";

import { TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

interface TestModeSidebarProps {
  compact?: boolean;
  collapsed?: boolean;
  hideWhenCollapsed?: boolean;
  onCollapsedChange?: (collapsed: boolean) => void;
}

const testModes = [
  { value: "integration", label: "End-to-End Test", icon: Workflow },
  { value: "loadtest", label: "Load Test", icon: Zap },
];

interface TooltipState {
  label: string;
  left: number;
  top: number;
}

export function TestModeSidebar({
  compact = false,
  collapsed = false,
  hideWhenCollapsed = false,
  onCollapsedChange,
}: TestModeSidebarProps) {
  const isCollapsed = compact ? false : collapsed;
  const [tooltip, setTooltip] = useState<TooltipState | null>(null);

  if (!compact && hideWhenCollapsed && isCollapsed) {
    return null;
  }

  const showTooltip = (label: string, element: HTMLElement) => {
    const rect = element.getBoundingClientRect();
    setTooltip({
      label,
      left: rect.right + 8,
      top: rect.top + rect.height / 2,
    });
  };

  return (
    <aside
      aria-label="Test modes"
      className={cn(
        "shrink-0 border-border/50 bg-card/40 transition-[width] duration-200 ease-out",
        compact ? "flex items-center gap-2 border-b px-3 py-2" : "border-r p-2",
        !compact && (isCollapsed ? "w-14" : "w-[184px]"),
      )}
    >
      {!compact && (
        <div className={cn("mb-2 flex", isCollapsed ? "justify-center" : "justify-end")}>
          <button
            type="button"
            aria-label={isCollapsed ? "Expand test mode sidebar" : "Collapse test mode sidebar"}
            title={isCollapsed ? "Expand" : "Collapse"}
            className="inline-flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onClick={() => onCollapsedChange?.(!isCollapsed)}
          >
            {isCollapsed ? <PanelLeftOpen className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
          </button>
        </div>
      )}

      <TabsList
        className={cn(
          "h-auto border-0 bg-transparent p-0 shadow-none",
          compact && "grid grid-cols-2 gap-1",
          compact && (isCollapsed ? "w-[88px]" : "flex-1"),
          !compact && "flex w-full flex-col gap-1",
        )}
      >
        {testModes.map(({ value, label, icon: Icon }) => (
          <TabsTrigger
            key={value}
            value={value}
            aria-label={isCollapsed ? label : undefined}
            title={isCollapsed ? label : undefined}
            onMouseEnter={(event) => isCollapsed && showTooltip(label, event.currentTarget)}
            onMouseLeave={() => setTooltip((current) => current?.label === label ? null : current)}
            onFocus={(event) => isCollapsed && showTooltip(label, event.currentTarget)}
            onBlur={() => setTooltip((current) => current?.label === label ? null : current)}
            className={cn(
              "group gap-2 text-xs text-muted-foreground shadow-none data-[state=active]:!bg-primary/15 data-[state=active]:!text-primary data-[state=active]:shadow-none",
              "hover:text-foreground",
              compact && (isCollapsed ? "h-9 w-10 px-0" : "px-3"),
              !compact && "h-10 w-full px-3",
              isCollapsed ? "justify-center px-0" : "justify-start",
            )}
          >
            <Icon className="h-4 w-4 shrink-0" />
            {!isCollapsed && <span className="truncate">{label}</span>}
          </TabsTrigger>
        ))}
      </TabsList>
      {isCollapsed && tooltip ? createPortal(
        <div
          role="tooltip"
          className="fixed z-[2147483647] -translate-y-1/2 whitespace-nowrap rounded-md border border-border bg-[hsl(var(--popover))] px-3 py-1.5 text-xs text-popover-foreground shadow-xl"
          style={{ left: tooltip.left, top: tooltip.top }}
        >
          {tooltip.label}
        </div>,
        document.body,
      ) : null}
    </aside>
  );
}
