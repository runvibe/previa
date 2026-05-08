# Wave Diagnostics UI Semantics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the load-test results UI distinguish compensated scheduler delay from real wave loss, so successful open-loop runs do not look failed when all planned HTTP starts were emitted.

**Architecture:** Keep the runner and main execution algorithm unchanged. Add a small UI-only diagnostics helper that derives real wave loss from lifecycle buckets, then update labels/cards/tests to present `schedulerLaggedStarts` as compensated timing pressure and actual loss as `planned - httpStarted` when positive.

**Tech Stack:** React, TypeScript, Vitest, Testing Library, existing Recharts result panel.

---

## File Structure

- Modify: `app/src/components/LoadTestResultsPanel.tsx`
  - Use derived diagnostics to render clearer cards.
  - Rename warning cards without changing backend fields.
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`
  - Add coverage for compensated scheduler lag with zero real loss.
  - Add coverage for actual missed HTTP starts when `planned > httpStarted`.
- Modify: `app/src/i18n/locales/pt-BR.json`
  - Replace misleading Portuguese labels.
  - Add labels for actual wave loss.
- Modify: `app/src/i18n/locales/en.json`
  - Same semantic update in English.
- Create: `app/src/lib/wave-diagnostics.ts`
  - Pure helper for UI-derived metrics.
- Create: `app/src/lib/wave-diagnostics.test.ts`
  - Unit tests for derived diagnostics.

## Terms

- **Compensated scheduler delay:** the scheduler woke late, but later emitted the planned amount. This maps to current `schedulerLagMs` and `schedulerLaggedStarts`.
- **Actual wave loss:** the configured wave planned more HTTP starts than the runner actually started. This should be derived from lifecycle buckets as `sum(max(planned - httpStarted, 0))`, ignoring the final wave-end bucket where `planned` is intentionally `0`.
- **Wave surplus:** the runner started more HTTP requests than planned in a bucket, usually because it compensated a previous delayed bucket. This is diagnostic only and should not be shown as failure.

## Task 1: Add Derived Diagnostics Helper

**Files:**
- Create: `app/src/lib/wave-diagnostics.ts`
- Create: `app/src/lib/wave-diagnostics.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `app/src/lib/wave-diagnostics.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { deriveWaveDiagnostics } from "@/lib/wave-diagnostics";
import type { LoadTestMetrics } from "@/types/load-test";

const baseMetrics: LoadTestMetrics = {
  totalSent: 0,
  totalSuccess: 0,
  totalError: 0,
  avgLatency: 0,
  p95: 0,
  p99: 0,
  rps: 0,
  latencyHistory: [],
  rpsHistory: [],
  errors: [],
  startTime: 0,
  elapsedMs: 0,
};

describe("deriveWaveDiagnostics", () => {
  it("treats compensated scheduler delay as no actual wave loss", () => {
    const diagnostics = deriveWaveDiagnostics({
      ...baseMetrics,
      schedulerLagMs: 3107,
      schedulerLaggedStarts: 705,
      lifecycleBuckets: [
        { elapsedMs: 0, planned: 300, httpStarted: 300 },
        { elapsedMs: 1_000, planned: 300, httpStarted: 300 },
        { elapsedMs: 2_000, planned: 303, httpStarted: 305 },
        { elapsedMs: 3_000, planned: 303, httpStarted: 301 },
        { elapsedMs: 120_000, planned: 0, httpStarted: 7 },
      ],
    });

    expect(diagnostics.actualMissedStarts).toBe(2);
    expect(diagnostics.surplusStarts).toBe(9);
    expect(diagnostics.schedulerDelayWasCompensated).toBe(true);
    expect(diagnostics.hasActualWaveLoss).toBe(false);
  });

  it("reports actual wave loss when planned HTTP starts are not emitted", () => {
    const diagnostics = deriveWaveDiagnostics({
      ...baseMetrics,
      schedulerLagMs: 500,
      schedulerLaggedStarts: 30,
      lifecycleBuckets: [
        { elapsedMs: 0, planned: 100, httpStarted: 100 },
        { elapsedMs: 1_000, planned: 120, httpStarted: 90 },
        { elapsedMs: 2_000, planned: 130, httpStarted: 120 },
      ],
    });

    expect(diagnostics.actualMissedStarts).toBe(40);
    expect(diagnostics.surplusStarts).toBe(0);
    expect(diagnostics.schedulerDelayWasCompensated).toBe(false);
    expect(diagnostics.hasActualWaveLoss).toBe(true);
  });

  it("falls back to cumulative planned and http started counters", () => {
    const diagnostics = deriveWaveDiagnostics({
      ...baseMetrics,
      dispatchSubmitted: 1_000,
      httpStarted: 990,
    });

    expect(diagnostics.actualMissedStarts).toBe(10);
    expect(diagnostics.hasActualWaveLoss).toBe(true);
  });
});
```

- [ ] **Step 2: Run the new tests and verify failure**

Run:

```bash
npm --prefix app test -- wave-diagnostics.test.ts
```

Expected: FAIL because `app/src/lib/wave-diagnostics.ts` does not exist.

- [ ] **Step 3: Implement the helper**

Create `app/src/lib/wave-diagnostics.ts`:

```ts
import type { LoadLifecycleBucket, LoadTestMetrics } from "@/types/load-test";

export interface WaveDiagnostics {
  plannedStarts: number;
  actualHttpStarts: number;
  actualMissedStarts: number;
  surplusStarts: number;
  hasActualWaveLoss: boolean;
  schedulerDelayWasCompensated: boolean;
}

function isFinalZeroPlanBucket(bucket: LoadLifecycleBucket): boolean {
  return (bucket.planned ?? 0) === 0 && (bucket.httpStarted ?? 0) > 0;
}

function fromLifecycleBuckets(metrics: LoadTestMetrics): WaveDiagnostics | null {
  const buckets = metrics.lifecycleBuckets ?? [];
  if (buckets.length === 0) return null;

  let plannedStarts = 0;
  let actualHttpStarts = 0;
  let actualMissedStarts = 0;
  let surplusStarts = 0;

  for (const bucket of buckets) {
    if (isFinalZeroPlanBucket(bucket)) {
      actualHttpStarts += bucket.httpStarted ?? 0;
      surplusStarts += bucket.httpStarted ?? 0;
      continue;
    }

    const planned = bucket.planned ?? 0;
    const actual = bucket.httpStarted ?? 0;
    plannedStarts += planned;
    actualHttpStarts += actual;

    if (planned > actual) {
      actualMissedStarts += planned - actual;
    } else {
      surplusStarts += actual - planned;
    }
  }

  const hasActualWaveLoss = actualMissedStarts > surplusStarts;

  return {
    plannedStarts,
    actualHttpStarts,
    actualMissedStarts,
    surplusStarts,
    hasActualWaveLoss,
    schedulerDelayWasCompensated:
      typeof metrics.schedulerLaggedStarts === "number" &&
      metrics.schedulerLaggedStarts > 0 &&
      !hasActualWaveLoss,
  };
}

function fromCumulativeCounters(metrics: LoadTestMetrics): WaveDiagnostics {
  const plannedStarts = metrics.dispatchSubmitted ?? metrics.scheduledStarts ?? 0;
  const actualHttpStarts = metrics.httpStarted ?? metrics.totalStarted ?? 0;
  const actualMissedStarts = Math.max(0, plannedStarts - actualHttpStarts);
  const surplusStarts = Math.max(0, actualHttpStarts - plannedStarts);
  const hasActualWaveLoss = actualMissedStarts > 0;

  return {
    plannedStarts,
    actualHttpStarts,
    actualMissedStarts,
    surplusStarts,
    hasActualWaveLoss,
    schedulerDelayWasCompensated:
      typeof metrics.schedulerLaggedStarts === "number" &&
      metrics.schedulerLaggedStarts > 0 &&
      !hasActualWaveLoss,
  };
}

export function deriveWaveDiagnostics(metrics: LoadTestMetrics): WaveDiagnostics {
  return fromLifecycleBuckets(metrics) ?? fromCumulativeCounters(metrics);
}
```

- [ ] **Step 4: Run helper tests and verify pass**

Run:

```bash
npm --prefix app test -- wave-diagnostics.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/wave-diagnostics.ts app/src/lib/wave-diagnostics.test.ts
git commit -m "Add wave diagnostics derivation"
```

## Task 2: Update Result Cards Semantics

**Files:**
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Write failing component expectations**

Update the existing `"shows wave dispatch adherence metrics when available"` test in `app/src/components/LoadTestResultsPanel.test.tsx`.

Replace the assertions for `missedStarts` and `schedulerLaggedStarts` with:

```ts
expect(screen.getByText("loadTestResults.compensatedSchedulerStarts")).toBeInTheDocument();
expect(screen.getByText("12")).toBeInTheDocument();
expect(screen.getByText("loadTestResults.actualMissedStarts")).toBeInTheDocument();
expect(screen.getByText("1")).toBeInTheDocument();
```

In the metrics payload for that test, add lifecycle buckets:

```ts
lifecycleBuckets: [
  { elapsedMs: 0, planned: 100, httpStarted: 99 },
  { elapsedMs: 1_000, planned: 100, httpStarted: 100 },
],
```

Add a new test below it:

```ts
it("shows compensated scheduler delay without marking the wave as actually lost", () => {
  render(
    <LoadTestResultsPanel
      metrics={{
        ...emptyMetrics,
        curveAdherence: 99.9,
        schedulerLagMs: 3107,
        schedulerLaggedStarts: 705,
        lifecycleBuckets: [
          { elapsedMs: 0, planned: 300, httpStarted: 300 },
          { elapsedMs: 1_000, planned: 300, httpStarted: 300 },
          { elapsedMs: 2_000, planned: 303, httpStarted: 305 },
          { elapsedMs: 3_000, planned: 303, httpStarted: 301 },
          { elapsedMs: 120_000, planned: 0, httpStarted: 7 },
        ],
      }}
      state="completed"
      totalRequests={0}
    />,
  );

  expect(screen.getByText("loadTestResults.compensatedSchedulerStarts")).toBeInTheDocument();
  expect(screen.getByText("705")).toBeInTheDocument();
  expect(screen.getByText("loadTestResults.actualMissedStarts")).toBeInTheDocument();
  expect(screen.getByText("0")).toBeInTheDocument();
});
```

- [ ] **Step 2: Run component test and verify failure**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel.test.tsx
```

Expected: FAIL because the component still renders old labels.

- [ ] **Step 3: Use derived diagnostics in the component**

In `app/src/components/LoadTestResultsPanel.tsx`, add the import:

```ts
import { deriveWaveDiagnostics } from "@/lib/wave-diagnostics";
```

Inside `LoadTestResultsPanel`, after existing memo/data declarations, add:

```ts
const waveDiagnostics = useMemo(() => deriveWaveDiagnostics(metrics), [metrics]);
```

Replace the current card block condition:

```tsx
{(typeof metrics.curveAdherence === "number" ||
  typeof metrics.missedStarts === "number" ||
  typeof metrics.readyRequests === "number") && (
```

with:

```tsx
{(typeof metrics.curveAdherence === "number" ||
  typeof metrics.schedulerLaggedStarts === "number" ||
  waveDiagnostics.actualMissedStarts > 0 ||
  typeof metrics.readyRequests === "number") && (
```

Replace the `metrics.missedStarts` card in that block with:

```tsx
<MetricCard
  icon={waveDiagnostics.hasActualWaveLoss ? AlertTriangle : Activity}
  label={t("loadTestResults.actualMissedStarts")}
  value={waveDiagnostics.hasActualWaveLoss ? waveDiagnostics.actualMissedStarts : 0}
  color={waveDiagnostics.hasActualWaveLoss ? "text-warning" : "text-success"}
/>
```

Add this card after the actual missed starts card:

```tsx
{typeof metrics.schedulerLaggedStarts === "number" && (
  <MetricCard
    icon={Clock}
    label={t("loadTestResults.compensatedSchedulerStarts")}
    value={metrics.schedulerLaggedStarts}
    color={waveDiagnostics.schedulerDelayWasCompensated ? "text-success" : "text-warning"}
  />
)}
```

In the later detailed diagnostics grid, remove the old `schedulerLaggedStarts` card or change its label to the same `compensatedSchedulerStarts` key. Do not render two separate cards with the same count.

- [ ] **Step 4: Run component tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel.test.tsx
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/components/LoadTestResultsPanel.tsx app/src/components/LoadTestResultsPanel.test.tsx
git commit -m "Clarify wave diagnostics cards"
```

## Task 3: Update Translations

**Files:**
- Modify: `app/src/i18n/locales/pt-BR.json`
- Modify: `app/src/i18n/locales/en.json`
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Update Portuguese labels**

In `app/src/i18n/locales/pt-BR.json`, replace:

```json
"loadTestResults.missedStarts": "Starts perdidos",
"loadTestResults.schedulerLaggedStarts": "Disparos perdidos por atraso",
```

with:

```json
"loadTestResults.missedStarts": "Starts não emitidos",
"loadTestResults.actualMissedStarts": "Disparos realmente perdidos",
"loadTestResults.compensatedSchedulerStarts": "Atrasos compensados",
"loadTestResults.schedulerLaggedStarts": "Atrasos do agendador",
```

- [ ] **Step 2: Update English labels**

In `app/src/i18n/locales/en.json`, replace:

```json
"loadTestResults.missedStarts": "Missed starts",
"loadTestResults.schedulerLaggedStarts": "Starts missed by lag",
```

with:

```json
"loadTestResults.missedStarts": "Unemitted starts",
"loadTestResults.actualMissedStarts": "Actual missed starts",
"loadTestResults.compensatedSchedulerStarts": "Compensated delays",
"loadTestResults.schedulerLaggedStarts": "Scheduler-delayed starts",
```

- [ ] **Step 3: Add translation regression assertions**

In `app/src/components/LoadTestResultsPanel.test.tsx`, keep the tests asserting i18n keys, because this suite mocks translations as keys. No extra test code is required beyond Task 2 once the new keys are rendered.

- [ ] **Step 4: Run component tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel.test.tsx
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json app/src/components/LoadTestResultsPanel.test.tsx
git commit -m "Rename wave diagnostic labels"
```

## Task 4: Verify Build and Runtime

**Files:**
- No source changes expected.

- [ ] **Step 1: Run focused frontend tests**

Run:

```bash
npm --prefix app test -- wave-diagnostics.test.ts LoadTestResultsPanel.test.tsx
```

Expected: PASS.

- [ ] **Step 2: Build frontend**

Run:

```bash
npm --prefix app run build
```

Expected: PASS. Existing Vite chunk warnings are acceptable if unchanged.

- [ ] **Step 3: Run release build**

Run:

```bash
cargo build --release
```

Expected: PASS.

- [ ] **Step 4: Restart local stack**

Run:

```bash
scripts/start-local-load-target-stack.sh
```

Expected output includes:

```text
Main:    http://127.0.0.1:5610
Target:  http://127.0.0.1:5620
Runners: http://127.0.0.1:5611 http://127.0.0.1:5612 http://127.0.0.1:5613
```

- [ ] **Step 5: Verify UI manually**

Open the printed load-test URL and inspect the latest run. Expected:

- `Disparos realmente perdidos` shows `0` for compensated runs where total planned equals HTTP started.
- `Atrasos compensados` shows the scheduler-delayed count.
- `Aderência` remains visible.
- The lifecycle chart still shows planned and HTTP started lines.

- [ ] **Step 6: Final commit and push**

If any build output or final files remain unstaged:

```bash
git status --short
git add app/src/lib/wave-diagnostics.ts app/src/lib/wave-diagnostics.test.ts app/src/components/LoadTestResultsPanel.tsx app/src/components/LoadTestResultsPanel.test.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json
git commit -m "Clarify wave load diagnostics UI"
git push
```

## Self-Review

- Spec coverage: the plan changes only UI semantics and derived diagnostics, preserving the current open-loop algorithm.
- Actual loss is derived independently from `missedStarts`, using lifecycle buckets first and cumulative counters as fallback.
- The plan removes the misleading user-facing idea that scheduler lag always means dropped requests.
- No backend contract changes are required.
- Verification includes focused tests, frontend build, release build, stack restart, and manual UI inspection.
