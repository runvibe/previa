# Wave Lifecycle Buckets And Dispatcher Plan Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the wave lifecycle graph trustworthy and then remove avoidable dispatcher/sender lag so the open-loop load generator is limited by infrastructure, not by ambiguous metrics or response coupling.

**Architecture:** Keep the existing cumulative counters for summary cards, but add explicit per-second lifecycle buckets as the source of truth for graphs and final history. The runner records lifecycle buckets at the moment each boundary happens, the main consolidates those buckets across runners, and the app renders the lifecycle chart from direct bucket values instead of reconstructing deltas from final cumulative snapshots. After that, split dispatcher preparation into its own worker pool so the scheduler clock, request preparation, and HTTP sender do not block each other.

**Tech Stack:** Rust/Tokio/Reqwest/SQLx/Serde on `previa-runner` and `previa-main`; React/TypeScript/Recharts/Vitest on `app`.

---

## File Structure

- Modify: `runner/src/server/models.rs`
  - Add `LoadLifecycleBucket`.
  - Add `lifecycle_buckets: Vec<LoadLifecycleBucket>` to `LoadTestMetrics`.

- Modify: `runner/src/server/metrics.rs`
  - Add bucket accumulator state.
  - Add record methods for planned, slot-enqueued, request-prepared, request-enqueued, send-task-spawned, send-started, HTTP-started, send-returned, body-completed, dispatcher-lagged, runtime-lagged.
  - Include lifecycle buckets in `snapshot_with_wave`.

- Modify: `runner/src/server/wave_metrics_actor.rs`
  - Carry `elapsed_ms` on lifecycle events.
  - Record each event into both cumulative counters and lifecycle buckets.

- Modify: `runner/src/server/wave_scheduler.rs`
  - Include scheduler slot `elapsed_ms` in planned/slot metrics.

- Modify: `runner/src/server/wave_dispatcher.rs`
  - Emit elapsed time for preparation/enqueue lifecycle events.
  - Add prepare-worker architecture in the second task group.

- Modify: `runner/src/server/wave_sender.rs`
  - Emit elapsed time for sender/HTTP lifecycle events.

- Modify: `main/src/server/models.rs`
  - Add `RunnerLoadLifecycleBucket` and `ConsolidatedLoadLifecycleBucket`.
  - Add `lifecycle_buckets` to parsed runner metrics and consolidated metrics.

- Modify: `main/src/server/execution/load_batch.rs`
  - Parse runner lifecycle buckets.
  - Consolidate lifecycle buckets across runners by `elapsedMs`.
  - Build final `rpsHistory` from real lifecycle buckets, not final cumulative counters repeated over all buckets.

- Modify: `app/src/types/load-test.ts`
  - Add TypeScript lifecycle bucket types.
  - Add `lifecycleBuckets` to `LoadTestMetrics`, `ConsolidatedLoadMetrics`, `RpsPoint`, and runner samples.

- Modify: `app/src/lib/api-client.ts`
  - Map `lifecycleBuckets` from remote history into UI metrics.

- Modify: `app/src/lib/remote-executor.ts`
  - Preserve live `lifecycleBuckets` in metrics snapshots and rps history points.

- Modify: `app/src/lib/load-lifecycle-chart.ts`
  - Prefer direct lifecycle bucket values.
  - Keep cumulative fallback only for older history.

- Modify: tests:
  - `runner/src/server/metrics.rs`
  - `runner/src/server/wave_metrics_actor.rs`
  - `runner/src/server/wave_scheduler.rs`
  - `runner/src/server/wave_dispatcher.rs`
  - `runner/src/server/wave_sender.rs`
  - `main/src/server/execution/load_batch.rs`
  - `app/src/lib/load-lifecycle-chart.ts` coverage through `app/src/components/LoadTestResultsPanel.test.tsx`
  - `app/src/lib/api-client.test.ts`

---

### Task 1: Add Runner Lifecycle Buckets

**Files:**
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/metrics.rs`
- Test: `runner/src/server/metrics.rs`

- [ ] **Step 1: Add the lifecycle bucket model**

Add this struct next to `LoadDispatchBucket` in `runner/src/server/models.rs`:

```rust
#[derive(Debug, Serialize, Clone, Default, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadLifecycleBucket {
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub planned: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub slot_enqueued: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub request_prepared: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub request_enqueued: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub send_task_spawned: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub send_started: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub http_started: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub http_send_returned: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub response_body_completed: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub dispatcher_lagged: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub runtime_lagged: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}
```

Add this field to `LoadTestMetrics`:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub lifecycle_buckets: Vec<LoadLifecycleBucket>,
```

Add `lifecycle_buckets: Vec::new(),` to `LoadTestMetrics::default()`.

- [ ] **Step 2: Add the failing metrics accumulator test**

Add this test to `runner/src/server/metrics.rs`:

```rust
#[test]
fn snapshot_includes_lifecycle_buckets() {
    let mut metrics = MetricsAccumulator::new();

    metrics.record_planned_at(1_050, 10);
    metrics.record_slot_enqueued_at(1_050, 10);
    metrics.record_request_prepared_at(1_110);
    metrics.record_request_enqueued_at(1_120);
    metrics.record_send_task_spawned_at(1_130);
    metrics.record_send_started_at(1_140);
    metrics.record_http_start_at(1_150);
    metrics.record_http_send_returned_at(1_160);
    metrics.record_response_body_completed_at(1_170, 1);
    metrics.record_dispatcher_lagged_starts_at(1_180, 2);
    metrics.record_runtime_lagged_start_at(1_190);

    let snapshot = metrics.snapshot(None, None);

    assert_eq!(snapshot.lifecycle_buckets.len(), 1);
    let bucket = &snapshot.lifecycle_buckets[0];
    assert_eq!(bucket.elapsed_ms, 1_000);
    assert_eq!(bucket.planned, 10);
    assert_eq!(bucket.slot_enqueued, 10);
    assert_eq!(bucket.request_prepared, 1);
    assert_eq!(bucket.request_enqueued, 1);
    assert_eq!(bucket.send_task_spawned, 1);
    assert_eq!(bucket.send_started, 1);
    assert_eq!(bucket.http_started, 1);
    assert_eq!(bucket.http_send_returned, 1);
    assert_eq!(bucket.response_body_completed, 1);
    assert_eq!(bucket.dispatcher_lagged, 2);
    assert_eq!(bucket.runtime_lagged, 1);
}
```

- [ ] **Step 3: Run the failing test**

Run:

```bash
cargo test -p previa-runner snapshot_includes_lifecycle_buckets
```

Expected: fails because the new record methods do not exist yet.

- [ ] **Step 4: Implement lifecycle bucket accumulation**

In `runner/src/server/metrics.rs`, import the new model:

```rust
use crate::server::models::{
    LoadDispatchBucket, LoadErrorSample, LoadLatencyBucket, LoadLifecycleBucket, LoadTestMetrics,
    RunnerInfoResponse,
};
```

Add this field to `MetricsAccumulator`:

```rust
lifecycle_buckets: BTreeMap<u64, LoadLifecycleBucket>,
```

Initialize it in `new()`:

```rust
lifecycle_buckets: BTreeMap::new(),
```

Add these helpers:

```rust
fn lifecycle_bucket_ms(elapsed_ms: u64) -> u64 {
    (elapsed_ms / 1000).saturating_mul(1000)
}

fn lifecycle_bucket_mut(&mut self, elapsed_ms: u64) -> &mut LoadLifecycleBucket {
    let bucket_ms = lifecycle_bucket_ms(elapsed_ms);
    self.lifecycle_buckets
        .entry(bucket_ms)
        .or_insert_with(|| LoadLifecycleBucket {
            elapsed_ms: bucket_ms,
            ..LoadLifecycleBucket::default()
        })
}
```

Add these record methods:

```rust
pub fn record_planned_at(&mut self, elapsed_ms: u64, count: usize) {
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.planned = bucket.planned.saturating_add(count);
}

pub fn record_slot_enqueued_at(&mut self, elapsed_ms: u64, count: usize) {
    self.record_slot_enqueued_count(count);
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.slot_enqueued = bucket.slot_enqueued.saturating_add(count);
}

pub fn record_request_prepared_at(&mut self, elapsed_ms: u64) {
    self.record_request_prepared();
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.request_prepared = bucket.request_prepared.saturating_add(1);
}

pub fn record_request_enqueued_at(&mut self, elapsed_ms: u64) {
    self.record_request_enqueued();
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.request_enqueued = bucket.request_enqueued.saturating_add(1);
}

pub fn record_send_task_spawned_at(&mut self, elapsed_ms: u64) {
    self.record_send_task_spawned();
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.send_task_spawned = bucket.send_task_spawned.saturating_add(1);
}

pub fn record_send_started_at(&mut self, elapsed_ms: u64) {
    self.record_send_started();
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.send_started = bucket.send_started.saturating_add(1);
}

pub fn record_http_start_at(&mut self, elapsed_ms: u64) {
    self.record_http_start();
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.http_started = bucket.http_started.saturating_add(1);
}

pub fn record_http_send_returned_at(&mut self, elapsed_ms: u64) {
    self.record_http_send_returned();
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.http_send_returned = bucket.http_send_returned.saturating_add(1);
}

pub fn record_response_body_completed_at(&mut self, elapsed_ms: u64, count: usize) {
    self.record_response_body_completed_count(count);
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.response_body_completed = bucket.response_body_completed.saturating_add(count);
}

pub fn record_dispatcher_lagged_starts_at(&mut self, elapsed_ms: u64, count: usize) {
    self.record_dispatcher_lagged_starts_count(count);
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.dispatcher_lagged = bucket.dispatcher_lagged.saturating_add(count);
}

pub fn record_runtime_lagged_start_at(&mut self, elapsed_ms: u64) {
    self.record_runtime_lagged_start();
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.runtime_lagged = bucket.runtime_lagged.saturating_add(1);
}
```

In `snapshot_with_wave`, populate:

```rust
lifecycle_buckets: self.lifecycle_buckets.values().cloned().collect(),
```

- [ ] **Step 5: Run runner metrics tests**

Run:

```bash
cargo test -p previa-runner snapshot_includes_lifecycle_buckets
cargo test -p previa-runner metrics
```

Expected: both pass.

---

### Task 2: Emit Lifecycle Events With Elapsed Time

**Files:**
- Modify: `runner/src/server/wave_metrics_actor.rs`
- Modify: `runner/src/server/wave_scheduler.rs`
- Modify: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/wave_sender.rs`
- Test: existing module tests in those files

- [ ] **Step 1: Change event variants**

Update `WaveMetricEvent` in `runner/src/server/wave_metrics_actor.rs`:

```rust
#[derive(Debug, Clone)]
pub enum WaveMetricEvent {
    Scheduler(WaveSchedulerMetric),
    PipelineStarted,
    DispatchStarted { elapsed_ms: u64 },
    SlotEnqueued { elapsed_ms: u64, count: usize },
    RequestPrepared { elapsed_ms: u64 },
    RequestEnqueued { elapsed_ms: u64 },
    SendTaskSpawned { elapsed_ms: u64 },
    SendStarted { elapsed_ms: u64 },
    HttpStarted { elapsed_ms: u64 },
    HttpSendReturned { elapsed_ms: u64 },
    HttpCompleted(usize),
    ResponseBodyCompleted { elapsed_ms: u64, count: usize },
    PipelineFinished { duration_ms: f64, success: bool },
    ErrorSample { step_id: String, http_status: Option<u16>, error: String },
    NetworkBytes { tx: u64, rx: u64 },
    DispatcherLaggedStarts { elapsed_ms: u64, count: usize },
    RuntimeLaggedStart { elapsed_ms: u64 },
    DependencyLimitedStarts(usize),
    Snapshot { wave: WaveMetricsSnapshot, runtime: Option<RunnerInfoResponse>, duration_ms: Option<u64> },
}
```

Update `WaveSchedulerMetric` in `runner/src/server/wave_scheduler.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveSchedulerMetric {
    DispatchScheduled { elapsed_ms: u64, count: usize },
    SlotEnqueued { elapsed_ms: u64, count: usize },
    SchedulerLag { elapsed_ms: u64, lag_ms: u64, missed_starts: usize },
    SlotBackpressure { elapsed_ms: u64, dropped_starts: usize },
}
```

- [ ] **Step 2: Update the metrics actor**

Replace the lifecycle match arms in `run_wave_metrics_actor` with elapsed-aware calls:

```rust
WaveMetricEvent::Scheduler(WaveSchedulerMetric::DispatchScheduled { elapsed_ms, count }) => {
    accumulator.record_dispatch_submitted_count(count);
    accumulator.record_planned_at(elapsed_ms, count);
}
WaveMetricEvent::Scheduler(WaveSchedulerMetric::SlotEnqueued { .. }) => {}
WaveMetricEvent::Scheduler(WaveSchedulerMetric::SchedulerLag { lag_ms, missed_starts, .. }) => {
    accumulator.record_scheduler_lag_ms(lag_ms);
    accumulator.record_scheduler_lagged_starts_count(missed_starts);
}
WaveMetricEvent::Scheduler(WaveSchedulerMetric::SlotBackpressure { dropped_starts, .. }) => {
    accumulator.record_scheduler_lagged_starts_count(dropped_starts);
}
WaveMetricEvent::SlotEnqueued { elapsed_ms, count } => {
    accumulator.record_slot_enqueued_at(elapsed_ms, count);
}
WaveMetricEvent::RequestPrepared { elapsed_ms } => accumulator.record_request_prepared_at(elapsed_ms),
WaveMetricEvent::RequestEnqueued { elapsed_ms } => accumulator.record_request_enqueued_at(elapsed_ms),
WaveMetricEvent::SendTaskSpawned { elapsed_ms } => accumulator.record_send_task_spawned_at(elapsed_ms),
WaveMetricEvent::SendStarted { elapsed_ms } => accumulator.record_send_started_at(elapsed_ms),
WaveMetricEvent::HttpStarted { elapsed_ms } => accumulator.record_http_start_at(elapsed_ms),
WaveMetricEvent::HttpSendReturned { elapsed_ms } => accumulator.record_http_send_returned_at(elapsed_ms),
WaveMetricEvent::ResponseBodyCompleted { elapsed_ms, count } => {
    accumulator.record_response_body_completed_at(elapsed_ms, count);
}
WaveMetricEvent::DispatcherLaggedStarts { elapsed_ms, count } => {
    accumulator.record_dispatcher_lagged_starts_at(elapsed_ms, count);
}
WaveMetricEvent::RuntimeLaggedStart { elapsed_ms } => {
    accumulator.record_runtime_lagged_start_at(elapsed_ms);
}
```

- [ ] **Step 3: Update scheduler emissions**

In `run_wave_scheduler_loop`, emit elapsed-aware metrics:

```rust
let _ = metric_tx.send(WaveSchedulerMetric::DispatchScheduled {
    elapsed_ms: tick.elapsed_ms,
    count: tick.scheduled_starts,
});

if tick.scheduler_lag_ms > 0 || tick.missed_due_to_scheduler_lag > 0 {
    let _ = metric_tx.send(WaveSchedulerMetric::SchedulerLag {
        elapsed_ms: tick.elapsed_ms,
        lag_ms: tick.scheduler_lag_ms,
        missed_starts: tick.missed_due_to_scheduler_lag,
    });
}
```

In `try_send_slot_or_metric`, use `slot.elapsed_ms` for `SlotEnqueued` and `SlotBackpressure`.

- [ ] **Step 4: Update dispatcher emissions**

In `dispatch_slot_requests`, replace lifecycle sends with elapsed-aware versions:

```rust
let prepared_elapsed_ms = args.started.elapsed().as_millis() as u64;
let _ = args.metric_tx.send(WaveMetricEvent::RequestPrepared {
    elapsed_ms: prepared_elapsed_ms,
});
```

```rust
let enqueue_elapsed_ms = args.started.elapsed().as_millis() as u64;
let _ = args.metric_tx.send(WaveMetricEvent::RequestEnqueued {
    elapsed_ms: enqueue_elapsed_ms,
});
```

For expired slots:

```rust
let _ = args.metric_tx.send(WaveMetricEvent::DispatcherLaggedStarts {
    elapsed_ms: args.slot.elapsed_ms,
    count: args.slot.planned_starts,
});
```

For runtime lag:

```rust
let _ = args.metric_tx.send(WaveMetricEvent::RuntimeLaggedStart {
    elapsed_ms: actual_elapsed_ms,
});
```

- [ ] **Step 5: Update sender emissions**

In `WaveSender::spawn_observer`, calculate elapsed at each lifecycle boundary:

```rust
let spawned_elapsed_ms = self.started.elapsed().as_millis() as u64;
let _ = metric_tx.send(WaveMetricEvent::SendTaskSpawned {
    elapsed_ms: spawned_elapsed_ms,
});
```

Inside the spawned task:

```rust
let dispatch_elapsed_ms = started.elapsed().as_millis() as u64;
let _ = metric_tx.send(WaveMetricEvent::SendStarted {
    elapsed_ms: dispatch_elapsed_ms,
});
let _ = metric_tx.send(WaveMetricEvent::DispatchStarted {
    elapsed_ms: dispatch_elapsed_ms,
});
let _ = metric_tx.send(WaveMetricEvent::HttpStarted {
    elapsed_ms: dispatch_elapsed_ms,
});
```

In hooks:

```rust
move || {
    let metric_tx = metrics_for_send.clone();
    async move {
        let _ = metric_tx.send(WaveMetricEvent::HttpSendReturned {
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
    }
}
```

```rust
move || {
    let metric_tx = metrics_for_body.clone();
    async move {
        let _ = metric_tx.send(WaveMetricEvent::ResponseBodyCompleted {
            elapsed_ms: started.elapsed().as_millis() as u64,
            count: 1,
        });
    }
}
```

- [ ] **Step 6: Update and run runner tests**

Update existing pattern matches in tests to include the new fields.

Run:

```bash
cargo test -p previa-runner
```

Expected: all runner tests pass.

---

### Task 3: Consolidate Lifecycle Buckets In Main

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Test: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Add main-side models**

Add to `main/src/server/models.rs` near `RunnerLoadDispatchBucket`:

```rust
#[derive(Debug, Clone, Default)]
pub struct RunnerLoadLifecycleBucket {
    pub elapsed_ms: u64,
    pub planned: usize,
    pub slot_enqueued: usize,
    pub request_prepared: usize,
    pub request_enqueued: usize,
    pub send_task_spawned: usize,
    pub send_started: usize,
    pub http_started: usize,
    pub http_send_returned: usize,
    pub response_body_completed: usize,
    pub dispatcher_lagged: usize,
    pub runtime_lagged: usize,
}
```

Add `pub lifecycle_buckets: Vec<RunnerLoadLifecycleBucket>,` to `RunnerLoadMetricsPoint`.

Add a serializable consolidated bucket:

```rust
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidatedLoadLifecycleBucket {
    pub elapsed_ms: u64,
    pub planned: usize,
    pub slot_enqueued: usize,
    pub request_prepared: usize,
    pub request_enqueued: usize,
    pub send_task_spawned: usize,
    pub send_started: usize,
    pub http_started: usize,
    pub http_send_returned: usize,
    pub response_body_completed: usize,
    pub dispatcher_lagged: usize,
    pub runtime_lagged: usize,
}
```

Add `pub lifecycle_buckets: Vec<ConsolidatedLoadLifecycleBucket>,` to `ConsolidatedLoadMetrics`.

- [ ] **Step 2: Add a failing consolidation test**

Add to `main/src/server/execution/load_batch.rs` tests:

```rust
#[test]
fn consolidated_metrics_sum_lifecycle_buckets_by_elapsed_ms() {
    let mut latest = HashMap::new();
    latest.insert(
        "runner-a".to_owned(),
        RunnerLoadLine {
            node: "runner-a".to_owned(),
            runner_event: "metrics".to_owned(),
            received_at: 1,
            payload: json!({
                "totalSent": 0,
                "totalSuccess": 0,
                "totalError": 0,
                "rps": 0.0,
                "startTime": 10_000,
                "elapsedMs": 2_000,
                "lifecycleBuckets": [
                    {"elapsedMs": 1_000, "planned": 10, "sendStarted": 9, "httpStarted": 8}
                ]
            }),
        },
    );
    latest.insert(
        "runner-b".to_owned(),
        RunnerLoadLine {
            node: "runner-b".to_owned(),
            runner_event: "metrics".to_owned(),
            received_at: 1,
            payload: json!({
                "totalSent": 0,
                "totalSuccess": 0,
                "totalError": 0,
                "rps": 0.0,
                "startTime": 10_000,
                "elapsedMs": 2_000,
                "lifecycleBuckets": [
                    {"elapsedMs": 1_000, "planned": 7, "sendStarted": 6, "httpStarted": 5}
                ]
            }),
        },
    );

    let metrics = consolidate_load_metrics(&latest, LoadLatencySummary::default()).unwrap();

    assert_eq!(metrics.lifecycle_buckets.len(), 1);
    assert_eq!(metrics.lifecycle_buckets[0].elapsed_ms, 1_000);
    assert_eq!(metrics.lifecycle_buckets[0].planned, 17);
    assert_eq!(metrics.lifecycle_buckets[0].send_started, 15);
    assert_eq!(metrics.lifecycle_buckets[0].http_started, 13);
}
```

- [ ] **Step 3: Parse lifecycle buckets**

In `parse_runner_load_metrics`, parse `lifecycleBuckets`:

```rust
fn extract_lifecycle_buckets(value: &Value) -> Vec<RunnerLoadLifecycleBucket> {
    value
        .get("lifecycleBuckets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(RunnerLoadLifecycleBucket {
                        elapsed_ms: item.get("elapsedMs")?.as_u64()?,
                        planned: optional_usize(item, "planned").unwrap_or(0),
                        slot_enqueued: optional_usize(item, "slotEnqueued").unwrap_or(0),
                        request_prepared: optional_usize(item, "requestPrepared").unwrap_or(0),
                        request_enqueued: optional_usize(item, "requestEnqueued").unwrap_or(0),
                        send_task_spawned: optional_usize(item, "sendTaskSpawned").unwrap_or(0),
                        send_started: optional_usize(item, "sendStarted").unwrap_or(0),
                        http_started: optional_usize(item, "httpStarted").unwrap_or(0),
                        http_send_returned: optional_usize(item, "httpSendReturned").unwrap_or(0),
                        response_body_completed: optional_usize(item, "responseBodyCompleted").unwrap_or(0),
                        dispatcher_lagged: optional_usize(item, "dispatcherLagged").unwrap_or(0),
                        runtime_lagged: optional_usize(item, "runtimeLagged").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
```

- [ ] **Step 4: Consolidate lifecycle buckets**

In `consolidate_load_metrics`, create a `BTreeMap<u64, ConsolidatedLoadLifecycleBucket>` and sum each runner bucket by `elapsed_ms`.

Use this merge shape:

```rust
let entry = lifecycle_by_elapsed
    .entry(bucket.elapsed_ms)
    .or_insert_with(|| ConsolidatedLoadLifecycleBucket {
        elapsed_ms: bucket.elapsed_ms,
        planned: 0,
        slot_enqueued: 0,
        request_prepared: 0,
        request_enqueued: 0,
        send_task_spawned: 0,
        send_started: 0,
        http_started: 0,
        http_send_returned: 0,
        response_body_completed: 0,
        dispatcher_lagged: 0,
        runtime_lagged: 0,
    });

entry.planned = entry.planned.saturating_add(bucket.planned);
entry.slot_enqueued = entry.slot_enqueued.saturating_add(bucket.slot_enqueued);
entry.request_prepared = entry.request_prepared.saturating_add(bucket.request_prepared);
entry.request_enqueued = entry.request_enqueued.saturating_add(bucket.request_enqueued);
entry.send_task_spawned = entry.send_task_spawned.saturating_add(bucket.send_task_spawned);
entry.send_started = entry.send_started.saturating_add(bucket.send_started);
entry.http_started = entry.http_started.saturating_add(bucket.http_started);
entry.http_send_returned = entry.http_send_returned.saturating_add(bucket.http_send_returned);
entry.response_body_completed = entry.response_body_completed.saturating_add(bucket.response_body_completed);
entry.dispatcher_lagged = entry.dispatcher_lagged.saturating_add(bucket.dispatcher_lagged);
entry.runtime_lagged = entry.runtime_lagged.saturating_add(bucket.runtime_lagged);
```

Populate `ConsolidatedLoadMetrics.lifecycle_buckets` with `lifecycle_by_elapsed.into_values().collect()`.

- [ ] **Step 5: Add lifecycle bucket data to final rps history**

In `build_rps_history_sample`, find the consolidated lifecycle bucket for the sample elapsed ms and insert direct values:

```rust
if let Some(bucket) = metrics.lifecycle_buckets.iter().find(|bucket| bucket.elapsed_ms == elapsed_ms) {
    sample.insert("lifecycleBucket".to_owned(), serde_json::to_value(bucket).unwrap_or(Value::Null));
}
```

This keeps cumulative fields backward compatible, but gives the frontend a direct per-second bucket.

- [ ] **Step 6: Run main tests**

Run:

```bash
cargo test -p previa-main consolidated_metrics_sum_lifecycle_buckets_by_elapsed_ms
cargo test -p previa-main
```

Expected: all main tests pass.

---

### Task 4: Render Lifecycle Chart From Direct Buckets

**Files:**
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/api-client.ts`
- Modify: `app/src/lib/remote-executor.ts`
- Modify: `app/src/lib/load-lifecycle-chart.ts`
- Test: `app/src/components/LoadTestResultsPanel.test.tsx`
- Test: `app/src/lib/api-client.test.ts`

- [ ] **Step 1: Add TypeScript lifecycle types**

Add to `app/src/types/load-test.ts`:

```ts
export interface LoadLifecycleBucket {
  elapsedMs: number;
  planned?: number;
  slotEnqueued?: number;
  requestPrepared?: number;
  requestEnqueued?: number;
  sendTaskSpawned?: number;
  sendStarted?: number;
  httpStarted?: number;
  httpSendReturned?: number;
  responseBodyCompleted?: number;
  dispatcherLagged?: number;
  runtimeLagged?: number;
}
```

Add `lifecycleBuckets?: LoadLifecycleBucket[];` to `LoadTestMetrics`, `ConsolidatedLoadMetrics`, and `RunnerRpsSample`.

Add `lifecycleBucket?: LoadLifecycleBucket;` to `RpsPoint`.

- [ ] **Step 2: Update API mapping**

In `app/src/lib/api-client.ts`, add:

```ts
lifecycleBuckets: Array.isArray(consolidated?.lifecycleBuckets)
  ? consolidated.lifecycleBuckets
  : [],
```

to the `metrics` object in `loadRecordToRun`.

- [ ] **Step 3: Update remote executor mapping**

In `toFullMetrics` in both `runRemoteLoadTest` and `reconnectToLoadExecution`, add:

```ts
lifecycleBuckets: consolidated?.lifecycleBuckets ?? event.lifecycleBuckets ?? [],
```

In `buildRpsHistoryPoint`, map direct lifecycle bucket for the sample elapsed time:

```ts
const lifecycleBucket = consolidated?.lifecycleBuckets?.find(
  (bucket) => bucket.elapsedMs === sampleElapsedMs,
) ?? event.lifecycleBuckets?.find(
  (bucket) => bucket.elapsedMs === sampleElapsedMs,
);
```

Return:

```ts
lifecycleBucket,
```

- [ ] **Step 4: Update lifecycle chart builder**

In `app/src/lib/load-lifecycle-chart.ts`, add this branch before cumulative delta fallback:

```ts
const bucket = point.lifecycleBucket
  ?? metrics.lifecycleBuckets?.find((item) => item.elapsedMs === time * 1000);

if (bucket) {
  row.planned += bucket.planned ?? 0;
  row.sendStarted += bucket.sendStarted ?? 0;
  row.httpStarted += bucket.httpStarted ?? 0;
  row.httpSendReturned += bucket.httpSendReturned ?? 0;
  row.responseBodyCompleted += bucket.responseBodyCompleted ?? 0;
  continue;
}
```

- [ ] **Step 5: Add frontend tests**

Add a test to `app/src/components/LoadTestResultsPanel.test.tsx`:

```ts
it("builds lifecycle chart from direct lifecycle buckets", () => {
  const metrics = createMetrics({
    startTime: 1_000,
    rpsHistory: [
      {
        timestamp: 2_000,
        elapsedMs: 1_000,
        rps: 0,
        lifecycleBucket: {
          elapsedMs: 1_000,
          planned: 30,
          sendStarted: 29,
          httpStarted: 28,
          httpSendReturned: 20,
          responseBodyCompleted: 10,
        },
      },
    ],
  });

  const chart = buildLifecycleChartData(metrics);

  expect(chart.data).toEqual([
    {
      time: 1,
      planned: 30,
      sendStarted: 29,
      httpStarted: 28,
      httpSendReturned: 20,
      responseBodyCompleted: 10,
    },
  ]);
});
```

- [ ] **Step 6: Run frontend tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel
npm --prefix app test -- api-client
npm --prefix app run build
```

Expected: tests and build pass.

---

### Task 5: Split Request Preparation From Dispatcher Clock

**Files:**
- Modify: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_dispatcher.rs`

- [ ] **Step 1: Add prepare queue types**

In `runner/src/server/wave_dispatcher.rs`, add:

```rust
pub struct WavePrepareIntent {
    pub slot_elapsed_ms: u64,
    pub cursor: PipelineCursor,
}
```

Add:

```rust
pub struct WavePrepareRequest {
    pub slot_elapsed_ms: u64,
    pub cursor: PipelineCursor,
    pub pipeline: Arc<Pipeline>,
    pub specs: Arc<Vec<RuntimeSpec>>,
    pub env_groups: Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: Option<String>,
    pub started: Instant,
}
```

- [ ] **Step 2: Add a failing dispatcher test**

Add a test proving the dispatcher can drain a large slot without doing HTTP preparation inline:

```rust
#[tokio::test]
async fn dispatch_slot_enqueues_prepare_intents_without_preparing_inline() {
    let pipeline = Pipeline {
        id: None,
        name: "load".to_owned(),
        steps: vec![http_step("list_users")],
    };
    let mut ready = VecDeque::new();
    let (prepare_tx, mut prepare_rx) = mpsc::unbounded_channel();
    let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
    let missed = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let token = tokio_util::sync::CancellationToken::new();

    dispatch_slot_prepare_intents(DispatchSlotPrepareArgs {
        slot: WaveDispatchSlot {
            elapsed_ms: 0,
            expires_at_elapsed_ms: 100,
            planned_starts: 500,
            target_rps_limit: 5_000.0,
            scheduled_total: 500,
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
        },
        ready: &mut ready,
        pipeline: &pipeline,
        prepare_tx: &prepare_tx,
        metric_tx: &metric_tx,
        missed_starts: &missed,
        started,
        tick_ms: 100,
        token: &token,
    })
    .await;

    let mut count = 0;
    while prepare_rx.try_recv().is_ok() {
        count += 1;
    }

    assert_eq!(count, 500);
    assert_eq!(missed.load(Ordering::SeqCst), 0);
}
```

- [ ] **Step 3: Implement prepare intents**

Refactor `dispatch_slot_requests` into two functions:

```rust
pub async fn dispatch_slot_prepare_intents(args: DispatchSlotPrepareArgs<'_>) {
    // only chooses cursor, records PipelineStarted, checks expiration, and sends WavePrepareIntent
}

pub async fn prepare_wave_request(intent: WavePrepareRequest) -> Option<ReadyWaveRequest<PipelineCursor>> {
    // runs prepare_http_step and returns ReadyWaveRequest
}
```

The dispatcher thread should stay focused on draining slots and observer events. It must not call `prepare_http_step` inside the slot loop anymore.

- [ ] **Step 4: Add prepare worker pool**

Add `RUNNER_WAVE_PREPARE_THREADS` with default:

```rust
fn prepare_worker_threads() -> usize {
    std::env::var("RUNNER_WAVE_PREPARE_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(2)
                .clamp(2, 8)
        })
}
```

Workers consume prepare intents, call `prepare_http_step`, emit `RequestPrepared`, then send `ReadyWaveRequest` to the sender channel and emit `RequestEnqueued`.

- [ ] **Step 5: Preserve response-driven continuations**

Keep `handle_step_result` in the dispatcher thread. Continuations still enter `ready: VecDeque<PipelineCursor>` only after responses arrive. The open-loop scheduler still creates new pipeline cursors when there is no continuation ready.

- [ ] **Step 6: Run runner tests**

Run:

```bash
cargo test -p previa-runner wave_dispatcher
cargo test -p previa-runner
```

Expected: all runner tests pass.

---

### Task 6: Verify End-To-End With A New Load Test

**Files:**
- No code files unless verification reveals a failure.

- [ ] **Step 1: Run all automated checks**

Run:

```bash
cargo test -p previa-runner
cargo test -p previa-main
npm --prefix app test -- LoadTestResultsPanel
npm --prefix app test -- api-client
npm --prefix app run build
cargo build --release
```

Expected: all pass.

- [ ] **Step 2: Restart local main and three runners**

Run:

```bash
for port in 5610 5611 5612 5613; do
  lsof -ti tcp:$port | while read pid; do [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; done
done
sleep 1
screen -S previa-wave -X quit >/dev/null 2>&1 || true
screen -dmS previa-wave zsh -lc '
  cd /Users/assis/projects/previa
  RUST_LOG=info PORT=5611 target/release/previa-runner > /tmp/previa-runner-5611.log 2>&1 &
  RUST_LOG=info PORT=5612 target/release/previa-runner > /tmp/previa-runner-5612.log 2>&1 &
  RUST_LOG=info PORT=5613 target/release/previa-runner > /tmp/previa-runner-5613.log 2>&1 &
  RUST_LOG=info PREVIA_APP_ENABLED=1 ORCHESTRATOR_DATABASE_URL=sqlite:///private/tmp/previa-verify-5610.db PORT=5610 RUNNER_ENDPOINTS=http://127.0.0.1:5611,http://127.0.0.1:5612,http://127.0.0.1:5613 target/release/previa-main > /tmp/previa-main-5610.log 2>&1
'
sleep 2
curl -s http://127.0.0.1:5610/info | jq -c '{activeRunners, runners: [.runners[].endpoint]}'
```

Expected:

```json
{"activeRunners":3,"runners":["http://127.0.0.1:5611","http://127.0.0.1:5612","http://127.0.0.1:5613"]}
```

- [ ] **Step 3: Execute the CRUD Users load test**

Open:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Run the same wave scenario: `0ms -> 10%`, `120000ms -> 80%`, interpolation `smooth`.

- [ ] **Step 4: Analyze the latest history record**

Run:

```bash
latest_id=$(sqlite3 /private/tmp/previa-verify-5610.db "select id from load_history where pipeline_id='019de1a7-4dfd-7662-8b53-a317b9bdbe23' order by started_at_ms desc limit 1;")
curl -s "http://127.0.0.1:5610/api/v1/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/tests/load/$latest_id" > /tmp/latest-load.json
jq '.finalConsolidated | {scheduledStarts, slotEnqueued, requestPrepared, requestEnqueued, sendTaskSpawned, sendStarted, httpStarted, httpSendReturned, responseBodyCompleted, dispatcherLaggedStarts, runtimeLaggedStarts, lifecycleBuckets: (.lifecycleBuckets | length), rpsHistory: (.rpsHistory | length)}' /tmp/latest-load.json
```

Expected:

- `lifecycleBuckets` is greater than `100`.
- `rpsHistory` is greater than `100`.
- Lifecycle bucket values vary by second instead of showing one giant first-second delta.
- `scheduledStarts`, `slotEnqueued`, `requestPrepared`, `sendStarted`, and `httpStarted` are close unless infrastructure is saturated.

- [ ] **Step 5: Commit and push**

If all checks pass:

```bash
git status --short
git add runner/src/server/models.rs runner/src/server/metrics.rs runner/src/server/wave_metrics_actor.rs runner/src/server/wave_scheduler.rs runner/src/server/wave_dispatcher.rs runner/src/server/wave_sender.rs runner/src/server/wave_executor.rs main/src/server/models.rs main/src/server/execution/load_batch.rs app/src/types/load-test.ts app/src/lib/api-client.ts app/src/lib/remote-executor.ts app/src/lib/load-lifecycle-chart.ts app/src/components/LoadTestResultsPanel.test.tsx app/src/lib/api-client.test.ts
git commit -m "feat: add wave lifecycle buckets"
git push origin codex/wave-load-test
```

Expected: commit and push succeed.

---

## Self-Review

- Spec coverage: The plan fixes the unreliable lifecycle graph, preserves direct per-second lifecycle metrics in runner/main/app, and then addresses dispatcher lag by separating slot handling from request preparation.
- Placeholder scan: No task uses TBD/TODO/fill-later language; each task names files, commands, and expected outcomes.
- Type consistency: The plan uses `LoadLifecycleBucket` in runner/app and `RunnerLoadLifecycleBucket`/`ConsolidatedLoadLifecycleBucket` in main; JSON fields are camelCase (`lifecycleBuckets`, `lifecycleBucket`, `elapsedMs`) to match existing API conventions.
