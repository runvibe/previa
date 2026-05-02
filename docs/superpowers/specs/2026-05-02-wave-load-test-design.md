# Wave Load Test Design

## Goal

Make Wave Load Test the default load-test model in Previa. Instead of asking
users to tune exact request counts and fixed concurrency, Previa should let them
draw a timeline of load intensity and execute it as a controlled flow of
requests.

The core model is:

```text
time -> intensity %
```

Example:

```text
0ms -> 10%
60000ms -> 80%
80000ms -> 30%
```

This model covers baseline, ramp-up, ramp-down, spike, soak, stress, and custom
irregular load profiles through the same wave primitive.

## Product Model

Wave is the canonical load-test model. The runner does not need a separate
`mode: "wave"` flag because a load test is represented as a wave.

The current load-test shape using `totalRequests`, `concurrency`, and
`rampUpSeconds` may remain accepted by `previa-main` as a compatibility layer,
but new UI flows and new runner execution should use the wave contract.

## Runner Contract

The runner receives the full load profile when execution starts:

```json
{
  "pipeline": {},
  "load": {
    "points": [
      { "atMs": 0, "intensity": 10 },
      { "atMs": 60000, "intensity": 80 },
      { "atMs": 80000, "intensity": 30 }
    ],
    "interpolation": "smooth",
    "runnerMaxRps": 1000,
    "maxInFlight": 200,
    "gracePeriodMs": 30000
  }
}
```

`points` define the wave. `intensity` is a percentage from `0` to `100`.

`runnerMaxRps` is the safe flow ceiling for that runner. It is derived from the
capacity currently represented by `RUNNER_RPS_PER_NODE`. In the wave model,
`100%` means `runnerMaxRps`.

`maxInFlight` prevents request buildup when the target system becomes slow.

`gracePeriodMs` controls how long the runner waits for in-flight work after the
timeline ends.

## Interpolation

The runner converts sparse points into intermediate values through
interpolation.

Supported values:

- `smooth`: default. Uses smoothstep for organic wave motion.
- `linear`: straight-line interpolation between points.
- `step`: holds the previous value until the next point.

For a segment from point `a` to point `b`:

```text
t = (nowMs - a.atMs) / (b.atMs - a.atMs)
```

Linear:

```text
value = a.intensity + (b.intensity - a.intensity) * t
```

Smooth:

```text
s = t * t * (3 - 2 * t)
value = a.intensity + (b.intensity - a.intensity) * s
```

Step:

```text
value = a.intensity
```

## Tick Selection

The runner samples the wave on an automatic tick. The tick is dynamic, with a
maximum of `1000ms` and a minimum of `100ms`.

```text
minPointIntervalMs = smallest positive interval between consecutive points
tickMs = min(1000, max(100, minPointIntervalMs / 10))
```

This gives short wave segments enough samples while avoiding overly chatty
control loops for long tests.

## Runner Execution

The runner owns the fine-grained flow control.

For each tick:

1. Read elapsed time since execution start.
2. Interpolate the current wave intensity.
3. Convert intensity to a local RPS ceiling:

   ```text
   localRpsLimit = runnerMaxRps * intensity / 100
   ```

4. Apply a leaky-bucket or token-bucket limiter before starting new pipeline
   executions.
5. Respect `maxInFlight` before starting additional work.
6. Emit metrics with target intensity, computed RPS limit, actual RPS, and
   in-flight count.

When the final point is reached, the runner stops starting new executions and
waits up to `gracePeriodMs` for in-flight executions to finish.

## Main Orchestration

`previa-main` remains the orchestrator and aggregator, but it does not need to
control the wave in real time.

Responsibilities:

- accept and validate the wave payload
- select healthy runners
- compute each runner's planned capacity from `RUNNER_RPS_PER_NODE`
- send each runner the full `load` config at execution start
- aggregate SSE output
- save the original load config and final metrics in load history
- preserve compatibility with older load-test requests where practical

The main process may later add live rebalance or runtime updates, but the first
wave implementation should not depend on a live control channel.

## UI

The UI should present load testing as a wave editor:

- x-axis: elapsed timeline
- y-axis: intensity percentage
- points: `{ atMs, intensity }`
- default interpolation: `smooth`
- presets: baseline, ramp-up, spike, soak, stress

Presets should generate editable points. They should not create separate load
models.

The UI may display estimated RPS using the selected runner capacity, but the
primary editing surface should remain percentage-based.

## Metrics

Wave runs should expose enough data to compare intent and reality:

- `targetIntensity`
- `targetRpsLimit`
- `actualRps`
- `inFlight`
- `runnerMaxRps`
- `tickMs`
- existing success/error/latency/runtime metrics

History should save the wave config exactly as requested, plus final
consolidated metrics.

## Validation

Validation rules:

- at least two points
- first point must be at `0ms`
- `atMs` values must be strictly increasing
- `intensity` must be between `0` and `100`
- `interpolation` must be one of `smooth`, `linear`, or `step`
- `runnerMaxRps` must be positive
- `maxInFlight` must be positive
- `gracePeriodMs` must be non-negative

## Compatibility

The current classic config can be handled as legacy input:

```json
{
  "config": {
    "totalRequests": 1000,
    "concurrency": 20,
    "rampUpSeconds": 10
  }
}
```

For the first migration phase, `previa-main` can continue forwarding the
classic shape to runners if needed. The target architecture is for runner load
execution to use the `load` wave shape only.

## Testing

Tests should cover:

- interpolation math for `smooth`, `linear`, and `step`
- automatic tick calculation with `100ms` minimum and `1000ms` maximum
- load payload validation
- token/leaky bucket behavior under low and high intensity
- `maxInFlight` limiting when pipeline latency rises
- runner completion at final timeline point
- grace-period behavior
- main-to-runner payload distribution
- load-history persistence of wave config and final metrics

## Non-Goals

This design does not require live control updates from `previa-main` to runners.
It also does not require editable Bezier handles or spline curves in the first
release. The initial smooth interpolation is mathematical smoothstep between
points.
