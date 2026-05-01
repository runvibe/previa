import { Workflow, Zap } from "lucide-react";

import { TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

interface TestModeSidebarProps {
  compact?: boolean;
}

export function TestModeSidebar({ compact = false }: TestModeSidebarProps) {
  return (
    <aside
      aria-label="Test modes"
      className={cn(
        "shrink-0 border-border/50 bg-card/30",
        compact ? "border-b px-3 py-2" : "w-[180px] border-r p-3",
      )}
    >
      <TabsList
        className={cn(
          "h-auto bg-transparent p-0",
          compact ? "grid w-full grid-cols-2 gap-1" : "flex w-full flex-col gap-1",
        )}
      >
        <TabsTrigger
          value="integration"
          className={cn(
            "gap-2 text-xs",
            compact ? "px-3" : "h-9 w-full justify-start px-3",
          )}
        >
          <Workflow className="h-3.5 w-3.5" />
          End-to-End Test
        </TabsTrigger>
        <TabsTrigger
          value="loadtest"
          className={cn(
            "gap-2 text-xs",
            compact ? "px-3" : "h-9 w-full justify-start px-3",
          )}
        >
          <Zap className="h-3.5 w-3.5" />
          Load Test
        </TabsTrigger>
      </TabsList>
    </aside>
  );
}
