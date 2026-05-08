# Load Results Layout Priority Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize the load-test results screen so the most important information appears first: outcome, wave adherence, response quality, generator diagnostics, then runner infrastructure.

**Architecture:** This is a frontend-only layout change in `LoadTestResultsPanel`. Keep metric calculation and backend contracts unchanged; only group existing metrics and charts into semantic sections with stable test IDs and concise translated section titles. Tests should assert DOM order so future changes cannot accidentally push the wave/RPS signal below low-level diagnostics.

**Tech Stack:** React, TypeScript, Vitest, React Testing Library, Recharts, i18next locale JSON.

---

## File Structure

- Modify `app/src/components/LoadTestResultsPanel.tsx`
  - Add small local section helpers near `MetricCard`.
  - Group existing metric cards and charts into ordered semantic sections.
  - Add `data-testid` hooks for section order tests.
- Modify `app/src/components/LoadTestResultsPanel.test.tsx`
  - Add tests that assert the visual order of the main sections.
  - Add tests that verify low-level lifecycle counters remain in diagnostics, after outcome/wave/response sections.
- Modify `app/src/i18n/locales/pt-BR.json`
  - Add concise Portuguese section labels.
- Modify `app/src/i18n/locales/en.json`
  - Add concise English section labels.

No API, model, runner, or scheduler files should change.

## Target Information Hierarchy

The rendered order inside `LoadTestResultsPanel` must become:

1. `load-results-outcome`
   - Nodes used, when available.
   - Progress, when `totalRequests > 0`.
   - Sent, success, error.
   - RPS, time, target intensity, RPS limit.
   - Curve adherence and actual missed starts.
2. `load-results-wave`
   - HTTP RPS over time.
   - Configured wave.
   - Wave lifecycle.
3. `load-results-response`
   - Avg, P95, P99.
   - Pending responses.
   - Latency over time.
   - Error samples, when available.
4. `load-results-generator`
   - Compensated scheduler starts, ready requests, scheduler lag.
   - Dispatch/request/send counters.
   - Sender queue, sender lag, HTTP send p95, observation p95.
   - Observer backlog.
5. `load-results-runner-infra`
   - Runner CPU.
   - Runner memory.
   - Runner network.

This order keeps the user's main question visible first: "the wave asked for X; did the emitted HTTP RPS follow it?"

## Task 1: Add Section Helpers and Locale Labels

**Files:**
- Modify `app/src/components/LoadTestResultsPanel.tsx`
- Modify `app/src/i18n/locales/pt-BR.json`
- Modify `app/src/i18n/locales/en.json`

- [ ] **Step 1: Add section helper components near `MetricCard`**

Insert these helpers after `MetricCard` in `app/src/components/LoadTestResultsPanel.tsx`:

```tsx
function ResultsSection({
  title,
  testId,
  children,
}: {
  title: string;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <section data-testid={testId} className="space-y-2">
      <h3 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </h3>
      <div className="space-y-2">{children}</div>
    </section>
  );
}
```

Keep this helper local to the component file. Do not create a shared UI abstraction yet, because this grouping is specific to the load-test result hierarchy.

- [ ] **Step 2: Add Portuguese labels**

Add the following keys near the existing `loadTestResults.*` entries in `app/src/i18n/locales/pt-BR.json`:

```json
"loadTestResults.sectionOutcome": "Resultado",
"loadTestResults.sectionWave": "Curva",
"loadTestResults.sectionResponse": "Resposta",
"loadTestResults.sectionGenerator": "Gerador",
"loadTestResults.sectionRunnerInfra": "Infra dos runners",
```

- [ ] **Step 3: Add English labels**

Add the following keys near the existing `loadTestResults.*` entries in `app/src/i18n/locales/en.json`:

```json
"loadTestResults.sectionOutcome": "Outcome",
"loadTestResults.sectionWave": "Wave",
"loadTestResults.sectionResponse": "Response",
"loadTestResults.sectionGenerator": "Generator",
"loadTestResults.sectionRunnerInfra": "Runner infrastructure",
```

- [ ] **Step 4: Run a focused type/build check for the app**

Run:

```bash
npm --prefix app run build
```

Expected: build succeeds. Existing Vite warnings are acceptable if the command exits with code 0.

## Task 2: Move Outcome Metrics to the First Section

**Files:**
- Modify `app/src/components/LoadTestResultsPanel.tsx`
- Modify `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Wrap the top-level outcome elements**

In `LoadTestResultsPanel`, replace the first loose blocks inside:

```tsx
return (
  <div className="space-y-4 p-1">
```

with a `ResultsSection` that contains, in order:

- the existing `nodesInfo` block, unchanged;
- the existing `totalRequests > 0` progress block, unchanged;
- the sent/success/error metric grid;
- the RPS/time/target metric grid;
- the adherence/actual missed starts metric grid.

Use this wrapper and keep the existing inner blocks intact when moving them. The wrapper starts before the `nodesInfo` conditional and closes after the adherence/actual missed starts grid:

```tsx
return (
  <div className="space-y-4 p-1">
    <ResultsSection title={t("loadTestResults.sectionOutcome")} testId="load-results-outcome">
      <div className="grid grid-cols-3 gap-2">
        <MetricCard icon={Zap} label={t("loadTestResults.sent")} value={metrics.totalSent} />
        <MetricCard icon={CheckCircle2} label={t("loadTestResults.success")} value={metrics.totalSuccess} color="text-success" />
        <MetricCard icon={AlertCircle} label={t("loadTestResults.error")} value={metrics.totalError} color="text-destructive" />
      </div>

      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <MetricCard icon={TrendingUp} label="RPS" value={metrics.rps} color="text-primary" />
        <MetricCard icon={Clock} label={t("loadTestResults.elapsedLabel", "Time")} value={`${Math.round(metrics.elapsedMs / 1000)}s`} />
        {typeof metrics.targetIntensity === "number" && (
          <MetricCard icon={Gauge} label={t("loadTestResults.targetIntensity")} value={`${metrics.targetIntensity.toFixed(1)}%`} color="text-primary" />
        )}
        {typeof metrics.targetRpsLimit === "number" && (
          <MetricCard icon={Gauge} label={t("loadTestResults.targetRpsLimit")} value={metrics.targetRpsLimit.toFixed(1)} color="text-primary" />
        )}
      </div>

      {(typeof metrics.curveAdherence === "number" || waveDiagnostics.actualMissedStarts > 0) && (
        <div className="grid grid-cols-2 gap-2">
          {typeof metrics.curveAdherence === "number" && (
            <MetricCard
              icon={Activity}
              label={t("loadTestResults.curveAdherence")}
              value={`${parseFloat(metrics.curveAdherence.toFixed(1))}%`}
              color="text-success"
            />
          )}
          <MetricCard
            icon={waveDiagnostics.hasActualWaveLoss ? AlertTriangle : Activity}
            label={t("loadTestResults.actualMissedStarts")}
            value={waveDiagnostics.hasActualWaveLoss ? waveDiagnostics.actualMissedStarts : 0}
            color={waveDiagnostics.hasActualWaveLoss ? "text-warning" : "text-success"}
          />
        </div>
      )}
    </ResultsSection>
```

Move only existing cards. Do not change values, calculations, colors, or labels.

- [ ] **Step 2: Remove duplicate outcome cards from the old locations**

Delete the previous loose blocks that rendered sent/success/error, RPS/time, target intensity/RPS limit, and curve adherence/actual missed starts. Those exact cards now live inside `load-results-outcome`.

Keep latency cards for Task 4. Keep `metrics.inFlight`, `metrics.schedulerLaggedStarts`, and `metrics.readyRequests` for later sections.

- [ ] **Step 3: Add a DOM order helper to tests**

Add this helper near `expectMetricValue` in `app/src/components/LoadTestResultsPanel.test.tsx`:

```tsx
function expectBefore(first: HTMLElement, second: HTMLElement) {
  expect(Boolean(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING)).toBe(true);
}
```

- [ ] **Step 4: Add a failing section-order test**

Add this test inside the existing `describe("LoadTestResultsPanel", () => {` block:

```tsx
it("shows outcome before wave, response, generator diagnostics, and runner infrastructure", () => {
  const metrics = {
    totalSent: 1200,
    totalSuccess: 1198,
    totalError: 2,
    rps: 80,
    avgLatency: 42,
    p95: 90,
    p99: 110,
    elapsedMs: 15_000,
    errors: [],
    targetIntensity: 50,
    targetRpsLimit: 100,
    curveAdherence: 99.7,
    schedulerLaggedStarts: 3,
    readyRequests: 0,
    dispatchSubmitted: 1200,
    dispatchStarted: 1200,
    httpStarted: 1200,
    httpSendReturned: 1200,
    responseBodyCompleted: 1198,
    senderQueueDepth: 0,
    senderStartLagP95Ms: 1,
    httpSendDurationP95Ms: 1,
    responseObservationDurationP95Ms: 2,
    schedulerLagMs: 12,
    inFlight: 2,
    latencyHistory: [
      { index: 1, latency: 40 },
      { index: 2, latency: 42 },
    ],
    rpsHistory: [
      { elapsedMs: 1000, rps: 50, totalSent: 50 },
      { elapsedMs: 2000, rps: 80, totalSent: 130 },
    ],
    lifecycleHistory: [
      { elapsedMs: 1000, scheduledStarts: 50, httpStarted: 50 },
      { elapsedMs: 2000, scheduledStarts: 80, httpStarted: 80 },
    ],
    runnerResourceHistory: [
      {
        node: "runner-a",
        elapsedMs: 1000,
        cpuUsagePercent: 10,
        memoryMb: 128,
        networkRxKb: 1,
        networkTxKb: 2,
        networkTotalKb: 3,
      },
    ],
  };

  render(
    <LoadTestResultsPanel
      metrics={metrics}
      state="completed"
      totalRequests={0}
      config={{
        load: {
          durationMs: 2000,
          interpolation: "linear",
          points: [
            { atMs: 0, intensity: 10 },
            { atMs: 2000, intensity: 80 },
          ],
        },
      }}
    />,
  );

  const outcome = screen.getByTestId("load-results-outcome");
  const wave = screen.getByTestId("load-results-wave");
  const response = screen.getByTestId("load-results-response");
  const generator = screen.getByTestId("load-results-generator");
  const runnerInfra = screen.getByTestId("load-results-runner-infra");

  expectBefore(outcome, wave);
  expectBefore(wave, response);
  expectBefore(response, generator);
  expectBefore(generator, runnerInfra);
});
```

- [ ] **Step 5: Run the new test and verify it fails before the remaining layout tasks**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel.test.tsx
```

Expected before completing Tasks 3-5: FAIL because `load-results-wave`, `load-results-response`, `load-results-generator`, or `load-results-runner-infra` are not all implemented yet.

## Task 3: Group the Wave Comparison Section

**Files:**
- Modify `app/src/components/LoadTestResultsPanel.tsx`

- [ ] **Step 1: Wrap RPS, configured wave, and lifecycle charts in `load-results-wave`**

Move the existing RPS chart block above the configured-wave block, then wrap all three wave-related chart blocks:

```tsx
{(rpsChartData.length > 1 || (waveConfig && waveChartData.length > 1) || lifecycleChartData.length > 0) && (
  <ResultsSection title={t("loadTestResults.sectionWave")} testId="load-results-wave">
    {rpsChartData.length > 1 && <div data-testid="rps-over-time-chart" className="glass rounded-lg p-3 space-y-2">move the current RPS chart JSX here</div>}
    {waveConfig && waveChartData.length > 1 && <div data-testid="configured-wave-chart" className="glass rounded-lg p-3 space-y-2">move the current configured wave chart JSX here</div>}
    {lifecycleChartData.length > 0 && <div data-testid="wave-lifecycle-chart" className="glass rounded-lg p-3 space-y-2">move the current lifecycle chart JSX here</div>}
  </ResultsSection>
)}
```

The inner chart implementations remain the same. Add only `data-testid="rps-over-time-chart"` to the RPS chart container.

- [ ] **Step 2: Keep RPS as the first chart in the section**

Inside `load-results-wave`, order must be:

```tsx
{rpsChartData.length > 1 && <div data-testid="rps-over-time-chart">current RPS chart</div>}
{waveConfig && waveChartData.length > 1 && <div data-testid="configured-wave-chart">current configured wave chart</div>}
{lifecycleChartData.length > 0 && <div data-testid="wave-lifecycle-chart">current lifecycle chart</div>}
```

RPS is first because it is the observed output. Configured wave is second because it is the expected shape. Lifecycle is third because it explains why the first two differ.

## Task 4: Group Response Quality Separately

**Files:**
- Modify `app/src/components/LoadTestResultsPanel.tsx`

- [ ] **Step 1: Create the response section after wave**

Place this after `load-results-wave`:

```tsx
{(metrics.avgLatency > 0 ||
  typeof metrics.inFlight === "number" ||
  latencyChartData.length > 1 ||
  (metrics.errors && metrics.errors.length > 0)) && (
  <ResultsSection title={t("loadTestResults.sectionResponse")} testId="load-results-response">
    {metrics.avgLatency > 0 && (
      <div className="grid grid-cols-3 gap-2">
        <MetricCard icon={Clock} label={t("loadTestResults.avg")} value={`${metrics.avgLatency}ms`} />
        <MetricCard icon={Activity} label="P95" value={`${metrics.p95}ms`} />
        <MetricCard icon={Activity} label="P99" value={`${metrics.p99}ms`} />
      </div>
    )}

    {typeof metrics.inFlight === "number" && (
      <div className="grid grid-cols-2 gap-2">
        <MetricCard icon={Activity} label={t("loadTestResults.inFlight")} value={metrics.inFlight} />
      </div>
    )}

    {latencyChartData.length > 1 && (
      <div className="glass rounded-lg p-3 space-y-2">move the current latency chart JSX here</div>
    )}

    {metrics.errors && metrics.errors.length > 0 && (
      <div className="glass rounded-lg p-3 space-y-2">move the current error samples JSX here</div>
    )}
  </ResultsSection>
)}
```

Move the existing latency chart and error samples blocks here without changing their internals.

- [ ] **Step 2: Remove latency cards from the old mixed metric grid**

The old mixed grid should no longer include:

```tsx
{metrics.avgLatency > 0 && (
  <>
    <MetricCard icon={Clock} label={t("loadTestResults.avg")} value={`${metrics.avgLatency}ms`} />
    <MetricCard icon={Activity} label="P95" value={`${metrics.p95}ms`} />
    <MetricCard icon={Activity} label="P99" value={`${metrics.p99}ms`} />
  </>
)}
```

The latency metrics now belong only to `load-results-response`.

## Task 5: Group Generator Diagnostics

**Files:**
- Modify `app/src/components/LoadTestResultsPanel.tsx`

- [ ] **Step 1: Add derived booleans for readability**

Before `return`, add these booleans:

```tsx
const hasGeneratorSummary =
  typeof metrics.schedulerLaggedStarts === "number" ||
  typeof metrics.readyRequests === "number" ||
  typeof metrics.schedulerLagMs === "number";

const hasGeneratorDetails =
  typeof metrics.dispatchSubmitted === "number" ||
  typeof metrics.dispatchStarted === "number" ||
  typeof metrics.httpSendReturned === "number" ||
  typeof metrics.responseBodyCompleted === "number" ||
  typeof metrics.dependencyLimitedStarts === "number" ||
  typeof metrics.dispatcherLaggedStarts === "number" ||
  typeof metrics.runtimeLaggedStarts === "number" ||
  typeof metrics.senderLaggedStarts === "number" ||
  typeof metrics.senderQueueDepth === "number" ||
  typeof metrics.senderStartLagP95Ms === "number" ||
  typeof metrics.httpSendDurationP95Ms === "number" ||
  typeof metrics.responseObservationDurationP95Ms === "number" ||
  typeof metrics.slotEnqueued === "number" ||
  typeof metrics.requestPrepared === "number" ||
  typeof metrics.requestEnqueued === "number" ||
  typeof metrics.sendTaskSpawned === "number" ||
  typeof metrics.sendStarted === "number" ||
  typeof metrics.httpStarted === "number" ||
  typeof metrics.outstandingRequests === "number";
```

- [ ] **Step 2: Wrap diagnostics in `load-results-generator`**

Replace the current diagnostics grids with:

```tsx
{(hasGeneratorSummary || hasGeneratorDetails) && (
  <ResultsSection title={t("loadTestResults.sectionGenerator")} testId="load-results-generator">
    {hasGeneratorSummary && (
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {typeof metrics.schedulerLaggedStarts === "number" && (
          <MetricCard
            icon={Clock}
            label={t("loadTestResults.compensatedSchedulerStarts")}
            value={metrics.schedulerLaggedStarts}
            color={waveDiagnostics.schedulerDelayWasCompensated ? "text-success" : "text-warning"}
          />
        )}
        {typeof metrics.readyRequests === "number" && (
          <MetricCard
            icon={ListChecks}
            label={t("loadTestResults.readyRequests")}
            value={metrics.readyRequests}
            color="text-primary"
          />
        )}
        {typeof metrics.schedulerLagMs === "number" && (
          <MetricCard
            icon={Clock}
            label={t("loadTestResults.schedulerLagMs")}
            value={`${metrics.schedulerLagMs}ms`}
            color="text-warning"
          />
        )}
      </div>
    )}

    {hasGeneratorDetails && (
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4 xl:grid-cols-5">move the current detailed diagnostic cards here</div>
    )}
  </ResultsSection>
)}
```

Move the existing detailed diagnostic cards into the second grid. Do not duplicate `schedulerLagMs`; it should move to the summary grid above.

- [ ] **Step 3: Preserve existing diagnostic assertions**

The existing test named `shows wave dispatch adherence metrics when available` should still pass without changing metric expectations. If any query becomes ambiguous because of duplicated labels, remove the duplicate card rather than weakening the test.

## Task 6: Group Runner Infrastructure

**Files:**
- Modify `app/src/components/LoadTestResultsPanel.tsx`

- [ ] **Step 1: Wrap runner resource charts**

Wrap CPU, memory, and network charts:

```tsx
{runnerNames.length > 0 &&
  (cpuChartData.length > 0 || memoryChartData.length > 0 || networkChartData.length > 0) && (
    <ResultsSection title={t("loadTestResults.sectionRunnerInfra")} testId="load-results-runner-infra">
      {runnerNames.length > 0 && cpuChartData.length > 0 && (
        <div className="glass rounded-lg p-3 space-y-2">move the current CPU chart JSX here</div>
      )}

      {runnerNames.length > 0 && memoryChartData.length > 0 && (
        <div className="glass rounded-lg p-3 space-y-2">move the current memory chart JSX here</div>
      )}

      {runnerNames.length > 0 && networkChartData.length > 0 && (
        <div className="glass rounded-lg p-3 space-y-2">move the current network chart JSX here</div>
      )}
    </ResultsSection>
  )}
```

Keep chart internals unchanged.

- [ ] **Step 2: Ensure no empty section renders**

Run a test render with metrics that have no runner history:

```tsx
render(<LoadTestResultsPanel metrics={metricsWithoutRunnerHistory} state="completed" totalRequests={0} />);
expect(screen.queryByTestId("load-results-runner-infra")).not.toBeInTheDocument();
```

Add this assertion to an existing test if there is already a minimal metrics test; otherwise create a short new one.

## Task 7: Strengthen Layout Tests

**Files:**
- Modify `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Assert RPS is before configured wave and lifecycle**

Add this test:

```tsx
it("prioritizes observed RPS before configured wave and lifecycle diagnostics", () => {
  const metrics = {
    totalSent: 100,
    totalSuccess: 100,
    totalError: 0,
    rps: 50,
    avgLatency: 0,
    p95: 0,
    p99: 0,
    elapsedMs: 2000,
    errors: [],
    rpsHistory: [
      { elapsedMs: 1000, rps: 20, totalSent: 20 },
      { elapsedMs: 2000, rps: 50, totalSent: 70 },
    ],
    lifecycleHistory: [
      { elapsedMs: 1000, scheduledStarts: 20, httpStarted: 20 },
      { elapsedMs: 2000, scheduledStarts: 50, httpStarted: 50 },
    ],
  };

  render(
    <LoadTestResultsPanel
      metrics={metrics}
      state="completed"
      totalRequests={0}
      config={{
        load: {
          durationMs: 2000,
          interpolation: "linear",
          points: [
            { atMs: 0, intensity: 10 },
            { atMs: 2000, intensity: 80 },
          ],
        },
      }}
    />,
  );

  expectBefore(screen.getByTestId("rps-over-time-chart"), screen.getByTestId("configured-wave-chart"));
  expectBefore(screen.getByTestId("configured-wave-chart"), screen.getByTestId("wave-lifecycle-chart"));
});
```

- [ ] **Step 2: Assert low-level counters are below response quality**

Add this assertion to the section-order test:

```tsx
expect(screen.getByText("loadTestResults.httpStarted")).toBeInTheDocument();
expectBefore(screen.getByTestId("load-results-response"), screen.getByText("loadTestResults.httpStarted").closest("div")!);
```

This confirms implementation counters do not appear before user-facing response quality.

- [ ] **Step 3: Run the focused component test**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel.test.tsx
```

Expected: PASS.

## Task 8: Visual Verification in the Local App

**Files:**
- No source edits unless the visual check exposes a bug.

- [ ] **Step 1: Build frontend**

Run:

```bash
npm --prefix app run build
```

Expected: PASS.

- [ ] **Step 2: Run Rust release build**

Run:

```bash
cargo build --release
```

Expected: PASS.

- [ ] **Step 3: Restart local services**

Run:

```bash
scripts/start-local-load-target-stack.sh
```

Expected: main app listens on `http://127.0.0.1:5610` and runners become healthy.

- [ ] **Step 4: Open the load-test page**

Open:

```text
http://127.0.0.1:5610/projects/019e02b8-644f-7c60-863b-b22c40eb5c26/pipeline/019e02b8-64aa-7a22-81d7-474e7f1d5cbc/load-test
```

Expected visual order:

```text
Resultado
Curva
Resposta
Gerador
Infra dos runners
```

Expected chart priority:

```text
HTTP RPS ao longo do tempo
Onda configurada
Ciclo da wave
Latência ao longo do tempo
Runner CPU
Runner memory
Runner network
```

## Task 9: Commit and Push

**Files:**
- All files modified in previous tasks.

- [ ] **Step 1: Review git diff**

Run:

```bash
git diff -- app/src/components/LoadTestResultsPanel.tsx app/src/components/LoadTestResultsPanel.test.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json
```

Expected: diff only reorders frontend UI sections, adds section labels, and adds tests. No backend or runner algorithm changes.

- [ ] **Step 2: Stage changes**

Run:

```bash
git add app/src/components/LoadTestResultsPanel.tsx app/src/components/LoadTestResultsPanel.test.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json
```

- [ ] **Step 3: Commit**

Run:

```bash
git commit -m "Improve load results layout priority"
```

- [ ] **Step 4: Push**

Run:

```bash
git push origin codex/wave-load-test
```

## Self-Review

- Spec coverage: the plan prioritizes the result, the wave comparison, response quality, generator diagnostics, and runner infrastructure in that order.
- No backend scope: the plan does not alter runner dispatch, metrics collection, API contracts, or database models.
- Test coverage: the plan adds section order tests and chart order tests so the intended hierarchy is enforceable.
- UI constraints: section headers are concise domain labels; the plan does not add explanatory instructional text to the application.
