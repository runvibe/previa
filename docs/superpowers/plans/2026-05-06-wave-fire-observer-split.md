# Wave Fire/Observer Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the wave sender fire requests on schedule without being slowed by response handling, body reading, assertion evaluation, or continuation routing.

**Architecture:** Split the current sender into two responsibility paths. The fire path only receives prepared requests, checks slot deadline, records launch metrics, and hands the request to an observer path. The observer path runs independently and owns `reqwest::send()`, response body, assertions, network metrics, `response_in_flight`, and continuation delivery.

**Tech Stack:** Rust, Tokio runtimes/threads, `mpsc::UnboundedSender`, Reqwest, existing Previa runner metrics, main/app telemetry, existing cargo/npm verification flow.

---

## Current Finding

The latest load test proves the scheduler and preparation layers are doing the right thing:

```text
planned == requestPrepared == requestEnqueued
```

The loss appears after enqueue, inside the sender:

```text
60-69s    planned 14795 | httpStarted 11496 | senderLagged 3299
90-99s    planned 21619 | httpStarted 17862 | senderLagged 3751
110-119s  planned 23859 | httpStarted 16815 | senderLagged 7056
```

That means the next fix must not change the wave math. It must remove response/completion work from the path that accepts and launches scheduled requests.

## File Structure

- Modify: `runner/src/server/wave_sender.rs`
  - Split current worker responsibilities into fire workers and observer workers.
  - Move `SendStarted`, `DispatchStarted`, `HttpStarted`, `SendTaskSpawned`, and deadline-drop logic into fire workers.
  - Move `send_prepared_http_step_with_hooks`, `HttpSendReturned`, `ResponseBodyCompleted`, `HttpCompleted`, network bytes, and `WaveObserverEvent` send into observer tasks.

- Modify: `runner/src/server/wave_metrics_actor.rs`
  - No new metric type is required for the core split.
  - Keep existing `SenderLaggedStarts` and `SenderQueueDepth`.

- Modify: `runner/src/server/metrics.rs`
  - No required model change.
  - Optional: add a future `observerQueueDepth` only if tests show observer intake can fall behind.

- Modify: `runner/src/server/models.rs`
  - No required model change unless adding optional observer queue depth.

- Modify: `main/src/server/execution/load_batch.rs`
  - No required aggregation change for the core split.
  - Optional: aggregate observer queue depth if added.

- Modify: `app/src/components/LoadTestResultsPanel.tsx`
  - No required UI change for the core split.
  - Optional: add observer queue depth card if added.

---

### Task 1: Add a Regression Test for Fire Path Independence

**Files:**
- Modify: `runner/src/server/wave_sender.rs`
- Test: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Write the failing test**

Add this test to `runner/src/server/wave_sender.rs` inside `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn sender_fire_path_accepts_requests_without_polling_responses() {
    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
    let (observer_tx, mut observer_rx) = mpsc::unbounded_channel();
    let (result_tx, _result_rx) = mpsc::unbounded_channel::<WaveObserverEvent<usize>>();
    let ready_to_send = Arc::new(AtomicUsize::new(0));
    let response_in_flight = Arc::new(AtomicUsize::new(0));
    let token = tokio_util::sync::CancellationToken::new();
    let started = Instant::now();

    for index in 0..128usize {
        ready_to_send.fetch_add(1, Ordering::SeqCst);
        request_tx
            .send(test_ready_wave_request(index, started, 0, 60_000))
            .expect("request should enqueue");
    }
    drop(request_tx);

    run_fire_only_sender_for_test(
        started,
        metric_tx.clone(),
        Arc::clone(&response_in_flight),
        Arc::clone(&ready_to_send),
        request_rx,
        observer_tx,
        token.clone(),
    )
    .await;

    let mut observer_commands = 0usize;
    while observer_rx.try_recv().is_ok() {
        observer_commands += 1;
    }

    let mut http_started = 0usize;
    while let Ok(event) = metric_rx.try_recv() {
        if matches!(event, WaveMetricEvent::HttpStarted { .. }) {
            http_started += 1;
        }
    }

    assert_eq!(observer_commands, 128);
    assert_eq!(http_started, 128);
    assert_eq!(ready_to_send.load(Ordering::SeqCst), 0);
    assert_eq!(response_in_flight.load(Ordering::SeqCst), 128);
    drop(result_tx);
}
```

Also add this test helper in the same test module:

```rust
fn test_ready_wave_request(
    cursor: usize,
    started: Instant,
    scheduled_elapsed_ms: u64,
    expires_at_elapsed_ms: u64,
) -> ReadyWaveRequest<usize> {
    let step = PipelineStep {
        id: format!("step-{cursor}"),
        name: "GET".to_owned(),
        description: None,
        method: "GET".to_owned(),
        url: "http://127.0.0.1/test".to_owned(),
        headers: HashMap::new(),
        body: None,
        operation_id: None,
        delay: None,
        retry: None,
        asserts: Vec::new(),
    };
    let prepared = PreparedHttpStep {
        step_id: step.id.clone(),
        attempt: 1,
        max_attempts: 1,
        method: reqwest::Method::GET,
        url: reqwest::Url::parse("http://127.0.0.1/test").unwrap(),
        request: previa_runner::StepRequest {
            method: "GET".to_owned(),
            url: "http://127.0.0.1/test".to_owned(),
            headers: HashMap::new(),
            body: None,
        },
        started_at: started,
    };

    ReadyWaveRequest {
        step,
        cursor,
        context: HashMap::new(),
        prepared,
        specs: Arc::new(Vec::new()),
        env_groups: Arc::new(Vec::new()),
        selected_env_group_slug: None,
        scheduled_elapsed_ms,
        expires_at_elapsed_ms,
        sender_enqueued_elapsed_ms: scheduled_elapsed_ms,
    }
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run:

```bash
cargo test -p previa-runner sender_fire_path_accepts_requests_without_polling_responses
```

Expected: fail because `run_fire_only_sender_for_test` and the observer command type do not exist yet.

- [ ] **Step 3: Commit the failing regression test**

```bash
git add runner/src/server/wave_sender.rs
git commit -m "test: capture fire path independence"
```

---

### Task 2: Introduce Observer Commands and a Fire-Only Sender Loop

**Files:**
- Modify: `runner/src/server/wave_sender.rs`
- Test: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Add an observer command type**

Near `SenderWorkerCommand<C>`, add:

```rust
struct ObserverCommand<C> {
    request: ReadyWaveRequest<C>,
}
```

- [ ] **Step 2: Replace response polling in `run_sender_worker`**

Change `run_sender_worker` so it no longer owns `FuturesUnordered` and no longer calls `observe_ready_request`. Its job is to fire and forward:

```rust
async fn run_sender_worker<C>(
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    mut worker_rx: mpsc::UnboundedReceiver<SenderWorkerCommand<C>>,
    observer_tx: mpsc::UnboundedSender<ObserverCommand<C>>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
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
                    ready_to_send: &ready_to_send,
                    token: &token,
                }) {
                    continue;
                }

                let dispatch_elapsed_ms = started.elapsed().as_millis() as u64;
                ready_to_send.fetch_sub(1, Ordering::SeqCst);
                response_in_flight.fetch_add(1, Ordering::SeqCst);

                let _ = metric_tx.send(WaveMetricEvent::SenderQueueDepth {
                    depth: ready_to_send.load(Ordering::SeqCst),
                });
                let _ = metric_tx.send(WaveMetricEvent::SendTaskSpawned {
                    elapsed_ms: dispatch_elapsed_ms,
                });
                let _ = metric_tx.send(WaveMetricEvent::SendStarted {
                    elapsed_ms: dispatch_elapsed_ms,
                });
                let _ = metric_tx.send(WaveMetricEvent::DispatchStarted {
                    elapsed_ms: dispatch_elapsed_ms,
                });
                let _ = metric_tx.send(WaveMetricEvent::HttpStarted {
                    elapsed_ms: dispatch_elapsed_ms,
                });

                if observer_tx.send(ObserverCommand { request }).is_err() {
                    response_in_flight.fetch_sub(1, Ordering::SeqCst);
                    break;
                }
            }
            else => break,
        }
    }
}
```

- [ ] **Step 3: Add a test-only wrapper**

Add this function under `#[cfg(test)]`:

```rust
#[cfg(test)]
async fn run_fire_only_sender_for_test<C>(
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    mut request_rx: mpsc::UnboundedReceiver<ReadyWaveRequest<C>>,
    observer_tx: mpsc::UnboundedSender<ObserverCommand<C>>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let (worker_tx, worker_rx) = mpsc::unbounded_channel();
    let worker = tokio::spawn(run_sender_worker(
        started,
        metric_tx,
        response_in_flight,
        ready_to_send,
        worker_rx,
        observer_tx,
        token,
    ));

    while let Some(request) = request_rx.recv().await {
        worker_tx
            .send(SenderWorkerCommand { request })
            .expect("worker should receive request");
    }
    drop(worker_tx);
    worker.await.expect("worker should finish");
}
```

- [ ] **Step 4: Run the regression test**

Run:

```bash
cargo test -p previa-runner sender_fire_path_accepts_requests_without_polling_responses
```

Expected: pass.

- [ ] **Step 5: Commit fire-only sender loop**

```bash
git add runner/src/server/wave_sender.rs
git commit -m "refactor: split wave sender fire path"
```

---

### Task 3: Add an Independent Observer Runtime

**Files:**
- Modify: `runner/src/server/wave_sender.rs`
- Test: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Move response accounting into observer task**

Replace `observe_ready_request` with a wrapper that always decrements `response_in_flight` when done:

```rust
async fn run_observer_request<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    request: ReadyWaveRequest<C>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let result = observe_ready_request(
        client,
        started,
        metric_tx,
        request,
        token,
    )
    .await;

    response_in_flight.fetch_sub(1, Ordering::SeqCst);
    if let Some(event) = result {
        let _ = observer_tx.send(event);
    }
}
```

Then remove `SendStarted`, `DispatchStarted`, and `HttpStarted` from `observe_ready_request`; those are now fire-path metrics.

- [ ] **Step 2: Add observer loop**

Add:

```rust
async fn run_observer_loop<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    mut observer_rx: mpsc::UnboundedReceiver<ObserverCommand<C>>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let mut join_set = JoinSet::new();

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                break;
            }
            Some(command) = observer_rx.recv() => {
                join_set.spawn(run_observer_request(
                    Arc::clone(&client),
                    started,
                    metric_tx.clone(),
                    Arc::clone(&response_in_flight),
                    observer_tx.clone(),
                    command.request,
                    token.clone(),
                ));
            }
            Some(_) = join_set.join_next(), if !join_set.is_empty() => {}
            else => break,
        }
    }

    while join_set.join_next().await.is_some() {}
}
```

Move the existing `JoinSet` import out from `#[cfg(test)]` because production observer loop now uses it:

```rust
use tokio::task::JoinSet;
```

- [ ] **Step 3: Start observer loop from `WaveSender::run`**

At the start of `WaveSender::run`, create an observer channel and spawn the observer loop:

```rust
let (observer_command_tx, observer_command_rx) = mpsc::unbounded_channel();
let observer = tokio::spawn(run_observer_loop(
    Arc::clone(&self.client),
    self.started,
    self.metric_tx.clone(),
    Arc::clone(&self.response_in_flight),
    observer_command_rx,
    self.observer_tx.clone(),
    self.token.clone(),
));
```

Pass `observer_command_tx.clone()` into every `run_sender_worker` call.

At shutdown:

```rust
drop(worker_txs);
for worker in workers {
    if cancelled {
        worker.abort();
    }
    let _ = worker.await;
}
drop(observer_command_tx);
if cancelled {
    observer.abort();
}
let _ = observer.await;
```

- [ ] **Step 4: Add observer completion test**

Add:

```rust
#[tokio::test]
async fn observer_decrements_in_flight_after_response_completion() {
    let server = httpmock::MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(httpmock::Method::GET).path("/ok");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!({"ok": true}));
        })
        .await;

    let client = Arc::new(Client::new());
    let started = Instant::now();
    let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
    let (observer_command_tx, observer_command_rx) = mpsc::unbounded_channel();
    let (observer_tx, mut observer_result_rx) = mpsc::unbounded_channel();
    let response_in_flight = Arc::new(AtomicUsize::new(1));
    let token = tokio_util::sync::CancellationToken::new();

    let mut request = test_ready_wave_request(7, started, 0, 60_000);
    request.step.url = format!("{}/ok", server.base_url());
    request.prepared.url = reqwest::Url::parse(&request.step.url).unwrap();
    request.prepared.request.url = request.step.url.clone();

    observer_command_tx
        .send(ObserverCommand { request })
        .expect("observer command should enqueue");
    drop(observer_command_tx);

    run_observer_loop(
        client,
        started,
        metric_tx,
        Arc::clone(&response_in_flight),
        observer_command_rx,
        observer_tx,
        token,
    )
    .await;

    assert_eq!(response_in_flight.load(Ordering::SeqCst), 0);
    let event = observer_result_rx
        .try_recv()
        .expect("observer should emit completed event");
    assert_eq!(event.cursor, 7);
    assert_eq!(event.result.status, "success");
}
```

- [ ] **Step 5: Run sender tests**

Run:

```bash
cargo test -p previa-runner wave_sender
```

Expected: all wave sender tests pass.

- [ ] **Step 6: Commit observer runtime split**

```bash
git add runner/src/server/wave_sender.rs
git commit -m "refactor: isolate wave response observer"
```

---

### Task 4: Recalibrate Metrics Semantics

**Files:**
- Modify: `runner/src/server/wave_sender.rs`
- Modify only if needed: `app/src/components/LoadTestResultsPanel.tsx`
- Test: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Confirm metric ownership**

After the split, metric ownership must be:

```text
Fire path:
- SenderQueueDepth
- SendTaskSpawned
- SendStarted
- DispatchStarted
- HttpStarted
- SenderLaggedStarts

Observer path:
- HttpSendReturned
- ResponseBodyCompleted
- HttpCompleted
- NetworkBytes
- WaveObserverEvent
```

Do not record `HttpStarted` in the observer path. If it remains there, the graph can still lag behind the fire path.

- [ ] **Step 2: Add assertion to existing sender event test**

Update the existing test `sender_emits_dispatch_events_for_accepted_requests` so it verifies exactly one `HttpStarted` per accepted request and no duplicate event from observer:

```rust
assert_eq!(http_started_count, accepted_request_count);
assert_eq!(dispatch_started_count, accepted_request_count);
assert_eq!(send_started_count, accepted_request_count);
```

- [ ] **Step 3: Run metric tests**

Run:

```bash
cargo test -p previa-runner sender_emits_dispatch_events_for_accepted_requests
cargo test -p previa-runner sender_drops_expired_request_instead_of_late_catchup
```

Expected: both pass.

- [ ] **Step 4: Commit metric ownership correction**

```bash
git add runner/src/server/wave_sender.rs
git commit -m "fix: record wave http start on fire path"
```

---

### Task 5: Verify With the Real Three-Runner Scenario

**Files:**
- No required code files.
- Use logs and DB records:
  - `/private/tmp/previa-verify-5610.db`
  - `/tmp/previa-main-5610.log`
  - `/tmp/previa-runner-5611.log`
  - `/tmp/previa-runner-5612.log`
  - `/tmp/previa-runner-5613.log`

- [ ] **Step 1: Run full automated verification**

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

```text
previa-runner: pass
previa-main: pass
Vitest focused tests: pass
tsc: pass
app build: pass
cargo build --release: pass
```

- [ ] **Step 2: Restart main and three runners from release binaries**

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

- [ ] **Step 3: Execute the same load test from the UI**

Open:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Run the same wave:

```text
0s -> 10%
120s -> 80%
interpolation: smooth
3 runners
```

- [ ] **Step 4: Analyze the latest run from SQLite**

Run:

```bash
python3 - <<'PY'
import sqlite3, json
con = sqlite3.connect('/private/tmp/previa-verify-5610.db')
rec = con.execute(
    'select final_consolidated_json from load_history order by started_at_ms desc limit 1'
).fetchone()
metrics = json.loads(rec[0])
buckets = metrics.get('lifecycleBuckets') or []
print({
    'scheduledStarts': metrics.get('scheduledStarts'),
    'httpStarted': metrics.get('httpStarted'),
    'senderLaggedStarts': metrics.get('senderLaggedStarts'),
    'curveAdherence': metrics.get('curveAdherence'),
    'readyRequests': metrics.get('readyRequests'),
    'outstandingRequests': metrics.get('outstandingRequests'),
})
for start in range(0, 121000, 10000):
    group = [b for b in buckets if start <= b.get('elapsedMs', 0) < start + 10000]
    planned = sum(b.get('planned', 0) for b in group)
    http = sum(b.get('httpStarted', 0) for b in group)
    lag = sum(b.get('senderLagged', 0) for b in group)
    if planned:
        print(f'{start//1000:03d}-{(start+9000)//1000:03d}s planned={planned} httpStarted={http} senderLagged={lag}')
PY
```

Expected improvement:

```text
senderLaggedStarts should be materially lower than 24.3K from the previous run.
For 0-60s, senderLagged should remain near zero.
For 60-120s, any remaining gap should correlate with actual infra saturation, not observer backlog.
```

- [ ] **Step 5: Commit final verification notes if code changed after Task 4**

If only logs changed, do not commit logs. If a code tweak was needed:

```bash
git add runner/src/server/wave_sender.rs
git commit -m "fix: keep wave fire path independent"
```

---

## Risk Notes

- Counting `HttpStarted` on the fire path means it represents “runner launched the request task”, not “response headers arrived”. That is intentional for open-loop RPS.
- `HttpSendReturned` remains the marker for `reqwest::send()` completion.
- `ResponseBodyCompleted` remains the marker for body drain.
- `HttpCompleted` remains the marker for a complete `StepExecutionResult`.
- If `senderLaggedStarts` stays high after this split, the remaining bottleneck is likely CPU/runtime scheduling/socket pressure in the runner infrastructure.

## Self-Review

- Spec coverage: covers the required split between request launch and response observation.
- Placeholder scan: no `TBD`, no deferred “handle later” tasks.
- Type consistency: `ObserverCommand<C>`, `ReadyWaveRequest<C>`, and `WaveObserverEvent<C>` are consistently named across tasks.
- Verification: includes unit tests, full cargo/npm checks, release build, restart, and real-run SQLite analysis.
