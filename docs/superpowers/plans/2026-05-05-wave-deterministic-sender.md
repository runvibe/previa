# Wave Deterministic Sender Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the wave sender deterministic enough that requests are either started inside their intended tick window or reported as runner-side saturation, without late catch-up distorting the wave.

**Architecture:** Keep scheduler and dispatcher as the source of timing truth, but carry each slot's timing metadata all the way into the sender. Replace per-request Tokio task spawning with a fixed sharded sender pool that registers HTTP futures inside long-lived worker tasks. Expired sender-queue items are dropped and counted as sender lag instead of being sent late.

**Tech Stack:** Rust, Tokio, Reqwest, futures-util, SSE metrics, existing Previa runner/main/app telemetry.

---

## Current Root Cause

The last analyzed run showed:

- `scheduledStarts`: `161905`
- `dispatchStarted` / `httpStarted`: `161827`
- total scheduling loss: only `78`
- but per-second buckets oscillated after roughly `99s`
- after the configured wave ended at `120s`, the runner still sent `6032` delayed requests

That means the wave math and dispatcher volume are mostly correct, but the sender still allows late catch-up. When sender/runtime pressure grows, prepared requests wait in `request_rx`; once the sender catches up, it sends them in later buckets. That preserves total count but breaks the temporal wave shape.

The fix is not to throttle harder. The fix is to make the sender honor each request's tick deadline:

- If the sender starts it within the slot window, count it as sent.
- If the sender only sees it after expiration, drop it and count `senderLaggedStarts`.
- Keep response/body observation outside the dispatch timing decision.

## File Structure

- Modify: `runner/Cargo.toml`
  - Add `futures-util = "0.3"` for `FuturesUnordered`.

- Modify: `runner/src/server/wave_dispatcher.rs`
  - Carry slot timing metadata through `WavePrepareIntent` and `ReadyWaveRequest`.
  - Add tests proving requests keep `scheduled_elapsed_ms` and `expires_at_elapsed_ms`.

- Modify: `runner/src/server/wave_sender.rs`
  - Replace per-request `tokio::spawn` with a fixed sharded worker pool.
  - Drop expired requests before starting HTTP.
  - Record sender lag metrics when a request misses its sender deadline.

- Modify: `runner/src/server/wave_metrics_actor.rs`
  - Add `SenderLaggedStarts` and `SenderQueueDepth` metric events.

- Modify: `runner/src/server/metrics.rs`
  - Add cumulative sender lag counters and lifecycle bucket fields.

- Modify: `runner/src/server/models.rs`
  - Expose `senderLaggedStarts`, `senderQueueDepth`, and `senderLagged` lifecycle data.

- Modify: `main/src/server/models.rs`
  - Parse and aggregate sender lag metrics from runners.

- Modify: `main/src/server/utils.rs`
  - Parse new runner fields.

- Modify: `main/src/server/execution/load_batch.rs`
  - Consolidate sender lag fields and include them in final/live payloads.

- Modify: `app/src/types/load-test.ts`
  - Add optional sender lag fields.

- Modify: `app/src/lib/remote-executor.ts`
  - Preserve sender lag fields during SSE parsing and aggregation.

- Modify: `app/src/lib/api-client.ts`
  - Preserve sender lag fields in history mapping.

- Modify: `app/src/components/LoadTestResultsPanel.tsx`
  - Show sender lag as runner saturation diagnostics.

---

### Task 1: Carry Slot Deadlines Into Prepared Requests

**Files:**
- Modify: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/wave_sender.rs`
- Test: `runner/src/server/wave_dispatcher.rs`

- [ ] **Step 1: Write the failing dispatcher metadata test**

Add this test inside `runner/src/server/wave_dispatcher.rs` `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn prepared_request_carries_slot_deadline_to_sender() {
    let pipeline = Pipeline {
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
    };
    let (prepare_tx, mut prepare_rx) = mpsc::unbounded_channel();
    let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
    let missed_starts = Arc::new(AtomicUsize::new(0));
    let token = tokio_util::sync::CancellationToken::new();
    let mut ready = VecDeque::new();
    let started = Instant::now();

    dispatch_slot_prepare_intents(DispatchSlotPrepareArgs {
        slot: WaveDispatchSlot {
            elapsed_ms: 4_200,
            expires_at_elapsed_ms: 4_300,
            planned_starts: 1,
            target_rps_limit: 10.0,
            scheduled_total: 1,
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
        },
        ready: &mut ready,
        pipeline: &pipeline,
        prepare_tx: &prepare_tx,
        metric_tx: &metric_tx,
        missed_starts: &missed_starts,
        started,
        tick_ms: 100,
        token: &token,
    })
    .await;

    let intent = prepare_rx.try_recv().expect("prepare intent should exist");
    assert_eq!(intent.scheduled_elapsed_ms, 4_200);
    assert_eq!(intent.expires_at_elapsed_ms, 4_300);
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test -p previa-runner prepared_request_carries_slot_deadline_to_sender
```

Expected: fail with missing fields on `WavePrepareIntent`.

- [ ] **Step 3: Add deadline fields to dispatcher intent and sender request**

In `runner/src/server/wave_dispatcher.rs`, change `WavePrepareIntent`:

```rust
#[derive(Debug)]
pub struct WavePrepareIntent {
    pub cursor: PipelineCursor,
    pub scheduled_elapsed_ms: u64,
    pub expires_at_elapsed_ms: u64,
}
```

In `dispatch_slot_prepare_intents`, change the send call to:

```rust
if args
    .prepare_tx
    .send(WavePrepareIntent {
        cursor,
        scheduled_elapsed_ms: args.slot.elapsed_ms,
        expires_at_elapsed_ms: args.slot.expires_at_elapsed_ms,
    })
    .is_err()
{
    error!("wave prepare workers stopped before accepting cursor");
    break;
}
```

In `runner/src/server/wave_sender.rs`, add the same metadata to `ReadyWaveRequest<C>`:

```rust
pub struct ReadyWaveRequest<C> {
    pub step: PipelineStep,
    pub cursor: C,
    pub context: HashMap<String, StepExecutionResult>,
    pub prepared: PreparedHttpStep,
    pub specs: Arc<Vec<RuntimeSpec>>,
    pub env_groups: Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: Option<String>,
    pub scheduled_elapsed_ms: u64,
    pub expires_at_elapsed_ms: u64,
    pub sender_enqueued_elapsed_ms: u64,
}
```

In `prepare_and_enqueue_wave_request`, capture enqueue time before sending:

```rust
let enqueue_elapsed_ms = config.started.elapsed().as_millis() as u64;
config.ready_to_send.fetch_add(1, Ordering::SeqCst);
if config
    .request_tx
    .send(ReadyWaveRequest {
        step,
        context: intent.cursor.context.clone(),
        cursor: intent.cursor,
        prepared,
        specs: Arc::clone(&config.specs),
        env_groups: Arc::clone(&config.env_groups),
        selected_env_group_slug: config.selected_env_group_slug.clone(),
        scheduled_elapsed_ms: intent.scheduled_elapsed_ms,
        expires_at_elapsed_ms: intent.expires_at_elapsed_ms,
        sender_enqueued_elapsed_ms: enqueue_elapsed_ms,
    })
    .is_err()
{
    config.ready_to_send.fetch_sub(1, Ordering::SeqCst);
    error!("wave sender stopped before accepting prepared request");
    return;
}

let _ = config.metric_tx.send(WaveMetricEvent::RequestEnqueued {
    elapsed_ms: enqueue_elapsed_ms,
});
```

- [ ] **Step 4: Run the test**

Run:

```bash
cargo test -p previa-runner prepared_request_carries_slot_deadline_to_sender
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add runner/src/server/wave_dispatcher.rs runner/src/server/wave_sender.rs
git commit -m "feat: carry wave sender deadlines"
```

---

### Task 2: Add Sender Saturation Metrics

**Files:**
- Modify: `runner/src/server/wave_metrics_actor.rs`
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/models.rs`
- Test: `runner/src/server/metrics.rs`
- Test: `runner/src/server/wave_metrics_actor.rs`

- [ ] **Step 1: Write the failing metrics accumulator test**

Add this test to `runner/src/server/metrics.rs`:

```rust
#[test]
fn snapshot_includes_sender_lagged_starts_and_bucket() {
    let mut metrics = MetricsAccumulator::new();

    metrics.record_planned_at(10_000, 5);
    metrics.record_sender_lagged_starts_at(10_000, 2);
    metrics.record_sender_queue_depth(17);

    let snapshot = metrics.snapshot_with_wave(None, None, None);

    assert_eq!(snapshot.sender_lagged_starts, Some(2));
    assert_eq!(snapshot.sender_queue_depth, Some(17));
    assert_eq!(snapshot.lifecycle_buckets.len(), 1);
    assert_eq!(snapshot.lifecycle_buckets[0].elapsed_ms, 10_000);
    assert_eq!(snapshot.lifecycle_buckets[0].sender_lagged, 2);
}
```

- [ ] **Step 2: Write the failing metrics actor test**

Add this test to `runner/src/server/wave_metrics_actor.rs`:

```rust
#[tokio::test]
async fn metrics_actor_records_sender_saturation() {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (snapshot_tx, mut snapshot_rx) = watch::channel(LoadTestMetrics::default());

    let actor = tokio::spawn(run_wave_metrics_actor(event_rx, snapshot_tx));

    event_tx
        .send(WaveMetricEvent::SenderLaggedStarts {
            elapsed_ms: 2_000,
            count: 3,
        })
        .unwrap();
    event_tx
        .send(WaveMetricEvent::SenderQueueDepth { depth: 9 })
        .unwrap();
    event_tx
        .send(WaveMetricEvent::Snapshot {
            wave: WaveMetricsSnapshot {
                target_intensity: 50.0,
                target_rps_limit: 500.0,
                in_flight: 0,
                runner_max_rps: 1_000.0,
                tick_ms: 100,
                scheduled_starts: 10,
                missed_starts: 3,
                ready_requests: 9,
                active_pipelines: 9,
                outstanding_requests: 0,
            },
            runtime: None,
            duration_ms: None,
            scope: MetricsSnapshotScope::Full,
        })
        .unwrap();

    snapshot_rx.changed().await.unwrap();
    let snapshot = snapshot_rx.borrow().clone();

    assert_eq!(snapshot.sender_lagged_starts, Some(3));
    assert_eq!(snapshot.sender_queue_depth, Some(9));

    drop(event_tx);
    actor.await.unwrap();
}
```

- [ ] **Step 3: Run both failing tests**

Run:

```bash
cargo test -p previa-runner snapshot_includes_sender_lagged_starts_and_bucket metrics_actor_records_sender_saturation
```

Expected: fail with missing fields/events.

- [ ] **Step 4: Add fields to runner models**

In `runner/src/server/models.rs`, add optional fields to `LoadTestMetrics`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub sender_lagged_starts: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub sender_queue_depth: Option<usize>,
```

Add to `LoadTestMetrics::default()`:

```rust
sender_lagged_starts: None,
sender_queue_depth: None,
```

Add to `LoadLifecycleBucket`:

```rust
pub sender_lagged: usize,
```

Update all `LoadLifecycleBucket` literals with `sender_lagged: 0`.

- [ ] **Step 5: Add accumulator state and methods**

In `runner/src/server/metrics.rs`, add fields:

```rust
sender_lagged_starts: usize,
sender_queue_depth: usize,
```

Initialize them to `0` in `MetricsAccumulator::new()`.

Add methods:

```rust
pub fn record_sender_lagged_starts_at(&mut self, elapsed_ms: u64, count: usize) {
    self.sender_lagged_starts = self.sender_lagged_starts.saturating_add(count);
    let bucket = self.lifecycle_bucket_mut(elapsed_ms);
    bucket.sender_lagged = bucket.sender_lagged.saturating_add(count);
}

pub fn record_sender_queue_depth(&mut self, depth: usize) {
    self.sender_queue_depth = depth;
}
```

In `snapshot_with_wave_scope`, set:

```rust
sender_lagged_starts: (self.sender_lagged_starts > 0).then_some(self.sender_lagged_starts),
sender_queue_depth: (self.sender_queue_depth > 0).then_some(self.sender_queue_depth),
```

- [ ] **Step 6: Add metrics actor events**

In `runner/src/server/wave_metrics_actor.rs`, add enum variants:

```rust
SenderLaggedStarts {
    elapsed_ms: u64,
    count: usize,
},
SenderQueueDepth {
    depth: usize,
},
```

Handle them:

```rust
WaveMetricEvent::SenderLaggedStarts { elapsed_ms, count } => {
    accumulator.record_sender_lagged_starts_at(elapsed_ms, count);
}
WaveMetricEvent::SenderQueueDepth { depth } => {
    accumulator.record_sender_queue_depth(depth);
}
```

- [ ] **Step 7: Run the tests**

Run:

```bash
cargo test -p previa-runner snapshot_includes_sender_lagged_starts_and_bucket
cargo test -p previa-runner metrics_actor_records_sender_saturation
```

Expected: both pass.

- [ ] **Step 8: Commit**

```bash
git add runner/src/server/models.rs runner/src/server/metrics.rs runner/src/server/wave_metrics_actor.rs
git commit -m "feat: report wave sender saturation"
```

---

### Task 3: Replace Per-Request Spawn With Sharded Sender Workers

**Files:**
- Modify: `runner/Cargo.toml`
- Modify: `runner/src/server/wave_sender.rs`
- Test: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Add `futures-util` to the runner package**

In `runner/Cargo.toml`, add:

```toml
futures-util = "0.3"
```

- [ ] **Step 2: Write the failing expired-request test**

Add this test to `runner/src/server/wave_sender.rs`:

```rust
#[tokio::test]
async fn sender_drops_expired_request_instead_of_late_catchup() {
    let started = Instant::now() - Duration::from_millis(500);
    let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
    let response_in_flight = Arc::new(AtomicUsize::new(0));
    let ready_to_send = Arc::new(AtomicUsize::new(1));
    let token = tokio_util::sync::CancellationToken::new();

    let dropped = drop_if_expired(SenderDeadlineCheck {
        scheduled_elapsed_ms: 100,
        expires_at_elapsed_ms: 200,
        started,
        metric_tx: &metric_tx,
        response_in_flight: &response_in_flight,
        ready_to_send: &ready_to_send,
        token: &token,
    });

    assert!(dropped);
    assert_eq!(ready_to_send.load(Ordering::SeqCst), 0);
    assert_eq!(response_in_flight.load(Ordering::SeqCst), 0);
    assert!(matches!(
        metric_rx.try_recv(),
        Ok(WaveMetricEvent::SenderLaggedStarts {
            elapsed_ms: 100,
            count: 1
        })
    ));
}
```

- [ ] **Step 3: Implement the expiration helper**

In `runner/src/server/wave_sender.rs`, add:

```rust
struct SenderDeadlineCheck<'a> {
    scheduled_elapsed_ms: u64,
    expires_at_elapsed_ms: u64,
    started: Instant,
    metric_tx: &'a mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: &'a Arc<AtomicUsize>,
    ready_to_send: &'a Arc<AtomicUsize>,
    token: &'a tokio_util::sync::CancellationToken,
}

fn drop_if_expired(args: SenderDeadlineCheck<'_>) -> bool {
    let elapsed_ms = args.started.elapsed().as_millis() as u64;
    if args.token.is_cancelled() || elapsed_ms <= args.expires_at_elapsed_ms {
        return false;
    }

    args.ready_to_send.fetch_sub(1, Ordering::SeqCst);
    let _ = args.metric_tx.send(WaveMetricEvent::SenderLaggedStarts {
        elapsed_ms: args.scheduled_elapsed_ms,
        count: 1,
    });
    let _ = args.metric_tx.send(WaveMetricEvent::SenderQueueDepth {
        depth: args.ready_to_send.load(Ordering::SeqCst),
    });
    false == false
}
```

Immediately simplify the final line to:

```rust
true
```

- [ ] **Step 4: Run the test**

Run:

```bash
cargo test -p previa-runner sender_drops_expired_request_instead_of_late_catchup
```

Expected: pass.

- [ ] **Step 5: Replace `spawn_observer` with sharded workers**

In `runner/src/server/wave_sender.rs`, replace the current per-request `spawn_observer` path with:

```rust
use futures_util::stream::{FuturesUnordered, StreamExt};
```

Add worker command type:

```rust
struct SenderWorkerCommand<C> {
    request: ReadyWaveRequest<C>,
}
```

Add worker result future:

```rust
async fn observe_ready_request<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    request: ReadyWaveRequest<C>,
    token: tokio_util::sync::CancellationToken,
) -> Option<WaveObserverEvent<C>>
where
    C: Send + 'static,
{
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

    let metrics_for_send = metric_tx.clone();
    let metrics_for_body = metric_tx.clone();
    let result = send_prepared_http_step_with_hooks(
        client.as_ref(),
        request.prepared,
        &request.step,
        &request.context,
        Some(request.specs.as_slice()),
        Some(request.env_groups.as_slice()),
        request.selected_env_group_slug.as_deref(),
        || token.is_cancelled(),
        move || {
            let metric_tx = metrics_for_send.clone();
            async move {
                let _ = metric_tx.send(WaveMetricEvent::HttpSendReturned {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
        },
        move || {
            let metric_tx = metrics_for_body.clone();
            async move {
                let _ = metric_tx.send(WaveMetricEvent::ResponseBodyCompleted {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    count: 1,
                });
            }
        },
    )
    .await?;

    let (network_tx_bytes, network_rx_bytes) =
        estimate_results_network_bytes(std::slice::from_ref(&result));
    if result.request.is_some() {
        let _ = metric_tx.send(WaveMetricEvent::HttpCompleted(1));
    }
    let _ = metric_tx.send(WaveMetricEvent::NetworkBytes {
        tx: network_tx_bytes,
        rx: network_rx_bytes,
    });

    Some(WaveObserverEvent {
        cursor: request.cursor,
        result,
    })
}
```

Add the worker loop:

```rust
async fn run_sender_worker<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    mut worker_rx: mpsc::UnboundedReceiver<SenderWorkerCommand<C>>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let mut in_flight = FuturesUnordered::new();

    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            Some(command) = worker_rx.recv() => {
                let request = command.request;
                if drop_if_expired(SenderDeadlineCheck {
                    scheduled_elapsed_ms: request.scheduled_elapsed_ms,
                    expires_at_elapsed_ms: request.expires_at_elapsed_ms,
                    started,
                    metric_tx: &metric_tx,
                    response_in_flight: &response_in_flight,
                    ready_to_send: &ready_to_send,
                    token: &token,
                }) {
                    continue;
                }

                ready_to_send.fetch_sub(1, Ordering::SeqCst);
                response_in_flight.fetch_add(1, Ordering::SeqCst);
                let _ = metric_tx.send(WaveMetricEvent::SenderQueueDepth {
                    depth: ready_to_send.load(Ordering::SeqCst),
                });
                let _ = metric_tx.send(WaveMetricEvent::SendTaskSpawned {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });

                in_flight.push(observe_ready_request(
                    Arc::clone(&client),
                    started,
                    metric_tx.clone(),
                    request,
                    token.clone(),
                ));
            }
            Some(result) = in_flight.next(), if !in_flight.is_empty() => {
                response_in_flight.fetch_sub(1, Ordering::SeqCst);
                if let Some(event) = result {
                    let _ = observer_tx.send(event);
                }
            }
            else => break,
        }
    }
}
```

Replace `WaveSender::run` so it creates `sender_worker_count()` worker channels and routes requests round-robin:

```rust
pub async fn run(mut self) {
    let worker_count = sender_worker_count();
    let mut workers = Vec::with_capacity(worker_count);
    let mut worker_txs = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let (worker_tx, worker_rx) = mpsc::unbounded_channel();
        worker_txs.push(worker_tx);
        workers.push(tokio::spawn(run_sender_worker(
            Arc::clone(&self.client),
            self.started,
            self.metric_tx.clone(),
            Arc::clone(&self.response_in_flight),
            Arc::clone(&self.ready_to_send),
            worker_rx,
            self.observer_tx.clone(),
            self.token.clone(),
        )));
    }

    let mut next_worker = 0usize;
    while !self.token.is_cancelled() {
        let Some(request) = self.request_rx.recv().await else {
            break;
        };
        if worker_txs.is_empty() {
            break;
        }
        let target = next_worker % worker_txs.len();
        next_worker = next_worker.wrapping_add(1);
        if worker_txs[target]
            .send(SenderWorkerCommand { request })
            .is_err()
        {
            break;
        }
    }

    drop(worker_txs);
    for worker in workers {
        worker.abort();
        let _ = worker.await;
    }
}
```

Rename `sender_worker_threads()` to `sender_worker_count()` or keep the old env var name if you want backwards compatibility:

```rust
fn sender_worker_count() -> usize {
    std::env::var("RUNNER_WAVE_SENDER_WORKERS")
        .or_else(|_| std::env::var("RUNNER_WAVE_SENDER_THREADS"))
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(2)
                .clamp(2, 16)
        })
}
```

- [ ] **Step 6: Update existing sender tests**

Update `sender_worker_threads_uses_positive_env_value` and `sender_worker_threads_ignores_zero_env_value` to call `sender_worker_count()`.

Update any `ReadyWaveRequest` test literals with:

```rust
scheduled_elapsed_ms: 0,
expires_at_elapsed_ms: u64::MAX,
sender_enqueued_elapsed_ms: 0,
```

- [ ] **Step 7: Run sender tests**

Run:

```bash
cargo test -p previa-runner wave_sender
```

Expected: all sender tests pass.

- [ ] **Step 8: Commit**

```bash
git add runner/Cargo.toml runner/src/server/wave_sender.rs
git commit -m "feat: shard wave sender workers"
```

---

### Task 4: Parse and Display Sender Saturation in Main/App

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/remote-executor.ts`
- Modify: `app/src/lib/api-client.ts`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Test: `main/src/server/execution/load_batch.rs`
- Test: `app/src/components/LoadTestResultsPanel.test.tsx`
- Test: `app/src/lib/remote-executor.test.ts`
- Test: `app/src/lib/api-client.test.ts`

- [ ] **Step 1: Write the failing main consolidation test**

Add this test to `main/src/server/execution/load_batch.rs`:

```rust
#[test]
fn consolidates_sender_saturation_metrics() {
    let latest = HashMap::from([
        (
            "http://runner-a:3000".to_owned(),
            RunnerLoadLine {
                node: "http://runner-a:3000".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 10,
                    "totalSuccess": 10,
                    "totalError": 0,
                    "rps": 10.0,
                    "startTime": 1_000,
                    "elapsedMs": 2_000,
                    "senderLaggedStarts": 2,
                    "senderQueueDepth": 11,
                    "lifecycleBuckets": [
                        { "elapsedMs": 1_000, "planned": 10, "httpStarted": 8, "senderLagged": 2 }
                    ]
                }),
            },
        ),
        (
            "http://runner-b:3000".to_owned(),
            RunnerLoadLine {
                node: "http://runner-b:3000".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 20,
                    "totalSuccess": 20,
                    "totalError": 0,
                    "rps": 20.0,
                    "startTime": 1_000,
                    "elapsedMs": 2_000,
                    "senderLaggedStarts": 3,
                    "senderQueueDepth": 7,
                    "lifecycleBuckets": [
                        { "elapsedMs": 1_000, "planned": 10, "httpStarted": 7, "senderLagged": 3 }
                    ]
                }),
            },
        ),
    ]);

    let metrics = consolidate_load_metrics(&latest, LoadLatencySummary::default()).unwrap();

    assert_eq!(metrics.sender_lagged_starts, Some(5));
    assert_eq!(metrics.sender_queue_depth, Some(18));
    assert_eq!(metrics.lifecycle_buckets[0].sender_lagged, 5);
}
```

- [ ] **Step 2: Add main parser/model fields**

In `main/src/server/models.rs`, add to `RunnerLoadMetricsPoint`:

```rust
pub sender_lagged_starts: Option<usize>,
pub sender_queue_depth: Option<usize>,
```

Add to `RunnerLoadLifecycleBucket` and `ConsolidatedLoadLifecycleBucket`:

```rust
pub sender_lagged: usize,
```

Add to `ConsolidatedLoadMetrics`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub sender_lagged_starts: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub sender_queue_depth: Option<usize>,
```

In `main/src/server/utils.rs`, parse:

```rust
sender_lagged_starts: get_usize_field(payload, "senderLaggedStarts"),
sender_queue_depth: get_usize_field(payload, "senderQueueDepth"),
```

In lifecycle parsing:

```rust
sender_lagged: get_usize_field(item, "senderLagged").unwrap_or(0),
```

- [ ] **Step 3: Consolidate sender saturation**

In `main/src/server/execution/load_batch.rs`, add counters:

```rust
let mut sender_lagged_starts = 0usize;
let mut sender_lagged_starts_nodes = 0usize;
let mut sender_queue_depth = 0usize;
let mut sender_queue_depth_nodes = 0usize;
```

Inside the node loop:

```rust
if let Some(value) = metrics.sender_lagged_starts {
    sender_lagged_starts = sender_lagged_starts.saturating_add(value);
    sender_lagged_starts_nodes += 1;
}
if let Some(value) = metrics.sender_queue_depth {
    sender_queue_depth = sender_queue_depth.saturating_add(value);
    sender_queue_depth_nodes += 1;
}
```

Inside lifecycle merge:

```rust
entry.sender_lagged = entry.sender_lagged.saturating_add(bucket.sender_lagged);
```

In `ConsolidatedLoadMetrics`:

```rust
sender_lagged_starts: (sender_lagged_starts_nodes > 0).then_some(sender_lagged_starts),
sender_queue_depth: (sender_queue_depth_nodes > 0).then_some(sender_queue_depth),
```

- [ ] **Step 4: Run main test**

Run:

```bash
cargo test -p previa-main consolidates_sender_saturation_metrics
```

Expected: pass.

- [ ] **Step 5: Add app types and parsing**

In `app/src/types/load-test.ts`, add optional fields to `RemoteMetricsEvent`, `LoadTestMetrics`, and `ConsolidatedLoadMetrics`:

```ts
senderLaggedStarts?: number;
senderQueueDepth?: number;
```

Add to `LoadLifecycleBucket`:

```ts
senderLagged?: number;
```

In `app/src/lib/remote-executor.ts`, parse and aggregate:

```ts
senderLaggedStarts: toNumber(value.senderLaggedStarts),
senderQueueDepth: toNumber(value.senderQueueDepth),
```

When aggregating:

```ts
senderLaggedStarts: (acc.senderLaggedStarts ?? 0) + (item.senderLaggedStarts ?? 0) || undefined,
senderQueueDepth: (acc.senderQueueDepth ?? 0) + (item.senderQueueDepth ?? 0) || undefined,
```

In lifecycle extraction:

```ts
senderLagged: toNumber(item.senderLagged) ?? 0,
```

In `app/src/lib/api-client.ts`, map from `consolidated`.

- [ ] **Step 6: Show the diagnostic in results**

In `app/src/components/LoadTestResultsPanel.tsx`, add a compact metric next to existing wave lifecycle diagnostics:

```tsx
<MetricCard
  label={t("loadTestResults.senderLaggedStarts")}
  value={formatCount(metrics.senderLaggedStarts ?? 0)}
/>
<MetricCard
  label={t("loadTestResults.senderQueueDepth")}
  value={formatCount(metrics.senderQueueDepth ?? 0)}
/>
```

Add locale keys in `app/src/i18n/locales/pt-BR.json` and `app/src/i18n/locales/en.json`:

```json
"loadTestResults.senderLaggedStarts": "Atrasos do sender",
"loadTestResults.senderQueueDepth": "Fila do sender"
```

```json
"loadTestResults.senderLaggedStarts": "Sender lagged starts",
"loadTestResults.senderQueueDepth": "Sender queue"
```

- [ ] **Step 7: Run app tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel remote-executor api-client
cd app && ./node_modules/.bin/tsc --noEmit
```

Expected: tests and typecheck pass.

- [ ] **Step 8: Commit**

```bash
git add main/src/server/models.rs main/src/server/utils.rs main/src/server/execution/load_batch.rs app/src/types/load-test.ts app/src/lib/remote-executor.ts app/src/lib/api-client.ts app/src/components/LoadTestResultsPanel.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json
git commit -m "feat: surface wave sender saturation"
```

---

### Task 5: Verification With Real Wave Load

**Files:**
- No code changes unless a prior task fails verification.

- [ ] **Step 1: Run Rust and app verification**

Run:

```bash
cargo test -p previa-runner
cargo test -p previa-main
npm --prefix app test -- LoadTestResultsPanel remote-executor api-client
cd app && ./node_modules/.bin/tsc --noEmit
npm --prefix app run build
cargo build --release
```

Expected:

- runner tests pass
- main tests pass
- app tests pass
- TypeScript passes
- app production build succeeds
- release build succeeds

- [ ] **Step 2: Restart main and three runners**

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
curl -s http://127.0.0.1:5610/info | jq -c '{activeRunners, runners: [.runners[].endpoint]}'
```

Expected:

```json
{"activeRunners":3,"runners":["http://127.0.0.1:5611","http://127.0.0.1:5612","http://127.0.0.1:5613"]}
```

- [ ] **Step 3: Run the same CRUD Users wave test**

Open:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Use the same wave shape:

```text
0s -> 10%
120s -> 80%
smooth interpolation
3 runners
```

- [ ] **Step 4: Analyze the latest result via API**

Run:

```bash
curl -s 'http://127.0.0.1:5610/api/v1/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/tests/load?limit=1' \
  | jq '.[0].finalConsolidated | {
      scheduledStarts,
      dispatchStarted,
      httpStarted,
      senderLaggedStarts,
      senderQueueDepth,
      missedStarts,
      curveAdherence,
      lifecycleBuckets: (.lifecycleBuckets | length)
    }'
```

Expected interpretation:

- If `senderLaggedStarts` is `0` or near `0`, the sender is keeping up.
- If `senderLaggedStarts` is high, the runner/host is saturated, and late catch-up should no longer distort buckets after the wave end.
- Buckets after the final wave point should not show large `httpStarted` catch-up unless those requests were still inside their sender deadline.

- [ ] **Step 5: Commit final verification note if docs changed**

If verification produced a documented observation, update the plan or a follow-up diagnostics doc and commit:

```bash
git add docs/superpowers/plans/2026-05-05-wave-deterministic-sender.md
git commit -m "docs: plan deterministic wave sender"
```

---

## Acceptance Criteria

- The sender no longer spawns one Tokio task per request.
- Prepared requests carry `scheduled_elapsed_ms` and `expires_at_elapsed_ms`.
- Requests expired in the sender queue are dropped and counted as `senderLaggedStarts`.
- Late catch-up after the final wave point is eliminated or explicitly reported as sender saturation.
- `planned`, `sendStarted`, `httpStarted`, `httpSendReturned`, `responseBodyCompleted`, and `senderLagged` are visible per bucket.
- The app clearly shows sender saturation diagnostics.
- `cargo test -p previa-runner`, `cargo test -p previa-main`, targeted app tests, TypeScript, app build, and `cargo build --release` all pass.

## Self-Review

- Spec coverage: the plan addresses the current jitter, late catch-up, and the need to attribute failures to runner infrastructure rather than application behavior.
- Placeholder scan: every task includes concrete files, commands, and implementation details.
- Type consistency: `senderLaggedStarts`, `senderQueueDepth`, and `senderLagged` are used consistently across runner, main, and app.
