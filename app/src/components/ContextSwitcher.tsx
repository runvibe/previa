import { Activity, Server, WifiOff } from "lucide-react";

import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";

export function ContextSwitcher() {
  const activeContext = useOrchestratorStore((state) => state.activeContext);
  const info = useOrchestratorStore((state) => state.info);
  const apiBaseUrl = activeContext?.url ?? window.location.origin;
  const isConnected = info !== null;
  const label = info?.context ?? "Backend indisponível";

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex h-8 max-w-[260px] items-center gap-2 rounded-lg px-2.5 text-xs font-normal transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
          aria-label={`API: ${label}`}
          title={`${label} - ${apiBaseUrl}`}
        >
          <span
            className={cn(
              "h-2 w-2 shrink-0 rounded-full",
              isConnected ? "bg-success" : "bg-muted-foreground/50",
            )}
          />
          <span className="truncate">{label}</span>
          <span className="hidden max-w-[150px] truncate text-muted-foreground lg:inline">
            {apiBaseUrl}
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 border-border bg-popover p-4 text-popover-foreground shadow-xl">
        <div className="space-y-3">
          <div className="flex items-start gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
              {isConnected ? <Server className="h-4 w-4" /> : <WifiOff className="h-4 w-4" />}
            </div>
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-semibold">{label}</p>
              <p className="break-all font-mono text-xs text-muted-foreground">{apiBaseUrl}</p>
            </div>
          </div>
          {info ? (
            <div className="grid grid-cols-2 gap-2 text-xs">
              <div className="rounded-md border border-border bg-muted/40 p-2">
                <p className="text-muted-foreground">Runners</p>
                <p className="mt-1 font-semibold">{info.activeRunners}/{info.totalRunners} ativos</p>
              </div>
              <div className="rounded-md border border-border bg-muted/40 p-2">
                <p className="text-muted-foreground">Status</p>
                <p className="mt-1 inline-flex items-center gap-1 font-semibold text-success">
                  <Activity className="h-3.5 w-3.5" />
                  Conectado
                </p>
              </div>
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">
              A API base vem de VITE_PREVIA_API_BASE_URL quando definida. Caso contrario, o app usa a origin atual.
            </p>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
