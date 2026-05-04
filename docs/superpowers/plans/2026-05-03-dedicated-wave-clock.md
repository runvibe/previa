# Dedicated Wave Clock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the wave load-test clock to an isolated OS thread so target request starts are generated independently from HTTP response pressure, while making late dispatch visible and non-destructive to the curve.

**Architecture:** The wave scheduler becomes a synchronous clock loop running in a named `std::thread`, using the existing `DispatchClock` and nonblocking channel sends. Each emitted slot carries a validity window; the Tokio dispatcher either turns a fresh slot into request work or drops a stale slot and records dispatcher lag instead of sending old traffic late. Metrics distinguish clock starvation (`schedulerLagMs` / `schedulerLaggedStarts`) from dispatcher starvation (`dispatcherLaggedStarts`) and slot channel backpressure.

**Tech Stack:** Rust, `std::thread`, `std::time::Instant`, Tokio `mpsc`, `CancellationToken`, existing `DispatchClock`, existing `WaveSender`, existing main/app load-test metrics pipeline.

---

## Correctness Contract

- The wave clock must not run on the same Tokio worker pool used by HTTP send futures, response observers, SSE, runtime sampling, or pipeline continuation work.
- The clock loop must never await HTTP work, response work, metrics publishing, or pipeline preparation.
- The clock loop may only calculate the next target from `LoadProfile`, enqueue a lightweight `WaveDispatchSlot` with `try_send`, emit lightweight metric events, and sleep on its own OS thread.
- `dispatchStarted` remains the measurement of request start attempts and should track the wave while the runner host has enough CPU/runtime capacity.
- If the dedicated clock itself wakes late, record `schedulerLagMs` and `schedulerLaggedStarts`; this means the runner host or OS cannot schedule the clock thread on time.
- If the clock emits a slot on time but Tokio consumes it after its validity window, drop that slot and record `dispatcherLaggedStarts`; this means the HTTP/dispatcher runtime is saturated.
- The dispatcher must not send stale slots late to "catch up", because late bursts distort the configured wave.
- Response success, response failure, slow response, and target endpoint errors must not reduce future request-start scheduling. They can only affect response metrics and pipeline continuation availability.
- The existing response grace period remains only for observing responses after the wave ends; it must not extend request-start scheduling.

## File Structure

- Modify `runner/src/server/wave_scheduler.rs`
  - Add slot deadline fields.
  - Add pure expiration helper.
  - Replace the async scheduler entrypoint with a synchronous loop.
  - Add `spawn_wave_scheduler_thread(...)` to create a named OS thread.

- Modify `runner/src/server/wave_executor.rs`
  - Use `spawn_wave_scheduler_thread(...)` instead of `tokio::spawn(run_wave_scheduler(...))`.
  - Drop expired slots before request preparation.
  - Record dispatcher lag as missed starts and metrics.
  - Join the scheduler thread during shutdown without blocking cancellation.

- Modify `runner/src/server/wave_metrics_actor.rs`
  - Add a `DispatcherLaggedStarts(usize)` metric event.
  - Route it into `MetricsAccumulator`.

- Modify `runner/src/server/metrics.rs`
  - Add `dispatcher_lagged_starts` to the accumulator and snapshot export.

- Modify `runner/src/server/models.rs`
  - Add `dispatcher_lagged_starts: Option<usize>` to `LoadTestMetrics`.

- Modify main aggregation files after locating current fields with `rg "scheduler_lagged_starts|runtime_lagged_starts|dependency_limited_starts"`:
  - `main/src/...` files that deserialize runner metrics and aggregate runner totals.
  - Preserve snake_case JSON compatibility.

- Modify app files after locating current fields with `rg "schedulerLaggedStarts|runtimeLaggedStarts|dependencyLimitedStarts|scheduler_lagged_starts"`:
  - `app/src/types/load-test.ts`
  - Load-test result panel and diagnostics labels.
  - Locale files only for visible labels added by the result panel.

---

### Task 1: Add Slot Deadlines and Expiration Helper

**Files:**
- Modify: `runner/src/server/wave_scheduler.rs`
- Test: `runner/src/server/wave_scheduler.rs`

- [ ] **Step 1: Write failing tests for slot validity**

Add these tests inside the existing `#[cfg(test)] mod tests` in `runner/src/server/wave_scheduler.rs`:

```rust
#[test]
fn dispatch_slot_is_fresh_until_expiration_elapsed_ms() {
    let slot = WaveDispatchSlot {
        elapsed_ms: 500,
        expires_at_elapsed_ms: 600,
        planned_starts: 10,
        target_rps_limit: 100.0,
        scheduled_total: 20,
        scheduler_lag_ms: 0,
        missed_due_to_scheduler_lag: 0,
    };

    assert!(!slot_is_expired(&slot, 599));
    assert!(!slot_is_expired(&slot, 600));
    assert!(slot_is_expired(&slot, 601));
}

#[test]
fn build_slot_from_clock_tick_sets_expiration_to_next_tick_boundary() {
    let mut clock = DispatchClock::new(100);
    let tick = clock.plan_tick(500, 100.0);
    let slot = build_dispatch_slot(tick, 100);

    assert_eq!(slot.elapsed_ms, 500);
    assert_eq!(slot.expires_at_elapsed_ms, 600);
    assert_eq!(slot.planned_starts, 10);
    assert_eq!(slot.target_rps_limit, 100.0);
}
```

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cargo test -p previa-runner wave_scheduler
```

Expected: FAIL with missing `expires_at_elapsed_ms`, `slot_is_expired`, and `build_dispatch_slot`.

- [ ] **Step 3: Extend `WaveDispatchSlot`**

Update the struct in `runner/src/server/wave_scheduler.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveDispatchSlot {
    pub elapsed_ms: u64,
    pub expires_at_elapsed_ms: u64,
    pub planned_starts: usize,
    pub target_rps_limit: f64,
    pub scheduled_total: usize,
    pub scheduler_lag_ms: u64,
    pub missed_due_to_scheduler_lag: usize,
}
```

- [ ] **Step 4: Add pure slot helpers**

Add this below `WaveSchedulerMetric`:

```rust
pub fn build_dispatch_slot(
    tick: crate::server::load_dispatch::DispatchTick,
    tick_ms: u64,
) -> WaveDispatchSlot {
    WaveDispatchSlot {
        elapsed_ms: tick.elapsed_ms,
        expires_at_elapsed_ms: tick.elapsed_ms.saturating_add(tick_ms),
        planned_starts: tick.scheduled_starts,
        target_rps_limit: tick.target_rps,
        scheduled_total: tick.scheduled_total,
        scheduler_lag_ms: tick.scheduler_lag_ms,
        missed_due_to_scheduler_lag: tick.missed_due_to_scheduler_lag,
    }
}

pub fn slot_is_expired(slot: &WaveDispatchSlot, actual_elapsed_ms: u64) -> bool {
    actual_elapsed_ms > slot.expires_at_elapsed_ms
}
```

- [ ] **Step 5: Replace manual test struct construction**

Update every `WaveDispatchSlot { ... }` in `runner/src/server/wave_scheduler.rs` tests to include:

```rust
expires_at_elapsed_ms: 100,
```

Use `expires_at_elapsed_ms: 600` for slots whose `elapsed_ms` is `500`, and `expires_at_elapsed_ms: 200` for slots whose `elapsed_ms` is `100`.

- [ ] **Step 6: Verify slot tests**

Run:

```bash
cargo test -p previa-runner wave_scheduler
```

Expected: PASS.

---

### Task 2: Move the Wave Clock to a Dedicated OS Thread

**Files:**
- Modify: `runner/src/server/wave_scheduler.rs`
- Test: `runner/src/server/wave_scheduler.rs`

- [ ] **Step 1: Write a failing thread scheduler test**

Add this test to `runner/src/server/wave_scheduler.rs`:

```rust
fn short_flat_load() -> LoadProfile {
    LoadProfile {
        points: vec![
            crate::server::models::LoadPoint {
                at_ms: 0,
                intensity: 50.0,
            },
            crate::server::models::LoadPoint {
                at_ms: 250,
                intensity: 50.0,
            },
        ],
        interpolation: crate::server::models::LoadInterpolation::Linear,
        runner_max_rps: None,
        grace_period_ms: 0,
    }
}

#[tokio::test]
async fn scheduler_thread_emits_slots_without_tokio_spawn() {
    let (slot_tx, mut slot_rx) = mpsc::channel(8);
    let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
    let token = CancellationToken::new();

    let handle = spawn_wave_scheduler_thread(short_flat_load(), 100, slot_tx, metric_tx, token);

    let first = tokio::time::timeout(std::time::Duration::from_millis(300), slot_rx.recv())
        .await
        .expect("scheduler thread should emit a slot")
        .expect("slot channel should stay open while scheduler runs");

    assert!(first.planned_starts > 0);
    assert_eq!(first.expires_at_elapsed_ms, first.elapsed_ms + 100);

    handle.join().expect("scheduler thread should exit cleanly");
}
```

- [ ] **Step 2: Run and confirm RED**

Run:

```bash
cargo test -p previa-runner scheduler_thread_emits_slots_without_tokio_spawn
```

Expected: FAIL because `spawn_wave_scheduler_thread` does not exist.

- [ ] **Step 3: Replace the async scheduler loop with a synchronous loop**

In `runner/src/server/wave_scheduler.rs`, replace `pub async fn run_wave_scheduler(...)` with:

```rust
pub fn spawn_wave_scheduler_thread(
    load: LoadProfile,
    tick_ms: u64,
    slot_tx: mpsc::Sender<WaveDispatchSlot>,
    metric_tx: mpsc::UnboundedSender<WaveSchedulerMetric>,
    token: CancellationToken,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("previa-wave-clock".to_string())
        .spawn(move || {
            run_wave_scheduler_loop(load, tick_ms, slot_tx, metric_tx, token);
        })
        .expect("failed to spawn previa wave scheduler thread")
}

pub fn run_wave_scheduler_loop(
    load: LoadProfile,
    tick_ms: u64,
    slot_tx: mpsc::Sender<WaveDispatchSlot>,
    metric_tx: mpsc::UnboundedSender<WaveSchedulerMetric>,
    token: CancellationToken,
) {
    let started = std::time::Instant::now();
    let end_ms = crate::server::load_wave::timeline_end_ms(&load);
    let mut clock = DispatchClock::new(tick_ms);
    let mut next_wake = started;

    loop {
        if token.is_cancelled() {
            break;
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        if elapsed_ms >= end_ms {
            break;
        }

        let target_rps_limit = crate::server::load_wave::local_rps_limit(&load, elapsed_ms);
        let tick = clock.plan_tick(elapsed_ms, target_rps_limit);
        let slot = build_dispatch_slot(tick, tick_ms);
        let _ = try_send_slot_or_metric(&slot_tx, &metric_tx, slot);

        next_wake += std::time::Duration::from_millis(tick_ms);
        let now = std::time::Instant::now();
        if next_wake > now {
            std::thread::sleep(next_wake - now);
        } else {
            next_wake = now;
        }
    }
}
```

- [ ] **Step 4: Remove the Tokio timer import dependency**

Keep `tokio::sync::mpsc` because `Sender::try_send` can be called from a normal thread. Do not import or call `tokio::time::sleep` in `runner/src/server/wave_scheduler.rs`.

- [ ] **Step 5: Verify scheduler tests**

Run:

```bash
cargo test -p previa-runner wave_scheduler
```

Expected: PASS.

---

### Task 3: Wire the Executor to the Dedicated Clock

**Files:**
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Update the scheduler import**

Change the import in `runner/src/server/wave_executor.rs` from:

```rust
use crate::server::wave_scheduler::{WaveDispatchSlot, WaveSchedulerMetric, run_wave_scheduler};
```

to:

```rust
use crate::server::wave_scheduler::{
    WaveDispatchSlot, WaveSchedulerMetric, slot_is_expired, spawn_wave_scheduler_thread,
};
```

- [ ] **Step 2: Spawn the dedicated scheduler thread**

Replace:

```rust
let scheduler_task = tokio::spawn(run_wave_scheduler(
    load.clone(),
    tick_ms,
    slot_tx,
    scheduler_metric_tx,
    token.child_token(),
));
```

with:

```rust
let scheduler_token = token.child_token();
let scheduler_thread = spawn_wave_scheduler_thread(
    load.clone(),
    tick_ms,
    slot_tx,
    scheduler_metric_tx,
    scheduler_token.clone(),
);
```

- [ ] **Step 3: Cancel and join the scheduler thread during shutdown**

Replace:

```rust
scheduler_task.abort();
let _ = scheduler_task.await;
```

with:

```rust
scheduler_token.cancel();
if let Err(err) = scheduler_thread.join() {
    error!("wave scheduler thread panicked: {:?}", err);
}
```

- [ ] **Step 4: Run the focused compiler check**

Run:

```bash
cargo test -p previa-runner wave_executor --no-run
```

Expected: PASS compilation, or a direct compiler error for every missed struct field after adding `expires_at_elapsed_ms`.

- [ ] **Step 5: Fix any `WaveDispatchSlot` construction in executor tests**

For every test slot in `runner/src/server/wave_executor.rs`, add a validity window:

```rust
expires_at_elapsed_ms: elapsed_ms.saturating_add(tick_ms),
```

Use the literal field values already present in each test when `elapsed_ms` or `tick_ms` is not available.

- [ ] **Step 6: Verify executor compiles**

Run:

```bash
cargo test -p previa-runner wave_executor --no-run
```

Expected: PASS.

---

### Task 4: Drop Stale Slots Instead of Sending Late Bursts

**Files:**
- Modify: `runner/src/server/wave_executor.rs`
- Modify: `runner/src/server/wave_metrics_actor.rs`
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/models.rs`
- Test: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_metrics_actor.rs`
- Test: `runner/src/server/metrics.rs`

- [ ] **Step 1: Write the failing dispatcher-expiration test**

Add a test to `runner/src/server/wave_executor.rs` near existing dispatcher tests:

```rust
#[tokio::test]
async fn expired_dispatch_slot_records_lag_without_enqueuing_requests() {
    let pipeline = Pipeline {
        id: "p1".to_string(),
        name: "pipeline".to_string(),
        steps: vec![PipelineStep::Http(crate::server::tests::http_step("GET", "http://example.test"))],
    };
    let specs = Arc::new(Vec::new());
    let env_groups = Arc::new(Vec::new());
    let (request_tx, mut request_rx) = mpsc::unbounded_channel();
    let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
    let ready_to_send = Arc::new(AtomicUsize::new(0));
    let missed_starts = Arc::new(AtomicUsize::new(0));
    let token = tokio_util::sync::CancellationToken::new();
    let mut ready = VecDeque::new();
    let started = Instant::now() - std::time::Duration::from_millis(1_000);

    dispatch_slot_requests(DispatchSlotRequestArgs {
        slot: WaveDispatchSlot {
            elapsed_ms: 100,
            expires_at_elapsed_ms: 200,
            planned_starts: 7,
            target_rps_limit: 70.0,
            scheduled_total: 7,
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
        },
        ready: &mut ready,
        pipeline: &pipeline,
        specs: &specs,
        env_groups: &env_groups,
        selected_env_group_slug: &None,
        request_tx: &request_tx,
        metric_tx: &metric_tx,
        ready_to_send: &ready_to_send,
        missed_starts: &missed_starts,
        started,
        tick_ms: 100,
        token: &token,
    })
    .await;

    assert_eq!(missed_starts.load(Ordering::SeqCst), 7);
    assert_eq!(ready_to_send.load(Ordering::SeqCst), 0);
    assert!(request_rx.try_recv().is_err());
    assert!(matches!(
        metric_rx.try_recv(),
        Ok(WaveMetricEvent::DispatcherLaggedStarts(7))
    ));
}
```

If this test cannot use `crate::server::tests::http_step`, create the smallest existing `PipelineStep::Http` test helper already used in the file and keep the request URL as `http://example.test`.

- [ ] **Step 2: Run and confirm RED**

Run:

```bash
cargo test -p previa-runner expired_dispatch_slot_records_lag_without_enqueuing_requests
```

Expected: FAIL because `DispatcherLaggedStarts` and expiration handling do not exist.

- [ ] **Step 3: Add dispatcher lag metric event**

In `runner/src/server/wave_metrics_actor.rs`, add this enum variant:

```rust
DispatcherLaggedStarts(usize),
```

Handle it in `run_wave_metrics_actor`:

```rust
WaveMetricEvent::DispatcherLaggedStarts(count) => {
    accumulator.record_dispatcher_lagged_starts_count(count);
}
```

- [ ] **Step 4: Add accumulator support**

In `runner/src/server/metrics.rs`, add a field to `MetricsAccumulator`:

```rust
dispatcher_lagged_starts: usize,
```

Initialize it in `MetricsAccumulator::new()` or `Default`:

```rust
dispatcher_lagged_starts: 0,
```

Add the recorder:

```rust
pub fn record_dispatcher_lagged_starts_count(&mut self, count: usize) {
    self.dispatcher_lagged_starts = self.dispatcher_lagged_starts.saturating_add(count);
}
```

Add it to snapshot creation:

```rust
dispatcher_lagged_starts: Some(self.dispatcher_lagged_starts),
```

- [ ] **Step 5: Add model field**

In `runner/src/server/models.rs`, add this field to `LoadTestMetrics` with the other wave diagnostics:

```rust
pub dispatcher_lagged_starts: Option<usize>,
```

- [ ] **Step 6: Drop expired slots at the top of dispatch**

At the start of `dispatch_slot_requests` in `runner/src/server/wave_executor.rs`, before the `for` loop, add:

```rust
let actual_elapsed_ms = args.started.elapsed().as_millis() as u64;
if slot_is_expired(&args.slot, actual_elapsed_ms) {
    args.missed_starts
        .fetch_add(args.slot.planned_starts, Ordering::SeqCst);
    let _ = args
        .metric_tx
        .send(WaveMetricEvent::DispatcherLaggedStarts(args.slot.planned_starts));
    return;
}
```

Keep the existing per-request `RuntimeLaggedStart` classification below this block. That metric remains useful for smaller lags inside a still-fresh slot.

- [ ] **Step 7: Extend metrics actor test**

In `runner/src/server/wave_metrics_actor.rs`, update `metrics_actor_applies_dispatch_and_scheduler_events`:

```rust
event_tx
    .send(WaveMetricEvent::DispatcherLaggedStarts(6))
    .unwrap();
```

and add the assertion:

```rust
assert_eq!(snapshot.dispatcher_lagged_starts, Some(6));
```

- [ ] **Step 8: Verify runner metrics tests**

Run:

```bash
cargo test -p previa-runner wave_metrics_actor metrics::tests:: dispatcher_lagged
cargo test -p previa-runner expired_dispatch_slot_records_lag_without_enqueuing_requests
```

Expected: PASS. If the first command does not match local test names, run:

```bash
cargo test -p previa-runner wave_metrics_actor
cargo test -p previa-runner metrics
```

Expected: PASS.

---

### Task 5: Propagate `dispatcherLaggedStarts` Through Main and App

**Files:**
- Modify: main metric model and aggregation files found by `rg`
- Modify: `app/src/types/load-test.ts`
- Modify: load-test result panel components found by `rg "schedulerLaggedStarts|runtimeLaggedStarts|dependencyLimitedStarts"`
- Modify: locale files only if the panel uses translated metric labels
- Test: main load-test aggregation tests
- Test: app load-test panel tests

- [ ] **Step 1: Locate exact files**

Run:

```bash
rg "scheduler_lagged_starts|runtime_lagged_starts|dependency_limited_starts|dispatch_started" main runner app/src
rg "schedulerLaggedStarts|runtimeLaggedStarts|dependencyLimitedStarts|dispatchStarted" app/src
```

Expected: output lists the Rust aggregation files and the TypeScript type/panel files that currently carry existing diagnostics.

- [ ] **Step 2: Add snake_case field wherever main deserializes runner metrics**

In the main Rust struct that mirrors runner `LoadTestMetrics`, add:

```rust
pub dispatcher_lagged_starts: Option<usize>,
```

If the struct uses `serde(rename_all = "camelCase")`, use the matching existing style. If the existing runner payload is snake_case, keep the field as snake_case.

- [ ] **Step 3: Aggregate by sum**

In the main load-test aggregation function, add:

```rust
dispatcher_lagged_starts: sum_optional_usize(
    runners
        .iter()
        .map(|runner| runner.metrics.dispatcher_lagged_starts),
),
```

If the file uses a different helper name than `sum_optional_usize`, use the same helper that currently aggregates `scheduler_lagged_starts`.

- [ ] **Step 4: Add TypeScript field**

In `app/src/types/load-test.ts`, add the camelCase field next to the other diagnostics:

```ts
dispatcherLaggedStarts?: number | null
```

- [ ] **Step 5: Show the metric in diagnostics**

In the load-test results panel, add a diagnostic label next to scheduler/runtime/dependency lag:

```ts
{
  label: 'Atraso do dispatcher',
  value: formatCount(metrics.dispatcherLaggedStarts),
  hint: 'Slots emitidos pelo relógio, mas consumidos tarde demais para preservar a onda.',
}
```

If the component uses i18n keys instead of literal strings, add:

```json
"dispatcherLaggedStarts": "Atraso do dispatcher",
"dispatcherLaggedStartsHint": "Slots emitidos pelo relógio, mas consumidos tarde demais para preservar a onda."
```

to `app/src/i18n/locales/pt-BR.json` and equivalent concise English text to `app/src/i18n/locales/en.json`:

```json
"dispatcherLaggedStarts": "Dispatcher lag",
"dispatcherLaggedStartsHint": "Slots emitted by the clock but consumed too late to preserve the wave."
```

- [ ] **Step 6: Update panel test expectations**

In the existing load-test results panel test, include a metric fixture value:

```ts
dispatcherLaggedStarts: 12,
```

Assert the label is visible:

```ts
expect(screen.getByText(/Atraso do dispatcher|Dispatcher lag/i)).toBeInTheDocument()
expect(screen.getByText('12')).toBeInTheDocument()
```

- [ ] **Step 7: Verify main and app**

Run:

```bash
cargo test -p previa-main load
npm --prefix app test -- LoadTestResultsPanel
```

Expected: PASS.

---

### Task 6: Full Verification, Restart, and Runtime Acceptance

**Files:**
- No source files beyond Tasks 1-5.
- Runtime validation uses the existing local app and runners.

- [ ] **Step 1: Run full relevant test suite**

Run:

```bash
cargo test -p previa-runner
cargo test -p previa-main
npm --prefix app test -- LoadTestResultsPanel LoadTestConfigPanel LoadTestTab
npm --prefix app run build
cargo build --release
```

Expected:
- Runner tests PASS.
- Main tests PASS.
- App tests PASS.
- App build PASS, allowing existing Vite chunk/dynamic import warnings.
- Release build PASS.

- [ ] **Step 2: Restart local main and three runners**

Use the existing local run scripts or commands already used in this branch. Confirm:

```bash
curl -s http://127.0.0.1:5610/info
curl -s http://127.0.0.1:5611/info
curl -s http://127.0.0.1:5612/info
curl -s http://127.0.0.1:5613/info
```

Expected:
- Main reports healthy.
- All three runners report the new build and are active in main.

- [ ] **Step 3: Execute the known CRUD Users load test**

Run the same project/pipeline from the UI or existing API route:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Use the same wave shape that previously degraded after around second 70:

```text
0ms -> 10%
120000ms -> 80%
interpolation: smooth
runners: 3
runnerMaxRps: null
```

- [ ] **Step 4: Analyze the latest run numerically**

Fetch latest run details using the existing load-test history endpoint or local DB query used in prior analysis. Compare per 10-second buckets:

```text
targetRps
actualDispatchStartedRps
actual/target ratio
schedulerLagMs
schedulerLaggedStarts
dispatcherLaggedStarts
slotBackpressure-derived lag
readyRequests
outstandingRequests
cpu/memory per runner
```

Expected acceptance:
- Until local runner infrastructure saturates, `actualDispatchStartedRps / targetRps` stays close to the wave shape.
- `schedulerLagMs` and `schedulerLaggedStarts` are near zero compared with the previous run if the OS can schedule the dedicated clock.
- If the Tokio dispatcher cannot keep up, the loss appears as `dispatcherLaggedStarts`, not as delayed bursts.
- If the slot channel fills, the loss appears as slot backpressure/scheduler lag diagnostics.
- Target endpoint errors do not reduce future request-start scheduling by themselves.

- [ ] **Step 5: Commit and push**

Run:

```bash
git status --short
git add runner/src/server/wave_scheduler.rs runner/src/server/wave_executor.rs runner/src/server/wave_metrics_actor.rs runner/src/server/metrics.rs runner/src/server/models.rs main app docs/superpowers/plans/2026-05-03-dedicated-wave-clock.md
git commit -m "fix: isolate wave clock on dedicated thread"
git push origin codex/wave-load-test
```

Expected:
- Commit succeeds.
- Push succeeds.

## Implementation Notes

- Keep `RUNNER_RPS_PER_NODE` as the optional network safety ceiling. It is not a wave control mechanism.
- Do not reintroduce an in-flight response limit for wave request starts. Response observation can accumulate until host infrastructure becomes the bottleneck.
- Use `saturating_add` for all counters that can receive batch increments.
- The slot channel remains bounded at `1024` so infrastructure saturation is observable instead of silently consuming memory.
- If `std::thread::Builder::spawn` fails, panic at startup of the load run with a clear message. A runner unable to create the clock thread cannot provide valid open-loop semantics.
- Do not count expired slots as `dispatchStarted`; they were planned, but they were not actually started.
- Keep `missedStarts` as the total visible loss counter, and let detailed diagnostics explain the cause through `schedulerLaggedStarts`, `dispatcherLaggedStarts`, runtime lag, and dependency-limited starts.

## Self-Review

- Spec coverage: the plan isolates the wave clock from Tokio HTTP work, prevents stale catch-up bursts, preserves response-independent scheduling, and adds diagnostics to identify whether the bottleneck is OS clock scheduling or Tokio dispatcher saturation.
- Placeholder scan: every task names concrete files, concrete commands, expected outcomes, and code snippets for new interfaces.
- Type consistency: `dispatcher_lagged_starts` is the Rust/JSON field; `dispatcherLaggedStarts` is the TypeScript/UI field; `WaveMetricEvent::DispatcherLaggedStarts(usize)` is the event connecting dispatcher to accumulator.
