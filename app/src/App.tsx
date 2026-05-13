import { Suspense, lazy } from "react";
import { Toaster } from "@/components/ui/toaster";
import { Toaster as Sonner } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import ProjectsPage from "./pages/ProjectsPage";
import ProjectFlowPage from "./pages/ProjectFlowPage";
import RunnersPage from "./pages/RunnersPage";
import LoginPage from "./pages/LoginPage";
import AccessManagementPage from "./pages/AccessManagementPage";
import NotFound from "./pages/NotFound";
import { DotsLoader } from "./components/DotsLoader";
import { AppShell } from "./components/AppShell";
import { AuthGate } from "./components/AuthGate";

const SpecDiffPage = lazy(() => import("./pages/SpecDiffPage"));

const queryClient = new QueryClient();

const App = () => {
  return (
  <QueryClientProvider client={queryClient}>
    <TooltipProvider>
      <Toaster />
      <Sonner />
      <BrowserRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
        <Routes>
          <Route path="/login" element={<LoginPage />} />
          <Route element={<AuthGate />}>
            <Route element={<AppShell />}>
              <Route path="/" element={<ProjectsPage />} />
              <Route path="/access" element={<AccessManagementPage />} />
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
