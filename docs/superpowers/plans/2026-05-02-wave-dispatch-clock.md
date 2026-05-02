# Wave Dispatch Clock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Previa wave load tests follow the configured HTTP start-rate curve as closely as possible, even when target responses are slow.

**Architecture:** Replace the passive leaky-bucket request gate with a runner-local dispatch clock. The clock computes per-tick HTTP start slots from the wave, expires unused slots instead of accumulating backlog, and drives an aggressive pipeline feeder so ready HTTP steps are available before each tick.

**Tech Stack:** Rust/Tokio/Axum in `runner` and `main`, React/TypeScript/Recharts in `app`, existing SSE load-test aggregation and history persistence.

---

## Stage 1: Runner Dispatch Engine

This stage changes the runner so the wave owns time. The runner should try to start the exact number of HTTP requests scheduled for each tick, independently of response completion.

### Files

- Create: `runner/src/server/load_dispatch.rs`
- Modify: `runner/src/server/mod.rs`
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/handlers/load.rs`
- Test: `runner/src/server/load_dispatch.rs`
- Test: `runner/src/server/metrics.rs`

### Task 1: Add Dispatch Clock Math

- [ ] **Step 1: Write failing tests for per-tick scheduling**

Add this test module to new file `runner/src/server/load_dispatch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_exact_integer_slots_per_tick() {
        let mut clock = DispatchClock::new(100);

        let first = clock.plan_tick(0, 2400.0);
        assert_eq!(first.scheduled_starts, 240);
        assert_eq!(first.target_rps, 2400.0);

        let second = clock.plan_tick(100, 2400.0);
        assert_eq!(second.scheduled_starts, 240);
        assert_eq!(second.scheduled_total, 480);
    }

    #[test]
    fn carries_fractional_slots_without_backlog_debt() {
        let mut clock = DispatchClock::new(100);

        let first = clock.plan_tick(0, 15.0);
        assert_eq!(first.scheduled_starts, 1);

        let second = clock.plan_tick(100, 15.0);
        assert_eq!(second.scheduled_starts, 2);

        let third = clock.plan_tick(200, 15.0);
        assert_eq!(third.scheduled_starts, 1);
        assert_eq!(third.scheduled_total, 4);
    }

    #[test]
    fn does_not_repay_missed_slots_in_later_ticks() {
        let mut state = DispatchRuntimeState::new(100);
        state.open_tick(DispatchTick {
            elapsed_ms: 0,
            target_rps: 1000.0,
            scheduled_starts: 100,
            scheduled_total: 100,
        });

        assert_eq!(state.finish_tick(), DispatchTickReport {
            scheduled_starts: 100,
            actual_starts: 0,
            missed_starts: 100,
        });

        state.open_tick(DispatchTick {
            elapsed_ms: 100,
            target_rps: 1000.0,
            scheduled_starts: 100,
            scheduled_total: 200,
        });

        assert_eq!(state.available_slots(), 100);
    }
}
```

- [ ] **Step 2: Run the focused test and confirm it fails**

Run:

```bash
cargo test -p previa-runner load_dispatch::tests -- --nocapture
```

Expected: compile failure because `load_dispatch` and its structs do not exist.

- [ ] **Step 3: Implement the dispatch clock**

Create `runner/src/server/load_dispatch.rs` with:

```rust
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchTick {
    pub elapsed_ms: u64,
    pub target_rps: f64,
    pub scheduled_starts: usize,
    pub scheduled_total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchTickReport {
    pub scheduled_starts: usize,
    pub actual_starts: usize,
    pub missed_starts: usize,
}

#[derive(Debug)]
pub struct DispatchClock {
    tick_ms: u64,
    fractional_carry: f64,
    scheduled_total: usize,
}

impl DispatchClock {
    pub fn new(tick_ms: u64) -> Self {
        Self {
            tick_ms,
            fractional_carry: 0.0,
            scheduled_total: 0,
        }
    }

    pub fn plan_tick(&mut self, elapsed_ms: u64, target_rps: f64) -> DispatchTick {
        let raw_slots = target_rps.max(0.0) * self.tick_ms as f64 / 1000.0 + self.fractional_carry;
        let scheduled_starts = raw_slots.floor() as usize;
        self.fractional_carry = raw_slots - scheduled_starts as f64;
        self.scheduled_total = self.scheduled_total.saturating_add(scheduled_starts);

        DispatchTick {
            elapsed_ms,
            target_rps,
            scheduled_starts,
            scheduled_total: self.scheduled_total,
        }
    }
}

#[derive(Debug)]
pub struct DispatchRuntimeState {
    tick_ms: u64,
    generation: AtomicU64,
    slots: AtomicUsize,
    scheduled_in_tick: AtomicUsize,
    actual_in_tick: AtomicUsize,
    scheduled_total: AtomicUsize,
    actual_total: AtomicUsize,
    waiters: AtomicUsize,
    notify: Notify,
}

impl DispatchRuntimeState {
    pub fn new(tick_ms: u64) -> Self {
        Self {
            tick_ms,
            generation: AtomicU64::new(0),
            slots: AtomicUsize::new(0),
            scheduled_in_tick: AtomicUsize::new(0),
            actual_in_tick: AtomicUsize::new(0),
            scheduled_total: AtomicUsize::new(0),
            actual_total: AtomicUsize::new(0),
            waiters: AtomicUsize::new(0),
            notify: Notify::new(),
        }
    }

    pub fn open_tick(&self, tick: DispatchTick) {
        self.slots.store(tick.scheduled_starts, Ordering::SeqCst);
        self.scheduled_in_tick.store(tick.scheduled_starts, Ordering::SeqCst);
        self.actual_in_tick.store(0, Ordering::SeqCst);
        self.scheduled_total.store(tick.scheduled_total, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn finish_tick(&self) -> DispatchTickReport {
        let scheduled_starts = self.scheduled_in_tick.load(Ordering::SeqCst);
        let actual_starts = self.actual_in_tick.load(Ordering::SeqCst);
        self.slots.store(0, Ordering::SeqCst);
        DispatchTickReport {
            scheduled_starts,
            actual_starts,
            missed_starts: scheduled_starts.saturating_sub(actual_starts),
        }
    }

    pub fn available_slots(&self) -> usize {
        self.slots.load(Ordering::SeqCst)
    }

    pub fn waiting_ready_requests(&self) -> usize {
        self.waiters.load(Ordering::SeqCst)
    }

    pub fn scheduled_total(&self) -> usize {
        self.scheduled_total.load(Ordering::SeqCst)
    }

    pub fn actual_total(&self) -> usize {
        self.actual_total.load(Ordering::SeqCst)
    }

    pub async fn acquire(&self, should_cancel: impl Fn() -> bool) -> bool {
        self.waiters.fetch_add(1, Ordering::SeqCst);
        let result = loop {
            if should_cancel() {
                break false;
            }

            let mut current = self.slots.load(Ordering::SeqCst);
            while current > 0 {
                match self.slots.compare_exchange(
                    current,
                    current - 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        self.actual_in_tick.fetch_add(1, Ordering::SeqCst);
                        self.actual_total.fetch_add(1, Ordering::SeqCst);
                        break true;
                    }
                    Err(next) => current = next,
                }
            }

            self.notify.notified().await;
        };
        self.waiters.fetch_sub(1, Ordering::SeqCst);
        result
    }
}
```

- [ ] **Step 4: Register the module**

Modify `runner/src/server/mod.rs`:

```rust
pub mod load_bucket;
pub mod load_dispatch;
pub mod load_wave;
```

- [ ] **Step 5: Run the focused test**

Run:

```bash
cargo test -p previa-runner load_dispatch::tests -- --nocapture
```

Expected: all dispatch clock tests pass.

### Task 2: Add Dispatch Metrics

- [ ] **Step 1: Extend runner load metrics model**

Modify `runner/src/server/models.rs` `LoadTestMetrics`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub scheduled_starts: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub missed_starts: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub ready_requests: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub active_pipelines: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub outstanding_requests: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub curve_adherence: Option<f64>,
```

- [ ] **Step 2: Extend wave snapshot**

Modify `runner/src/server/metrics.rs` `WaveMetricsSnapshot`:

```rust
pub scheduled_starts: usize,
pub missed_starts: usize,
pub ready_requests: usize,
pub active_pipelines: usize,
pub outstanding_requests: usize,
```

- [ ] **Step 3: Serialize adherence in `snapshot_with_wave`**

Inside `snapshot_with_wave`, add:

```rust
let curve_adherence = wave.as_ref().map(|value| {
    if value.scheduled_starts == 0 {
        100.0
    } else {
        round2(((value.scheduled_starts.saturating_sub(value.missed_starts)) as f64
            / value.scheduled_starts as f64)
            * 100.0)
    }
});
```

Then populate the new fields:

```rust
scheduled_starts: wave.as_ref().map(|value| value.scheduled_starts),
missed_starts: wave.as_ref().map(|value| value.missed_starts),
ready_requests: wave.as_ref().map(|value| value.ready_requests),
active_pipelines: wave.as_ref().map(|value| value.active_pipelines),
outstanding_requests: wave.as_ref().map(|value| value.outstanding_requests),
curve_adherence,
```

- [ ] **Step 4: Add metrics tests**

Add to `runner/src/server/metrics.rs` tests:

```rust
#[test]
fn snapshot_includes_dispatch_adherence() {
    let metrics = MetricsAccumulator::new();
    let snapshot = metrics.snapshot_with_wave(
        None,
        None,
        Some(WaveMetricsSnapshot {
            target_intensity: 80.0,
            target_rps_limit: 800.0,
            in_flight: 50,
            runner_max_rps: 1000.0,
            tick_ms: 100,
            scheduled_starts: 80,
            missed_starts: 4,
            ready_requests: 20,
            active_pipelines: 200,
            outstanding_requests: 150,
        }),
    );

    assert_eq!(snapshot.scheduled_starts, Some(80));
    assert_eq!(snapshot.missed_starts, Some(4));
    assert_eq!(snapshot.curve_adherence, Some(95.0));
}
```

- [ ] **Step 5: Run metrics tests**

Run:

```bash
cargo test -p previa-runner server::metrics::tests -- --nocapture
```

Expected: all runner metrics tests pass.

### Task 3: Replace Bucket Gate With Dispatch Clock

- [ ] **Step 1: Add a focused helper for wave snapshots**

In `runner/src/server/handlers/load.rs`, add a private helper near `run_wave_load`:

```rust
fn wave_snapshot(
    load: &LoadProfile,
    elapsed_ms: u64,
    tick_ms: u64,
    in_flight: usize,
    dispatch: &DispatchRuntimeState,
    missed_starts: usize,
    outstanding_requests: usize,
) -> crate::server::metrics::WaveMetricsSnapshot {
    crate::server::metrics::WaveMetricsSnapshot {
        target_intensity: sample_intensity(load, elapsed_ms),
        target_rps_limit: local_rps_limit(load, elapsed_ms),
        in_flight,
        runner_max_rps: load.runner_max_rps,
        tick_ms,
        scheduled_starts: dispatch.scheduled_total(),
        missed_starts,
        ready_requests: dispatch.waiting_ready_requests(),
        active_pipelines: in_flight,
        outstanding_requests,
    }
}
```

- [ ] **Step 2: Replace `FlowBucket` state**

In `run_wave_load`, remove:

```rust
let bucket = Arc::new(tokio::sync::Mutex::new(FlowBucket::new(
    local_rps_limit(&load, 0),
    0,
)));
```

Add:

```rust
let dispatch = Arc::new(DispatchRuntimeState::new(tick_ms));
let missed_starts = Arc::new(AtomicUsize::new(0));
let outstanding_requests = Arc::new(AtomicUsize::new(0));
```

Add imports:

```rust
use crate::server::load_dispatch::{DispatchClock, DispatchRuntimeState};
```

- [ ] **Step 3: Drive ticks from the dispatch clock**

In the main wave loop, before spawning feeder work, compute and open the current tick:

```rust
let mut dispatch_clock = DispatchClock::new(tick_ms);
```

Inside each loop tick:

```rust
let elapsed_ms = started.elapsed().as_millis() as u64;
let target_rps_limit = local_rps_limit(&load, elapsed_ms);
let tick = dispatch_clock.plan_tick(elapsed_ms, target_rps_limit);
dispatch.open_tick(tick);
```

After sleeping for the tick, record misses:

```rust
tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms)).await;
let report = dispatch.finish_tick();
missed_starts.fetch_add(report.missed_starts, Ordering::SeqCst);
```

- [ ] **Step 4: Change request gate acquisition**

Replace the bucket acquisition closure passed to `execute_pipeline_with_runtime_request_gate` with:

```rust
let dispatch = Arc::clone(&dispatch);
let metrics = Arc::clone(&metrics_for_gate);
let token = token_for_gate.clone();
let outstanding_requests = Arc::clone(&outstanding_requests);

Box::pin(async move {
    if dispatch.acquire(|| token.is_cancelled()).await {
        outstanding_requests.fetch_add(1, Ordering::SeqCst);
        let mut lock = metrics.lock().await;
        lock.record_http_start();
    }
})
```

After pipeline results are known, decrement by the number of HTTP responses observed:

```rust
let completed_http = results.iter().filter(|result| result.request.is_some()).count();
outstanding_requests.fetch_sub(completed_http, Ordering::SeqCst);
lock.record_http_completed_count(completed_http);
```

- [ ] **Step 5: Make the feeder aggressive**

Replace the existing `while in_flight < load.max_in_flight` loop condition with:

```rust
let desired_ready = (local_rps_limit(&load, elapsed_ms) * 0.5).ceil() as usize;
let desired_active = load
    .max_in_flight
    .max(desired_ready.saturating_mul(2))
    .max(dispatch.waiting_ready_requests().saturating_mul(2));

while in_flight.load(Ordering::SeqCst) < desired_active {
    // existing task spawn body
}
```

This makes `maxInFlight` a floor, not the ceiling that prevents the curve from being reached.

- [ ] **Step 6: Stop sending at wave end**

When `elapsed_ms >= end_ms`, call:

```rust
dispatch.finish_tick();
```

Then stop opening new ticks and only wait for in-flight tasks during `gracePeriodMs`. Do not call `dispatch.open_tick` during grace.

- [ ] **Step 7: Run focused runner tests**

Run:

```bash
cargo test -p previa-runner load_dispatch::tests server::metrics::tests -- --nocapture
```

Expected: all tests pass.

### Task 4: Verify Stage 1 With a Controlled Run

- [ ] **Step 1: Build runner and main**

Run:

```bash
cargo build -p previa-runner -p previa-main
```

Expected: build succeeds.

- [ ] **Step 2: Restart local main and three runners**

Use the existing local screen/session workflow for ports `5610`, `5611`, `5612`, and `5613`.

Expected:

```txt
main /health -> 200
runner 5611 /info -> 200
runner 5612 /info -> 200
runner 5613 /info -> 200
```

- [ ] **Step 3: Execute the CRUD Users load test**

Use the browser or API with the same wave:

```json
{
  "points": [
    { "atMs": 0, "intensity": 10 },
    { "atMs": 3000, "intensity": 80 }
  ],
  "interpolation": "smooth",
  "maxInFlight": 5000,
  "gracePeriodMs": 30000
}
```

- [ ] **Step 4: Analyze persisted history**

Fetch the latest run:

```bash
curl -fsS 'http://127.0.0.1:5610/api/v1/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/tests/load?pipelineIndex=0&limit=1' \
  > /tmp/previa-wave-clock-history.json
```

Expected:

- `targetRpsLimit` reaches `2400`.
- `httpStarted` deltas during the first `3s` are close to the configured curve.
- `missedStarts` remains low unless the runner or OS physically cannot create enough requests.
- After `3000ms`, no new dispatch slots are opened.

- [ ] **Step 5: Commit Stage 1**

```bash
git add runner/src/server/load_dispatch.rs runner/src/server/mod.rs runner/src/server/models.rs runner/src/server/metrics.rs runner/src/server/handlers/load.rs
git commit -m "feat(runner): dispatch wave requests by clock"
```

## Stage 2: Aggregation, UI, and Diagnostics

This stage makes the result view prove whether the wave was followed and why it missed when it could not.

### Files

- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/load-rps-chart.ts`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`
- Modify: `app/src/i18n/locales/pt-BR.json`
- Modify: `app/src/i18n/locales/en.json`

### Task 5: Aggregate Dispatch Metrics in `previa-main`

- [ ] **Step 1: Extend `RunnerLoadMetricsPoint`**

Modify `main/src/server/models.rs`:

```rust
pub scheduled_starts: Option<usize>,
pub missed_starts: Option<usize>,
pub ready_requests: Option<usize>,
pub active_pipelines: Option<usize>,
pub outstanding_requests: Option<usize>,
pub curve_adherence: Option<f64>,
```

- [ ] **Step 2: Parse new fields from runner payloads**

Modify `main/src/server/utils.rs`:

```rust
scheduled_starts: get_usize_field(payload, "scheduledStarts"),
missed_starts: get_usize_field(payload, "missedStarts"),
ready_requests: get_usize_field(payload, "readyRequests"),
active_pipelines: get_usize_field(payload, "activePipelines"),
outstanding_requests: get_usize_field(payload, "outstandingRequests"),
curve_adherence: get_f64_field(payload, "curveAdherence"),
```

- [ ] **Step 3: Include dispatch metrics in `rpsHistory` runner samples**

Modify `main/src/server/execution/load_batch.rs` `build_rps_history_sample` runner JSON:

```rust
"scheduledStarts": metrics.scheduled_starts,
"missedStarts": metrics.missed_starts,
"readyRequests": metrics.ready_requests,
"activePipelines": metrics.active_pipelines,
"outstandingRequests": metrics.outstanding_requests,
"curveAdherence": metrics.curve_adherence,
```

- [ ] **Step 4: Include consolidated totals**

In the same file, include consolidated fields:

```rust
"scheduledStarts": metrics.scheduled_starts,
"missedStarts": metrics.missed_starts,
"readyRequests": metrics.ready_requests,
"activePipelines": metrics.active_pipelines,
"outstandingRequests": metrics.outstanding_requests,
"curveAdherence": metrics.curve_adherence,
```

- [ ] **Step 5: Run main tests**

Run:

```bash
cargo test -p previa-main load_batch -- --nocapture
```

Expected: load batch aggregation tests pass.

### Task 6: Make the RPS Chart Use Configured Target Only

- [ ] **Step 1: Add TypeScript dispatch fields**

Modify `app/src/types/load-test.ts` in `RpsPoint`, `RunnerRpsSample`, `RemoteMetricsEvent`, `LoadTestMetrics`, and `ConsolidatedLoadMetrics`:

```ts
scheduledStarts?: number;
missedStarts?: number;
readyRequests?: number;
activePipelines?: number;
outstandingRequests?: number;
curveAdherence?: number;
```

- [ ] **Step 2: Fix target-line estimation**

Modify `app/src/lib/load-rps-chart.ts`:

```ts
function estimateTargetRpsLimit(
  _point: { targetIntensity?: number; targetRpsLimit?: number },
  metrics: LoadTestMetrics,
  waveConfig: WaveLoadConfig | null,
  elapsedMs: number,
) {
  if (typeof metrics.runnerMaxRps !== "number" || !waveConfig) return undefined;
  const intensity = sampleWaveIntensity(waveConfig, elapsedMs);
  return typeof intensity === "number" ? roundOne((metrics.runnerMaxRps * intensity) / 100) : undefined;
}
```

This intentionally ignores stale `point.targetRpsLimit`.

- [ ] **Step 3: Fix first-sample RPS**

In `buildRpsChartData`, replace fallbacks that use `runner.rps` or `point.rps` for the first HTTP sample with `0`:

```ts
const intervalRps = previousRunner
  && typeof previousRunner.httpStarted === "number"
  && typeof runner.httpStarted === "number"
  && intervalSeconds > 0
  ? Math.max(0, (runner.httpStarted - previousRunner.httpStarted) / intervalSeconds)
  : 0;
```

And:

```ts
const total = typeof previousHttpStarted === "number"
  && typeof point.httpStarted === "number"
  && intervalSeconds > 0
  ? Math.max(0, (point.httpStarted - previousHttpStarted) / intervalSeconds)
  : 0;
```

- [ ] **Step 4: Add chart tests**

Modify `app/src/components/LoadTestResultsPanel.test.tsx` to assert:

```ts
expect(buildRpsChartData(metrics, waveConfig).data[0].rpsTotal).toBe(0);
expect(buildRpsChartData(metrics, waveConfig).data[1].targetRpsLimit).toBe(2400);
```

- [ ] **Step 5: Run app chart tests**

Run:

```bash
cd app && npm test -- LoadTestResultsPanel
```

Expected: all load result panel tests pass.

### Task 7: Add Wave Adherence UI

- [ ] **Step 1: Add result cards**

Modify `app/src/components/LoadTestResultsPanel.tsx` to show cards when available:

```tsx
{typeof metrics.curveAdherence === "number" && (
  <MetricCard icon={Activity} label={t("loadTestResults.curveAdherence")} value={`${metrics.curveAdherence.toFixed(1)}%`} color="text-emerald-600" />
)}
{typeof metrics.missedStarts === "number" && (
  <MetricCard icon={AlertTriangle} label={t("loadTestResults.missedStarts")} value={metrics.missedStarts.toLocaleString()} color="text-amber-600" />
)}
{typeof metrics.readyRequests === "number" && (
  <MetricCard icon={ListChecks} label={t("loadTestResults.readyRequests")} value={metrics.readyRequests.toLocaleString()} color="text-primary" />
)}
```

- [ ] **Step 2: Add translations**

Modify `app/src/i18n/locales/pt-BR.json`:

```json
"loadTestResults.curveAdherence": "Aderência à onda",
"loadTestResults.missedStarts": "Starts perdidos",
"loadTestResults.readyRequests": "Requests prontos"
```

Modify `app/src/i18n/locales/en.json`:

```json
"loadTestResults.curveAdherence": "Wave adherence",
"loadTestResults.missedStarts": "Missed starts",
"loadTestResults.readyRequests": "Ready requests"
```

- [ ] **Step 3: Run app build**

Run:

```bash
cd app && npm run build
```

Expected: app build succeeds.

### Task 8: Final Verification and Release

- [ ] **Step 1: Run Rust tests**

```bash
cargo test
```

Expected: all Rust tests pass.

- [ ] **Step 2: Run release build**

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] **Step 3: Run app tests and build**

```bash
cd app && npm test -- LoadTestResultsPanel && npm run build
```

Expected: tests and app build pass.

- [ ] **Step 4: Run an end-to-end wave verification**

Run the same CRUD Users wave with 3 runners and compare `rpsHistory` deltas to the configured wave.

Expected:

- actual HTTP start RPS follows the configured wave during `0ms..3000ms`;
- `missedStarts` explains any deviation;
- no new dispatch slots are created after the wave endpoint;
- slow responses increase outstanding requests, not post-wave dispatch.

- [ ] **Step 5: Commit Stage 2**

```bash
git add main/src/server/models.rs main/src/server/utils.rs main/src/server/execution/load_batch.rs app/src/types/load-test.ts app/src/lib/load-rps-chart.ts app/src/components/LoadTestResultsPanel.tsx app/src/components/LoadTestResultsPanel.test.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json
git commit -m "feat: report wave dispatch adherence"
```

- [ ] **Step 6: Push branch**

```bash
git push origin codex/wave-load-test
```

Expected: branch pushes successfully.

## Self-Review

- Spec coverage: the plan covers the two requested stages: runner dispatch control first, metrics/UI diagnostics second.
- Placeholder scan: no open implementation placeholders remain.
- Type consistency: dispatch fields use camelCase over JSON and snake_case in Rust.
- Scope check: this is one coherent feature because the UI depends on metrics emitted by the new runner dispatch engine.
