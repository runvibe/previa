# Dedicated Wave Dispatcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move wave slot consumption and request preparation into a dedicated dispatcher thread/runtime so the configured wave is not delayed by HTTP response observers, SSE snapshots, or the main Tokio runtime.

**Architecture:** Keep the current dedicated `previa-wave-clock` thread as the source of `WaveDispatchSlot`s. Add a second dedicated `previa-wave-dispatcher` thread with a single-thread Tokio runtime that owns pipeline cursors, continuation queues, observer-event draining, slot expiration checks, and `prepare_http_step`; it outputs `ReadyWaveRequest`s to the existing `WaveSender`. The main `run_wave_load` loop becomes orchestration plus metrics/SSE snapshots, reading lightweight dispatcher state instead of doing hot-path dispatch work itself.

**Tech Stack:** Rust, Tokio current-thread runtime, Tokio `mpsc`, `watch`, `CancellationToken`, `std::thread`, existing `WaveDispatchSlot`, existing `WaveSender`, existing `WaveMetricEvent` diagnostics.

---

## Correctness Contract

- The wave clock remains isolated in `previa-wave-clock`.
- The dispatcher must run outside the main Tokio runtime in its own named OS thread.
- The dispatcher is the only owner of `VecDeque<PipelineCursor>` and all pipeline continuation state.
- The main load loop must not call `prepare_http_step`, must not consume `WaveDispatchSlot` directly, and must not drain observer events.
- The dispatcher must preserve current open-loop behavior:
  - fresh slots create new starts or ready continuations;
  - expired slots are dropped and counted as `DispatcherLaggedStarts`;
  - response errors do not block future first-step starts;
  - dependency-limited continuations still increment `DependencyLimitedStarts`;
  - no late catch-up burst is sent after a slot expires.
- `dispatchStarted` remains emitted by `WaveSender` when a prepared request is accepted for HTTP start.
- `dispatcherLaggedStarts` should drop materially after this work. If it remains high, the runner host is saturated at request preparation or channel transfer, not at the clock.

## File Structure

- Create `runner/src/server/wave_dispatcher.rs`
  - Owns `PipelineCursor`.
  - Owns `next_cursor_for_slot`, `dispatch_slot_requests`, observer draining, `handle_step_result`, `handle_prepare_error`, `record_terminal_pipeline`, and `max_attempts_for_step`.
  - Exposes `spawn_wave_dispatcher_thread(...) -> WaveDispatcherHandle`.
  - Publishes a `WaveDispatcherSnapshot` through a `watch::Sender`.

- Modify `runner/src/server/mod.rs`
  - Register `pub mod wave_dispatcher;`.

- Modify `runner/src/server/wave_executor.rs`
  - Remove dispatcher/cursor code.
  - Spawn `spawn_wave_dispatcher_thread(...)`.
  - Use dispatcher snapshot values for `ready_requests` in SSE metrics.
  - Keep main responsibilities limited to actor orchestration, runtime sampling, metrics snapshots, grace-period response observation, and shutdown.

- Modify `runner/src/server/wave_sender.rs`
  - No functional change expected.
  - Type parameter `PipelineCursor` will come from `wave_dispatcher`.

- Modify tests in:
  - `runner/src/server/wave_dispatcher.rs`
  - `runner/src/server/wave_executor.rs`

---

### Task 1: Extract Dispatcher Types and Pure Queue Helpers

**Files:**
- Create: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/mod.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_dispatcher.rs`

- [ ] **Step 1: Create the new dispatcher module with cursor and snapshot types**

Create `runner/src/server/wave_dispatcher.rs`:

```rust
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tokio::sync::{mpsc, watch};
use tracing::error;

use previa_runner::{
    Pipeline, PipelineStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult, prepare_http_step,
};

use crate::server::wave_emitter::{StartLagClass, classify_start_lag};
use crate::server::wave_metrics_actor::WaveMetricEvent;
use crate::server::wave_scheduler::{WaveDispatchSlot, slot_is_expired};
use crate::server::wave_sender::{ReadyWaveRequest, WaveObserverEvent};

#[derive(Debug)]
pub struct PipelineCursor {
    pub step_index: usize,
    pub attempt: usize,
    pub context: HashMap<String, StepExecutionResult>,
    pub pipeline_started_at: Instant,
}

impl PipelineCursor {
    pub fn new(started_at: Instant) -> Self {
        Self {
            step_index: 0,
            attempt: 1,
            context: HashMap::new(),
            pipeline_started_at: started_at,
        }
    }
}

pub type ObserverEvent = WaveObserverEvent<PipelineCursor>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WaveDispatcherSnapshot {
    pub ready_continuations: usize,
    pub ready_to_send: usize,
}

impl WaveDispatcherSnapshot {
    pub fn ready_requests(self) -> usize {
        self.ready_continuations.saturating_add(self.ready_to_send)
    }
}

const OBSERVER_EVENTS_PER_TICK_BUDGET: usize = 1024;

pub fn next_cursor_for_slot(
    ready: &mut VecDeque<PipelineCursor>,
    create: impl FnOnce() -> PipelineCursor,
) -> PipelineCursor {
    ready.pop_front().unwrap_or_else(create)
}
```

- [ ] **Step 2: Register the module**

In `runner/src/server/mod.rs`, add:

```rust
pub mod wave_dispatcher;
```

- [ ] **Step 3: Move queue helper tests into the new module**

Add this test module to `runner/src/server/wave_dispatcher.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_cursor_prefers_ready_continuations_before_starting_new_pipeline() {
        let mut ready = VecDeque::new();
        ready.push_back(PipelineCursor {
            step_index: 2,
            attempt: 1,
            context: HashMap::new(),
            pipeline_started_at: Instant::now(),
        });
        let mut started_new = false;

        let cursor = next_cursor_for_slot(&mut ready, || {
            started_new = true;
            PipelineCursor::new(Instant::now())
        });

        assert_eq!(cursor.step_index, 2);
        assert!(!started_new);
    }

    #[test]
    fn next_cursor_starts_new_pipeline_when_no_continuation_is_ready() {
        let mut ready = VecDeque::new();
        let cursor = next_cursor_for_slot(&mut ready, || PipelineCursor::new(Instant::now()));

        assert_eq!(cursor.step_index, 0);
        assert_eq!(cursor.attempt, 1);
        assert!(cursor.context.is_empty());
    }

    #[test]
    fn dispatcher_snapshot_counts_ready_continuations_and_ready_to_send() {
        let snapshot = WaveDispatcherSnapshot {
            ready_continuations: 7,
            ready_to_send: 5,
        };

        assert_eq!(snapshot.ready_requests(), 12);
    }
}
```

- [ ] **Step 4: Run and confirm the new module tests pass**

Run:

```bash
cargo test -p previa-runner wave_dispatcher::tests::next_cursor
cargo test -p previa-runner wave_dispatcher::tests::dispatcher_snapshot_counts_ready_continuations_and_ready_to_send
```

Expected: PASS after the module is registered.

---

### Task 2: Move Observer Draining and Step Result Handling

**Files:**
- Modify: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_dispatcher.rs`

- [ ] **Step 1: Move result-drain functions into `wave_dispatcher.rs`**

Add these functions below `next_cursor_for_slot`:

```rust
pub async fn drain_observer_events_budgeted(
    event_rx: &mut mpsc::UnboundedReceiver<ObserverEvent>,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
    budget: usize,
) -> usize {
    let mut drained = 0usize;
    while drained < budget {
        let Ok(event) = event_rx.try_recv() else {
            break;
        };
        handle_step_result(event.result, event.cursor, ready, pipeline, metric_tx).await;
        drained += 1;
    }
    drained
}

pub async fn drain_all_observer_events(
    event_rx: &mut mpsc::UnboundedReceiver<ObserverEvent>,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
) {
    while drain_observer_events_budgeted(
        event_rx,
        ready,
        pipeline,
        metric_tx,
        OBSERVER_EVENTS_PER_TICK_BUDGET,
    )
    .await
        > 0
    {}
}

async fn handle_step_result(
    result: StepExecutionResult,
    mut cursor: PipelineCursor,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
) {
    let Some(step) = pipeline.steps.get(cursor.step_index) else {
        record_terminal_pipeline(metric_tx, cursor, false, Some(&result)).await;
        return;
    };
    let max_attempts = max_attempts_for_step(step);

    if result.status == "error" && cursor.attempt < max_attempts {
        cursor.attempt += 1;
        ready.push_back(cursor);
        return;
    }

    if result.status == "error" {
        record_terminal_pipeline(metric_tx, cursor, false, Some(&result)).await;
        return;
    }

    cursor.context.insert(result.step_id.clone(), result);
    cursor.step_index += 1;
    cursor.attempt = 1;

    if cursor.step_index >= pipeline.steps.len() {
        record_terminal_pipeline(metric_tx, cursor, true, None).await;
    } else {
        ready.push_back(cursor);
    }
}

async fn record_terminal_pipeline(
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
    cursor: PipelineCursor,
    success: bool,
    result: Option<&StepExecutionResult>,
) {
    let duration_ms = cursor.pipeline_started_at.elapsed().as_millis() as f64;
    let _ = metric_tx.send(WaveMetricEvent::PipelineFinished {
        duration_ms,
        success,
    });
    if !success {
        if let Some(result) = result {
            let http_status = result.response.as_ref().map(|response| response.status);
            let error = result.error.as_deref().unwrap_or("pipeline failed");
            let _ = metric_tx.send(WaveMetricEvent::ErrorSample {
                step_id: result.step_id.clone(),
                http_status,
                error: error.to_owned(),
            });
        }
    }
}

fn max_attempts_for_step(step: &PipelineStep) -> usize {
    step.retry.unwrap_or(0).saturating_add(1)
}
```

- [ ] **Step 2: Move the observer budget test**

Move `observer_drain_respects_per_tick_budget` from `runner/src/server/wave_executor.rs` to `runner/src/server/wave_dispatcher.rs`.

Use this exact pipeline in the test:

```rust
let pipeline = Pipeline {
    id: Some("p".to_owned()),
    name: "pipeline".to_owned(),
    description: None,
    steps: Vec::new(),
};
```

Use this event payload:

```rust
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
```

- [ ] **Step 3: Run the moved test**

Run:

```bash
cargo test -p previa-runner observer_drain_respects_per_tick_budget
```

Expected: PASS.

---

### Task 3: Move Slot Dispatching Into the Dispatcher Module

**Files:**
- Modify: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_dispatcher.rs`

- [ ] **Step 1: Add `DispatchSlotRequestArgs` and `dispatch_slot_requests` to `wave_dispatcher.rs`**

Add this code below `drain_all_observer_events`:

```rust
pub struct DispatchSlotRequestArgs<'a> {
    pub slot: WaveDispatchSlot,
    pub ready: &'a mut VecDeque<PipelineCursor>,
    pub pipeline: &'a Pipeline,
    pub specs: &'a Arc<Vec<RuntimeSpec>>,
    pub env_groups: &'a Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: &'a Option<String>,
    pub request_tx: &'a mpsc::UnboundedSender<ReadyWaveRequest<PipelineCursor>>,
    pub metric_tx: &'a mpsc::UnboundedSender<WaveMetricEvent>,
    pub ready_to_send: &'a Arc<AtomicUsize>,
    pub missed_starts: &'a Arc<AtomicUsize>,
    pub started: Instant,
    pub tick_ms: u64,
    pub token: &'a tokio_util::sync::CancellationToken,
}

pub async fn dispatch_slot_requests(args: DispatchSlotRequestArgs<'_>) {
    let actual_elapsed_ms = args.started.elapsed().as_millis() as u64;
    if slot_is_expired(&args.slot, actual_elapsed_ms) {
        args.missed_starts
            .fetch_add(args.slot.planned_starts, Ordering::SeqCst);
        let _ = args.metric_tx.send(WaveMetricEvent::DispatcherLaggedStarts(
            args.slot.planned_starts,
        ));
        return;
    }

    for _ in 0..args.slot.planned_starts {
        if args.token.is_cancelled() {
            break;
        }

        let was_ready_empty = args.ready.is_empty();
        let cursor = next_cursor_for_slot(args.ready, || PipelineCursor::new(Instant::now()));
        if was_ready_empty && cursor.step_index == 0 && cursor.context.is_empty() {
            let _ = args.metric_tx.send(WaveMetricEvent::PipelineStarted);
        }

        let Some(step) = args.pipeline.steps.get(cursor.step_index).cloned() else {
            record_terminal_pipeline(args.metric_tx, cursor, false, None).await;
            continue;
        };
        let max_attempts = max_attempts_for_step(&step);
        let prepared = match prepare_http_step(
            &step,
            &cursor.context,
            Some(args.specs.as_slice()),
            Some(args.env_groups.as_slice()),
            args.selected_env_group_slug.as_deref(),
            cursor.attempt,
            max_attempts,
        ) {
            Ok(prepared) => prepared,
            Err(result) => {
                handle_prepare_error(
                    result,
                    cursor,
                    args.ready,
                    args.pipeline,
                    args.metric_tx,
                    args.missed_starts,
                )
                .await;
                continue;
            }
        };

        let actual_elapsed_ms = args.started.elapsed().as_millis() as u64;
        if classify_start_lag(args.slot.elapsed_ms, actual_elapsed_ms, args.tick_ms)
            == StartLagClass::RuntimeLagged
        {
            args.missed_starts.fetch_add(1, Ordering::SeqCst);
            let _ = args.metric_tx.send(WaveMetricEvent::RuntimeLaggedStart);
        }

        args.ready_to_send.fetch_add(1, Ordering::SeqCst);
        if args
            .request_tx
            .send(ReadyWaveRequest {
                step,
                context: cursor.context.clone(),
                cursor,
                prepared,
                specs: Arc::clone(args.specs),
                env_groups: Arc::clone(args.env_groups),
                selected_env_group_slug: args.selected_env_group_slug.clone(),
            })
            .is_err()
        {
            args.ready_to_send.fetch_sub(1, Ordering::SeqCst);
            error!("wave sender stopped before accepting prepared request");
            break;
        }
    }
}

async fn handle_prepare_error(
    result: StepExecutionResult,
    mut cursor: PipelineCursor,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
    missed_starts: &Arc<AtomicUsize>,
) {
    let max_attempts = pipeline
        .steps
        .get(cursor.step_index)
        .map(max_attempts_for_step)
        .unwrap_or(1);

    if cursor.attempt < max_attempts {
        cursor.attempt += 1;
        ready.push_back(cursor);
        return;
    }

    if cursor.step_index > 0 {
        missed_starts.fetch_add(1, Ordering::SeqCst);
        let _ = metric_tx.send(WaveMetricEvent::DependencyLimitedStarts(1));
    }
    record_terminal_pipeline(metric_tx, cursor, false, Some(&result)).await;
}
```

- [ ] **Step 2: Move expired-slot test into `wave_dispatcher.rs`**

Move `expired_dispatch_slot_records_lag_without_enqueuing_requests` from `runner/src/server/wave_executor.rs` to `runner/src/server/wave_dispatcher.rs`.

Use this exact pipeline:

```rust
let pipeline = Pipeline {
    id: Some("p".to_owned()),
    name: "pipeline".to_owned(),
    description: None,
    steps: Vec::new(),
};
```

- [ ] **Step 3: Run dispatcher tests**

Run:

```bash
cargo test -p previa-runner wave_dispatcher
```

Expected: PASS.

---

### Task 4: Add the Dedicated Dispatcher Thread Runtime

**Files:**
- Modify: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_dispatcher.rs`

- [ ] **Step 1: Add dispatcher config and handle types**

Add these types to `runner/src/server/wave_dispatcher.rs`:

```rust
pub struct WaveDispatcherConfig {
    pub pipeline: Arc<Pipeline>,
    pub specs: Arc<Vec<RuntimeSpec>>,
    pub env_groups: Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: Option<String>,
    pub started: Instant,
    pub tick_ms: u64,
}

pub struct WaveDispatcherChannels {
    pub slot_rx: mpsc::Receiver<WaveDispatchSlot>,
    pub request_tx: mpsc::UnboundedSender<ReadyWaveRequest<PipelineCursor>>,
    pub observer_rx: mpsc::UnboundedReceiver<ObserverEvent>,
    pub metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    pub snapshot_tx: watch::Sender<WaveDispatcherSnapshot>,
}

pub struct WaveDispatcherShared {
    pub ready_to_send: Arc<AtomicUsize>,
    pub missed_starts: Arc<AtomicUsize>,
}

pub struct WaveDispatcherHandle {
    token: tokio_util::sync::CancellationToken,
    join: std::thread::JoinHandle<()>,
}

impl WaveDispatcherHandle {
    pub fn stop(self) {
        self.token.cancel();
        if let Err(err) = self.join.join() {
            error!("wave dispatcher thread panicked: {:?}", err);
        }
    }
}
```

- [ ] **Step 2: Add `publish_dispatcher_snapshot`**

Add:

```rust
fn publish_dispatcher_snapshot(
    snapshot_tx: &watch::Sender<WaveDispatcherSnapshot>,
    ready: &VecDeque<PipelineCursor>,
    ready_to_send: &Arc<AtomicUsize>,
) {
    let _ = snapshot_tx.send(WaveDispatcherSnapshot {
        ready_continuations: ready.len(),
        ready_to_send: ready_to_send.load(Ordering::SeqCst),
    });
}
```

- [ ] **Step 3: Add async dispatcher loop**

Add:

```rust
pub async fn run_wave_dispatcher_loop(
    config: WaveDispatcherConfig,
    mut channels: WaveDispatcherChannels,
    shared: WaveDispatcherShared,
    token: tokio_util::sync::CancellationToken,
) {
    let mut ready = VecDeque::new();

    while !token.is_cancelled() {
        tokio::select! {
            biased;

            Some(slot) = channels.slot_rx.recv() => {
                dispatch_slot_requests(DispatchSlotRequestArgs {
                    slot,
                    ready: &mut ready,
                    pipeline: &config.pipeline,
                    specs: &config.specs,
                    env_groups: &config.env_groups,
                    selected_env_group_slug: &config.selected_env_group_slug,
                    request_tx: &channels.request_tx,
                    metric_tx: &channels.metric_tx,
                    ready_to_send: &shared.ready_to_send,
                    missed_starts: &shared.missed_starts,
                    started: config.started,
                    tick_ms: config.tick_ms,
                    token: &token,
                })
                .await;

                drain_observer_events_budgeted(
                    &mut channels.observer_rx,
                    &mut ready,
                    &config.pipeline,
                    &channels.metric_tx,
                    OBSERVER_EVENTS_PER_TICK_BUDGET,
                )
                .await;

                publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
            }
            Some(event) = channels.observer_rx.recv() => {
                handle_step_result(event.result, event.cursor, &mut ready, &config.pipeline, &channels.metric_tx).await;
                publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
            }
            else => break,
        }
    }

    drain_all_observer_events(
        &mut channels.observer_rx,
        &mut ready,
        &config.pipeline,
        &channels.metric_tx,
    )
    .await;
    publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
}
```

- [ ] **Step 4: Add thread spawn function**

Add:

```rust
pub fn spawn_wave_dispatcher_thread(
    config: WaveDispatcherConfig,
    channels: WaveDispatcherChannels,
    shared: WaveDispatcherShared,
    token: tokio_util::sync::CancellationToken,
) -> WaveDispatcherHandle {
    let dispatcher_token = token.child_token();
    let thread_token = dispatcher_token.clone();
    let join = std::thread::Builder::new()
        .name("previa-wave-dispatcher".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build previa wave dispatcher runtime");
            runtime.block_on(run_wave_dispatcher_loop(
                config,
                channels,
                shared,
                thread_token,
            ));
        })
        .expect("failed to spawn previa wave dispatcher thread");

    WaveDispatcherHandle {
        token: dispatcher_token,
        join,
    }
}
```

- [ ] **Step 5: Add focused dispatcher-thread test**

Add this test:

```rust
#[tokio::test]
async fn dispatcher_thread_consumes_fresh_slot_and_enqueues_ready_request() {
    let pipeline = Arc::new(Pipeline {
        id: Some("p".to_owned()),
        name: "pipeline".to_owned(),
        description: None,
        steps: vec![PipelineStep {
            id: "s1".to_owned(),
            name: "GET".to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: "http://example.test/users".to_owned(),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            asserts: Vec::new(),
        }],
    });
    let specs = Arc::new(Vec::new());
    let env_groups = Arc::new(Vec::new());
    let (slot_tx, slot_rx) = mpsc::channel(8);
    let (request_tx, mut request_rx) = mpsc::unbounded_channel();
    let (observer_tx, observer_rx) = mpsc::unbounded_channel();
    drop(observer_tx);
    let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
    let (snapshot_tx, mut snapshot_rx) = watch::channel(WaveDispatcherSnapshot::default());
    let ready_to_send = Arc::new(AtomicUsize::new(0));
    let missed_starts = Arc::new(AtomicUsize::new(0));
    let token = tokio_util::sync::CancellationToken::new();

    let handle = spawn_wave_dispatcher_thread(
        WaveDispatcherConfig {
            pipeline,
            specs,
            env_groups,
            selected_env_group_slug: None,
            started: Instant::now(),
            tick_ms: 100,
        },
        WaveDispatcherChannels {
            slot_rx,
            request_tx,
            observer_rx,
            metric_tx,
            snapshot_tx,
        },
        WaveDispatcherShared {
            ready_to_send: Arc::clone(&ready_to_send),
            missed_starts: Arc::clone(&missed_starts),
        },
        token,
    );

    slot_tx
        .send(WaveDispatchSlot {
            elapsed_ms: 0,
            expires_at_elapsed_ms: 1_000,
            planned_starts: 1,
            target_rps_limit: 10.0,
            scheduled_total: 1,
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
        })
        .await
        .unwrap();

    let request = tokio::time::timeout(std::time::Duration::from_millis(300), request_rx.recv())
        .await
        .expect("dispatcher should enqueue request")
        .expect("request channel should stay open");

    assert_eq!(request.step.id, "s1");
    assert_eq!(ready_to_send.load(Ordering::SeqCst), 1);

    snapshot_rx.changed().await.unwrap();
    assert_eq!(snapshot_rx.borrow().ready_requests(), 1);

    handle.stop();
}
```

- [ ] **Step 6: Verify dispatcher module**

Run:

```bash
cargo test -p previa-runner wave_dispatcher
```

Expected: PASS.

---

### Task 5: Wire `run_wave_load` to the Dispatcher Thread

**Files:**
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Replace dispatcher imports**

In `runner/src/server/wave_executor.rs`, remove these imports:

```rust
use std::collections::{HashMap, VecDeque};
use previa_runner::{PipelineStep, StepExecutionResult, prepare_http_step};
use crate::server::wave_emitter::{StartLagClass, classify_start_lag};
use crate::server::wave_scheduler::slot_is_expired;
```

Add:

```rust
use crate::server::wave_dispatcher::{
    PipelineCursor, WaveDispatcherChannels, WaveDispatcherConfig, WaveDispatcherShared,
    WaveDispatcherSnapshot, spawn_wave_dispatcher_thread,
};
```

- [ ] **Step 2: Remove moved code from `wave_executor.rs`**

Delete these definitions from `runner/src/server/wave_executor.rs`:

```rust
struct PipelineCursor { ... }
impl PipelineCursor { ... }
type ObserverEvent = ...
const OBSERVER_EVENTS_PER_TICK_BUDGET: usize = 1024;
fn next_cursor_for_slot(...)
struct DispatchSlotRequestArgs<'a> { ... }
async fn dispatch_slot_requests(...)
async fn drain_observer_events_budgeted(...)
async fn drain_all_observer_events(...)
async fn handle_step_result(...)
async fn handle_prepare_error(...)
async fn record_terminal_pipeline(...)
fn max_attempts_for_step(...)
```

- [ ] **Step 3: Change slot receiver ownership**

Replace:

```rust
let (slot_tx, mut slot_rx) = mpsc::channel::<WaveDispatchSlot>(1024);
```

with:

```rust
let (slot_tx, slot_rx) = mpsc::channel::<WaveDispatchSlot>(1024);
```

- [ ] **Step 4: Add dispatcher snapshot channel**

After the metrics snapshot channel:

```rust
let (dispatcher_snapshot_tx, dispatcher_snapshot_rx) =
    watch::channel(WaveDispatcherSnapshot::default());
```

- [ ] **Step 5: Spawn the dispatcher thread before the sender**

After `scheduler_thread` is spawned and before `WaveSender::new(...)`, add:

```rust
let dispatcher_token = token.child_token();
let dispatcher_handle = spawn_wave_dispatcher_thread(
    WaveDispatcherConfig {
        pipeline: Arc::clone(&pipeline),
        specs: Arc::clone(&specs),
        env_groups: Arc::clone(&env_groups),
        selected_env_group_slug: selected_env_group_slug.clone(),
        started,
        tick_ms,
    },
    WaveDispatcherChannels {
        slot_rx,
        request_tx: request_tx.clone(),
        observer_rx: event_rx,
        metric_tx: metric_tx.clone(),
        snapshot_tx: dispatcher_snapshot_tx,
    },
    WaveDispatcherShared {
        ready_to_send: Arc::clone(&ready_to_send),
        missed_starts: Arc::clone(&missed_starts),
    },
    dispatcher_token.clone(),
);
```

Because `event_rx` moves into the dispatcher, remove every later use of `event_rx` from `run_wave_load`.

- [ ] **Step 6: Replace the slot loop with snapshot polling**

Replace the entire:

```rust
while let Some(slot) = slot_rx.recv().await {
    ...
}
```

with:

```rust
while !token.is_cancelled() && started.elapsed().as_millis() as u64 <= end_ms {
    let snapshot = *dispatcher_snapshot_rx.borrow();
    let scheduled_total = snapshot_rx.borrow().scheduled_starts.unwrap_or_default();
    send_metrics_snapshot(SnapshotArgs {
        load: &load,
        started,
        end_ms,
        tick_ms,
        scheduled_total,
        missed_total: missed_starts.load(Ordering::SeqCst),
        ready_requests: snapshot.ready_requests(),
        response_in_flight: response_in_flight.load(Ordering::SeqCst),
        metric_tx: &metric_tx,
        snapshot_rx: &mut snapshot_rx,
        runtime_sampler: &runtime_sampler,
        tx: &tx,
        token: &token,
        event: "metrics",
        duration_ms: None,
    })
    .await;

    tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms.min(250))).await;
}
```

- [ ] **Step 7: Use dispatcher snapshot in grace loop**

In the grace-period loop, replace:

```rust
ready_requests: ready
    .len()
    .saturating_add(ready_to_send.load(Ordering::SeqCst)),
```

with:

```rust
ready_requests: dispatcher_snapshot_rx.borrow().ready_requests(),
```

Make the same replacement in the final `send_metrics_snapshot(...)`.

- [ ] **Step 8: Stop dispatcher before dropping request sender**

In shutdown, after stopping the scheduler and before `drop(request_tx)`, add:

```rust
dispatcher_handle.stop();
```

Do not call `drain_all_observer_events` in `run_wave_load`; that is now owned by the dispatcher.

- [ ] **Step 9: Compile focused runner target**

Run:

```bash
cargo test -p previa-runner wave_executor --no-run
```

Expected: PASS compilation.

---

### Task 6: Preserve Scheduled and Lag Metrics Without the Slot Loop

**Files:**
- Modify: `runner/src/server/wave_scheduler.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_scheduler.rs`
- Test: `runner/src/server/wave_metrics_actor.rs`

- [ ] **Step 1: Emit scheduler metric events directly from the clock thread**

In `runner/src/server/wave_scheduler.rs`, inside `run_wave_scheduler_loop`, after `let tick = clock.plan_tick(...)`, add:

```rust
let _ = metric_tx.send(WaveSchedulerMetric::DispatchScheduled {
    count: tick.scheduled_starts,
});
if tick.scheduler_lag_ms > 0 || tick.missed_due_to_scheduler_lag > 0 {
    let _ = metric_tx.send(WaveSchedulerMetric::SchedulerLag {
        lag_ms: tick.scheduler_lag_ms,
        missed_starts: tick.missed_due_to_scheduler_lag,
    });
}
```

Then call `try_send_slot_or_metric(...)` as it does now.

- [ ] **Step 2: Update metric bridge in `run_wave_load`**

Replace the current bridge match:

```rust
match event {
    WaveSchedulerMetric::SlotBackpressure { dropped_starts } => {
        missed_bridge.fetch_add(dropped_starts, Ordering::SeqCst);
        let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
    }
    WaveSchedulerMetric::DispatchScheduled { .. }
    | WaveSchedulerMetric::SchedulerLag { .. } => {}
}
```

with:

```rust
match event {
    WaveSchedulerMetric::SlotBackpressure { dropped_starts } => {
        missed_bridge.fetch_add(dropped_starts, Ordering::SeqCst);
        let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
    }
    WaveSchedulerMetric::SchedulerLag { missed_starts, .. } => {
        missed_bridge.fetch_add(missed_starts, Ordering::SeqCst);
        let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
    }
    WaveSchedulerMetric::DispatchScheduled { .. } => {
        let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
    }
}
```

- [ ] **Step 3: Update scheduler tests**

In `runner/src/server/wave_scheduler.rs`, add:

```rust
#[tokio::test]
async fn scheduler_thread_emits_dispatch_scheduled_metric() {
    let (slot_tx, mut slot_rx) = mpsc::channel(8);
    let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
    let token = CancellationToken::new();

    let handle = spawn_wave_scheduler_thread(short_flat_load(), 100, slot_tx, metric_tx, token);

    let metric = tokio::time::timeout(std::time::Duration::from_millis(300), metric_rx.recv())
        .await
        .expect("scheduler should emit a metric")
        .expect("metric channel should stay open");

    assert!(matches!(
        metric,
        WaveSchedulerMetric::DispatchScheduled { count } if count > 0
    ));
    assert!(slot_rx.recv().await.is_some());

    handle.join().expect("scheduler thread should exit cleanly");
}
```

- [ ] **Step 4: Verify scheduler and metrics actor**

Run:

```bash
cargo test -p previa-runner wave_scheduler
cargo test -p previa-runner wave_metrics_actor
```

Expected: PASS.

---

### Task 7: Full Verification and Runtime Analysis

**Files:**
- No new source files beyond Tasks 1-6.

- [ ] **Step 1: Run full verification**

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
- App build PASS with only existing Vite chunk warnings.
- Release build PASS.

- [ ] **Step 2: Restart local main and three runners**

Restart the same local processes:

```bash
kill $(lsof -tiTCP:5610 -sTCP:LISTEN) $(lsof -tiTCP:5611 -sTCP:LISTEN) $(lsof -tiTCP:5612 -sTCP:LISTEN) $(lsof -tiTCP:5613 -sTCP:LISTEN)

screen -dmS previa-runner-5611 sh -c 'cd /Users/assis/projects/previa && PREVIA_HOME=/Users/assis/.previa PORT=5611 ./target/release/previa-runner'
screen -dmS previa-runner-5612 sh -c 'cd /Users/assis/projects/previa && PREVIA_HOME=/Users/assis/.previa PORT=5612 ./target/release/previa-runner'
screen -dmS previa-runner-5613 sh -c 'cd /Users/assis/projects/previa && PREVIA_HOME=/Users/assis/.previa PORT=5613 ./target/release/previa-runner'
screen -dmS previa-main-5610 sh -c 'cd /Users/assis/projects/previa && PREVIA_HOME=/Users/assis/.previa PORT=5610 ORCHESTRATOR_DATABASE_URL=sqlite:///tmp/previa-verify-5610.db RUNNER_ENDPOINTS=http://127.0.0.1:5611,http://127.0.0.1:5612,http://127.0.0.1:5613 PREVIA_APP_ENABLED=true ./target/release/previa-main'
```

Validate:

```bash
curl -s http://127.0.0.1:5610/info
```

Expected:

```json
{
  "totalRunners": 3,
  "activeRunners": 3
}
```

- [ ] **Step 3: Execute and analyze the known wave test**

Run the same UI test:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Fetch latest history:

```bash
curl -s 'http://127.0.0.1:5610/api/v1/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/tests/load?limit=1' > /tmp/latest-load-list.json
```

Analyze 10-second windows with the same fields:

```text
targetRps
dispatchRps
dispatchRatio
missedStarts
schedulerLaggedStarts
dispatcherLaggedStarts
runtimeLaggedStarts
readyRequests
outstandingRequests
```

Expected acceptance:
- `schedulerLaggedStarts` remains low, similar to the dedicated-clock result.
- `dispatcherLaggedStarts` is materially lower than `11776`.
- `curveAdherence` is materially higher than `92.02%`.
- If adherence still drops at high RPS, `readyRequests`, `outstandingRequests`, CPU, and memory identify the next infra bottleneck.

- [ ] **Step 4: Commit and push**

Run:

```bash
git status --short
git add runner/src/server/wave_dispatcher.rs runner/src/server/mod.rs runner/src/server/wave_executor.rs runner/src/server/wave_scheduler.rs docs/superpowers/plans/2026-05-04-dedicated-wave-dispatcher.md
git commit -m "fix: isolate wave dispatcher runtime"
git push origin codex/wave-load-test
```

Expected:
- Commit succeeds.
- Push succeeds.

## Implementation Notes

- Do not create a separate HTTP client per dispatcher; `WaveSender` keeps the shared `reqwest::Client`.
- Keep `request_tx` unbounded for now because the open-loop contract prioritizes start scheduling over response pressure. If memory becomes the next bottleneck, add a separate memory/infra diagnostic rather than silently throttling the wave.
- The dispatcher runtime should be `new_current_thread()`, not a multi-thread runtime, because its job is ordered slot consumption and request preparation, not HTTP response concurrency.
- Use `tokio::select! { biased; ... }` so slots are handled before observer events when both are waiting.
- Keep observer draining budgeted after every slot to avoid starving multi-step continuations, but never let observer draining sit in the main load loop again.
- Preserve `missedStarts` as the sum of scheduler lag, slot backpressure, dispatcher lag, runtime lag, and dependency-limited starts.
- `dispatchSubmitted` should continue to mean planned starts submitted by the clock, not starts accepted by the dispatcher.
- `dispatchStarted` should continue to mean actual request starts accepted by `WaveSender`.

## Self-Review

- Spec coverage: the plan isolates the dispatcher in a dedicated OS thread/runtime, keeps the existing clock isolation, preserves open-loop request-start semantics, and gives a concrete runtime validation against the latest observed `dispatcherLaggedStarts = 11776` bottleneck.
- Placeholder scan: every task names concrete files, commands, expected outcomes, and code snippets for new interfaces and moved functions.
- Type consistency: `PipelineCursor`, `ObserverEvent`, `WaveDispatcherSnapshot`, `WaveDispatcherConfig`, `WaveDispatcherChannels`, `WaveDispatcherShared`, and `WaveDispatcherHandle` are introduced before later tasks reference them.
