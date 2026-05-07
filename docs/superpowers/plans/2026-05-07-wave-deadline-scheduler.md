# Wave Deadline Scheduler Implementation Plan

## Goal

Make the wave runner preserve the configured request start curve with finer temporal precision, without letting response time, response errors, or body observation feed back into request dispatch.

The current implementation is already open-loop in total volume: `scheduledStarts`, `sendStarted`, and `httpStarted` stay very close. The remaining problem is temporal: under aggressive waves, many starts are emitted in bursts inside each scheduling window, so per-second RPS can oscillate even when the total count is correct.

The fix is to make each request carry an explicit start deadline and make the sender start requests by deadline, not by batch arrival order.

## Architecture

Keep the current three-stage runner split:

1. Scheduler calculates the wave demand.
2. Dispatcher prepares pipeline/request work.
3. Sender starts HTTP sends independently from response observation.

Add one refinement:

1. Dispatcher distributes the starts inside each scheduler tick and assigns a `target_start_elapsed_ms` to every ready request.
2. Sender owns a deadline queue and starts each request as close as possible to that target.
3. Response observation remains detached. It can report success/error/latency later, but it never blocks or slows future starts.

## Data Model

Extend `ReadyWaveRequest` in `runner/src/server/wave_sender.rs`:

```rust
struct ReadyWaveRequest {
    pipeline: PreparedPipeline,
    scheduled_elapsed_ms: u64,
    target_start_elapsed_ms: u64,
    expires_at_elapsed_ms: u64,
    sender_enqueued_elapsed_ms: u64,
}
```

`scheduled_elapsed_ms` keeps the original wave slot identity.

`target_start_elapsed_ms` is the exact desired start time for this request inside that slot.

`expires_at_elapsed_ms` remains useful only for stale-request accounting. It must not be used as the main start clock.

## Implementation Tasks

### 1. Add deadline spreading helper

Add a pure helper near the dispatcher/scheduler code that converts a slot into evenly spaced target deadlines.

Example behavior:

```rust
fn spread_deadlines(slot_elapsed_ms: u64, tick_ms: u64, count: usize) -> Vec<u64> {
    if count == 0 {
        return Vec::new();
    }

    (0..count)
        .map(|index| slot_elapsed_ms + ((index as u64 * tick_ms) / count as u64))
        .collect()
}
```

Expected examples:

```rust
assert_eq!(spread_deadlines(10_000, 1_000, 1), vec![10_000]);
assert_eq!(spread_deadlines(10_000, 1_000, 5), vec![10_000, 10_200, 10_400, 10_600, 10_800]);
assert_eq!(spread_deadlines(10_000, 100, 4), vec![10_000, 10_025, 10_050, 10_075]);
```

This preserves the wave count while reducing burstiness inside each tick.

### 2. Carry target deadlines into ready requests

Update the place that creates `ReadyWaveRequest` so each scheduled start receives one deadline from `spread_deadlines`.

The dispatcher should know:

- the slot elapsed time,
- the tick duration,
- the number of starts planned for that slot,
- the index of the current start inside that slot.

The request should be built like:

```rust
ReadyWaveRequest {
    pipeline,
    scheduled_elapsed_ms: slot_elapsed_ms,
    target_start_elapsed_ms,
    expires_at_elapsed_ms: slot_elapsed_ms + tick_ms.saturating_mul(2),
    sender_enqueued_elapsed_ms,
}
```

The target deadline should be clamped to the test duration when needed so no request is scheduled after the active wave window.

### 3. Replace sender FIFO start with deadline queue

Update `run_sender_worker` in `runner/src/server/wave_sender.rs`.

Today the sender starts requests mostly in receive order. Replace that with a local deadline queue:

```rust
#[derive(Eq, PartialEq)]
struct DeadlineReadyRequest {
    target_start_elapsed_ms: u64,
    sequence: u64,
    request: ReadyWaveRequest,
}
```

Use a min-heap via `BinaryHeap<Reverse<DeadlineReadyRequest>>`, ordered by:

1. `target_start_elapsed_ms`
2. `sequence`

The sender loop should:

1. receive ready requests and push them into the heap,
2. sleep until the next deadline when the heap has future work,
3. pop all due requests,
4. emit `SendStarted`,
5. call `start_ready_request`,
6. continue polling `FuturesUnordered` for completed send futures.

Important: sleeping for a future deadline must not prevent already due requests from being started, and response futures must continue being polled while the sender is waiting.

### 4. Measure start lag precisely

Add a sender start lag metric:

```rust
let actual_start_elapsed_ms = started.elapsed().as_millis() as u64;
let sender_start_lag_ms =
    actual_start_elapsed_ms.saturating_sub(request.target_start_elapsed_ms);
```

Record this in the metrics actor before HTTP starts.

New metric fields:

- `senderStartLagAvgMs`
- `senderStartLagP95Ms`
- `senderStartLagP99Ms`
- `senderStartLagMaxMs`

This answers the key question: "o runner começou o envio no horário que a onda mandou?"

### 5. Measure HTTP send lag separately

Keep distinguishing request start from request completion.

Inside `start_ready_request`, measure:

- `httpStarted`: when the HTTP future starts,
- `httpSendReturned`: when `reqwest.send().await` returns headers or error,
- `responseBodyCompleted`: when body observation ends.

Add:

- `httpSendDurationAvgMs`
- `httpSendDurationP95Ms`
- `httpSendDurationP99Ms`
- `responseObservationDurationAvgMs`
- `responseObservationDurationP95Ms`
- `responseObservationDurationP99Ms`

This separates runner scheduling health from target/network behavior.

### 6. Update lifecycle buckets

Extend `LoadLifecycleBucket` in `runner/src/server/models.rs` with per-second lag summaries:

```rust
pub sender_start_lag_ms_max: u64,
pub http_send_duration_ms_max: u64,
pub response_observation_duration_ms_max: u64,
```

For UI clarity, use max per bucket first. It is easier to inspect and enough to find the second where the runner stops keeping up.

### 7. Update frontend types and charts

Update:

- `app/src/types/load-test.ts`
- `app/src/components/LoadTestResultsPanel.tsx`
- `app/src/components/LoadTestResultsPanel.test.tsx`
- locale files in `app/src/i18n/locales/`

Add metric cards:

- `Start lag p95`
- `HTTP send p95`
- `Observation p95`

Add the new lag lines to the existing "ciclo de vida do wave" chart:

- `senderStartLagMsMax`
- `httpSendDurationMsMax`
- `responseObservationDurationMsMax`

Use a secondary axis if the chart already mixes counts and milliseconds.

### 8. Tests

Add Rust tests:

1. `spread_deadlines_evenly_inside_tick`
2. `spread_deadlines_handles_single_start`
3. `deadline_queue_orders_by_target_then_sequence`
4. `sender_records_start_lag_against_target_deadline`
5. Existing open-loop tests must continue to pass.

Add frontend tests:

1. lifecycle chart renders lag series labels,
2. cards render p95 lag metrics when present,
3. old metrics without lag fields still render safely.

### 9. Verification

Run:

```bash
cargo test
npm --prefix app test -- LoadTestResultsPanel
npm --prefix app run tsc -- --noEmit
npm --prefix app run build
cargo build --release
```

Then restart the full local stack:

```bash
./scripts/start-local-stack.sh
```

Run the same load-test scenario with 3 runners and compare:

- `scheduledStarts` vs `sendStarted` vs `httpStarted`
- `curveAdherence`
- `senderLaggedStarts`
- `senderStartLagP95Ms`
- `senderStartLagP99Ms`
- lifecycle bucket lag around the seconds where the graph oscillates

Expected outcome:

- total volume remains close to the wave,
- per-second RPS becomes smoother,
- if the target/gateway fails, failures appear in HTTP/response metrics, not as missing starts,
- if the runner host is the bottleneck, `senderStartLagP95Ms` and `senderStartLagP99Ms` expose it directly.

