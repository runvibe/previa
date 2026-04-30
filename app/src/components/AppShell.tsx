import { createContext, useCallback, useContext, useLayoutEffect, useMemo, useState } from "react";
import { CircleHelp, EllipsisVertical, Github, Server } from "lucide-react";
import { Outlet, useNavigate } from "react-router-dom";

import { AppHeader } from "@/components/AppHeader";
import { ContextSwitcher } from "@/components/ContextSwitcher";
import { EventsPanel } from "@/components/EventsPanel";
import { InstallAppButton } from "@/components/InstallAppButton";
import { OnboardingModal } from "@/components/OnboardingModal";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export interface AppHeaderConfig {
  projectName?: string;
  pipelineName?: string;
  onBackToProjects?: () => void;
  onDashboard?: () => void;
  isDashboardActive?: boolean;
  headerActions?: React.ReactNode;
  mobileHeaderActions?: React.ReactNode;
}

const AppHeaderContext = createContext<((config: AppHeaderConfig) => void) | null>(null);
const GITHUB_REPOSITORY_URL = "https://github.com/runvibe/previa";

function isSameHeaderConfig(current: AppHeaderConfig, next: AppHeaderConfig) {
  return (
    current.projectName === next.projectName &&
    current.pipelineName === next.pipelineName &&
    current.onBackToProjects === next.onBackToProjects &&
    current.onDashboard === next.onDashboard &&
    current.isDashboardActive === next.isDashboardActive &&
    current.headerActions === next.headerActions &&
    current.mobileHeaderActions === next.mobileHeaderActions
  );
}

function MobileHeaderActionRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/30 px-3 py-2">
      <span className="text-sm font-medium text-foreground">{label}</span>
      <div className="flex shrink-0 items-center">{children}</div>
    </div>
  );
}

export function AppShell() {
  const navigate = useNavigate();
  const [headerConfig, setHeaderConfigState] = useState<AppHeaderConfig>({});
  const [isOnboardingOpen, setIsOnboardingOpen] = useState(false);
  const [isMobileActionsOpen, setIsMobileActionsOpen] = useState(false);

  const setHeaderConfig = useCallback((nextConfig: AppHeaderConfig) => {
    setHeaderConfigState((currentConfig) => (
      isSameHeaderConfig(currentConfig, nextConfig) ? currentConfig : nextConfig
    ));
  }, []);

  const handleOpenOnboarding = useCallback(() => {
    setIsMobileActionsOpen(false);
    setIsOnboardingOpen(true);
  }, []);

  const shellHeaderActions = useMemo(() => (
    <>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-9 w-9 rounded-full"
        onClick={handleOpenOnboarding}
        aria-label="Abrir ajuda de instalação"
        title="Ajuda"
      >
        <CircleHelp className="h-4 w-4" />
      </Button>
      <InstallAppButton />
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-9 w-9 rounded-full"
        onClick={() => navigate("/runners")}
        aria-label="Gerenciar runners"
        title="Runners"
      >
        <Server className="h-4 w-4" />
      </Button>
      <Button asChild variant="ghost" size="icon" className="h-9 w-9 rounded-full">
        <a
          href={GITHUB_REPOSITORY_URL}
          target="_blank"
          rel="noreferrer"
          aria-label="Abrir repositório no GitHub"
          title="GitHub"
        >
          <Github className="h-4 w-4" />
        </a>
      </Button>
      {headerConfig.headerActions}
    </>
  ), [handleOpenOnboarding, headerConfig.headerActions, navigate]);

  const mobileHeaderActions = useMemo(() => (
    <DropdownMenu open={isMobileActionsOpen} onOpenChange={setIsMobileActionsOpen}>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-9 w-9 rounded-full sm:hidden"
          aria-label="Abrir ações do header"
          title="Mais ações"
        >
          <EllipsisVertical className="h-4 w-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72 sm:hidden">
        <div className="px-2 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
          Ações rápidas
        </div>
        <div className="space-y-2 p-2">
          <MobileHeaderActionRow label="Guia">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-9 w-9 rounded-full"
              onClick={handleOpenOnboarding}
              aria-label="Abrir ajuda de instalação"
              title="Ajuda"
            >
              <CircleHelp className="h-4 w-4" />
            </Button>
          </MobileHeaderActionRow>
          <MobileHeaderActionRow label="Download">
            <InstallAppButton />
          </MobileHeaderActionRow>
          <MobileHeaderActionRow label="Runners">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-9 w-9 rounded-full"
              onClick={() => {
                navigate("/runners");
                setIsMobileActionsOpen(false);
              }}
              aria-label="Gerenciar runners"
              title="Runners"
            >
              <Server className="h-4 w-4" />
            </Button>
          </MobileHeaderActionRow>
          <MobileHeaderActionRow label="GitHub">
            <Button asChild variant="ghost" size="icon" className="h-9 w-9 rounded-full">
              <a
                href={GITHUB_REPOSITORY_URL}
                target="_blank"
                rel="noreferrer"
                aria-label="Abrir repositório no GitHub"
                title="GitHub"
                onClick={() => setIsMobileActionsOpen(false)}
              >
                <Github className="h-4 w-4" />
              </a>
            </Button>
          </MobileHeaderActionRow>
          <MobileHeaderActionRow label="Eventos">
            <EventsPanel />
          </MobileHeaderActionRow>
          {headerConfig.headerActions ? (
            <MobileHeaderActionRow label="Settings">
              {headerConfig.headerActions}
            </MobileHeaderActionRow>
          ) : null}
          <MobileHeaderActionRow label="Contexto">
            <ContextSwitcher />
          </MobileHeaderActionRow>
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  ), [handleOpenOnboarding, headerConfig.headerActions, isMobileActionsOpen, navigate]);

  return (
    <AppHeaderContext.Provider value={setHeaderConfig}>
      <div className="flex h-full min-h-0 flex-col overflow-hidden bg-background">
        <AppHeader
          {...headerConfig}
          headerActions={shellHeaderActions}
          mobileHeaderActions={mobileHeaderActions}
        />
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <Outlet />
        </div>
        <OnboardingModal open={isOnboardingOpen} onOpenChange={setIsOnboardingOpen} />
        <OnboardingModal />
      </div>
    </AppHeaderContext.Provider>
  );
}

export function useAppHeader(config: AppHeaderConfig) {
  const setHeaderConfig = useContext(AppHeaderContext);

  if (!setHeaderConfig) {
    throw new Error("useAppHeader must be used within AppShell");
  }

  useLayoutEffect(() => {
    setHeaderConfig(config);
  }, [config, setHeaderConfig]);
}
