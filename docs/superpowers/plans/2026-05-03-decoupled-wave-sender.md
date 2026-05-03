# Decoupled Wave Sender Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure wave load request sending is not delayed by response observation, body reading, assertions, or pipeline continuation work.

**Architecture:** Split the runner wave path into a scheduler/producer, a dedicated HTTP sender, and response observers. The scheduler computes wave slots and prepares requests; the sender owns the hot path that starts HTTP requests and records `httpStarted`; observers handle response/body/assertions and feed pipeline continuations back asynchronously.

**Tech Stack:** Rust, Tokio `mpsc`, `JoinSet`, `reqwest::Client`, existing `previa_runner::prepare_http_step` and `send_prepared_http_step_with_hooks`, Vitest/React for UI metric labels only if needed.

---

## File Structure

- Modify `runner/src/server/wave_executor.rs`
  - Keep wave timing, pipeline cursor production, prepare errors, continuation handling, and SSE snapshots.
  - Stop spawning response observer tasks directly from the wave loop.
  - Send prepared requests to a dedicated sender through an internal channel.

- Create `runner/src/server/wave_sender.rs`
  - Own the hot send path.
  - Receive prepared requests from the wave executor.
  - Record `httpStarted` immediately before starting the HTTP send task.
  - Spawn response observer tasks.
  - Drain finished response tasks independently from the wave scheduler loop.
  - Send `ObserverEvent` back to `wave_executor`.

- Modify `runner/src/server/mod.rs`
  - Register `wave_sender`.

- Modify `runner/src/server/metrics.rs`
  - Add or expose counters only if needed for sender-level diagnostics.
  - Prefer existing counters first: `httpStarted`, `httpSendReturned`, `responseBodyCompleted`, `runtimeLaggedStarts`, `readyRequests`, `outstandingRequests`.

- Modify `runner/src/server/load_dispatch.rs`
  - Keep `DispatchClock` unchanged unless tests prove clock semantics are currently part of the coupling.

- Optional modify `app/src/components/LoadTestResultsPanel.tsx`
  - Only if metric names need clarification after backend metrics are corrected.

---

## Behavior Contract

The implementation must preserve this contract:

- The wave scheduler must not await HTTP response, body, assertions, or observer task completion before scheduling the next tick.
- A slow or failed response may increase `pendingResponses` and delay only that pipeline continuation.
- A slow or failed response must not delay starting unrelated new pipelines.
- `httpStarted` is the primary RPS line and should track the wave while request preparation and `client.execute` start are healthy.
- `readyRequests` means prepared requests waiting for the sender.
- `runtimeLaggedStarts` means the runner could not start requests in the tick window.
- `dependencyLimitedStarts` means the pipeline could not produce a dependent next step because prior context was unavailable or invalid.

---

### Task 1: Add a Sender Unit Test Harness

**Files:**
- Create: `runner/src/server/wave_sender.rs`
- Modify: `runner/src/server/mod.rs`

- [ ] **Step 1: Create a minimal sender module with a failing test**

Add this module skeleton:

```rust
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc;
use tokio::task::JoinSet;

#[derive(Debug)]
pub struct ReadyWaveRequest<T> {
    pub payload: T,
}

pub async fn run_test_sender<T, F, Fut>(
    mut rx: mpsc::UnboundedReceiver<ReadyWaveRequest<T>>,
    started: Arc<AtomicUsize>,
    mut send: F,
) where
    T: Send + 'static,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    while let Some(request) = rx.recv().await {
        started.fetch_add(1, Ordering::SeqCst);
        let fut = send(request.payload);
        tasks.spawn(fut);
    }
    tasks.abort_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn sender_starts_later_requests_without_waiting_for_prior_response_task() {
        let (tx, rx) = mpsc::unbounded_channel();
        let started = Arc::new(AtomicUsize::new(0));
        let blocker = Arc::new(Notify::new());

        let sender_started = Arc::clone(&started);
        let sender_blocker = Arc::clone(&blocker);
        let sender = tokio::spawn(run_test_sender(rx, sender_started, move |_payload: usize| {
            let blocker = Arc::clone(&sender_blocker);
            async move {
                blocker.notified().await;
            }
        }));

        tx.send(ReadyWaveRequest { payload: 1 }).unwrap();
        tx.send(ReadyWaveRequest { payload: 2 }).unwrap();
        tx.send(ReadyWaveRequest { payload: 3 }).unwrap();

        timeout(Duration::from_millis(100), async {
            loop {
                if started.load(Ordering::SeqCst) == 3 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("sender should start all requests while response tasks are blocked");

        drop(tx);
        sender.abort();
    }
}
```

- [ ] **Step 2: Register the module**

In `runner/src/server/mod.rs`, add:

```rust
mod wave_sender;
```

- [ ] **Step 3: Run the focused test**

Run:

```bash
cargo test -p previa-runner wave_sender::tests::sender_starts_later_requests_without_waiting_for_prior_response_task
```

Expected: PASS. This establishes the sender behavior in isolation before wiring real HTTP.

- [ ] **Step 4: Commit**

```bash
git add runner/src/server/wave_sender.rs runner/src/server/mod.rs
git commit -m "test: add wave sender independence harness"
```

---

### Task 2: Move Real HTTP Observation Into `wave_sender`

**Files:**
- Modify: `runner/src/server/wave_sender.rs`
- Modify: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Move request payload ownership into the sender**

In `runner/src/server/wave_sender.rs`, replace the test-only generic request with real wave request types:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reqwest::Client;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use previa_runner::{
    PipelineStep, PreparedHttpStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult,
    send_prepared_http_step_with_hooks,
};

use crate::server::metrics::{MetricsAccumulator, estimate_results_network_bytes};

pub struct ReadyWaveRequest<C> {
    pub step: PipelineStep,
    pub cursor: C,
    pub prepared: PreparedHttpStep,
    pub specs: Arc<Vec<RuntimeSpec>>,
    pub env_groups: Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: Option<String>,
}

pub struct WaveObserverEvent<C> {
    pub cursor: C,
    pub result: StepExecutionResult,
}

pub struct WaveSender<C> {
    client: Arc<Client>,
    metrics: Arc<tokio::sync::Mutex<MetricsAccumulator>>,
    response_in_flight: Arc<AtomicUsize>,
    request_rx: mpsc::UnboundedReceiver<ReadyWaveRequest<C>>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    token: tokio_util::sync::CancellationToken,
}
```

- [ ] **Step 2: Implement the production sender loop**

Add:

```rust
impl<C> WaveSender<C>
where
    C: Send + 'static,
{
    pub fn new(
        client: Arc<Client>,
        metrics: Arc<tokio::sync::Mutex<MetricsAccumulator>>,
        response_in_flight: Arc<AtomicUsize>,
        request_rx: mpsc::UnboundedReceiver<ReadyWaveRequest<C>>,
        observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
        token: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            client,
            metrics,
            response_in_flight,
            request_rx,
            observer_tx,
            token,
        }
    }

    pub async fn run(mut self) {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                maybe_request = self.request_rx.recv() => {
                    let Some(request) = maybe_request else {
                        break;
                    };
                    if self.token.is_cancelled() {
                        break;
                    }
                    self.spawn_observer(&mut tasks, request).await;
                }
                Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                    if let Err(err) = joined {
                        if !err.is_cancelled() {
                            tracing::error!("wave sender observer task failed: {err}");
                        }
                    }
                }
            }
        }

        if !tasks.is_empty() {
            tasks.abort_all();
        }
        while let Some(joined) = tasks.join_next().await {
            if let Err(err) = joined {
                if !err.is_cancelled() {
                    tracing::error!("wave sender observer task failed during shutdown: {err}");
                }
            }
        }
    }
}
```

- [ ] **Step 3: Implement `spawn_observer`**

Add:

```rust
impl<C> WaveSender<C>
where
    C: Send + 'static,
{
    async fn spawn_observer(&self, tasks: &mut JoinSet<()>, request: ReadyWaveRequest<C>) {
        self.response_in_flight.fetch_add(1, Ordering::SeqCst);
        {
            let mut lock = self.metrics.lock().await;
            lock.record_http_start();
        }

        let client = Arc::clone(&self.client);
        let metrics = Arc::clone(&self.metrics);
        let metrics_for_send = Arc::clone(&self.metrics);
        let metrics_for_body = Arc::clone(&self.metrics);
        let response_in_flight = Arc::clone(&self.response_in_flight);
        let observer_tx = self.observer_tx.clone();
        let token = self.token.clone();

        tasks.spawn(async move {
            let result = send_prepared_http_step_with_hooks(
                client.as_ref(),
                request.prepared,
                &request.step,
                &std::collections::HashMap::new(),
                Some(request.specs.as_slice()),
                Some(request.env_groups.as_slice()),
                request.selected_env_group_slug.as_deref(),
                || token.is_cancelled(),
                move || {
                    let metrics = Arc::clone(&metrics_for_send);
                    async move {
                        let mut lock = metrics.lock().await;
                        lock.record_http_send_returned();
                    }
                },
                move || {
                    let metrics = Arc::clone(&metrics_for_body);
                    async move {
                        let mut lock = metrics.lock().await;
                        lock.record_response_body_completed_count(1);
                    }
                },
            )
            .await;

            response_in_flight.fetch_sub(1, Ordering::SeqCst);
            let Some(result) = result else {
                return;
            };

            let (network_tx_bytes, network_rx_bytes) =
                estimate_results_network_bytes(std::slice::from_ref(&result));
            {
                let mut lock = metrics.lock().await;
                if result.request.is_some() {
                    lock.record_http_completed_count(1);
                }
                lock.add_network_bytes(network_tx_bytes, network_rx_bytes);
            }

            let _ = observer_tx.send(WaveObserverEvent {
                cursor: request.cursor,
                result,
            });
        });
    }
}
```

Important implementation note: Step 4 immediately replaces the empty context in the intermediate snippet with the cursor context carried by `ReadyWaveRequest`. `send_prepared_http_step_with_hooks` must receive that context to preserve template/assertion behavior.

- [ ] **Step 4: Move cursor context into the ready request**

Update `ReadyWaveRequest<C>` to carry the prepared send context explicitly:

```rust
pub struct ReadyWaveRequest<C> {
    pub step: PipelineStep,
    pub cursor: C,
    pub context: std::collections::HashMap<String, StepExecutionResult>,
    pub prepared: PreparedHttpStep,
    pub specs: Arc<Vec<RuntimeSpec>>,
    pub env_groups: Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: Option<String>,
}
```

Then pass `&request.context` to `send_prepared_http_step_with_hooks`.

- [ ] **Step 5: Run sender tests**

Run:

```bash
cargo test -p previa-runner wave_sender
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add runner/src/server/wave_sender.rs
git commit -m "feat: add dedicated wave HTTP sender"
```

---

### Task 3: Wire `run_wave_load` To The Dedicated Sender

**Files:**
- Modify: `runner/src/server/wave_executor.rs`
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Replace direct `JoinSet` observer spawning**

In `runner/src/server/wave_executor.rs`, remove:

```rust
let mut tasks = JoinSet::new();
let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ObserverEvent>();
```

Replace with:

```rust
let (request_tx, request_rx) = mpsc::unbounded_channel();
let (event_tx, mut event_rx) = mpsc::unbounded_channel();
let sender = crate::server::wave_sender::WaveSender::new(
    Arc::clone(&http_client),
    Arc::clone(&metrics),
    Arc::clone(&response_in_flight),
    request_rx,
    event_tx,
    token.clone(),
);
let sender_task = tokio::spawn(sender.run());
```

- [ ] **Step 2: Stop draining observer task joins in the wave loop**

Remove calls like:

```rust
log_drain_report(drain_finished_tasks(&mut tasks).await);
```

from the hot wave scheduling loop. Keep only:

```rust
drain_observer_events(&mut event_rx, &mut ready, &pipeline, &metrics).await;
```

This ensures the wave scheduler does not spend tick time joining response observer tasks.

- [ ] **Step 3: Send prepared requests to the sender**

Replace:

```rust
spawn_observed_step(ObservedStepArgs {
    tasks: &mut tasks,
    client: Arc::clone(&http_client),
    metrics: Arc::clone(&metrics),
    response_in_flight: Arc::clone(&response_in_flight),
    event_tx: event_tx.clone(),
    token: token.clone(),
    step,
    cursor,
    prepared,
    specs: Arc::clone(&specs),
    env_groups: Arc::clone(&env_groups),
    selected_env_group_slug: selected_env_group_slug.clone(),
});
```

with:

```rust
let _ = request_tx.send(crate::server::wave_sender::ReadyWaveRequest {
    step,
    context: cursor.context.clone(),
    cursor,
    prepared,
    specs: Arc::clone(&specs),
    env_groups: Arc::clone(&env_groups),
    selected_env_group_slug: selected_env_group_slug.clone(),
});
```

- [ ] **Step 4: Remove obsolete local observer task code**

Delete from `wave_executor.rs`:

```rust
struct ObservedStepArgs<'a> { ... }
fn spawn_observed_step(args: ObservedStepArgs<'_>) { ... }
async fn drain_finished_tasks(tasks: &mut JoinSet<()>) -> DrainReport { ... }
fn log_drain_report(report: DrainReport) { ... }
```

Keep `ObserverEvent` or replace it with:

```rust
use crate::server::wave_sender::WaveObserverEvent as ObserverEvent;
```

- [ ] **Step 5: Shutdown sender cleanly**

At the end of `run_wave_load`, before final snapshot:

```rust
drop(request_tx);
if !sender_task.is_finished() {
    sender_task.abort();
}
let _ = sender_task.await;
```

If this drops pending response observation too early, change shutdown to:

```rust
drop(request_tx);
let _ = tokio::time::timeout(
    tokio::time::Duration::from_millis(load.grace_period_ms),
    sender_task,
).await;
```

Use the timeout form if tests show final metrics need graceful response draining.

- [ ] **Step 6: Run focused runner tests**

Run:

```bash
cargo test -p previa-runner wave_executor wave_sender load_dispatch load_wave
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add runner/src/server/wave_executor.rs runner/src/server/wave_sender.rs
git commit -m "refactor: decouple wave scheduler from response observers"
```

---

### Task 4: Add Regression Test For Scheduler Not Joining Response Tasks

**Files:**
- Modify: `runner/src/server/wave_executor.rs`
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Add a test seam for sender injection**

Define a small trait-like enum or function parameter under `#[cfg(test)]` if needed. Prefer this minimal helper in `wave_sender.rs`:

```rust
#[cfg(test)]
pub async fn count_started_without_completing_observers(requests: usize) -> usize {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::{Notify, mpsc};
    use tokio::time::{Duration, timeout};

    let (tx, rx) = mpsc::unbounded_channel();
    let started = Arc::new(AtomicUsize::new(0));
    let blocker = Arc::new(Notify::new());
    let sender_started = Arc::clone(&started);
    let sender_blocker = Arc::clone(&blocker);

    let task = tokio::spawn(super::wave_sender::run_test_sender(
        rx,
        sender_started,
        move |_payload: usize| {
            let blocker = Arc::clone(&sender_blocker);
            async move {
                blocker.notified().await;
            }
        },
    ));

    for payload in 0..requests {
        tx.send(super::wave_sender::ReadyWaveRequest { payload }).unwrap();
    }

    timeout(Duration::from_millis(100), async {
        loop {
            let count = started.load(Ordering::SeqCst);
            if count == requests {
                break count;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all requests should be started while observers are blocked");

    drop(tx);
    task.abort();
    started.load(Ordering::SeqCst)
}
```

- [ ] **Step 2: Add regression assertion**

Add test:

```rust
#[tokio::test]
async fn blocked_response_observers_do_not_prevent_sender_from_starting_new_requests() {
    let started = crate::server::wave_sender::count_started_without_completing_observers(500).await;
    assert_eq!(started, 500);
}
```

- [ ] **Step 3: Run test**

Run:

```bash
cargo test -p previa-runner blocked_response_observers_do_not_prevent_sender_from_starting_new_requests
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add runner/src/server/wave_sender.rs runner/src/server/wave_executor.rs
git commit -m "test: protect wave sender from response observer backpressure"
```

---

### Task 5: Clarify Metrics For The New Architecture

**Files:**
- Modify: `runner/src/server/metrics.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Modify: `app/src/i18n/locales/pt-BR.json`
- Modify: `app/src/i18n/locales/en.json`

- [ ] **Step 1: Keep existing names but clarify meanings**

Use current metrics if possible:

```text
httpStarted = HTTP request start count; primary RPS source.
httpSendReturned = reqwest returned response or send error.
responseBodyCompleted = response body was fully consumed.
pendingResponses = existing inFlight/outstandingRequests.
readyRequests = prepared requests waiting for sender.
runtimeLaggedStarts = scheduled starts missed because runner runtime did not begin them in the tick window.
dependencyLimitedStarts = pipeline continuation could not be prepared due dependency/context failure.
```

- [ ] **Step 2: Update UI labels only if needed**

In `pt-BR.json`, prefer:

```json
"loadTestResults.inFlight": "Respostas pendentes",
"loadTestResults.readyRequests": "Requests prontas",
"loadTestResults.runtimeLaggedStarts": "Atrasos do emissor",
"loadTestResults.dependencyLimitedStarts": "Bloqueios por dependência"
```

In `en.json`, prefer:

```json
"loadTestResults.inFlight": "Pending responses",
"loadTestResults.readyRequests": "Ready requests",
"loadTestResults.runtimeLaggedStarts": "Sender lag",
"loadTestResults.dependencyLimitedStarts": "Dependency blocked"
```

- [ ] **Step 3: Update result panel test**

Add or update a Vitest assertion in `app/src/components/LoadTestResultsPanel.test.tsx`:

```ts
it("labels wave diagnostics by sender and response responsibility", () => {
  render(
    <LoadTestResultsPanel
      metrics={{
        ...emptyMetrics,
        inFlight: 12,
        readyRequests: 7,
        runtimeLaggedStarts: 3,
        dependencyLimitedStarts: 2,
      }}
      state="running"
      totalRequests={0}
    />,
  );

  expect(screen.getByText("loadTestResults.inFlight")).toBeInTheDocument();
  expect(screen.getByText("loadTestResults.readyRequests")).toBeInTheDocument();
  expect(screen.getByText("loadTestResults.runtimeLaggedStarts")).toBeInTheDocument();
  expect(screen.getByText("loadTestResults.dependencyLimitedStarts")).toBeInTheDocument();
});
```

- [ ] **Step 4: Run UI tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/components/LoadTestResultsPanel.test.tsx app/src/components/LoadTestResultsPanel.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json runner/src/server/metrics.rs main/src/server/execution/load_batch.rs
git commit -m "chore: clarify wave sender diagnostics"
```

---

### Task 6: Verify With The CRUD Users Scenario

**Files:**
- No code files required.
- Runtime logs: `/tmp/previa-main-5610.log`, `/tmp/previa-runner-5611.log`, `/tmp/previa-runner-5612.log`, `/tmp/previa-runner-5613.log`

- [ ] **Step 1: Build release**

Run:

```bash
cargo build --release
```

Expected: `Finished 'release' profile`.

- [ ] **Step 2: Restart all local processes by killing listeners first**

Run:

```bash
for port in 5610 5611 5612 5613; do
  lsof -tiTCP:$port -sTCP:LISTEN | xargs -r kill
done

screen -S previa-main-5610 -X quit >/dev/null 2>&1 || true
screen -S previa-runner-5611 -X quit >/dev/null 2>&1 || true
screen -S previa-runner-5612 -X quit >/dev/null 2>&1 || true
screen -S previa-runner-5613 -X quit >/dev/null 2>&1 || true

screen -dmS previa-runner-5611 zsh -lc 'cd /Users/assis/projects/previa && env ADDRESS=127.0.0.1 PORT=5611 RUST_LOG=info PREVIA_HOME=/Users/assis/.previa LOG_FORMAT=json ./target/release/previa-runner > /tmp/previa-runner-5611.log 2>&1'
screen -dmS previa-runner-5612 zsh -lc 'cd /Users/assis/projects/previa && env ADDRESS=127.0.0.1 PORT=5612 RUST_LOG=info PREVIA_HOME=/Users/assis/.previa LOG_FORMAT=json ./target/release/previa-runner > /tmp/previa-runner-5612.log 2>&1'
screen -dmS previa-runner-5613 zsh -lc 'cd /Users/assis/projects/previa && env ADDRESS=127.0.0.1 PORT=5613 RUST_LOG=info PREVIA_HOME=/Users/assis/.previa LOG_FORMAT=json ./target/release/previa-runner > /tmp/previa-runner-5613.log 2>&1'
sleep 1
screen -dmS previa-main-5610 zsh -lc 'cd /Users/assis/projects/previa && env ADDRESS=127.0.0.1 PORT=5610 RUST_LOG=info PREVIA_HOME=/Users/assis/.previa LOG_FORMAT=json ORCHESTRATOR_DATABASE_URL=sqlite:///tmp/previa-verify-5610.db RUNNER_ENDPOINTS=http://127.0.0.1:5611,http://127.0.0.1:5612,http://127.0.0.1:5613 PREVIA_APP_ENABLED=true ./target/release/previa-main > /tmp/previa-main-5610.log 2>&1'
sleep 2
curl -s http://127.0.0.1:5610/info | jq '{activeRunners,totalRunners}'
```

Expected:

```json
{
  "activeRunners": 3,
  "totalRunners": 3
}
```

- [ ] **Step 3: Run the same load test from the UI**

Open:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Use the existing wave:

```json
{
  "points": [
    { "atMs": 0, "intensity": 10 },
    { "atMs": 120000, "intensity": 100 }
  ],
  "interpolation": "smooth",
  "gracePeriodMs": 30000
}
```

- [ ] **Step 4: Analyze the last run**

Run:

```bash
PROJECT_ID=019de1a7-4dfd-7662-8b53-a305e5714ca5
LATEST=$(curl -s "http://127.0.0.1:5610/api/v1/projects/$PROJECT_ID/tests/load?limit=1" | jq -r '.[0].executionId')
curl -s "http://127.0.0.1:5610/api/v1/projects/$PROJECT_ID/tests/load/$LATEST" > /tmp/latest-load.json
node - <<'NODE'
const fs = require('fs');
const data = JSON.parse(fs.readFileSync('/tmp/latest-load.json', 'utf8'));
const c = data.finalConsolidated;
const hist = c.rpsHistory || [];
const start = c.startTime || data.startedAtMs;
const firstLag = hist.find(p => (p.runtimeLaggedStarts || 0) > 0);
const below90 = hist.find(p => typeof p.curveAdherence === 'number' && p.curveAdherence < 90);
console.log(JSON.stringify({
  executionId: data.executionId,
  status: data.status,
  httpStarted: c.httpStarted,
  responseBodyCompleted: c.responseBodyCompleted,
  pendingResponses: c.inFlight,
  readyRequests: c.readyRequests,
  curveAdherence: c.curveAdherence,
  firstRuntimeLagSecond: firstLag ? Number(((firstLag.timestamp - start) / 1000).toFixed(1)) : null,
  firstBelow90Second: below90 ? Number(((below90.timestamp - start) / 1000).toFixed(1)) : null,
  avgLatency: c.avgLatency,
  p95: c.p95,
  p99: c.p99,
  errors: data.errors
}, null, 2));
NODE
```

Expected improvement:

```text
firstRuntimeLagSecond should move later or become null for the same wave.
curveAdherence should improve materially versus the prior 64.45%.
If target still collapses, pendingResponses/errors may still grow, but httpStarted should remain closer to target.
```

- [ ] **Step 5: Commit verification notes if docs changed**

If a follow-up diagnostic doc is added:

```bash
git add docs/previa
git commit -m "docs: record wave sender verification"
```

---

## Final Verification

- [ ] Run:

```bash
cargo test -p previa-runner
```

Expected: all runner tests pass.

- [ ] Run:

```bash
cargo test -p previa-main
```

Expected: all main tests pass.

- [ ] Run:

```bash
npm --prefix app test -- LoadTestResultsPanel LoadTestConfigPanel LoadTestTab
```

Expected: all selected UI tests pass.

- [ ] Run:

```bash
npm --prefix app run build
```

Expected: Vite build succeeds. Existing chunk warnings are acceptable.

- [ ] Run:

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] Run:

```bash
git diff --check
```

Expected: no whitespace errors.

- [ ] Push:

```bash
git push
```

Expected: current feature branch is updated remotely.

---

## Self-Review

**Spec coverage:** The plan directly covers the new directive: request sending must not wait for or be impacted by response observation. It also preserves pipeline dependency semantics: only dependent continuations wait for prior responses.

**Completeness scan:** The plan contains no `TBD` or open-ended “add tests later” steps. Each task has concrete files, commands, and expected outcomes.

**Type consistency:** The plan consistently uses `ReadyWaveRequest`, `WaveObserverEvent`, `WaveSender`, `httpStarted`, `readyRequests`, `pendingResponses`, and existing runner modules.
