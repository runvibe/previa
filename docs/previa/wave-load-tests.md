# Wave Load Tests

Wave is Previa's standard load-test model. Instead of configuring a fixed
number of requests or a fixed concurrency value, you draw a load curve over
time:

```text
elapsed time -> intensity %
```

Example:

```text
0ms     -> 10%
60s     -> 80%
80s     -> 30%
500s    -> 100%
```

This single model covers baseline tests, ramp-ups, ramp-downs, spikes, soak
tests, stress tests, and irregular production-like traffic.

## Core Idea

A wave point has two values:

- `atMs`: elapsed time since the beginning of the test, in milliseconds
- `intensity`: percentage of the per-runner RPS limit, from `0` to `100`

The runner converts the current intensity into a target request flow:

```text
target RPS per runner = runnerMaxRps * intensity / 100
target total RPS      = active runner count * target RPS per runner
```

For example, with 3 active runners and `runnerMaxRps = 600`:

```text
50% intensity = 3 * 600 * 0.50 = 900 total RPS
```

## Configuration

The wave load payload is sent as `load`:

```json
{
  "load": {
    "points": [
      { "atMs": 0, "intensity": 10 },
      { "atMs": 60000, "intensity": 80 },
      { "atMs": 80000, "intensity": 30 }
    ],
    "interpolation": "smooth",
    "runnerMaxRps": 600,
    "gracePeriodMs": 30000
  }
}
```

### `points`

`points` define the timeline. The first point must start at `0ms`, and times
must be ordered from low to high.

### `interpolation`

Interpolation defines how the runner fills the space between points:

- `smooth`: default for organic ramps; uses smoothstep easing
- `linear`: straight line between points
- `step`: holds the previous value until the next point

The UI preview uses the same interpolation shape so the drawn wave matches the
execution intent.

### `runnerMaxRps`

`runnerMaxRps` is the maximum target request rate for each runner in this test.
In the UI, the default is `600` and the current UI range is `1` to `1000`.

Prefer setting `runnerMaxRps` explicitly per load test. If an API caller omits
it, `previa-main` falls back to the capacity hint configured by
`RUNNER_RPS_PER_NODE` for runner planning.

`runnerMaxRps` is not a guarantee that the infrastructure can sustain that
rate. It is the target ceiling used to calculate the wave. CPU, scheduler
capacity, OS limits, network bandwidth, DNS, TLS, or the target service can
still become the bottleneck.

### `gracePeriodMs`

`gracePeriodMs` is extra observation time after the wave timeline stops
scheduling new requests. During this period, the runner waits for already
started work to complete or fail so final latency, success, and error metrics
can be recorded.

It does not extend the wave itself. The wave stops at the last point.

## Open-Loop Semantics

Wave scheduling is open-loop. The runner schedules request starts from the
clock and configured wave; it does not wait for HTTP responses before deciding
whether the next request should be started.

This distinction is important:

- slow responses should affect latency, errors, and response observation
  metrics
- slow responses should not directly throttle the wave scheduler
- when RPS cannot follow the wave, the cause should be runner or host
  infrastructure pressure, request preparation limits, network limits, or the
  target path itself

If a pipeline step depends on data from a previous response, that later step can
still be dependency-limited. The open-loop guarantee applies to dispatching
ready request starts according to the wave clock.

## UI Workflow

1. Open a project pipeline.
2. Go to `Load Test`.
3. Set the total duration.
4. Choose interpolation.
5. Set `Limite RPS por runner`.
6. Draw or drag points on the wave graph.
7. Start the test.

The editor shows intensity as a percentage and also estimates the planned
request rate at each point from:

```text
active runners * runnerMaxRps * intensity / 100
```

## API Example

```bash
curl -N "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/load" \
  -H "content-type: application/json" \
  -d '{
    "pipelineId": "users-crud",
    "selectedBaseUrlKey": "hml",
    "load": {
      "points": [
        { "atMs": 0, "intensity": 10 },
        { "atMs": 60000, "intensity": 80 },
        { "atMs": 120000, "intensity": 25 }
      ],
      "interpolation": "smooth",
      "runnerMaxRps": 600,
      "gracePeriodMs": 30000
    },
    "specs": []
  }'
```

## Reading Results

The load results prioritize charts that compare the requested wave with what
actually happened.

### HTTP RPS Over Time

The RPS chart shows:

- one line per runner
- a total RPS line across runners
- the target wave/RPS reference

Use this chart to answer: "Did the runners start requests close to the wave?"

### Configured Wave

This chart shows the wave that was requested. It is the intent baseline for the
test and should match the curve created in the editor.

### Wave Lifecycle

The lifecycle chart shows where work moved through the runner:

- planned slots
- request prepared
- request enqueued
- send task spawned
- send started
- HTTP started
- HTTP send returned
- response body completed

It also exposes lag signals such as scheduler, runtime, dispatcher, and sender
lag. Use these when the RPS chart diverges from the target wave.

## Diagnosing Divergence

When observed RPS does not follow the configured wave:

- If `planned` follows the wave but `sendStarted` or `httpStarted` falls
  behind, the runner host or send path is saturated.
- If `httpStarted` follows the wave but `httpSendReturned` or
  `responseBodyCompleted` falls behind, the target/network/response path is the
  bottleneck.
- If scheduler or sender lag grows, the local runtime may be CPU constrained or
  unable to schedule work fast enough.
- If dependency-limited starts grow, the pipeline needs prior response data
  before it can prepare later requests.

For deterministic local validation, use the
[local wave load target](../../README.md#local-wave-load-target). It lets you
compare configured wave, runner-side RPS, and target-side received RPS without
depending on an external service.

## Compatibility

Previa may still accept older load-test payloads with `config.totalRequests`,
`config.concurrency`, and `config.rampUpSeconds` for compatibility. New UI
flows and new automation should use the `load.points` wave model.

## See Also

- [Examples cookbook](./examples-cookbook.md)
- [Remote runners](./remote-runners.md)
- [Operations cheatsheet](./operations-cheatsheet.md)
- [Architecture at a glance](./architecture.md)
