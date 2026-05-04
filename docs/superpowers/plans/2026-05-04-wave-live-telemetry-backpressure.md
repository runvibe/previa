# Wave Live Telemetry Backpressure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make wave load-test telemetry real-time and bounded so the UI sees current execution seconds instead of delayed snapshots, while preserving complete final diagnostics.

**Architecture:** Split runner telemetry into lightweight live snapshots and complete final snapshots. Live metrics are emitted at a fixed low rate with only newly closed buckets, then the main process merges those incremental buckets per runner and publishes consolidated app snapshots on its own cadence. The app renders wave RPS from per-second `lifecycleBucket.httpStarted`, not from cumulative average RPS.

**Tech Stack:** Rust/Tokio/Reqwest/Serde/SQLx in `previa-runner` and `previa-main`; React/TypeScript/Recharts/Vitest in `app`.

---

## Current Root Cause

The runner currently sends `LoadTestMetrics` snapshots from `runner/src/server/wave_executor.rs` at `tick_ms.min(250)`. With a `100ms` wave tick that can be 10 live metric events per second per runner.

Each snapshot is built by `MetricsAccumulator::snapshot_with_wave` in `runner/src/server/metrics.rs` and includes complete cumulative arrays:

- `latencyBuckets`
- `dispatchBuckets`
- `lifecycleBuckets`
- `errorSamples`
- runtime fields
- all cumulative counters

The runner SSE channel in `runner/src/server/sse.rs` is unbounded, so execution is not slowed by the receiver. The main process receives the backlog later, parses each large JSON event in `main/src/server/execution/load_batch.rs`, refreshes snapshots in the per-event path, and also consolidates again in the batch flusher. The UI then observes delayed seconds.

The fix must make live telemetry bounded. It must not reduce the load generator's open-loop behavior.

## File Structure

- Modify: `runner/src/server/models.rs`
  - Add a small amount of metadata to identify whether a metrics payload is live or final.

- Modify: `runner/src/server/metrics.rs`
  - Add snapshot filtering for live bucket windows.
  - Keep full bucket arrays for final snapshots.

- Modify: `runner/src/server/wave_executor.rs`
  - Emit live metrics at a fixed cadence.
  - Send only newly closed buckets in live metrics.
  - Send a complete final snapshot on `complete`.

- Modify: `runner/src/server/wave_metrics_actor.rs`
  - Support live snapshot requests with a bucket range.
  - Keep current event accounting unchanged.

- Modify: `main/src/server/models.rs`
  - Add per-runner incremental state models if they are kept in shared modules.

- Modify: `main/src/server/execution/load_batch.rs`
  - Add a per-runner incremental accumulator.
  - Merge live bucket deltas instead of replacing bucket state.
  - Stop rebuilding live snapshots in the runner-read hot path.
  - Build final `rpsHistory` from consolidated lifecycle buckets.

- Modify: `main/src/server/utils.rs`
  - Parse optional snapshot mode and bucket arrays from both live and final payloads.

- Modify: `app/src/types/load-test.ts`
  - Add `snapshotMode?: "live" | "final"` if exposed to the UI.

- Modify: `app/src/lib/remote-executor.ts`
  - Preserve incremental lifecycle buckets.
  - Build RPS history points using bucket RPS.

- Modify: `app/src/lib/load-rps-chart.ts`
  - Prefer `lifecycleBucket.httpStarted` and runner lifecycle buckets over cumulative deltas.

- Modify: `app/src/lib/load-lifecycle-chart.ts`
  - Continue using direct lifecycle buckets.
  - Ensure it works when `metrics.rpsHistory` is shorter than `metrics.lifecycleBuckets`.

- Modify tests:
  - `runner/src/server/metrics.rs`
  - `runner/src/server/wave_metrics_actor.rs`
  - `runner/src/server/wave_executor.rs`
  - `main/src/server/execution/load_batch.rs`
  - `app/src/components/LoadTestResultsPanel.test.tsx`
  - `app/src/lib/remote-executor.test.ts`

---

### Task 1: Runner Live Snapshot Window

**Files:**
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/metrics.rs`
- Test: `runner/src/server/metrics.rs`

- [ ] **Step 1: Add metrics snapshot mode to the runner model**

In `runner/src/server/models.rs`, add this enum near the load metrics structs:

```rust
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LoadMetricsSnapshotMode {
    Live,
    Final,
}
```

Add this optional field to `LoadTestMetrics`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub snapshot_mode: Option<LoadMetricsSnapshotMode>,
```

Add this value to `LoadTestMetrics::default()`:

```rust
snapshot_mode: None,
```

- [ ] **Step 2: Add snapshot scope in metrics**

In `runner/src/server/metrics.rs`, add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsSnapshotScope {
    Full,
    LiveWindow {
        from_elapsed_ms: u64,
        through_elapsed_ms: u64,
    },
}
```

Add these helpers:

```rust
fn bucket_in_live_window(bucket_elapsed_ms: u64, from_elapsed_ms: u64, through_elapsed_ms: u64) -> bool {
    bucket_elapsed_ms >= from_elapsed_ms && bucket_elapsed_ms <= through_elapsed_ms
}

fn filtered_dispatch_buckets(
    buckets: &BTreeMap<u64, usize>,
    scope: MetricsSnapshotScope,
) -> Vec<LoadDispatchBucket> {
    buckets
        .iter()
        .filter(|(elapsed_ms, _)| match scope {
            MetricsSnapshotScope::Full => true,
            MetricsSnapshotScope::LiveWindow {
                from_elapsed_ms,
                through_elapsed_ms,
            } => bucket_in_live_window(**elapsed_ms, from_elapsed_ms, through_elapsed_ms),
        })
        .map(|(elapsed_ms, count)| LoadDispatchBucket {
            elapsed_ms: *elapsed_ms,
            count: *count,
        })
        .collect()
}

fn filtered_lifecycle_buckets(
    buckets: &BTreeMap<u64, LoadLifecycleBucket>,
    scope: MetricsSnapshotScope,
) -> Vec<LoadLifecycleBucket> {
    buckets
        .iter()
        .filter(|(elapsed_ms, _)| match scope {
            MetricsSnapshotScope::Full => true,
            MetricsSnapshotScope::LiveWindow {
                from_elapsed_ms,
                through_elapsed_ms,
            } => bucket_in_live_window(**elapsed_ms, from_elapsed_ms, through_elapsed_ms),
        })
        .map(|(_, bucket)| bucket.clone())
        .collect()
}
```

- [ ] **Step 3: Add the failing live-window test**

Add this test to `runner/src/server/metrics.rs`:

```rust
#[test]
fn live_snapshot_includes_only_requested_lifecycle_window() {
    let mut metrics = MetricsAccumulator::new();

    metrics.record_planned_at(0, 10);
    metrics.record_http_start_at(0);
    metrics.record_planned_at(1_000, 20);
    metrics.record_http_start_at(1_000);
    metrics.record_planned_at(2_000, 30);
    metrics.record_http_start_at(2_000);

    let snapshot = metrics.snapshot_with_wave_scope(
        None,
        None,
        None,
        MetricsSnapshotScope::LiveWindow {
            from_elapsed_ms: 1_000,
            through_elapsed_ms: 1_000,
        },
    );

    assert_eq!(snapshot.snapshot_mode, Some(LoadMetricsSnapshotMode::Live));
    assert_eq!(snapshot.lifecycle_buckets.len(), 1);
    assert_eq!(snapshot.lifecycle_buckets[0].elapsed_ms, 1_000);
    assert_eq!(snapshot.lifecycle_buckets[0].planned, 20);
    assert_eq!(snapshot.lifecycle_buckets[0].http_started, 1);
}
```

- [ ] **Step 4: Run the failing test**

Run:

```bash
cargo test -p previa-runner live_snapshot_includes_only_requested_lifecycle_window
```

Expected: fail because `snapshot_with_wave_scope` and `MetricsSnapshotScope` are not wired into `LoadTestMetrics` yet.

- [ ] **Step 5: Implement scoped snapshots**

Keep the existing `snapshot_with_wave` method as a full-snapshot wrapper:

```rust
pub fn snapshot_with_wave(
    &self,
    duration_ms: Option<u64>,
    runtime: Option<RunnerInfoResponse>,
    wave: Option<WaveMetricsSnapshot>,
) -> LoadTestMetrics {
    self.snapshot_with_wave_scope(duration_ms, runtime, wave, MetricsSnapshotScope::Full)
}
```

Add:

```rust
pub fn snapshot_with_wave_scope(
    &self,
    duration_ms: Option<u64>,
    runtime: Option<RunnerInfoResponse>,
    wave: Option<WaveMetricsSnapshot>,
    scope: MetricsSnapshotScope,
) -> LoadTestMetrics {
    let mut snapshot = self.snapshot_with_wave_inner(duration_ms, runtime, wave, scope);
    snapshot.snapshot_mode = Some(match scope {
        MetricsSnapshotScope::Full => LoadMetricsSnapshotMode::Final,
        MetricsSnapshotScope::LiveWindow { .. } => LoadMetricsSnapshotMode::Live,
    });
    snapshot
}
```

Move the current body of `snapshot_with_wave` into `snapshot_with_wave_inner(...)`, and replace the bucket fields with:

```rust
dispatch_buckets: filtered_dispatch_buckets(&self.dispatch_buckets, scope),
lifecycle_buckets: filtered_lifecycle_buckets(&self.lifecycle_buckets, scope),
```

Keep `latency_buckets` full only for `MetricsSnapshotScope::Full`. For live scope, emit an empty vector:

```rust
latency_buckets: match scope {
    MetricsSnapshotScope::Full => self
        .latency_histogram
        .iter()
        .map(|(duration_ms, count)| LoadLatencyBucket {
            duration_ms: *duration_ms,
            count: *count,
        })
        .collect(),
    MetricsSnapshotScope::LiveWindow { .. } => Vec::new(),
},
```

- [ ] **Step 6: Verify runner metrics tests**

Run:

```bash
cargo test -p previa-runner live_snapshot_includes_only_requested_lifecycle_window
cargo test -p previa-runner snapshot_includes_lifecycle_buckets
```

Expected: both pass.

---

### Task 2: Runner Live Metric Cadence And Final Full Snapshot

**Files:**
- Modify: `runner/src/server/wave_metrics_actor.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Test: `runner/src/server/wave_metrics_actor.rs`
- Test: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Extend the snapshot event**

In `runner/src/server/wave_metrics_actor.rs`, change `WaveMetricEvent::Snapshot` to:

```rust
Snapshot {
    wave: WaveMetricsSnapshot,
    runtime: Option<RunnerInfoResponse>,
    duration_ms: Option<u64>,
    scope: MetricsSnapshotScope,
}
```

Import the scope:

```rust
use crate::server::metrics::{MetricsAccumulator, MetricsSnapshotScope, WaveMetricsSnapshot};
```

In `publish_snapshot`, accept `scope: MetricsSnapshotScope` and call:

```rust
let snapshot = accumulator.snapshot_with_wave_scope(duration_ms, runtime, wave, scope);
```

- [ ] **Step 2: Add a test proving live events are scoped**

Add this test to `runner/src/server/wave_metrics_actor.rs`:

```rust
#[tokio::test]
async fn metrics_actor_publishes_scoped_live_snapshot() {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (snapshot_tx, mut snapshot_rx) = watch::channel(LoadTestMetrics::default());

    let actor = tokio::spawn(run_wave_metrics_actor(event_rx, snapshot_tx));

    event_tx
        .send(WaveMetricEvent::Scheduler(WaveSchedulerMetric::DispatchScheduled {
            elapsed_ms: 0,
            count: 10,
        }))
        .unwrap();
    event_tx
        .send(WaveMetricEvent::Scheduler(WaveSchedulerMetric::DispatchScheduled {
            elapsed_ms: 1_000,
            count: 20,
        }))
        .unwrap();
    event_tx
        .send(WaveMetricEvent::Snapshot {
            wave: WaveMetricsSnapshot {
                target_intensity: 10.0,
                target_rps_limit: 100.0,
                in_flight: 0,
                runner_max_rps: 1000.0,
                tick_ms: 100,
                scheduled_starts: 30,
                missed_starts: 0,
                ready_requests: 0,
                active_pipelines: 0,
                outstanding_requests: 0,
            },
            runtime: None,
            duration_ms: None,
            scope: MetricsSnapshotScope::LiveWindow {
                from_elapsed_ms: 1_000,
                through_elapsed_ms: 1_000,
            },
        })
        .unwrap();

    snapshot_rx.changed().await.unwrap();
    let snapshot = snapshot_rx.borrow().clone();

    assert_eq!(snapshot.snapshot_mode, Some(LoadMetricsSnapshotMode::Live));
    assert_eq!(snapshot.lifecycle_buckets.len(), 1);
    assert_eq!(snapshot.lifecycle_buckets[0].elapsed_ms, 1_000);
    assert_eq!(snapshot.lifecycle_buckets[0].planned, 20);

    drop(event_tx);
    actor.await.unwrap();
}
```

- [ ] **Step 3: Add live cadence helpers**

In `runner/src/server/wave_executor.rs`, add constants:

```rust
const WAVE_LIVE_METRICS_INTERVAL_MS: u64 = 1_000;
const WAVE_LIVE_BUCKET_LAG_MS: u64 = 1_000;
```

Add helper:

```rust
fn closed_bucket_through_elapsed_ms(elapsed_ms: u64) -> Option<u64> {
    elapsed_ms
        .checked_sub(WAVE_LIVE_BUCKET_LAG_MS)
        .map(|value| (value / 1_000).saturating_mul(1_000))
}
```

Add state in `run_wave_load` before the main loop:

```rust
let mut next_live_metrics_at_ms = 0u64;
let mut next_live_bucket_from_ms = 0u64;
```

- [ ] **Step 4: Send live metrics at 1s cadence**

Replace the unconditional `send_metrics_snapshot(...)` inside the load-phase loop with:

```rust
let elapsed_ms = started.elapsed().as_millis() as u64;
if elapsed_ms >= next_live_metrics_at_ms {
    if let Some(through_elapsed_ms) = closed_bucket_through_elapsed_ms(elapsed_ms) {
        if through_elapsed_ms >= next_live_bucket_from_ms {
            let dispatcher_snapshot = *dispatcher_snapshot_rx.borrow();
            let scheduled_total = snapshot_rx.borrow().scheduled_starts.unwrap_or_default();
            send_metrics_snapshot(SnapshotArgs {
                load: &load,
                started,
                end_ms,
                tick_ms,
                scheduled_total,
                missed_total: missed_starts.load(Ordering::SeqCst),
                ready_requests: dispatcher_snapshot.ready_requests(),
                response_in_flight: response_in_flight.load(Ordering::SeqCst),
                metric_tx: &metric_tx,
                snapshot_rx: &mut snapshot_rx,
                runtime_sampler: &runtime_sampler,
                tx: &tx,
                token: &token,
                event: "metrics",
                duration_ms: None,
                scope: MetricsSnapshotScope::LiveWindow {
                    from_elapsed_ms: next_live_bucket_from_ms,
                    through_elapsed_ms,
                },
            })
            .await;
            next_live_bucket_from_ms = through_elapsed_ms.saturating_add(1_000);
        }
    }
    next_live_metrics_at_ms = elapsed_ms.saturating_add(WAVE_LIVE_METRICS_INTERVAL_MS);
}
```

Add `scope: MetricsSnapshotScope` to `SnapshotArgs`.

In `send_metrics_snapshot`, pass that scope into `WaveMetricEvent::Snapshot`.

- [ ] **Step 5: Send final metrics as full snapshot**

For the final `send_metrics_snapshot(...)` before `complete`, pass:

```rust
scope: MetricsSnapshotScope::Full,
```

Then `complete` continues to send `snapshot_rx.borrow().clone()`, which now contains the full final arrays.

- [ ] **Step 6: Verify runner wave tests**

Run:

```bash
cargo test -p previa-runner wave_metrics_actor
cargo test -p previa-runner wave_executor
```

Expected: all runner tests pass.

---

### Task 3: Main Incremental Runner Metrics Accumulator

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Test: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Parse snapshot mode in main**

In `main/src/server/models.rs`, add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerLoadSnapshotMode {
    Live,
    Final,
}
```

Add this field to `RunnerLoadMetricsPoint`:

```rust
pub snapshot_mode: Option<RunnerLoadSnapshotMode>,
```

In `main/src/server/utils.rs`, add:

```rust
fn parse_snapshot_mode(payload: &Value) -> Option<RunnerLoadSnapshotMode> {
    match payload.get("snapshotMode").and_then(Value::as_str) {
        Some("live") => Some(RunnerLoadSnapshotMode::Live),
        Some("final") => Some(RunnerLoadSnapshotMode::Final),
        _ => None,
    }
}
```

Set it in `parse_runner_load_metrics`:

```rust
snapshot_mode: parse_snapshot_mode(payload),
```

- [ ] **Step 2: Add runner accumulator structs**

In `main/src/server/execution/load_batch.rs`, add:

```rust
#[derive(Debug, Clone, Default)]
pub struct RunnerLoadTelemetryState {
    pub latest_payload: Option<Value>,
    pub lifecycle_buckets: BTreeMap<u64, RunnerLoadLifecycleBucket>,
    pub dispatch_buckets: BTreeMap<u64, usize>,
}

#[derive(Debug, Clone, Default)]
pub struct LoadTelemetryState {
    pub runners: BTreeMap<String, RunnerLoadTelemetryState>,
}
```

Add helper:

```rust
fn merge_lifecycle_bucket(
    target: &mut BTreeMap<u64, RunnerLoadLifecycleBucket>,
    bucket: RunnerLoadLifecycleBucket,
) {
    target.insert(bucket.elapsed_ms, bucket);
}
```

This uses replacement, not addition, because each bucket sent by a runner is the current final value for that runner and elapsed second.

- [ ] **Step 3: Add payload rehydration**

Add:

```rust
fn runner_state_payload(state: &RunnerLoadTelemetryState) -> Option<Value> {
    let mut payload = state.latest_payload.clone()?;
    let object = payload.as_object_mut()?;

    object.insert(
        "lifecycleBuckets".to_owned(),
        Value::Array(
            state
                .lifecycle_buckets
                .values()
                .map(|bucket| {
                    json!({
                        "elapsedMs": bucket.elapsed_ms,
                        "planned": bucket.planned,
                        "slotEnqueued": bucket.slot_enqueued,
                        "requestPrepared": bucket.request_prepared,
                        "requestEnqueued": bucket.request_enqueued,
                        "sendTaskSpawned": bucket.send_task_spawned,
                        "sendStarted": bucket.send_started,
                        "httpStarted": bucket.http_started,
                        "httpSendReturned": bucket.http_send_returned,
                        "responseBodyCompleted": bucket.response_body_completed,
                        "dispatcherLagged": bucket.dispatcher_lagged,
                        "runtimeLagged": bucket.runtime_lagged,
                    })
                })
                .collect(),
        ),
    );

    object.insert(
        "dispatchBuckets".to_owned(),
        Value::Array(
            state
                .dispatch_buckets
                .iter()
                .map(|(elapsed_ms, count)| json!({ "elapsedMs": elapsed_ms, "count": count }))
                .collect(),
        ),
    );

    Some(payload)
}
```

- [ ] **Step 4: Add failing incremental merge test**

Add this test to `main/src/server/execution/load_batch.rs`:

```rust
#[test]
fn telemetry_state_merges_live_bucket_windows_for_same_runner() {
    let mut state = LoadTelemetryState::default();

    apply_runner_telemetry_line(
        &mut state,
        RunnerLoadLine {
            node: "runner-1".to_owned(),
            runner_event: "metrics".to_owned(),
            received_at: 1,
            payload: json!({
                "snapshotMode": "live",
                "totalSent": 1,
                "totalSuccess": 0,
                "totalError": 1,
                "rps": 1.0,
                "startTime": 1000,
                "elapsedMs": 1000,
                "lifecycleBuckets": [
                    { "elapsedMs": 0, "planned": 10, "httpStarted": 9 }
                ]
            }),
        },
    );

    apply_runner_telemetry_line(
        &mut state,
        RunnerLoadLine {
            node: "runner-1".to_owned(),
            runner_event: "metrics".to_owned(),
            received_at: 2,
            payload: json!({
                "snapshotMode": "live",
                "totalSent": 2,
                "totalSuccess": 0,
                "totalError": 2,
                "rps": 2.0,
                "startTime": 1000,
                "elapsedMs": 2000,
                "lifecycleBuckets": [
                    { "elapsedMs": 1000, "planned": 20, "httpStarted": 18 }
                ]
            }),
        },
    );

    let lines = telemetry_state_lines(&state);
    let metrics = parse_runner_load_metrics(&lines[0].payload).unwrap();

    assert_eq!(metrics.lifecycle_buckets.len(), 2);
    assert_eq!(metrics.lifecycle_buckets[0].elapsed_ms, 0);
    assert_eq!(metrics.lifecycle_buckets[1].elapsed_ms, 1_000);
    assert_eq!(metrics.lifecycle_buckets[1].planned, 20);
    assert_eq!(metrics.lifecycle_buckets[1].http_started, 18);
}
```

- [ ] **Step 5: Implement telemetry state application**

Add:

```rust
pub fn apply_runner_telemetry_line(state: &mut LoadTelemetryState, line: RunnerLoadLine) {
    let runner = state.runners.entry(line.node.clone()).or_default();
    if let Some(metrics) = parse_runner_load_metrics(&line.payload) {
        for bucket in metrics.lifecycle_buckets {
            merge_lifecycle_bucket(&mut runner.lifecycle_buckets, bucket);
        }
        for bucket in metrics.dispatch_buckets {
            runner.dispatch_buckets.insert(bucket.elapsed_ms, bucket.count);
        }
    }
    runner.latest_payload = Some(line.payload);
}

pub fn telemetry_state_lines(state: &LoadTelemetryState) -> Vec<RunnerLoadLine> {
    state
        .runners
        .iter()
        .filter_map(|(node, runner)| {
            Some(RunnerLoadLine {
                node: node.clone(),
                runner_event: "metrics".to_owned(),
                received_at: now_ms(),
                payload: runner_state_payload(runner)?,
            })
        })
        .collect()
}
```

- [ ] **Step 6: Verify main merge test**

Run:

```bash
cargo test -p previa-main telemetry_state_merges_live_bucket_windows_for_same_runner
```

Expected: pass.

---

### Task 4: Main Hot Path Stops Rebuilding Snapshots Per Runner Event

**Files:**
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `main/src/server/execution/load.rs`
- Test: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Replace `load_latest` with telemetry state in the load path**

In `main/src/server/execution/load.rs`, replace:

```rust
let load_latest: Arc<Mutex<HashMap<String, RunnerLoadLine>>> =
    Arc::new(Mutex::new(HashMap::new()));
```

with:

```rust
let load_telemetry: Arc<Mutex<LoadTelemetryState>> =
    Arc::new(Mutex::new(LoadTelemetryState::default()));
```

Keep `load_chunk` for small outgoing event batches.

- [ ] **Step 2: Update `forward_runner_stream_load_chunked` signature**

In `main/src/server/execution/load_batch.rs`, replace the `load_latest` parameter with:

```rust
load_telemetry: Arc<Mutex<LoadTelemetryState>>,
```

Inside the event loop, replace the current `load_latest` insertion and `refresh_load_snapshot(...)` call with:

```rust
{
    let mut state = load_telemetry.lock().await;
    apply_runner_telemetry_line(&mut state, line.clone());
}
```

Do not call `refresh_load_snapshot` from this per-event path.

- [ ] **Step 3: Update batch flusher to consolidate from telemetry state**

Change `flush_load_batches` to accept:

```rust
load_telemetry: Arc<Mutex<LoadTelemetryState>>,
```

At the top of the tick body, build the latest snapshot lines:

```rust
let latest_lines = {
    let state = load_telemetry.lock().await;
    telemetry_state_lines(&state)
};
```

Then consolidate with:

```rust
let latest_by_node = latest_lines
    .iter()
    .cloned()
    .map(|line| (line.node.clone(), line))
    .collect::<HashMap<_, _>>();
let consolidated = {
    let latency_summary = {
        let lock = load_latency.lock().await;
        summarize_load_latency(&lock)
    };
    consolidate_load_metrics(&latest_by_node, latency_summary)
};
```

- [ ] **Step 4: Add a no-hot-refresh test**

Add this unit test around the telemetry helpers:

```rust
#[test]
fn telemetry_state_lines_are_small_latest_payloads_with_merged_buckets() {
    let mut state = LoadTelemetryState::default();
    for second in 0..120u64 {
        apply_runner_telemetry_line(
            &mut state,
            RunnerLoadLine {
                node: "runner-1".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: second,
                payload: json!({
                    "snapshotMode": "live",
                    "totalSent": second as usize,
                    "totalSuccess": 0,
                    "totalError": second as usize,
                    "rps": 1.0,
                    "startTime": 1000,
                    "elapsedMs": second * 1000,
                    "lifecycleBuckets": [
                        { "elapsedMs": second * 1000, "planned": 10, "httpStarted": 10 }
                    ]
                }),
            },
        );
    }

    let lines = telemetry_state_lines(&state);
    let metrics = parse_runner_load_metrics(&lines[0].payload).unwrap();

    assert_eq!(metrics.lifecycle_buckets.len(), 120);
    assert_eq!(metrics.lifecycle_buckets[119].elapsed_ms, 119_000);
}
```

This proves the merge state keeps all buckets without requiring every live event to contain all buckets.

- [ ] **Step 5: Update final history path**

Where final history currently calls:

```rust
let final_lines = snapshot_latest_lines(&load_latest).await;
```

replace with:

```rust
let final_lines = {
    let state = load_telemetry.lock().await;
    telemetry_state_lines(&state)
};
```

Build `latest_snapshot` from `final_lines`:

```rust
let latest_snapshot = final_lines
    .iter()
    .cloned()
    .map(|line| (line.node.clone(), line))
    .collect::<HashMap<_, _>>();
```

- [ ] **Step 6: Verify main load tests**

Run:

```bash
cargo test -p previa-main load_batch
cargo test -p previa-main execution::load
```

Expected: all main load tests pass.

---

### Task 5: Final RPS History From Lifecycle Buckets

**Files:**
- Modify: `main/src/server/execution/load_batch.rs`
- Test: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Add failing final history test**

Add:

```rust
#[test]
fn final_rps_history_uses_lifecycle_buckets_through_grace_period() {
    let metrics = ConsolidatedLoadMetrics {
        total_started: Some(0),
        total_sent: 0,
        total_success: 0,
        total_error: 0,
        http_started: Some(30),
        http_completed: Some(0),
        dispatch_submitted: Some(20),
        dispatch_started: Some(30),
        http_send_returned: Some(0),
        response_body_completed: Some(0),
        dependency_limited_starts: None,
        dispatcher_lagged_starts: None,
        runtime_lagged_starts: None,
        scheduler_lag_ms: None,
        scheduler_lagged_starts: None,
        slot_enqueued: Some(20),
        request_prepared: Some(20),
        request_enqueued: Some(20),
        send_task_spawned: Some(30),
        send_started: Some(30),
        rps: 10.0,
        target_intensity: Some(0.0),
        target_rps_limit: None,
        in_flight: None,
        runner_max_rps: Some(100.0),
        tick_ms: Some(100),
        scheduled_starts: Some(20),
        missed_starts: Some(0),
        ready_requests: None,
        active_pipelines: None,
        outstanding_requests: None,
        curve_adherence: Some(100.0),
        avg_latency: 0,
        p95: 0,
        p99: 0,
        start_time: 1_000,
        elapsed_ms: 3_000,
        nodes_reporting: 1,
        lifecycle_buckets: vec![
            ConsolidatedLoadLifecycleBucket {
                elapsed_ms: 0,
                planned: 10,
                slot_enqueued: 10,
                request_prepared: 10,
                request_enqueued: 10,
                send_task_spawned: 10,
                send_started: 10,
                http_started: 10,
                http_send_returned: 0,
                response_body_completed: 0,
                dispatcher_lagged: 0,
                runtime_lagged: 0,
            },
            ConsolidatedLoadLifecycleBucket {
                elapsed_ms: 2_000,
                planned: 0,
                slot_enqueued: 0,
                request_prepared: 0,
                request_enqueued: 0,
                send_task_spawned: 20,
                send_started: 20,
                http_started: 20,
                http_send_returned: 0,
                response_body_completed: 0,
                dispatcher_lagged: 0,
                runtime_lagged: 0,
            },
        ],
    };

    let history = rebuild_final_rps_history(&metrics, &HashMap::new());

    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["elapsedMs"], json!(0));
    assert_eq!(history[0]["lifecycleBucket"]["httpStarted"], json!(10));
    assert_eq!(history[1]["elapsedMs"], json!(2_000));
    assert_eq!(history[1]["lifecycleBucket"]["planned"], json!(0));
    assert_eq!(history[1]["lifecycleBucket"]["httpStarted"], json!(20));
}
```

- [ ] **Step 2: Rebuild final history from lifecycle buckets**

Change `rebuild_final_rps_history` so it starts from consolidated lifecycle buckets:

```rust
let mut bucket_ms = BTreeMap::<u64, ()>::new();
for bucket in &metrics.lifecycle_buckets {
    bucket_ms.insert(bucket.elapsed_ms, ());
}
for line in latest_by_node.values() {
    let Some(metrics) = parse_runner_load_metrics(&line.payload) else {
        continue;
    };
    for bucket in metrics.lifecycle_buckets {
        bucket_ms.insert(bucket.elapsed_ms, ());
    }
    for bucket in metrics.dispatch_buckets {
        bucket_ms.insert(bucket.elapsed_ms, ());
    }
}
```

Keep the existing mapping through `build_rps_history_sample(...)`.

- [ ] **Step 3: Verify final history test**

Run:

```bash
cargo test -p previa-main final_rps_history_uses_lifecycle_buckets_through_grace_period
```

Expected: pass.

---

### Task 6: App RPS Chart Uses Lifecycle HTTP Buckets

**Files:**
- Modify: `app/src/lib/load-rps-chart.ts`
- Modify: `app/src/lib/load-lifecycle-chart.ts`
- Test: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Add failing RPS chart test**

Add this test to `app/src/components/LoadTestResultsPanel.test.tsx` near the RPS chart tests:

```ts
it("builds HTTP RPS chart from lifecycle httpStarted buckets before cumulative averages", () => {
  const metrics: LoadTestMetrics = {
    ...emptyMetrics,
    runnerMaxRps: 3000,
    rps: 1061.85,
    rpsHistory: [
      {
        timestamp: 1_000,
        elapsedMs: 0,
        rps: 1061.85,
        lifecycleBucket: { elapsedMs: 0, planned: 300, httpStarted: 280 },
        runners: [
          { runnerId: "runner-a", lifecycleBucket: { elapsedMs: 0, httpStarted: 90 } },
          { runnerId: "runner-b", lifecycleBucket: { elapsedMs: 0, httpStarted: 95 } },
          { runnerId: "runner-c", lifecycleBucket: { elapsedMs: 0, httpStarted: 95 } },
        ],
      },
      {
        timestamp: 2_000,
        elapsedMs: 1_000,
        rps: 1061.85,
        lifecycleBucket: { elapsedMs: 1_000, planned: 330, httpStarted: 315 },
        runners: [
          { runnerId: "runner-a", lifecycleBucket: { elapsedMs: 1_000, httpStarted: 100 } },
          { runnerId: "runner-b", lifecycleBucket: { elapsedMs: 1_000, httpStarted: 105 } },
          { runnerId: "runner-c", lifecycleBucket: { elapsedMs: 1_000, httpStarted: 110 } },
        ],
      },
    ],
  };

  expect(buildRpsChartData(metrics, null)).toEqual({
    data: [
      { time: 0, rpsTotal: 280, runner0: 90, runner1: 95, runner2: 95, targetRpsLimit: undefined },
      { time: 1, rpsTotal: 315, runner0: 100, runner1: 105, runner2: 110, targetRpsLimit: undefined },
    ],
    runnerSeries: [
      { key: "runner0", label: "runner-a" },
      { key: "runner1", label: "runner-b" },
      { key: "runner2", label: "runner-c" },
    ],
    usesHttpRps: true,
  });
});
```

- [ ] **Step 2: Implement lifecycle bucket preference**

In `app/src/lib/load-rps-chart.ts`, add:

```ts
function lifecycleHttpStarted(point: RpsPoint | RunnerRpsSample | undefined) {
  return point?.lifecycleBucket?.httpStarted;
}

function hasLifecycleHttpBucket(point: RpsPoint) {
  return typeof lifecycleHttpStarted(point) === "number"
    || point.runners?.some((runner) => typeof lifecycleHttpStarted(runner) === "number") === true;
}
```

Change `usesHttpRps` to include `hasLifecycleHttpBucket(point)`.

Add an `applyLifecycleHttpBucket` helper:

```ts
const applyLifecycleHttpBucket = (row: RpsChartRow, point: RpsPoint) => {
  if (point.runners && point.runners.length > 0) {
    for (const runner of point.runners) {
      const key = runnerKeyById.get(runner.runnerId);
      const value = lifecycleHttpStarted(runner);
      if (!key || typeof value !== "number") continue;
      row[key] = Math.max(row[key] ?? 0, value);
    }
    row.rpsTotal = runnerSeries.reduce((sum, runner) => sum + (row[runner.key] ?? 0), 0);
    return;
  }

  const value = lifecycleHttpStarted(point);
  if (typeof value === "number") {
    row.rpsTotal = Math.max(row.rpsTotal, value);
  }
};
```

Before direct dispatch bucket handling, add:

```ts
if (hasLifecycleHttpBucket(point)) {
  applyLifecycleHttpBucket(ensureRow(rows, currentBucket, point), point);
  continue;
}
```

Apply the same logic to the first point before checking direct dispatch buckets.

- [ ] **Step 3: Make lifecycle chart independent of rps history length**

In `app/src/lib/load-lifecycle-chart.ts`, change the empty-history guard:

```ts
const history = metrics.rpsHistory ?? [];
const directBuckets = metrics.lifecycleBuckets ?? [];
if (history.length === 0 && directBuckets.length === 0) return { data: [], series: SERIES };
```

Before looping over history, seed rows from direct buckets:

```ts
for (const bucket of directBuckets) {
  const time = Math.max(0, Math.floor(bucket.elapsedMs / 1000));
  const row = ensureRow(rows, time);
  row.planned = bucket.planned ?? 0;
  row.sendStarted = bucket.sendStarted ?? 0;
  row.httpStarted = bucket.httpStarted ?? 0;
  row.httpSendReturned = bucket.httpSendReturned ?? 0;
  row.responseBodyCompleted = bucket.responseBodyCompleted ?? 0;
}
```

When processing `rpsHistory`, if a direct row already exists for that time from `metrics.lifecycleBuckets`, do not add duplicate values.

- [ ] **Step 4: Verify app chart tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel
```

Expected: all LoadTestResultsPanel tests pass.

---

### Task 7: Temporal Curve Adherence Metric

**Files:**
- Modify: `main/src/server/execution/load_batch.rs`
- Test: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Add temporal adherence helper**

In `main/src/server/execution/load_batch.rs`, add:

```rust
fn temporal_curve_adherence(buckets: &[ConsolidatedLoadLifecycleBucket]) -> Option<f64> {
    let mut planned_total = 0usize;
    let mut matched_total = 0usize;

    for bucket in buckets {
        if bucket.planned == 0 {
            continue;
        }
        planned_total = planned_total.saturating_add(bucket.planned);
        matched_total = matched_total.saturating_add(bucket.http_started.min(bucket.planned));
    }

    if planned_total == 0 {
        return None;
    }

    let value = (matched_total as f64 / planned_total as f64) * 100.0;
    Some((value * 100.0).round() / 100.0)
}
```

- [ ] **Step 2: Add failing adherence test**

Add:

```rust
#[test]
fn temporal_adherence_penalizes_late_http_starts_after_wave_end() {
    let buckets = vec![
        ConsolidatedLoadLifecycleBucket {
            elapsed_ms: 0,
            planned: 100,
            slot_enqueued: 100,
            request_prepared: 100,
            request_enqueued: 100,
            send_task_spawned: 100,
            send_started: 50,
            http_started: 50,
            http_send_returned: 0,
            response_body_completed: 0,
            dispatcher_lagged: 0,
            runtime_lagged: 0,
        },
        ConsolidatedLoadLifecycleBucket {
            elapsed_ms: 1_000,
            planned: 0,
            slot_enqueued: 0,
            request_prepared: 0,
            request_enqueued: 0,
            send_task_spawned: 50,
            send_started: 50,
            http_started: 50,
            http_send_returned: 0,
            response_body_completed: 0,
            dispatcher_lagged: 0,
            runtime_lagged: 0,
        },
    ];

    assert_eq!(temporal_curve_adherence(&buckets), Some(50.0));
}
```

- [ ] **Step 3: Use temporal adherence in consolidation**

After `lifecycle_by_elapsed` is converted to `Vec<ConsolidatedLoadLifecycleBucket>`, compute:

```rust
let lifecycle_buckets: Vec<_> = lifecycle_by_elapsed.into_values().collect();
let temporal_curve_adherence = temporal_curve_adherence(&lifecycle_buckets);
```

Set:

```rust
curve_adherence: temporal_curve_adherence.or_else(|| {
    (scheduled_starts_nodes > 0).then(|| {
        if scheduled_starts == 0 {
            100.0
        } else {
            let value = ((scheduled_starts.saturating_sub(missed_starts)) as f64
                / scheduled_starts as f64)
                * 100.0;
            (value * 100.0).round() / 100.0
        }
    })
}),
lifecycle_buckets,
```

- [ ] **Step 4: Verify adherence test**

Run:

```bash
cargo test -p previa-main temporal_adherence
```

Expected: pass.

---

### Task 8: End-To-End Verification And Runtime Check

**Files:**
- No new source files.
- Verify affected Rust and TypeScript packages.

- [ ] **Step 1: Run runner tests**

Run:

```bash
cargo test -p previa-runner
```

Expected: all runner tests pass.

- [ ] **Step 2: Run main tests**

Run:

```bash
cargo test -p previa-main
```

Expected: all main tests pass.

- [ ] **Step 3: Run app tests and typecheck**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel remote-executor api-client
cd app && ./node_modules/.bin/tsc --noEmit
```

Expected: tests and TypeScript check pass.

- [ ] **Step 4: Build production app**

Run:

```bash
npm --prefix app run build
```

Expected: Vite build succeeds. Existing chunk-size warnings are acceptable.

- [ ] **Step 5: Run release build**

Run:

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] **Step 6: Restart local main and three runners**

Run:

```bash
for port in 5610 5611 5612 5613; do
  lsof -ti tcp:$port | while read pid; do [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; done
done
sleep 1
for port in 5610 5611 5612 5613; do
  lsof -ti tcp:$port | while read pid; do [ -n "$pid" ] && kill -9 "$pid" 2>/dev/null || true; done
done
screen -S previa-wave -X quit >/dev/null 2>&1 || true
screen -dmS previa-wave zsh -lc '
  cd /Users/assis/projects/previa
  RUST_LOG=info PORT=5611 target/release/previa-runner > /tmp/previa-runner-5611.log 2>&1 &
  RUST_LOG=info PORT=5612 target/release/previa-runner > /tmp/previa-runner-5612.log 2>&1 &
  RUST_LOG=info PORT=5613 target/release/previa-runner > /tmp/previa-runner-5613.log 2>&1 &
  RUST_LOG=info PREVIA_APP_ENABLED=1 ORCHESTRATOR_DATABASE_URL=sqlite:///private/tmp/previa-verify-5610.db PORT=5610 RUNNER_ENDPOINTS=http://127.0.0.1:5611,http://127.0.0.1:5612,http://127.0.0.1:5613 target/release/previa-main > /tmp/previa-main-5610.log 2>&1
'
sleep 2
curl -s http://127.0.0.1:5610/info | jq -c '{activeRunners, runners: [.runners[].endpoint], pids: [.runners[].runtime.pid]}'
```

Expected:

```json
{"activeRunners":3,"runners":["http://127.0.0.1:5611","http://127.0.0.1:5612","http://127.0.0.1:5613"],"pids":[...]}
```

- [ ] **Step 7: Manual load-test acceptance criteria**

Run the same CRUD Users wave:

```text
0s -> 10%
120s -> 80%
gracePeriodMs -> 30000
3 runners
```

After completion, inspect the latest history record:

```bash
sqlite3 -json /private/tmp/previa-verify-5610.db "
SELECT id, status, duration_ms, final_consolidated_json
FROM load_history
WHERE project_id='019de1a7-4dfd-7662-8b53-a305e5714ca5'
  AND pipeline_id='019de1a7-4dfd-7662-8b53-a317b9bdbe23'
ORDER BY started_at_ms DESC
LIMIT 1;
" | jq '.[0] | {
  id,
  status,
  duration_ms,
  runnerElapsedMs: (.final_consolidated_json | fromjson | .elapsedMs),
  rpsHistoryLen: (.final_consolidated_json | fromjson | .rpsHistory | length),
  lifecycleLen: (.final_consolidated_json | fromjson | .lifecycleBuckets | length)
}'
```

Acceptance:

- `duration_ms - runnerElapsedMs` should be close to normal completion overhead, not hundreds of seconds.
- `rpsHistory` should cover the same second range as `lifecycleBuckets`.
- HTTP RPS chart should show bucket-shaped load, not flat cumulative average.
- If infra cannot keep up, `curveAdherence` should drop instead of falsely reporting near 100%.

---

## Self-Review

- Spec coverage: The plan addresses runner payload size, runner live cadence, main hot-path consolidation, incremental bucket merge, final history correctness, app RPS source, and curve adherence.
- Placeholder scan: The plan contains concrete file paths, commands, expected results, and code snippets for each code task.
- Type consistency: `snapshotMode`, `LoadMetricsSnapshotMode`, `RunnerLoadSnapshotMode`, `MetricsSnapshotScope`, `lifecycleBuckets`, and `httpStarted` are consistently named across runner, main, and app.

## Execution Options

1. **Subagent-Driven (recommended)** - dispatch one fresh worker per task group and review after each group.
2. **Inline Execution** - execute this plan in the current session with verification checkpoints.

