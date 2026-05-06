# Wave Open-Loop Sender Correction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make wave load tests preserve the planned open-loop request starts even when the sender is late, while making the metrics distinguish planned starts, accepted fire starts, real HTTP send starts, send returns, and body completion.

**Architecture:** Keep the existing scheduler, dispatcher, prepare workers, sender thread, and observer flow. Change the sender so lateness is diagnostic instead of a drop condition, and move `HttpStarted` to the exact point immediately before `reqwest::RequestBuilder::send()` is awaited. Use `sendStarted` for wave adherence because it measures the load generator's start intent, and use `httpStarted` for the real HTTP client start.

**Tech Stack:** Rust, Tokio, reqwest, Axum/SSE, SQLx/SQLite history, React/TypeScript load-test UI, Vitest.

---

## File Structure

- Modify `engine/src/execution/http_step.rs`
  - Add an `on_send_started` hook to `send_prepared_http_step_with_hooks`.
  - Invoke it immediately before `request_builder.send()`.
  - Keep `send_prepared_http_step` source-compatible by passing a no-op hook.

- Modify `runner/src/server/wave_sender.rs`
  - Replace `drop_if_expired` behavior with a late-start recorder that does not skip sending.
  - Stop emitting `HttpStarted` in the fire worker.
  - Emit `HttpStarted` from the new engine hook, inside `observe_ready_request`, immediately before the real `.send()`.
  - Keep `SenderLaggedStarts` as a late-start diagnostic metric, not as a drop count.

- Modify `runner/src/server/wave_metrics_actor.rs`
  - Preserve existing event names where possible.
  - Ensure `SenderLaggedStarts` remains aggregated but now means "started late", not "dropped".

- Modify `runner/src/server/metrics.rs`
  - Keep lifecycle buckets compatible.
  - Ensure `httpStarted` buckets now represent real HTTP send starts.
  - Ensure `senderLaggedStarts` remains visible for late starts.

- Modify `main/src/server/execution/load_batch.rs`
  - Change `curveAdherence` to compare `planned` against `sendStarted`, not `httpStarted`.
  - Keep lifecycle bucket aggregation unchanged.

- Modify `app/src/components/LoadTestResultsPanel.tsx` and locale files under `app/src/i18n` or wherever load-test labels live
  - Change the label from "Descartes do sender" to "Starts atrasados no sender".
  - Keep the same payload field `senderLaggedStarts`.

- Modify tests:
  - `engine/src/execution/http_step.rs`
  - `runner/src/server/wave_sender.rs`
  - `runner/src/server/metrics.rs`
  - `runner/src/server/wave_metrics_actor.rs`
  - `main/src/server/execution/load_batch.rs`
  - `app/src/components/LoadTestResultsPanel.test.tsx`
  - `app/src/lib/remote-executor.test.ts`
  - `app/src/lib/api-client.test.ts`

---

### Task 1: Add a Real HTTP Start Hook in the Engine

**Files:**
- Modify: `engine/src/execution/http_step.rs`

- [ ] **Step 1: Update the hook signature**

Change:

```rust
pub async fn send_prepared_http_step_with_hooks<FCancel, FSend, FSendFuture, FBody, FBodyFuture>(
    client: &Client,
    prepared: PreparedHttpStep,
    step: &PipelineStep,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    mut should_cancel: FCancel,
    mut on_send_returned: FSend,
    mut on_body_completed: FBody,
) -> Option<StepExecutionResult>
```

To:

```rust
pub async fn send_prepared_http_step_with_hooks<
    FCancel,
    FStart,
    FStartFuture,
    FSend,
    FSendFuture,
    FBody,
    FBodyFuture,
>(
    client: &Client,
    prepared: PreparedHttpStep,
    step: &PipelineStep,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    mut should_cancel: FCancel,
    mut on_send_started: FStart,
    mut on_send_returned: FSend,
    mut on_body_completed: FBody,
) -> Option<StepExecutionResult>
where
    FCancel: FnMut() -> bool,
    FStart: FnMut() -> FStartFuture,
    FStartFuture: Future<Output = ()>,
    FSend: FnMut() -> FSendFuture,
    FSendFuture: Future<Output = ()>,
    FBody: FnMut() -> FBodyFuture,
    FBodyFuture: Future<Output = ()>,
```

- [ ] **Step 2: Invoke the hook immediately before real send**

Change:

```rust
let request = prepared.request.clone();
let Some(send_result) = await_with_cancel(request_builder.send(), &mut should_cancel).await
else {
    return None;
};
on_send_returned().await;
```

To:

```rust
let request = prepared.request.clone();
on_send_started().await;
let Some(send_result) = await_with_cancel(request_builder.send(), &mut should_cancel).await
else {
    return None;
};
on_send_returned().await;
```

- [ ] **Step 3: Keep the default helper source-compatible**

Change the call in `send_prepared_http_step` from:

```rust
send_prepared_http_step_with_hooks(
    client,
    prepared,
    step,
    context,
    specs,
    env_groups,
    selected_env_group_slug,
    should_cancel,
    || async {},
    || async {},
)
.await
```

To:

```rust
send_prepared_http_step_with_hooks(
    client,
    prepared,
    step,
    context,
    specs,
    env_groups,
    selected_env_group_slug,
    should_cancel,
    || async {},
    || async {},
    || async {},
)
.await
```

- [ ] **Step 4: Run engine tests**

Run:

```bash
cargo test -p previa-engine
```

Expected: all engine tests pass.

---

### Task 2: Stop Dropping Late Sender Requests

**Files:**
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Replace `drop_if_expired` with a non-dropping late recorder**

Replace:

```rust
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
    true
}
```

With:

```rust
fn record_if_sender_late(args: SenderDeadlineCheck<'_>) -> bool {
    let elapsed_ms = args.started.elapsed().as_millis() as u64;
    if args.token.is_cancelled() || elapsed_ms <= args.expires_at_elapsed_ms {
        return false;
    }

    let _ = args.metric_tx.send(WaveMetricEvent::SenderLaggedStarts {
        elapsed_ms: args.scheduled_elapsed_ms,
        count: 1,
    });
    true
}
```

- [ ] **Step 2: Continue sending even when late**

Change the fire worker block from:

```rust
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
```

To:

```rust
let sender_was_late = record_if_sender_late(SenderDeadlineCheck {
    scheduled_elapsed_ms: request.scheduled_elapsed_ms,
    expires_at_elapsed_ms: request.expires_at_elapsed_ms,
    started,
    metric_tx: &metric_tx,
    ready_to_send: &ready_to_send,
    token: &token,
});
drop(sender_was_late);
```

Then keep the existing `ready_to_send.fetch_sub(1, ...)`, `response_in_flight.fetch_add(1, ...)`, and observer forwarding path untouched.

- [ ] **Step 3: Remove optimistic `HttpStarted` from the fire worker**

Delete this block from `run_sender_worker`:

```rust
let _ = metric_tx.send(WaveMetricEvent::HttpStarted {
    elapsed_ms: dispatch_elapsed_ms,
});
```

Keep these events in the fire worker:

```rust
let _ = metric_tx.send(WaveMetricEvent::SendTaskSpawned {
    elapsed_ms: dispatch_elapsed_ms,
});
let _ = metric_tx.send(WaveMetricEvent::SendStarted {
    elapsed_ms: dispatch_elapsed_ms,
});
let _ = metric_tx.send(WaveMetricEvent::DispatchStarted {
    elapsed_ms: dispatch_elapsed_ms,
});
```

- [ ] **Step 4: Emit real `HttpStarted` through the engine hook**

In `observe_ready_request`, add a clone:

```rust
let metrics_for_start = metric_tx.clone();
let metrics_for_send = metric_tx.clone();
let metrics_for_body = metric_tx.clone();
```

Change the call to `send_prepared_http_step_with_hooks` so the first hook after cancellation is:

```rust
move || {
    let metric_tx = metrics_for_start.clone();
    async move {
        let _ = metric_tx.send(WaveMetricEvent::HttpStarted {
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
    }
},
```

Then keep the existing `HttpSendReturned` and `ResponseBodyCompleted` hooks after it.

- [ ] **Step 5: Update sender tests**

Update the test that currently expects expired requests to be dropped. The expected behavior becomes:

```rust
assert_eq!(ready_to_send.load(Ordering::SeqCst), 1);
assert!(matches!(
    metric_rx.recv().await,
    Some(WaveMetricEvent::SenderLaggedStarts { count: 1, .. })
));
```

Add a sender-worker test that enqueues an already-expired request and verifies both:

```rust
assert_eq!(response_in_flight.load(Ordering::SeqCst), 1);
assert_eq!(ready_to_send.load(Ordering::SeqCst), 0);
```

And that the observer channel receives the request instead of it being skipped.

- [ ] **Step 6: Run runner tests**

Run:

```bash
cargo test -p previa-runner
```

Expected: all runner tests pass.

---

### Task 3: Make Curve Adherence Use `sendStarted`

**Files:**
- Modify: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Change the adherence calculation**

Change:

```rust
let absolute_error: usize = lifecycle_by_elapsed
    .values()
    .map(|bucket| bucket.planned.abs_diff(bucket.http_started))
    .sum();
```

To:

```rust
let absolute_error: usize = lifecycle_by_elapsed
    .values()
    .map(|bucket| bucket.planned.abs_diff(bucket.send_started))
    .sum();
```

Reason: `curveAdherence` should answer "did the wave generator start the planned amount?", not "did reqwest reach its send future in the same second?".

- [ ] **Step 2: Update or add the unit test**

Add a test near the existing `load_batch` consolidation tests:

```rust
#[test]
fn curve_adherence_uses_send_started_not_http_started() {
    let mut lifecycle = BTreeMap::new();
    lifecycle.insert(
        1_000,
        ConsolidatedLoadLifecycleBucket {
            elapsed_ms: 1_000,
            planned: 100,
            send_started: 100,
            http_started: 60,
            ..Default::default()
        },
    );

    assert_eq!(lifecycle_curve_adherence(&lifecycle), Some(100.0));
}
```

If `ConsolidatedLoadLifecycleBucket` does not implement `Default`, instantiate all required numeric fields explicitly with zero.

- [ ] **Step 3: Run main tests**

Run:

```bash
cargo test -p previa-main
```

Expected: all main tests pass.

---

### Task 4: Fix Labels So Late Starts Are Not Presented as Drops

**Files:**
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Modify: app translation/source label files found by `rg "senderLaggedStarts|Descartes do sender|loadTestResults.senderLaggedStarts" app/src`
- Test: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Locate the label**

Run:

```bash
rg -n "senderLaggedStarts|Descartes do sender|loadTestResults.senderLaggedStarts" app/src
```

Expected: find the UI label for `loadTestResults.senderLaggedStarts`.

- [ ] **Step 2: Change Portuguese label**

Change the user-facing text from:

```ts
"Descartes do sender"
```

To:

```ts
"Starts atrasados no sender"
```

If English labels exist, change from:

```ts
"Sender drops"
```

To:

```ts
"Late sender starts"
```

- [ ] **Step 3: Keep field mapping unchanged**

Do not rename the API field in this task:

```ts
senderLaggedStarts
```

The field remains compatible with saved history and current SSE payloads.

- [ ] **Step 4: Run frontend tests**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel remote-executor api-client
```

Expected: all targeted frontend tests pass.

---

### Task 5: Verify End-to-End Metrics on the Same Scenario

**Files:**
- No source files changed.

- [ ] **Step 1: Run full targeted verification**

Run:

```bash
cargo test -p previa-engine
cargo test -p previa-runner
cargo test -p previa-main
npm --prefix app test -- LoadTestResultsPanel remote-executor api-client
cd app && ./node_modules/.bin/tsc --noEmit
npm --prefix app run build
```

Expected: all commands pass.

- [ ] **Step 2: Run release build**

Run:

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] **Step 3: Restart local stack**

Stop the current `previa-wave` screen/processes and start:

```bash
screen -S previa-wave -X quit || true
cd /Users/assis/projects/previa
screen -dmS previa-wave zsh -lc '
  cd /Users/assis/projects/previa
  RUST_LOG=info PORT=5611 target/release/previa-runner > /tmp/previa-runner-5611.log 2>&1 &
  RUST_LOG=info PORT=5612 target/release/previa-runner > /tmp/previa-runner-5612.log 2>&1 &
  RUST_LOG=info PORT=5613 target/release/previa-runner > /tmp/previa-runner-5613.log 2>&1 &
  RUST_LOG=info PREVIA_APP_ENABLED=1 ORCHESTRATOR_DATABASE_URL=sqlite:///private/tmp/previa-verify-5610.db PORT=5610 RUNNER_ENDPOINTS=http://127.0.0.1:5611,http://127.0.0.1:5612,http://127.0.0.1:5613 target/release/previa-main > /tmp/previa-main-5610.log 2>&1
'
```

Verify:

```bash
curl -s http://127.0.0.1:5610/info | jq .
```

Expected:

```json
{
  "activeRunners": 3
}
```

Other fields may also be present.

- [ ] **Step 4: Run the same load test manually from the UI**

Open:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Use the same wave shape that previously produced sender drops:

```text
0s -> 10%
59.4s -> 49%
120s -> 80%
interpolation: Step
```

- [ ] **Step 5: Validate the latest history row**

Run:

```bash
sqlite3 /private/tmp/previa-verify-5610.db \
  "select final_consolidated_json from load_history order by finished_at_ms desc limit 1" \
  | jq '{scheduledStarts, sendStarted, httpStarted, httpSendReturned, responseBodyCompleted, senderLaggedStarts, readyRequests, outstandingRequests, curveAdherence}'
```

Expected:

```json
{
  "scheduledStarts": 106902,
  "sendStarted": 106902,
  "senderLaggedStarts": 0
}
```

If the machine is under pressure, `senderLaggedStarts` may be greater than zero, but `sendStarted` must still equal `scheduledStarts`. That is the key behavioral requirement.

- [ ] **Step 6: Inspect per-second lifecycle**

Run:

```bash
sqlite3 /private/tmp/previa-verify-5610.db \
  "select final_consolidated_json from load_history order by finished_at_ms desc limit 1" \
  | jq -r '.lifecycleBuckets[] | [.elapsedMs/1000, .planned, .sendStarted, .httpStarted, .httpSendReturned, .responseBodyCompleted, .senderLagged] | @tsv'
```

Expected:

- `planned` and `sendStarted` stay close per second.
- `sum(planned) == sum(sendStarted)` for the full test.
- `httpStarted` may lag if the HTTP client/runtime is saturated.
- `httpSendReturned` and `responseBodyCompleted` may lag if target/application/network is saturated.
- `senderLagged` reports late starts but no longer implies dropped load.

---

### Task 6: Commit and Push

**Files:**
- All modified files from previous tasks.

- [ ] **Step 1: Review diff**

Run:

```bash
git status --short
git diff --stat
git diff -- runner/src/server/wave_sender.rs engine/src/execution/http_step.rs main/src/server/execution/load_batch.rs
```

Expected: diff only contains the open-loop sender correction, metrics semantics, tests, and label changes.

- [ ] **Step 2: Commit**

Run:

```bash
git add engine/src/execution/http_step.rs runner/src/server/wave_sender.rs runner/src/server/wave_metrics_actor.rs runner/src/server/metrics.rs main/src/server/execution/load_batch.rs app/src docs/superpowers/plans/2026-05-06-wave-open-loop-sender-correction.md
git commit -m "Fix wave open-loop sender late starts"
```

- [ ] **Step 3: Push**

Run:

```bash
git push origin codex/wave-load-test
```

---

## Self-Review

- Spec coverage: The plan preserves planned starts, stops dropping late sender requests, makes `HttpStarted` real, updates adherence semantics, updates UI wording, and defines verification against the same scenario.
- Placeholder scan: No TBD/TODO placeholders remain.
- Type consistency: The existing field `senderLaggedStarts` stays in API payloads; semantics change from "dropped" to "late". `sendStarted`, `httpStarted`, `httpSendReturned`, and `responseBodyCompleted` keep existing names.
