import { useState, useCallback, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from "@/components/ui/resizable";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { useIsMobile } from "@/hooks/use-mobile";
import { getApiUrl } from "@/stores/useOrchestratorStore";

import type { TryItDrawerProps, KeyValue, TryItResponse } from "./try-it/types";
import { buildUrl, generateSampleBody } from "./try-it/helpers";
import { TryItTitleBar } from "./try-it/TryItTitleBar";
import { TryItRequestPanel } from "./try-it/TryItRequestPanel";
import { TryItResponsePanel } from "./try-it/TryItResponsePanel";
import { RoutesSidebar } from "./try-it/RoutesSidebar";

export type { TryItDrawerProps };

export function TryItDrawer({ route, servers, onClose, allRoutes, onSelectRoute }: TryItDrawerProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const [routesSheetOpen, setRoutesSheetOpen] = useState(false);

  const [selectedServer, setSelectedServer] = useState<string>(() => {
    const keys = Object.keys(servers);
    return keys.length > 0 ? keys[0] : "";
  });
  const [customBaseUrl, setCustomBaseUrl] = useState("");
  const [headers, setHeaders] = useState<KeyValue[]>([
    { key: "Content-Type", value: "application/json" },
  ]);
  const [body, setBody] = useState(() => (route ? generateSampleBody(route) : "{}"));
  const [pathParams, setPathParams] = useState<KeyValue[]>(() => {
    if (!route) return [];
    return (route.parameters ?? [])
      .filter((p) => p.in === "path")
      .map((p) => ({ key: p.name, value: "" }));
  });
  const [queryParams, setQueryParams] = useState<KeyValue[]>(() => {
    if (!route) return [];
    return (route.parameters ?? [])
      .filter((p) => p.in === "query")
      .map((p) => ({ key: p.name, value: "" }));
  });

  const [response, setResponse] = useState<TryItResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [isDark, setIsDark] = useState(document.documentElement.classList.contains("dark"));
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    const obs = new MutationObserver(() => setIsDark(document.documentElement.classList.contains("dark")));
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => obs.disconnect();
  }, []);

  // Reset state when route changes
  useEffect(() => {
    if (!route) return;
    setBody(generateSampleBody(route));
    setPathParams(
      (route.parameters ?? [])
        .filter((p) => p.in === "path")
        .map((p) => ({ key: p.name, value: "" }))
    );
    setQueryParams(
      (route.parameters ?? [])
        .filter((p) => p.in === "query")
        .map((p) => ({ key: p.name, value: "" }))
    );
    setResponse(null);
    setError(null);
    const keys = Object.keys(servers);
    if (keys.length > 0) setSelectedServer(keys[0]);
    setRoutesSheetOpen(false);
  }, [route, servers]);

  const baseUrl = selectedServer ? servers[selectedServer] : customBaseUrl;
  const resolvedUrl = route ? buildUrl(baseUrl || "http://localhost", route.path, pathParams, queryParams) : "";

  const handleSend = useCallback(async () => {
    if (!route || !baseUrl) return;
    const apiUrl = getApiUrl();
    if (!apiUrl) {
      setError(t("tryIt.orchestratorNotConfigured"));
      return;
    }

    setLoading(true);
    setError(null);
    setResponse(null);

    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    const finalUrl = buildUrl(baseUrl, route.path, pathParams, queryParams);
    const headersObj: Record<string, string> = {};
    for (const h of headers) {
      if (h.key.trim()) headersObj[h.key.trim()] = h.value;
    }

    const proxyPayload: Record<string, unknown> = {
      method: route.method,
      url: finalUrl,
      headers: headersObj,
    };

    if (route.method !== "GET" && route.method !== "HEAD" && body.trim()) {
      try {
        proxyPayload.body = JSON.parse(body);
      } catch {
        proxyPayload.body = body;
      }
    }

    const start = performance.now();

    try {
      const proxyUrl = apiUrl.replace(/\/api\/v1$/, "") + "/proxy";
      const res = await fetch(proxyUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(proxyPayload),
        signal: controller.signal,
      });

      const duration = Math.round(performance.now() - start);
      const resBody = await res.text();
      let parsedBody: unknown;
      try {
        parsedBody = JSON.parse(resBody);
      } catch {
        parsedBody = resBody;
      }

      const resHeaders: Record<string, string> = {};
      res.headers.forEach((v, k) => { resHeaders[k] = v; });

      setResponse({ status: res.status, statusText: res.statusText, headers: resHeaders, body: parsedBody, duration });
    } catch (err: any) {
      if (err.name !== "AbortError") {
        setError(err.message || t("tryIt.networkError"));
      }
    } finally {
      setLoading(false);
    }
  }, [route, baseUrl, headers, body, pathParams, queryParams]);

  if (!route) return null;

  const serverKeys = Object.keys(servers);
  const hasBody = route.method !== "GET" && route.method !== "HEAD";
  const hasRoutes = allRoutes && allRoutes.length > 0;

  const requestPanel = (
    <TryItRequestPanel
      serverKeys={serverKeys}
      servers={servers}
      selectedServer={selectedServer}
      onSelectedServerChange={setSelectedServer}
      customBaseUrl={customBaseUrl}
      onCustomBaseUrlChange={setCustomBaseUrl}
      resolvedUrl={resolvedUrl}
      loading={loading}
      baseUrl={baseUrl}
      onSend={handleSend}
      pathParams={pathParams}
      onPathParamsChange={setPathParams}
      queryParams={queryParams}
      onQueryParamsChange={setQueryParams}
      headers={headers}
      onHeadersChange={setHeaders}
      hasBody={hasBody}
      body={body}
      onBodyChange={setBody}
      isDark={isDark}
    />
  );

  const responsePanel = (
    <TryItResponsePanel response={response} loading={loading} error={error} isDark={isDark} />
  );

  return (
    <div className="h-full w-full bg-background flex flex-col">
      <TryItTitleBar
        route={route}
        response={response}
        hasRoutes={!!hasRoutes}
        onClose={onClose}
        onOpenRoutes={() => setRoutesSheetOpen(true)}
      />

      {isMobile && hasRoutes && (
        <Sheet open={routesSheetOpen} onOpenChange={setRoutesSheetOpen}>
          <SheetContent side="left" className="w-screen max-w-none p-0 sm:max-w-none">
            <SheetHeader className="px-4 py-3 border-border/30">
              <SheetTitle className="text-sm">{t("tryIt.routes")}</SheetTitle>
            </SheetHeader>
            <RoutesSidebar allRoutes={allRoutes!} route={route} onSelectRoute={onSelectRoute} />
          </SheetContent>
        </Sheet>
      )}

      <div className="flex flex-1 min-h-0 overflow-hidden">
        {!isMobile && hasRoutes && (
          <div className="w-[220px] shrink-0 border-r border-border/50 flex flex-col">
            <div className="px-3 py-2 border-border/30">
              <span className="text-xs font-medium text-muted-foreground">{t("tryIt.routes")}</span>
            </div>
            <RoutesSidebar allRoutes={allRoutes!} route={route} onSelectRoute={onSelectRoute} />
          </div>
        )}

        {isMobile ? (
          <Tabs defaultValue="request" className="flex flex-1 flex-col overflow-hidden min-h-0">
            <div className="border-border/50 shrink-0">
              <TabsList className="mx-2 my-2 grid w-auto grid-cols-2">
                <TabsTrigger value="request">{t("tryIt.request")}</TabsTrigger>
                <TabsTrigger value="response">{t("tryIt.response")}</TabsTrigger>
              </TabsList>
            </div>
            <TabsContent value="request" className="flex-1 overflow-hidden m-0 flex flex-col min-h-0">
              {requestPanel}
            </TabsContent>
            <TabsContent value="response" className="flex-1 overflow-hidden m-0 flex flex-col min-h-0">
              {responsePanel}
            </TabsContent>
          </Tabs>
        ) : (
          <ResizablePanelGroup direction="horizontal" autoSaveId="tryit-req-res">
            <ResizablePanel defaultSize={50} minSize={25} className="flex flex-col min-w-0">
              {requestPanel}
            </ResizablePanel>
            <ResizableHandle />
            <ResizablePanel defaultSize={50} minSize={25} className="flex flex-col min-w-0">
              {responsePanel}
            </ResizablePanel>
          </ResizablePanelGroup>
        )}
      </div>
    </div>
  );
}
