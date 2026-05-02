# HTTP RPS Load Chart Design

## Goal

Make the load-test RPS chart represent real HTTP request throughput instead of
pipeline execution throughput.

The approved chart should show:

- one HTTP RPS line per runner
- one green dashed `RPS total` line that sums all runners at each sample
- one discrete `RPS alvo` reference line from the configured wave

This makes the result chart comparable to the wave configuration, because both
use the same unit: HTTP requests started per second.

## Current Problem

The current RPS chart is built from `rpsHistory`. When available, the UI
calculates interval throughput from `totalStarted`; otherwise it falls back to
`totalSent` or the runner-provided `rps`.

Today those counters represent pipeline executions. A pipeline with five HTTP
steps still counts as one started execution. That makes the chart misleading
when users expect RPS to mean HTTP requests per second.

## Metric Definition

`HTTP RPS` means:

```text
HTTP requests started per second
```

The request is counted immediately before the engine sends it through the HTTP
client. Requests that fail later still count as started load. Invalid methods or
invalid URLs do not count because no HTTP request is emitted.

Completion, success, error, and latency metrics remain separate.

## Engine Hook

Add an async request-start gate in `previa-engine` at the point where a resolved
and validated HTTP request is about to be sent.

The existing step hooks are not enough:

- `on_step_start` fires before URL and method validation
- `on_step_result` fires after the step completes

The new gate should represent the exact moment a real HTTP request starts. It
receives the resolved `StepRequest`, can wait before allowing the request to
continue, and returns without changing the step result contract.

Existing public execution helpers should continue to work by passing an
immediate no-op gate. Load-test execution can use the gated path to throttle and
count HTTP starts.

## Runner Metrics

The runner should track both pipeline-level and HTTP-level counters:

- `totalStarted`: existing pipeline executions started
- `totalSent`: existing pipeline executions completed
- `httpStarted`: HTTP requests started
- `httpCompleted`: HTTP requests that reached a response or transport error
- `rps`: HTTP requests started per second for new load-test metrics

`rps` should move to HTTP throughput for wave load-test runs. Existing
pipeline-level counters can remain for compatibility and summary cards.

## Wave Flow Control

The wave limiter should spend capacity per HTTP request, not per pipeline
execution.

At runtime:

1. A pipeline execution can be scheduled while respecting `maxInFlight`.
2. Before each HTTP step sends its request, the engine calls the request-start
   gate.
3. The runner waits in that gate until the local bucket allows another request.
4. The runner increments `httpStarted` at the same point.

This keeps the control loop and graph aligned. If the wave target is `100 HTTP
RPS`, the limiter controls HTTP request emission rather than pipeline launches.

## Main Aggregation

`previa-main` should preserve runner identity in RPS history samples.

Each sample should include enough data to build:

- individual runner HTTP RPS series
- total aggregated HTTP RPS
- target RPS limit from the wave

Recommended sample shape:

```json
{
  "timestamp": 1777720000000,
  "targetRpsLimit": 3000,
  "runners": [
    {
      "runnerId": "runner-5611",
      "httpStarted": 120,
      "httpCompleted": 118,
      "rps": 40.0
    }
  ]
}
```

The UI can calculate interval RPS from `httpStarted` deltas. If the server also
provides per-runner `rps`, the UI may use it as a fallback.

## UI

The load-test result chart title and legend should make the unit clear:

- chart title: `HTTP RPS ao longo do tempo`
- runner lines: one color per runner
- total line: green dashed `RPS total`
- target line: discrete dashed or dotted `RPS alvo`

The chart should prefer the new per-runner HTTP history when available. Older
history entries should keep the current fallback behavior so previous runs
remain viewable.

## Compatibility

Older runs may not have `httpStarted` or per-runner RPS history. For those runs,
the UI should keep the current behavior and label should avoid implying exact
HTTP RPS if the new data is missing.

API consumers that read `totalStarted` and `totalSent` should not break. New
HTTP counters are additive fields.

## Testing

Tests should cover:

- engine hook fires only for valid HTTP requests that are about to be sent
- runner increments `httpStarted` when the gate allows the request to continue
- wave limiter throttles HTTP requests rather than pipeline executions
- main aggregation preserves per-runner HTTP counters
- UI builds runner series and total series from `httpStarted` deltas
- UI falls back for older history without HTTP counters
