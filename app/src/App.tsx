import { Suspense, lazy, useEffect } from "react";
import { Toaster } from "@/components/ui/toaster";
import { Toaster as Sonner } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import ProjectsPage from "./pages/ProjectsPage";
import ProjectFlowPage from "./pages/ProjectFlowPage";
import RunnersPage from "./pages/RunnersPage";
import NotFound from "./pages/NotFound";
import { SpecSyncNotifier } from "./components/SpecSyncNotifier";
import { DotsLoader } from "./components/DotsLoader";
import { AppShell } from "./components/AppShell";
import { useOrchestratorStore } from "./stores/useOrchestratorStore";

const SpecDiffPage = lazy(() => import("./pages/SpecDiffPage"));
// Handle ?context= and ?add_context= query params on app init
function useContextQueryParam() {
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);

    // ?context= — switch to existing context by name/id, or auto-create from URL
    const contextVal = params.get("context");
    if (contextVal) {
      const store = useOrchestratorStore.getState();
      const found = store.contexts.find(
        (c) => c.name === contextVal || c.id === contextVal
      );
      if (found) {
        store.switchContext(found.id);
      } else if (/^https?:\/\//i.test(contextVal)) {
        const ctx = store.addContext(contextVal, contextVal);
        store.switchContext(ctx.id);
      }
      params.delete("context");
    }

    // ?add_context= — add a new context by URL, fetch /info for name
    const addContextUrl = params.get("add_context");
    if (addContextUrl) {
      const store = useOrchestratorStore.getState();
      const trimmed = addContextUrl.replace(/\/+$/, "");
      // Avoid duplicates
      const existing = store.contexts.find((c) => c.url === trimmed);
      if (!existing) {
        const ctx = store.addContext(trimmed, trimmed);
        store.switchContext(ctx.id);
        // Try to fetch name from /info
        const base = trimmed.replace(/\/api\/v1\/?$/, "").replace(/\/+$/, "");
        fetch(`${base}/info`, { signal: AbortSignal.timeout(6000) })
          .then((res) => (res.ok ? res.json() : null))
          .then((data) => {
            if (data?.context) {
              useOrchestratorStore.getState().updateContext(ctx.id, { name: data.context });
            }
          })
          .catch(() => { /* stays offline / unnamed */ });
      } else {
        store.switchContext(existing.id);
      }
      params.delete("add_context");
    }

    // Clean URL
    if (contextVal || addContextUrl) {
      const newSearch = params.toString();
      const newUrl = window.location.pathname + (newSearch ? `?${newSearch}` : "");
      window.history.replaceState({}, "", newUrl);
    }
  }, []);
}

const queryClient = new QueryClient();

const App = () => {
  useContextQueryParam();
  return (
  <QueryClientProvider client={queryClient}>
    <TooltipProvider>
      <Toaster />
      <Sonner />
      <BrowserRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
        <SpecSyncNotifier />
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/" element={<ProjectsPage />} />
            <Route path="/runners" element={<RunnersPage />} />
            <Route path="/projects/:id" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/pipeline/:pipelineId" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/pipeline/new/editor" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/pipeline/:pipelineId/editor" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/pipeline/:pipelineId/integration-test" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/pipeline/:pipelineId/load-test" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/pipeline/:pipelineId/dashboard" element={<ProjectFlowPage />} />
            {/* ai-create route removed — chat is always visible */}
            <Route path="/projects/:id/specs/new/editor" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/specs/new/try-it" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/specs/:specId/editor" element={<ProjectFlowPage />} />
            <Route path="/projects/:id/specs/:specId/try-it" element={<ProjectFlowPage />} />
            <Route
              path="/projects/:id/specs/:specId/diff"
              element={(
                <Suspense
                  fallback={
                    <div className="flex flex-1 items-center justify-center bg-background">
                      <DotsLoader />
                    </div>
                  }
                >
                  <SpecDiffPage />
                </Suspense>
              )}
            />
            <Route path="/projects/:id/dashboard" element={<ProjectFlowPage />} />
            {/* Legacy route redirect */}
            <Route path="/project/:id/*" element={<ProjectFlowPage />} />
          </Route>
          {/* ADD ALL CUSTOM ROUTES ABOVE THE CATCH-ALL "*" ROUTE */}
          <Route path="*" element={<NotFound />} />
        </Routes>
      </BrowserRouter>
    </TooltipProvider>
  </QueryClientProvider>
  );
};

export default App;
