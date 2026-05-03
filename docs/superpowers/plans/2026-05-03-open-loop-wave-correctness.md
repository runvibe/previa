# Open Loop Wave Correctness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the wave load algorithm provably open-loop: target errors, slow responses, body parsing, assertions, and dependent pipeline continuations must not control the request start rate.

**Architecture:** Keep the wave scheduler as the only owner of time and slots. The scheduler produces slots from the wave curve, prepares the next eligible request, and submits it to a hot sender path. Response observation runs on a separate lane with bounded feedback into the scheduler. If the runner misses the curve, the system records scheduler/dispatch/runtime lag instead of compensating with bursts.

**Tech Stack:** Rust, Tokio `mpsc`, `CancellationToken`, `reqwest::Client`, existing `previa_runner::prepare_http_step` and `send_prepared_http_step_with_hooks`, main load aggregation, React/TypeScript load chart diagnostics.

---

## Current Findings

The current implementation is partially open-loop:

- `runner/src/server/wave_executor.rs` computes slots from elapsed time and `target_rps_limit`, not from response status.
- If no dependent continuation is ready, `next_cursor_for_slot` creates a new pipeline cursor, so response latency does not directly block new initial requests.
- Prepared requests are submitted to `WaveSender` through an unbounded channel.

The current implementation is not fully open-loop:

- `wave_executor` drains all observer events before each tick. A flood of completed responses/errors can delay scheduling.
- `WaveSender::run` accepts new requests and joins finished observer tasks in the same `tokio::select!`, so response completion competes with request acceptance.
- `DispatchClock::plan_tick` repays delayed wall-clock time by scheduling a burst after lag. For load generation, missed time should become lag metrics, not later burst traffic.
- `httpStarted` currently means "request task entered the send path", not "network bytes were definitely written". This is useful, but the dashboard must expose it as dispatch-start RPS and keep `httpSendReturned`/errors for infrastructure diagnosis.

---

## Correctness Contract

- The wave scheduler must never await response headers, response body, assertions, retries, or pipeline completion to compute the next tick.
- A target failure must only affect response metrics and dependent continuations for that specific pipeline.
- If a dependent continuation is unavailable, the next wave slot must start a new pipeline request when the first step can be prepared.
- Observer feedback into the scheduler must be bounded per tick.
- Delayed ticks must record lag/missed slots and must not repay old slots with a later burst.
- Request start-rate diagnostics must distinguish:
  - `targetRps`: what the wave asked for.
  - `scheduledStarts`: slots created by the scheduler.
  - `dispatchStarted`: attempts accepted by the hot sender path.
  - `httpSendReturned`: reqwest `send()` returned with either response headers or send error.
  - `responseBodyCompleted`: body was read.
  - `observerBacklog`: responses still being observed.
  - `schedulerLagMs` / `dispatchLaggedStarts`: local runner/infra could not keep up.

---

## File Structure

- Modify `runner/src/server/load_dispatch.rs`
  - Make delayed ticks report lag instead of creating catch-up bursts.
  - Add deterministic clock tests for ramp, delay, fractional carry, and no debt repayment.

- Modify `runner/src/server/wave_executor.rs`
  - Bound response event draining outside the hot scheduling step.
  - Prefer schedule-first, drain-second behavior.
  - Keep response-dependent continuations in `ready`, but do not allow observer event volume to starve new pipeline starts.
  - Emit new lag/backlog metrics.

- Modify `runner/src/server/wave_sender.rs`
  - Split hot request acceptance from response observer completion handling.
  - Ensure request acceptance cannot be delayed by `JoinSet::join_next`.
  - Add tests where observer tasks never finish and sender still accepts all submitted requests.

- Modify `runner/src/server/metrics.rs`
  - Add counters for dispatch-start and scheduler/observer lag.
  - Keep existing counters compatible.

- Modify `runner/src/server/models.rs`
  - Serialize new metrics fields as camelCase.

- Modify `main/src/server/utils.rs`
- Modify `main/src/server/execution/load_batch.rs`
- Modify `main/src/server/models.rs`
  - Parse, aggregate, persist, and return new fields.

- Modify `app/src/types/load-test.ts`
- Modify `app/src/lib/load-rps-chart.ts`
- Modify `app/src/components/LoadTestResultsPanel.tsx`
- Modify `app/src/i18n/locales/pt-BR.json`
- Modify `app/src/i18n/locales/en.json`
  - Show target vs dispatch-start RPS as the primary curve comparison.
  - Show observer backlog, send returned, body completed, and local lag diagnostics separately.

---

### Task 1: Make Dispatch Clock Stop Replaying Missed Time

**Files:**
- Modify: `runner/src/server/load_dispatch.rs`

- [ ] **Step 1: Write the failing clock test**

Add this test to `runner/src/server/load_dispatch.rs`:

```rust
#[test]
fn delayed_tick_records_lag_without_repaying_missed_wall_time() {
    let mut clock = DispatchClock::new(100);

    let first = clock.plan_tick(0, 1000.0);
    assert_eq!(first.scheduled_starts, 100);
    assert_eq!(first.scheduler_lag_ms, 0);
    assert_eq!(first.missed_due_to_scheduler_lag, 0);

    let delayed = clock.plan_tick(500, 1000.0);
    assert_eq!(delayed.scheduled_starts, 100);
    assert_eq!(delayed.scheduler_lag_ms, 400);
    assert_eq!(delayed.missed_due_to_scheduler_lag, 400);
    assert_eq!(delayed.scheduled_total, 200);
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p previa-runner delayed_tick_records_lag_without_repaying_missed_wall_time
```

Expected: FAIL because `DispatchTick` does not expose lag fields and the current clock schedules a catch-up burst.

- [ ] **Step 3: Extend `DispatchTick`**

Update `DispatchTick`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchTick {
    pub elapsed_ms: u64,
    pub target_rps: f64,
    pub scheduled_starts: usize,
    pub scheduled_total: usize,
    pub scheduler_lag_ms: u64,
    pub missed_due_to_scheduler_lag: usize,
}
```

- [ ] **Step 4: Change `plan_tick` semantics**

Replace elapsed-window repayment with fixed tick-window scheduling:

```rust
pub fn plan_tick(&mut self, elapsed_ms: u64, target_rps: f64) -> DispatchTick {
    let expected_elapsed_ms = self.cursor_elapsed_ms;
    let scheduler_lag_ms = elapsed_ms.saturating_sub(expected_elapsed_ms);
    let missed_raw = target_rps.max(0.0) * scheduler_lag_ms as f64 / 1000.0;
    let missed_due_to_scheduler_lag = missed_raw.floor() as usize;

    let raw_slots =
        target_rps.max(0.0) * self.tick_ms as f64 / 1000.0 + self.fractional_carry;
    let scheduled_starts = raw_slots.floor() as usize;
    self.fractional_carry = raw_slots - scheduled_starts as f64;
    self.scheduled_total = self.scheduled_total.saturating_add(scheduled_starts);
    self.cursor_elapsed_ms = elapsed_ms.saturating_add(self.tick_ms);

    DispatchTick {
        elapsed_ms,
        target_rps,
        scheduled_starts,
        scheduled_total: self.scheduled_total,
        scheduler_lag_ms,
        missed_due_to_scheduler_lag,
    }
}
```

- [ ] **Step 5: Update existing tests**

Replace `delayed_tick_schedules_elapsed_wall_time_window` with the new no-debt behavior. Update every `DispatchTick` literal in tests with:

```rust
scheduler_lag_ms: 0,
missed_due_to_scheduler_lag: 0,
```

- [ ] **Step 6: Verify GREEN**

Run:

```bash
cargo test -p previa-runner load_dispatch
```

Expected: PASS.

---

### Task 2: Add Scheduler Lag Metrics

**Files:**
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Write failing metrics test**

Add this test in `runner/src/server/metrics.rs`:

```rust
#[test]
fn snapshot_includes_scheduler_lag_metrics() {
    let mut metrics = MetricsAccumulator::new();

    metrics.record_scheduler_lag_ms(400);
    metrics.record_scheduler_lagged_starts_count(12);

    let snapshot = metrics.snapshot(None, None);

    assert_eq!(snapshot.scheduler_lag_ms, Some(400));
    assert_eq!(snapshot.scheduler_lagged_starts, Some(12));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p previa-runner snapshot_includes_scheduler_lag_metrics
```

Expected: compile failure for missing fields/methods.

- [ ] **Step 3: Add accumulator fields and methods**

In `MetricsAccumulator`, add:

```rust
scheduler_lag_ms: u64,
scheduler_lagged_starts: usize,
```

Initialize both to zero. Add:

```rust
pub fn record_scheduler_lag_ms(&mut self, lag_ms: u64) {
    self.scheduler_lag_ms = self.scheduler_lag_ms.saturating_add(lag_ms);
}

pub fn record_scheduler_lagged_starts_count(&mut self, count: usize) {
    self.scheduler_lagged_starts = self.scheduler_lagged_starts.saturating_add(count);
}
```

- [ ] **Step 4: Serialize the fields**

In `LoadTestMetrics`, add:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub scheduler_lag_ms: Option<u64>,
#[serde(skip_serializing_if = "Option::is_none")]
pub scheduler_lagged_starts: Option<usize>,
```

In `snapshot_with_wave`, set:

```rust
scheduler_lag_ms: (self.scheduler_lag_ms > 0).then_some(self.scheduler_lag_ms),
scheduler_lagged_starts: (self.scheduler_lagged_starts > 0)
    .then_some(self.scheduler_lagged_starts),
```

- [ ] **Step 5: Record lag from dispatch ticks**

In `runner/src/server/wave_executor.rs`, immediately after `plan_tick`:

```rust
if tick.scheduler_lag_ms > 0 || tick.missed_due_to_scheduler_lag > 0 {
    missed_starts.fetch_add(tick.missed_due_to_scheduler_lag, Ordering::SeqCst);
    let mut lock = metrics.lock().await;
    lock.record_scheduler_lag_ms(tick.scheduler_lag_ms);
    lock.record_scheduler_lagged_starts_count(tick.missed_due_to_scheduler_lag);
}
```

- [ ] **Step 6: Verify GREEN**

Run:

```bash
cargo test -p previa-runner snapshot_includes_scheduler_lag_metrics
cargo test -p previa-runner load_dispatch
```

Expected: PASS.

---

### Task 3: Bound Observer Event Drain Outside The Scheduler Hot Step

**Files:**
- Modify: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Write failing drain-budget test**

Add a unit test in `wave_executor.rs`:

```rust
#[tokio::test]
async fn observer_drain_respects_per_tick_budget() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ObserverEvent>();
    let mut ready = VecDeque::new();
    let pipeline = Pipeline {
        id: Some("p".to_owned()),
        name: "pipeline".to_owned(),
        description: None,
        steps: Vec::new(),
    };
    let metrics = Arc::new(tokio::sync::Mutex::new(MetricsAccumulator::new()));

    for _ in 0..3 {
        tx.send(WaveObserverEvent {
            cursor: PipelineCursor::new(Instant::now()),
            result: StepExecutionResult {
                step_id: "missing".to_owned(),
                status: "error".to_owned(),
                request: None,
                response: None,
                error: Some("synthetic".to_owned()),
                duration: Some(0),
                attempts: None,
                attempt: Some(1),
                max_attempts: Some(1),
                assert_results: None,
            },
        })
        .unwrap();
    }

    let drained = drain_observer_events_budgeted(
        &mut rx,
        &mut ready,
        &pipeline,
        &metrics,
        2,
    )
    .await;

    assert_eq!(drained, 2);
    assert!(rx.try_recv().is_ok(), "one event should remain for a later tick");
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p previa-runner observer_drain_respects_per_tick_budget
```

Expected: compile failure because `drain_observer_events_budgeted` does not exist.

- [ ] **Step 3: Implement budgeted drain**

Replace `drain_observer_events` with:

```rust
const OBSERVER_EVENTS_PER_TICK_BUDGET: usize = 1024;

async fn drain_observer_events_budgeted(
    event_rx: &mut mpsc::UnboundedReceiver<ObserverEvent>,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metrics: &Arc<tokio::sync::Mutex<MetricsAccumulator>>,
    budget: usize,
) -> usize {
    let mut drained = 0usize;
    while drained < budget {
        let Ok(event) = event_rx.try_recv() else {
            break;
        };
        handle_step_result(event.result, event.cursor, ready, pipeline, metrics).await;
        drained += 1;
    }
    drained
}
```

- [ ] **Step 4: Drain after scheduling, not before scheduling**

Remove the pre-tick drain from the top of the wave loop:

```rust
drain_observer_events(&mut event_rx, &mut ready, &pipeline, &metrics).await;
```

After the `for _ in 0..tick.scheduled_starts` dispatch loop and before the metrics snapshot, add:

```rust
drain_observer_events_budgeted(
    &mut event_rx,
    &mut ready,
    &pipeline,
    &metrics,
    OBSERVER_EVENTS_PER_TICK_BUDGET,
)
.await;
```

This preserves schedule-first behavior: the next tick is planned and submitted before response feedback is processed. During grace/shutdown, use a larger budget or a full drain because the load phase has ended:

```rust
while drain_observer_events_budgeted(
    &mut event_rx,
    &mut ready,
    &pipeline,
    &metrics,
    OBSERVER_EVENTS_PER_TICK_BUDGET,
)
.await
    > 0
{}
```

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test -p previa-runner observer_drain_respects_per_tick_budget
cargo test -p previa-runner wave_executor
```

Expected: PASS.

---

### Task 4: Remove Response Join Competition From Sender Hot Path

**Files:**
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Write failing sender stress test**

Add this test to `wave_sender.rs`:

```rust
#[tokio::test]
async fn sender_accepts_many_requests_even_when_observers_never_finish() {
    let (tx, rx) = mpsc::unbounded_channel();
    let started = Arc::new(AtomicUsize::new(0));
    let blocker = Arc::new(Notify::new());

    let sender_started = Arc::clone(&started);
    let sender_blocker = Arc::clone(&blocker);
    let sender = tokio::spawn(run_test_sender(
        rx,
        sender_started,
        move |_payload: usize| {
            let blocker = Arc::clone(&sender_blocker);
            async move {
                blocker.notified().await;
            }
        },
    ));

    for payload in 0..10_000 {
        tx.send(TestReadyWaveRequest { payload }).unwrap();
    }

    timeout(Duration::from_secs(1), async {
        loop {
            if started.load(Ordering::SeqCst) == 10_000 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("sender hot path should accept requests even with blocked observers");

    drop(tx);
    sender.abort();
}
```

- [ ] **Step 2: Run the test and verify RED or timing sensitivity**

Run:

```bash
cargo test -p previa-runner sender_accepts_many_requests_even_when_observers_never_finish
```

Expected before implementation: this may pass for the test harness but the real sender still has `JoinSet` competition. If it passes, add a real-sender-focused test in the same file before changing production code.

- [ ] **Step 3: Split request acceptance from observer supervision**

Change `WaveSender::run` so the hot receive loop only receives requests and calls `spawn_observer`. Do not call `tasks.join_next()` in the same select that receives new work.

Preferred minimal shape:

```rust
pub async fn run(mut self) {
    while let Some(request) = self.request_rx.recv().await {
        self.ready_to_send.fetch_sub(1, Ordering::SeqCst);
        if self.token.is_cancelled() {
            break;
        }
        self.spawn_observer(request);
    }
}
```

Then change `spawn_observer` to use `tokio::spawn` directly:

```rust
fn spawn_observer(&self, request: ReadyWaveRequest<C>) {
    self.response_in_flight.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        // existing observer body
    });
}
```

The observer task must decrement `response_in_flight` in all normal return paths.

- [ ] **Step 4: Preserve shutdown safety**

If detached observer tasks can remain after grace, make cancellation explicit from `run_wave_load` by using a child token:

```rust
let observer_token = token.child_token();
```

Pass `observer_token.clone()` into `WaveSender`. After the grace deadline:

```rust
if response_in_flight.load(Ordering::SeqCst) > 0 {
    observer_token.cancel();
}
```

Then wait until `response_in_flight == 0` for a short bounded interval before final metrics.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test -p previa-runner wave_sender
```

Expected: PASS.

---

### Task 5: Add Dispatch-Started Metric And Make It The Primary RPS Line

**Files:**
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/wave_sender.rs`
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `main/src/server/models.rs`
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/load-rps-chart.ts`
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Write failing metric test**

Add to `runner/src/server/metrics.rs`:

```rust
#[test]
fn snapshot_includes_dispatch_started_counter() {
    let mut metrics = MetricsAccumulator::new();

    metrics.record_dispatch_started();
    metrics.record_dispatch_started();

    let snapshot = metrics.snapshot(None, None);

    assert_eq!(snapshot.dispatch_started, Some(2));
}
```

- [ ] **Step 2: Run test and verify RED**

Run:

```bash
cargo test -p previa-runner snapshot_includes_dispatch_started_counter
```

Expected: compile failure for missing field/method.

- [ ] **Step 3: Add metric**

Add to `MetricsAccumulator`:

```rust
dispatch_started: usize,
```

Add method:

```rust
pub fn record_dispatch_started(&mut self) {
    self.dispatch_started = self.dispatch_started.saturating_add(1);
}
```

Add to `LoadTestMetrics`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub dispatch_started: Option<usize>,
```

- [ ] **Step 4: Record it in sender hot path**

In `WaveSender::spawn_observer`, before `record_http_start()` or replacing it if the names are consolidated:

```rust
let mut lock = metrics.lock().await;
lock.record_dispatch_started();
lock.record_http_start();
```

Keep `http_started` for backward compatibility during this feature. The chart should prefer `dispatchStarted` when present and fall back to `httpStarted`.

- [ ] **Step 5: Aggregate through main**

Where `httpStarted` is parsed and summed, add `dispatchStarted`.

In `main/src/server/utils.rs`:

```rust
dispatch_started: get_usize_field(payload, "dispatchStarted"),
```

In `main/src/server/execution/load_batch.rs`, sum runner values and insert `dispatchStarted` into runner and consolidated samples.

- [ ] **Step 6: Update chart calculation**

In `app/src/lib/load-rps-chart.ts`, prefer `dispatchStarted`:

```ts
const usesHttpRps = history.some((point) =>
  typeof point.dispatchStarted === "number"
  || typeof point.httpStarted === "number"
  || point.runners?.some((runner) =>
    typeof runner.dispatchStarted === "number" || typeof runner.httpStarted === "number"
  ),
);
```

For deltas:

```ts
const currentStarted = runner.dispatchStarted ?? runner.httpStarted;
const previousStarted = previousRunner.dispatchStarted ?? previousRunner.httpStarted;
```

- [ ] **Step 7: Verify chart tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel
```

Expected: PASS after updating expected fixtures.

---

### Task 6: Add Open-Loop Failure-Mode Tests

**Files:**
- Modify: `runner/src/server/wave_sender.rs`
- Modify: `runner/src/server/load_dispatch.rs`
- Modify: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Add test for target responses not controlling sender start**

In `wave_sender.rs`, add a test that submits N requests while every observer future blocks forever. Assert all N are started.

Use the `sender_accepts_many_requests_even_when_observers_never_finish` test from Task 4 as the final regression.

- [ ] **Step 2: Add test for failed target responses not changing clock math**

In `load_dispatch.rs`, add:

```rust
#[test]
fn dispatch_clock_is_independent_from_failures_by_design() {
    let mut clock = DispatchClock::new(100);

    let a = clock.plan_tick(0, 1000.0);
    let b = clock.plan_tick(100, 1000.0);
    let c = clock.plan_tick(200, 1000.0);

    assert_eq!(a.scheduled_starts, 100);
    assert_eq!(b.scheduled_starts, 100);
    assert_eq!(c.scheduled_starts, 100);
    assert_eq!(c.scheduled_total, 300);
}
```

This test has no response input because response input must not exist in the clock API.

- [ ] **Step 3: Add test for continuation fallback**

Keep or extend the existing `next_cursor_prefers_ready_continuations_before_starting_new_pipeline` test with the inverse:

```rust
#[test]
fn next_cursor_starts_new_pipeline_when_no_continuation_is_ready() {
    let mut ready = VecDeque::new();
    let cursor = next_cursor_for_slot(&mut ready, || PipelineCursor::new(Instant::now()));

    assert_eq!(cursor.step_index, 0);
    assert_eq!(cursor.attempt, 1);
    assert!(cursor.context.is_empty());
}
```

- [ ] **Step 4: Verify focused tests**

Run:

```bash
cargo test -p previa-runner wave_sender
cargo test -p previa-runner load_dispatch
cargo test -p previa-runner next_cursor
```

Expected: PASS.

---

### Task 7: Surface Open-Loop Diagnostics In Main And UI

**Files:**
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `main/src/server/models.rs`
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Modify: `app/src/i18n/locales/pt-BR.json`
- Modify: `app/src/i18n/locales/en.json`
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Add fields to shared app type**

In `app/src/types/load-test.ts`, add optional fields wherever `httpStarted`, `runtimeLaggedStarts`, and `readyRequests` exist:

```ts
dispatchStarted?: number;
schedulerLagMs?: number;
schedulerLaggedStarts?: number;
observerBacklog?: number;
```

Use `outstandingRequests` as the backend source for `observerBacklog` unless a separate backend field is added.

- [ ] **Step 2: Add labels**

In `pt-BR.json`:

```json
"loadTestResults.dispatchStarted": "Disparos iniciados",
"loadTestResults.schedulerLagMs": "Atraso do agendador",
"loadTestResults.schedulerLaggedStarts": "Disparos perdidos por atraso",
"loadTestResults.observerBacklog": "Respostas em observacao"
```

In `en.json`:

```json
"loadTestResults.dispatchStarted": "Dispatch started",
"loadTestResults.schedulerLagMs": "Scheduler lag",
"loadTestResults.schedulerLaggedStarts": "Starts missed by lag",
"loadTestResults.observerBacklog": "Response observer backlog"
```

- [ ] **Step 3: Show diagnostics**

In `LoadTestResultsPanel.tsx`, place the new cards near existing wave diagnostics:

```tsx
{typeof metrics.dispatchStarted === "number" && (
  <MetricCard icon={Activity} label={t("loadTestResults.dispatchStarted")} value={metrics.dispatchStarted} />
)}
{typeof metrics.schedulerLagMs === "number" && (
  <MetricCard icon={TimerOff} label={t("loadTestResults.schedulerLagMs")} value={`${metrics.schedulerLagMs} ms`} />
)}
{typeof metrics.schedulerLaggedStarts === "number" && (
  <MetricCard icon={AlertTriangle} label={t("loadTestResults.schedulerLaggedStarts")} value={metrics.schedulerLaggedStarts} />
)}
```

- [ ] **Step 4: Update UI tests**

Add fixture metrics:

```ts
dispatchStarted: 120,
schedulerLagMs: 400,
schedulerLaggedStarts: 12,
outstandingRequests: 90,
```

Assert the labels render.

- [ ] **Step 5: Verify UI**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel
npm --prefix app run build
```

Expected: PASS.

---

### Task 8: End-To-End Verification Against A Failing Target

**Files:**
- No production files required unless verification reveals a bug.

- [ ] **Step 1: Run backend tests**

Run:

```bash
cargo test -p previa-runner
cargo test -p previa-main
```

Expected: PASS.

- [ ] **Step 2: Run frontend tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel LoadTestConfigPanel LoadTestTab
npm --prefix app run build
```

Expected: PASS.

- [ ] **Step 3: Run release build**

Run:

```bash
cargo build --release
```

Expected: PASS.

- [ ] **Step 4: Restart local main and three runners**

Use the existing local release startup process for ports `5610`, `5611`, `5612`, and `5613`.

Confirm:

```bash
curl -s http://127.0.0.1:5610/info | jq '{activeRunners,totalRunners}'
```

Expected:

```json
{
  "activeRunners": 3,
  "totalRunners": 3
}
```

- [ ] **Step 5: Execute the CRUD Users load test against the failing endpoint**

Run the load test from:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Use the same wave profile that previously degraded around 60-100 seconds.

- [ ] **Step 6: Analyze raw history**

Fetch the latest history record and compare:

```bash
curl -s 'http://127.0.0.1:5610/api/v1/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/tests/load?limit=1' | jq '.[0] | {executionId,status,requestedConfig}'
```

Then fetch the full record and compute per-window:

- target RPS
- dispatch-start RPS
- send-returned RPS
- response-body completion rate
- scheduler lag
- observer backlog
- target errors

Expected diagnosis:

- If `dispatchStartedRps` tracks `targetRps` while `502/503/error sending request` rise, the application/target is failing and the wave algorithm is correct.
- If `dispatchStartedRps` falls while `schedulerLagMs`, `schedulerLaggedStarts`, CPU, memory, socket errors, or ready queue rise, the generator infrastructure is the bottleneck.
- The runner must not show catch-up bursts caused by repaid missed time.

---

## Final Verification Checklist

- [ ] `cargo test -p previa-runner`
- [ ] `cargo test -p previa-main`
- [ ] `npm --prefix app test -- LoadTestResultsPanel LoadTestConfigPanel LoadTestTab`
- [ ] `npm --prefix app run build`
- [ ] `cargo build --release`
- [ ] Failing-target load test analyzed from raw API history
- [ ] No catch-up burst after scheduler lag
- [ ] Target errors do not reduce dispatch-start scheduling unless local infra is saturated

---

## Commit Plan

Use small commits:

```bash
git add runner/src/server/load_dispatch.rs
git commit -m "fix: stop wave dispatch from replaying missed time"

git add runner/src/server/metrics.rs runner/src/server/models.rs runner/src/server/wave_executor.rs
git commit -m "feat: add wave scheduler lag diagnostics"

git add runner/src/server/wave_sender.rs runner/src/server/wave_executor.rs
git commit -m "refactor: isolate wave sender hot path from observers"

git add main/src/server/utils.rs main/src/server/execution/load_batch.rs main/src/server/models.rs app/src/types/load-test.ts app/src/lib/load-rps-chart.ts app/src/components/LoadTestResultsPanel.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json
git commit -m "feat: surface open-loop wave diagnostics"
```

After the final release build succeeds, push the branch:

```bash
git push origin codex/wave-load-test
```
