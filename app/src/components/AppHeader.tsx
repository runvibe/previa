import { useEffect, useMemo, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

import { PreviaLogo } from "./PreviaLogo";
import { EventsPanel } from "./EventsPanel";
import { BarChart3, FolderOpen } from "lucide-react";
import { getOpenApiVersion } from "@/lib/api-client";
import { useOrchestratorStore } from "@/stores/useOrchestratorStore";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface AppHeaderProps {
  projectName?: string;
  pipelineName?: string;
  onBackToProjects?: () => void;
  onDashboard?: () => void;
  isDashboardActive?: boolean;
  headerActions?: React.ReactNode;
  mobileHeaderActions?: React.ReactNode;
}

function useBreadcrumbs(projectName?: string, pipelineName?: string) {
  const location = useLocation();
  const { id, pipelineId, specId } = useParams<{ id: string; pipelineId?: string; specId?: string }>();

  return useMemo(() => {
    if (!projectName || !id) return [];

    const basePath = `/projects/${id}`;
    const crumbs: { label: string; path?: string }[] = [
      { label: projectName, path: basePath },
    ];

    const pathname = location.pathname;

    if (pathname.includes("/dashboard")) {
      if (pipelineId) {
        const pipelineBasePath = `${basePath}/pipeline/${pipelineId}/integration-test`;
        crumbs.push({ label: "Pipe", path: pipelineBasePath });
        crumbs.push({ label: pipelineName || "Pipeline", path: pipelineBasePath });
      }
      crumbs.push({ label: "Dashboard" });
    } else if (pathname.includes("/specs/") && pathname.endsWith("/try-it")) {
      crumbs.push({ label: "Spec", path: specId ? `${basePath}/specs/${specId}/editor` : undefined });
      crumbs.push({ label: "Try It" });
    } else if (pathname.includes("/specs/") && pathname.endsWith("/diff")) {
      crumbs.push({ label: "Spec", path: specId ? `${basePath}/specs/${specId}/editor` : undefined });
      crumbs.push({ label: "Diff" });
    } else if (pathname.includes("/specs/") && pathname.endsWith("/editor")) {
      crumbs.push({ label: "Spec" });
      crumbs.push({ label: "Editor" });
    } else if (pathname.includes("/pipeline/")) {
      const pipelineBasePath = pipelineId ? `${basePath}/pipeline/${pipelineId}/integration-test` : undefined;
      crumbs.push({ label: "Pipe", path: pipelineBasePath });
      crumbs.push({ label: pipelineName || "Pipeline", path: pipelineBasePath });

      if (pathname.endsWith("/editor")) {
        crumbs.push({ label: "Editor" });
      } else if (pathname.endsWith("/load-test")) {
        crumbs.push({ label: "Load Test" });
      } else if (pathname.endsWith("/integration-test")) {
        crumbs.push({ label: "End-to-End Test" });
      }
    }

    return crumbs;
  }, [id, location.pathname, pipelineId, pipelineName, projectName, specId]);
}

export function AppHeader({ projectName, pipelineName, onBackToProjects, onDashboard, isDashboardActive, headerActions, mobileHeaderActions }: AppHeaderProps) {
  const navigate = useNavigate();
  const crumbs = useBreadcrumbs(projectName, pipelineName);
  const activeContextUrl = useOrchestratorStore((state) => state.activeContext?.url ?? null);
  const [apiVersion, setApiVersion] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setApiVersion(null);
    if (!activeContextUrl) return;

    getOpenApiVersion(activeContextUrl).then((version) => {
      if (!cancelled) setApiVersion(version);
    });

    return () => {
      cancelled = true;
    };
  }, [activeContextUrl]);

  const rightActions = useMemo(() => (
    <div className="flex items-center gap-1 sm:gap-2 shrink-0">
      <div className="sm:hidden">{mobileHeaderActions ?? headerActions}</div>
      <div className="hidden sm:flex items-center gap-1 sm:gap-2">
        {headerActions}
        <EventsPanel />
      </div>
    </div>
  ), [headerActions, mobileHeaderActions]);

  return (
    <header className="glass flex items-center justify-between h-14 px-3 sm:px-6 shadow-xs">
      <div className="flex items-center gap-1 min-w-0">
        <button
          onClick={onBackToProjects}
          className="flex items-center gap-1.5 min-w-0 hover:opacity-80 transition-opacity cursor-pointer p-0"
        >
          <PreviaLogo className="h-5 w-5 sm:h-6 sm:w-6 shrink-0" />
          <div className="flex flex-col items-start">
            <h1 className="text-base sm:text-lg font-bold tracking-tight whitespace-nowrap">Previa</h1>
            {apiVersion ? (
              <span className="text-[9px] leading-none text-muted-foreground font-medium tracking-wide self-end -mt-1">{apiVersion}</span>
            ) : null}
          </div>
        </button>
        {crumbs.length > 0 && (
          <Breadcrumb className="hidden sm:flex ml-2">
            <BreadcrumbList>
              {crumbs.map((crumb, i) => {
                const isLast = i === crumbs.length - 1;
                const isStackCrumb = i === 0;
                return (
                  <span key={`${crumb.label}-${crumb.path ?? i}`} className="contents">
                    <BreadcrumbSeparator>/</BreadcrumbSeparator>
                    <BreadcrumbItem>
                      {isStackCrumb && onDashboard ? (
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <button
                              type="button"
                              className="cursor-pointer truncate max-w-[120px] transition-colors hover:text-foreground"
                              aria-label={`${crumb.label} actions`}
                            >
                              {crumb.label}
                            </button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="start" className="w-44">
                            {crumb.path ? (
                              <DropdownMenuItem className="gap-2.5" onClick={() => navigate(crumb.path!)}>
                                <FolderOpen className="h-4 w-4" />
                                Open Stack
                              </DropdownMenuItem>
                            ) : null}
                            <DropdownMenuItem
                              className={isDashboardActive ? "gap-2.5 text-primary focus:text-primary" : "gap-2.5"}
                              onClick={onDashboard}
                            >
                              <BarChart3 className="h-4 w-4" />
                              Dashboard
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      ) : isLast || !crumb.path ? (
                        <BreadcrumbPage className="truncate max-w-[120px]">{crumb.label}</BreadcrumbPage>
                      ) : (
                        <BreadcrumbLink
                          className="cursor-pointer truncate max-w-[120px]"
                          onClick={() => navigate(crumb.path!)}
                        >
                          {crumb.label}
                        </BreadcrumbLink>
                      )}
                    </BreadcrumbItem>
                  </span>
                );
              })}
            </BreadcrumbList>
          </Breadcrumb>
        )}
      </div>
      {rightActions}
    </header>
  );
}
