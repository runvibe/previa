# Wave RPS Jitter Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make wave load-test RPS analysis precise enough to separate algorithm correctness from scheduler, dispatcher, sender, HTTP-client, and infrastructure limits.

**Architecture:** Keep the runner open-loop: response completion must never gate the creation of new request starts. Add explicit lifecycle counters around each runner boundary, fix final scheduled-start aggregation, and make chart target comparison respect the dynamic wave duration instead of treating the exact end timestamp as a full active bucket.

**Tech Stack:** Rust, Tokio, Axum SSE, Reqwest, SQLx persisted history, React/TypeScript, Recharts/Vitest.

---

## File Structure

- Modify `runner/src/server/models.rs`: add optional lifecycle counters to `LoadTestMetrics`.
- Modify `runner/src/server/metrics.rs`: store and serialize lifecycle counters from the metrics accumulator.
- Modify `runner/src/server/wave_metrics_actor.rs`: add metric events for scheduler slot acceptance, request preparation, request enqueue, sender spawn, and send start.
- Modify `runner/src/server/wave_scheduler.rs`: emit accepted-slot metrics when a slot enters the scheduler channel.
- Modify `runner/src/server/wave_dispatcher.rs`: emit preparation/enqueue metrics around dispatcher boundaries.
- Modify `runner/src/server/wave_sender.rs`: emit sender task and send-start metrics at the exact boundary where request sending is attempted.
- Modify `main/src/server/models.rs` and `main/src/server/utils.rs`: parse the new runner metrics.
- Modify `main/src/server/execution/load_batch.rs`: consolidate and expose the new fields in live/final metrics and runner samples.
- Modify `app/src/types/load-test.ts`: add optional lifecycle fields to the frontend metric type.
- Modify `app/src/lib/api-client.ts`: map the new backend fields into `LoadTestMetrics`.
- Modify `app/src/lib/load-rps-chart.ts`: exclude `elapsedMs >= durationMs` from active target comparison and handle partial final buckets.
- Modify `app/src/components/LoadTestResultsPanel.tsx`: expose diagnostic counters in the results panel.
- Test in `runner/src/server/*` unit tests, `main/src/server/execution/load_batch.rs` tests, `app/src/lib/load-rps-chart.test.ts` or existing panel tests.

---

### Task 1: Fix Final `scheduledStarts` Aggregation

**Files:**
- Modify: `main/src/server/execution/load_batch.rs`
- Test: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Write a failing main test**

Add a test proving final consolidation preserves the latest runner `scheduledStarts` total:

```rust
#[test]
fn rebuild_final_rps_history_preserves_scheduled_starts_from_final_runner_buckets() {
    let metrics = consolidated_metrics_from_runner_lines(vec![
        runner_line("http://127.0.0.1:5611", json!({
            "startTime": 1_000,
            "elapsedMs": 120_000,
            "totalSent": 10,
            "totalSuccess": 10,
            "totalError": 0,
            "rps": 100.0,
            "scheduledStarts": 1_000,
            "dispatchStarted": 1_000,
            "dispatchBuckets": [{ "elapsedMs": 0, "count": 100 }]
        })),
        runner_line("http://127.0.0.1:5612", json!({
            "startTime": 1_000,
            "elapsedMs": 120_000,
            "totalSent": 10,
            "totalSuccess": 10,
            "totalError": 0,
            "rps": 100.0,
            "scheduledStarts": 1_000,
            "dispatchStarted": 1_000,
            "dispatchBuckets": [{ "elapsedMs": 0, "count": 100 }]
        })),
    ]);

    assert_eq!(metrics.scheduled_starts, Some(2_000));
}
```

- [ ] **Step 2: Run the failing test**

Run: `cargo test -p previa-main rebuild_final_rps_history_preserves_scheduled_starts_from_final_runner_buckets`

Expected: fail today because the final consolidated value can fall back to `0` or be absent when the last snapshot wave metadata is not preserved.

- [ ] **Step 3: Implement the fix**

In consolidation, sum `scheduledStarts` from final runner payloads the same way `dispatchStarted`, `httpStarted`, and `missedStarts` are summed. The final value must be the latest cumulative scheduled total per runner, not the last live wave snapshot value.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p previa-main rebuild_final_rps_history_preserves_scheduled_starts_from_final_runner_buckets
cargo test -p previa-main
```

Expected: both pass.

---

### Task 2: Add Runner Lifecycle Counters

**Files:**
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/wave_metrics_actor.rs`
- Modify: `runner/src/server/wave_scheduler.rs`
- Modify: `runner/src/server/wave_dispatcher.rs`
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Write failing runner tests**

Add tests that assert a wave run can report these counters:

```rust
assert_eq!(snapshot.slot_enqueued, Some(3));
assert_eq!(snapshot.request_prepared, Some(3));
assert_eq!(snapshot.request_enqueued, Some(3));
assert_eq!(snapshot.send_task_spawned, Some(3));
assert_eq!(snapshot.send_started, Some(3));
```

Use the existing `wave_metrics_actor_applies_dispatch_and_scheduler_events` style test as the smallest place to validate accumulator behavior.

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p previa-runner wave_metrics_actor`

Expected: fail because the fields and events do not exist.

- [ ] **Step 3: Add metric fields**

Add optional camelCase fields to `LoadTestMetrics`:

```rust
pub slot_enqueued: Option<usize>,
pub request_prepared: Option<usize>,
pub request_enqueued: Option<usize>,
pub send_task_spawned: Option<usize>,
pub send_started: Option<usize>,
```

Initialize them as `None` in `Default`.

- [ ] **Step 4: Add accumulator methods**

Add counters and methods in `MetricsAccumulator`:

```rust
pub fn record_slot_enqueued_count(&mut self, count: usize) { ... }
pub fn record_request_prepared(&mut self) { ... }
pub fn record_request_enqueued(&mut self) { ... }
pub fn record_send_task_spawned(&mut self) { ... }
pub fn record_send_started(&mut self) { ... }
```

Serialize each as `Some(count)` only when `count > 0`.

- [ ] **Step 5: Add metric events**

Add events to `WaveMetricEvent`:

```rust
SlotEnqueued(usize),
RequestPrepared,
RequestEnqueued,
SendTaskSpawned,
SendStarted,
```

Handle them in `run_wave_metrics_actor`.

- [ ] **Step 6: Emit events at boundaries**

Emit:
- `SlotEnqueued(slot.planned_starts)` only after `slot_tx.try_send(slot)` succeeds.
- `RequestPrepared` after `prepare_http_step` succeeds.
- `RequestEnqueued` after `request_tx.send(...)` succeeds.
- `SendTaskSpawned` immediately before/after `tokio::spawn` in `WaveSender::spawn_observer`.
- `SendStarted` immediately inside the spawned task before `send_prepared_http_step_with_hooks`.

- [ ] **Step 7: Verify**

Run:

```bash
cargo test -p previa-runner wave_metrics_actor
cargo test -p previa-runner wave_dispatcher
cargo test -p previa-runner wave_sender
cargo test -p previa-runner
```

Expected: all pass.

---

### Task 3: Propagate Diagnostics Through Main

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Write failing main parsing/consolidation tests**

Add a test payload containing:

```json
{
  "slotEnqueued": 10,
  "requestPrepared": 10,
  "requestEnqueued": 10,
  "sendTaskSpawned": 10,
  "sendStarted": 10
}
```

Assert that parsing and consolidation preserve/sum the values.

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p previa-main load_batch`

Expected: fail because the fields are not parsed or inserted.

- [ ] **Step 3: Extend `RunnerLoadMetricsPoint` and parser**

Add optional fields:

```rust
pub slot_enqueued: Option<usize>,
pub request_prepared: Option<usize>,
pub request_enqueued: Option<usize>,
pub send_task_spawned: Option<usize>,
pub send_started: Option<usize>,
```

Parse with `get_usize_field(payload, "...")`.

- [ ] **Step 4: Extend consolidation and samples**

Add the fields to `ConsolidatedLoadMetrics`, sum them across nodes, and insert them into:
- final consolidated metrics
- `rpsHistory` samples
- per-runner sample objects

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-main load_batch
cargo test -p previa-main
```

Expected: all pass.

---

### Task 4: Fix Dynamic Wave-End Target Handling

**Files:**
- Modify: `app/src/lib/load-rps-chart.ts`
- Test: `app/src/components/LoadTestResultsPanel.test.tsx` or create `app/src/lib/load-rps-chart.test.ts`

- [ ] **Step 1: Write failing frontend tests**

Add tests for dynamic durations:

```ts
it("does not draw active target for buckets at or after the wave duration", () => {
  const data = buildRpsChartData(metricsWithDuration(120000), waveConfigWithEnd(120000)).data;
  expect(data.find((row) => row.time === 120)?.targetRpsLimit).toBeUndefined();
});

it("scales target for a partial final bucket", () => {
  const data = buildRpsChartData(metricsWithDuration(30500), waveConfigWithEnd(30500)).data;
  const final = data.find((row) => row.time === 30);
  expect(final?.targetRpsLimit).toBeLessThan(fullSecondTarget);
  expect(final?.targetRpsLimit).toBeGreaterThan(0);
});
```

- [ ] **Step 2: Run failing test**

Run: `npm --prefix app test -- load-rps-chart`

Expected: fail because current target estimation treats full-second buckets too broadly.

- [ ] **Step 3: Implement dynamic end handling**

In `estimateTargetRpsLimit`, use the wave end from the last point:

```ts
const waveEndMs = waveConfig.points.at(-1)?.atMs;
const bucketStartMs = elapsedMs;
const bucketEndMs = elapsedMs + 1000;
```

Rules:
- if `bucketStartMs >= waveEndMs`, return `undefined`
- if `bucketEndMs <= waveEndMs`, return normal target
- if `bucketStartMs < waveEndMs && bucketEndMs > waveEndMs`, multiply target by `(waveEndMs - bucketStartMs) / 1000`

- [ ] **Step 4: Verify**

Run:

```bash
npm --prefix app test -- load-rps-chart
npm --prefix app test -- LoadTestResultsPanel
./node_modules/.bin/tsc --noEmit
npm --prefix app run build
```

Expected: all pass.

---

### Task 5: Surface Diagnostics In The Results UI

**Files:**
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/api-client.ts`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Test: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Write failing UI test**

Add a test that renders metrics with:

```ts
slotEnqueued: 100,
requestPrepared: 99,
requestEnqueued: 98,
sendTaskSpawned: 98,
sendStarted: 97,
```

Assert that the diagnostics section shows those labels and values.

- [ ] **Step 2: Run failing test**

Run: `npm --prefix app test -- LoadTestResultsPanel`

Expected: fail because fields are not mapped or displayed.

- [ ] **Step 3: Add frontend types and mapping**

Add optional fields to `LoadTestMetrics` and map from `finalConsolidated` in `loadRecordToRun`.

- [ ] **Step 4: Add concise diagnostics display**

Add a compact diagnostics row/card near existing load metrics:

```text
Scheduled -> Slot -> Prepared -> Enqueued -> Spawned -> Send start
```

Use it only when at least one diagnostic field exists, so older histories remain clean.

- [ ] **Step 5: Verify**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel
./node_modules/.bin/tsc --noEmit
```

Expected: pass.

---

### Task 6: Re-run The Real Scenario And Decide If Sender Architecture Needs Work

**Files:**
- No code files unless diagnostics prove a specific bottleneck.

- [ ] **Step 1: Run a fresh 3-runner load test**

Use the current app at:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

- [ ] **Step 2: Extract the latest history record**

Run:

```bash
curl -s 'http://127.0.0.1:5610/api/v1/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/tests/load?pipelineIndex=0&limit=1' | jq '.[0]'
```

- [ ] **Step 3: Compare lifecycle counters**

Interpret:
- `scheduledStarts > slotEnqueued`: scheduler channel/backpressure.
- `slotEnqueued > requestPrepared`: dispatcher/prepare bottleneck.
- `requestPrepared > requestEnqueued`: dispatcher-to-sender channel issue.
- `requestEnqueued > sendTaskSpawned`: sender loop not draining fast enough.
- `sendTaskSpawned > sendStarted`: Tokio spawn/runtime delay.
- `sendStarted == dispatchStarted` but buckets jitter: request starts are happening, but bucket timing needs planned-slot attribution or worker/time-wheel isolation.

- [ ] **Step 4: Decide next architecture step**

Only if diagnostics show sender/runtime delay, create a follow-up plan for dedicated sender worker threads or planned-slot bucket attribution. Do not implement it in this plan.

---

## Final Verification

Run:

```bash
cargo test -p previa-main
cargo test -p previa-runner
./node_modules/.bin/tsc --noEmit
npm --prefix app run build
npm --prefix app test -- LoadTestResultsPanel
cargo build --release
```

Expected:
- Rust tests pass.
- TypeScript compiles.
- App build succeeds.
- Release build succeeds.
- Latest test history can explain where any RPS jitter is introduced.

---

## Self-Review

- Spec coverage: covers `scheduledStarts`, lifecycle diagnostics, dynamic wave-end target handling, UI visibility, and a final real-test decision point.
- Placeholder scan: no `TBD`, no vague “add tests”; every task has concrete files, commands, and expected behavior.
- Type consistency: lifecycle fields use snake_case in Rust internals and camelCase JSON/TypeScript names.
