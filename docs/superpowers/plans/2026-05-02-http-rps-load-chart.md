# HTTP RPS Load Chart Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make load-test RPS represent HTTP requests started per second and show per-runner lines plus an aggregated total line.

**Architecture:** `previa-engine` exposes an async request-start gate immediately before HTTP send. `previa-runner` uses that gate to throttle and count HTTP starts. `previa-main` preserves per-runner HTTP samples, and the React UI renders runner series, total RPS, and target RPS from those samples with legacy fallback.

**Tech Stack:** Rust, Tokio, Axum/SSE, serde JSON, React/TypeScript, Recharts, Vitest.

---

### Task 1: Engine Request-Start Gate

**Files:**
- Modify: `engine/src/execution/engine.rs`
- Modify: `engine/src/execution/mod.rs`
- Modify: `engine/src/lib.rs`
- Modify: `runner/src/lib.rs`

- [ ] **Step 1: Add an async gate type**

Add to `engine/src/execution/engine.rs`:

```rust
use std::future::Future;
use std::pin::Pin;

pub type RequestStartGate<'a> =
    dyn FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'a;
```

- [ ] **Step 2: Add a runtime helper that accepts the gate**

Add a public helper beside `execute_pipeline_with_runtime_hooks`:

```rust
pub async fn execute_pipeline_with_runtime_request_gate<FStart, FResult, FCancel, FGate>(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
    on_request_start: FGate,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
    FGate: for<'a> FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send,
{
    let client = Client::new();
    execute_pipeline_with_client_runtime_hooks(
        &client,
        pipeline,
        selected_base_url_key,
        specs,
        env_groups,
        selected_env_group_slug,
        on_step_start,
        on_step_result,
        should_cancel,
        on_request_start,
    )
    .await
}
```

- [ ] **Step 3: Thread no-op gates through existing helpers**

Update all existing calls to `execute_pipeline_with_client_runtime_hooks` to pass:

```rust
|_| Box::pin(async {})
```

- [ ] **Step 4: Await the gate immediately before HTTP send**

After `log_step_request(&step.id, &request);` and before `request_builder.send()`:

```rust
on_request_start(&request).await;
```

Invalid method and URL paths must not call the gate.

- [ ] **Step 5: Export the new helper**

Add `execute_pipeline_with_runtime_request_gate` to `engine/src/execution/mod.rs`, `engine/src/lib.rs`, and `runner/src/lib.rs`.

- [ ] **Step 6: Run engine tests**

Run: `cargo test -p previa-engine execution`

Expected: tests compile and pass.

### Task 2: Runner HTTP Metrics And Throttling

**Files:**
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/handlers/load.rs`

- [ ] **Step 1: Extend runner metrics**

Add `http_started` and `http_completed` counters to `MetricsAccumulator`, increment helpers, and serialized fields in `LoadTestMetrics`.

```rust
pub fn record_http_start(&mut self) {
    self.http_started += 1;
}

pub fn record_http_completed(&mut self) {
    self.http_completed += 1;
}
```

Change load-test `rps` calculation to:

```rust
round2((self.http_started as f64) / elapsed)
```

falling back to `total_sent / elapsed` only when no HTTP requests have started.

- [ ] **Step 2: Use the engine request gate in wave load execution**

In wave execution, replace `execute_pipeline_with_runtime_hooks` with `execute_pipeline_with_runtime_request_gate`.

Inside the gate:

```rust
loop {
    if token.is_cancelled() {
        return;
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let target_rps_limit = local_rps_limit(&load, elapsed_ms);
    bucket.lock().await.refill(target_rps_limit, elapsed_ms);
    if bucket.lock().await.try_acquire() {
        metrics.lock().await.record_http_start();
        return;
    }
    tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms.min(100))).await;
}
```

Use `Arc<tokio::sync::Mutex<FlowBucket>>` so concurrent pipeline executions share one limiter.

- [ ] **Step 3: Count completed HTTP attempts**

After each pipeline execution returns, increment `httpCompleted` by the number of step results with `request.is_some()`.

- [ ] **Step 4: Keep launch cadence from flooding**

Keep `maxInFlight` as the pipeline-execution cap. Launch pipelines opportunistically while under `maxInFlight`; the request gate controls HTTP RPS.

- [ ] **Step 5: Run runner tests**

Run: `cargo test -p previa-runner`

Expected: tests compile and pass.

### Task 3: Main Aggregation Samples

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Parse new runner fields**

Add optional `http_started` and `http_completed` to parsed runner load metrics.

- [ ] **Step 2: Add fields to consolidated metrics**

Add `httpStarted` and `httpCompleted` to the consolidated load metrics model and sum them across nodes.

- [ ] **Step 3: Preserve per-runner samples**

Change `build_rps_history_sample` to include a `runners` array built from the latest lines:

```json
{
  "timestamp": 123,
  "rps": 90,
  "httpStarted": 120,
  "httpCompleted": 118,
  "targetRpsLimit": 300,
  "runners": [
    {
      "runnerId": "runner-a",
      "httpStarted": 40,
      "httpCompleted": 39,
      "rps": 30
    }
  ]
}
```

- [ ] **Step 4: Run main aggregation tests**

Run: `cargo test -p previa-main load_batch`

Expected: tests compile and pass.

### Task 4: UI HTTP RPS Chart

**Files:**
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/load-rps-chart.ts`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`
- Modify: `app/src/i18n/locales/pt-BR.json`
- Modify: `app/src/i18n/locales/en.json`

- [ ] **Step 1: Extend TypeScript metrics types**

Add per-runner sample types:

```ts
export interface RunnerRpsSample {
  runnerId: string;
  httpStarted?: number;
  httpCompleted?: number;
  rps?: number;
}

export interface RpsPoint {
  timestamp: number;
  rps: number;
  totalStarted?: number;
  totalSent?: number;
  httpStarted?: number;
  httpCompleted?: number;
  targetIntensity?: number;
  targetRpsLimit?: number;
  runners?: RunnerRpsSample[];
}
```

- [ ] **Step 2: Build chart rows with dynamic runner keys**

Make `buildRpsChartData` return rows containing:

- `rpsTotal`
- `targetRpsLimit`
- one `runner:<id>` numeric field per runner
- `runnerKeys`
- `usesHttpRps`

Calculate interval RPS from `httpStarted` deltas for each runner.

- [ ] **Step 3: Render dynamic runner lines**

In `LoadTestResultsPanel`, render a `<Line>` for each runner key and keep `RPS total` as a green dashed line. Render `RPS alvo` as a subdued dashed line.

- [ ] **Step 4: Update labels**

Use `HTTP RPS ao longo do tempo` when HTTP samples exist. Keep legacy `RPS ao longo do tempo` for old runs.

- [ ] **Step 5: Run UI tests**

Run: `cd app && npm test -- LoadTestResultsPanel`

Expected: tests compile and pass.

### Task 5: Final Verification

**Files:**
- All changed files

- [ ] **Step 1: Run focused Rust tests**

Run:

```bash
cargo test -p previa-engine execution
cargo test -p previa-runner
cargo test -p previa-main load_batch
```

Expected: all pass.

- [ ] **Step 2: Run release build**

Run: `cargo build --release`

Expected: release build succeeds.

- [ ] **Step 3: Commit and push**

Run:

```bash
git add docs/superpowers/plans/2026-05-02-http-rps-load-chart.md engine/src/execution/engine.rs engine/src/execution/mod.rs engine/src/lib.rs runner/src/lib.rs runner/src/server/metrics.rs runner/src/server/models.rs runner/src/server/handlers/load.rs main/src/server/models.rs main/src/server/utils.rs main/src/server/execution/load_batch.rs app/src/types/load-test.ts app/src/lib/load-rps-chart.ts app/src/components/LoadTestResultsPanel.tsx app/src/components/LoadTestResultsPanel.test.tsx app/src/i18n/locales/pt-BR.json app/src/i18n/locales/en.json
git commit -m "feat: chart http rps by runner"
git push origin codex/wave-load-test
```
