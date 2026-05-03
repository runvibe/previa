# Wave Load Diagnostics Corrections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct the wave load test diagnostics so a run shows real latency percentiles, useful pipeline error samples, and no false ERROR logs for expected response observer cancellation.

**Architecture:** Keep request dispatch controlled by `runner/src/server/wave_executor.rs`; do not recouple wave starts to response completion. The runner will publish cumulative, compact diagnostic data in periodic load metrics. The main service will aggregate those runner snapshots without double-counting, and the app will surface the same diagnostics in the load test panel and stored history.

**Tech Stack:** Rust, Tokio, Reqwest, Axum/SSE, serde JSON, existing `previa-runner`, `previa-main`, React/TypeScript load-test dashboard.

---

## Current Evidence

Latest run `019dee82-c388-7da3-bd33-880bc6aaba38` showed:

- `scheduledStarts == dispatchSubmitted == httpStarted == 162052`
- `curveAdherence == 99.38`
- `avgLatency == p95 == p99 == 0`
- `totalSuccess == 0`, `totalError == 78022`
- logs contain many `wave response observer join error: task ... was cancelled`

This means the wave dispatch path is following the curve, but the diagnostics are not sufficient to explain the response/backend failure. The correction should not change the dispatch clock behavior.

## File Map

- Modify: `runner/src/server/models.rs`
  - Add compact latency histogram buckets and error sample structs to the runner load metrics contract.
- Modify: `runner/src/server/metrics.rs`
  - Store cumulative latency histogram and bounded error samples inside `MetricsAccumulator`.
- Modify: `runner/src/server/wave_executor.rs`
  - Pass terminal pipeline duration/error details into metrics.
  - Downgrade expected cancelled observer tasks from ERROR.
- Modify: `main/src/server/models.rs`
  - Extend `RunnerLoadMetricsPoint` with parsed runner latency histogram data.
- Modify: `main/src/server/utils.rs`
  - Parse `latencyBuckets`, `latencySampleCount`, and `latencyTotalDurationMs`.
- Modify: `main/src/server/execution/load_batch.rs`
  - Prefer cumulative runner histograms for consolidated avg/p95/p99.
  - Merge runner error samples into the existing load snapshot `errors` list with dedupe/cap.
- Modify: `app/src/types/load-test.ts`
  - Add `errors?: string[]` to `LoadTestMetrics`.
- Modify: `app/src/lib/remote-executor.ts`
  - Preserve load snapshot errors in metrics.
- Modify: `app/src/lib/api-client.ts`
  - Preserve stored load history errors when converting records to UI runs.
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
  - Show compact diagnostic error samples under the metric cards.
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`
  - Cover error sample rendering.

---

## Task 1: Publish Runner Latency Histogram

**Files:**
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/metrics.rs`

- [ ] **Step 1: Add failing runner metrics test**

Add this test to `runner/src/server/metrics.rs`:

```rust
#[test]
fn snapshot_includes_cumulative_latency_histogram() {
    let mut metrics = MetricsAccumulator::new();

    metrics.update(20.0, true);
    metrics.update(30.4, false);
    metrics.update(30.6, false);

    let snapshot = metrics.snapshot(None, None);

    assert_eq!(snapshot.total_sent, 3);
    assert_eq!(snapshot.total_success, 1);
    assert_eq!(snapshot.total_error, 2);
    assert_eq!(snapshot.latency_sample_count, Some(3));
    assert_eq!(snapshot.latency_total_duration_ms, Some(81));
    assert_eq!(snapshot.latency_buckets.len(), 3);
    assert_eq!(snapshot.latency_buckets[0].duration_ms, 20);
    assert_eq!(snapshot.latency_buckets[0].count, 1);
    assert_eq!(snapshot.latency_buckets[1].duration_ms, 30);
    assert_eq!(snapshot.latency_buckets[1].count, 1);
    assert_eq!(snapshot.latency_buckets[2].duration_ms, 31);
    assert_eq!(snapshot.latency_buckets[2].count, 1);
}
```

Run:

```bash
cargo test -p previa-runner snapshot_includes_cumulative_latency_histogram
```

Expected before implementation: compile failure for missing `latency_*` fields.

- [ ] **Step 2: Add runner metrics contract**

In `runner/src/server/models.rs`, add:

```rust
#[derive(Debug, Serialize, Clone, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadLatencyBucket {
    pub duration_ms: u64,
    pub count: usize,
}
```

Extend `LoadTestMetrics`:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub latency_buckets: Vec<LoadLatencyBucket>,
#[serde(skip_serializing_if = "Option::is_none")]
pub latency_sample_count: Option<usize>,
#[serde(skip_serializing_if = "Option::is_none")]
pub latency_total_duration_ms: Option<u64>,
```

- [ ] **Step 3: Store latency histogram in runner accumulator**

In `runner/src/server/metrics.rs`, import `BTreeMap` and add fields:

```rust
latency_sample_count: usize,
latency_total_duration_ms: u64,
latency_histogram: BTreeMap<u64, usize>,
```

Initialize them in `MetricsAccumulator::new()`:

```rust
latency_sample_count: 0,
latency_total_duration_ms: 0,
latency_histogram: BTreeMap::new(),
```

Change `update` so it records the duration instead of ignoring it:

```rust
pub fn update(&mut self, duration: f64, success: bool) {
    let duration_ms = if duration.is_finite() && duration >= 0.0 {
        duration.round() as u64
    } else {
        0
    };

    self.total_sent = self.total_sent.saturating_add(1);
    self.latency_sample_count = self.latency_sample_count.saturating_add(1);
    self.latency_total_duration_ms = self.latency_total_duration_ms.saturating_add(duration_ms);
    *self.latency_histogram.entry(duration_ms).or_insert(0) += 1;

    if success {
        self.total_success = self.total_success.saturating_add(1);
    } else {
        self.total_error = self.total_error.saturating_add(1);
    }
}
```

When building `LoadTestMetrics`, serialize buckets:

```rust
latency_buckets: self
    .latency_histogram
    .iter()
    .map(|(duration_ms, count)| crate::server::models::LoadLatencyBucket {
        duration_ms: *duration_ms,
        count: *count,
    })
    .collect(),
latency_sample_count: (self.latency_sample_count > 0).then_some(self.latency_sample_count),
latency_total_duration_ms: (self.latency_sample_count > 0)
    .then_some(self.latency_total_duration_ms),
```

- [ ] **Step 4: Verify runner test**

Run:

```bash
cargo test -p previa-runner snapshot_includes_cumulative_latency_histogram
```

Expected: PASS.

---

## Task 2: Aggregate Latency From Runner Histograms In Main

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/utils.rs`
- Modify: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Add failing main consolidation test**

Add this test to the `#[cfg(test)]` module in `main/src/server/execution/load_batch.rs`:

```rust
#[test]
fn consolidates_latency_from_runner_histograms() {
    let latest = HashMap::from([
        (
            "http://runner-a:3000".to_owned(),
            RunnerLoadLine {
                node: "http://runner-a:3000".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 2,
                    "totalSuccess": 1,
                    "totalError": 1,
                    "rps": 10.0,
                    "startTime": 1_000,
                    "elapsedMs": 1_000,
                    "latencySampleCount": 2,
                    "latencyTotalDurationMs": 300,
                    "latencyBuckets": [
                        { "durationMs": 100, "count": 1 },
                        { "durationMs": 200, "count": 1 }
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
                    "totalSent": 2,
                    "totalSuccess": 0,
                    "totalError": 2,
                    "rps": 20.0,
                    "startTime": 900,
                    "elapsedMs": 1_200,
                    "latencySampleCount": 2,
                    "latencyTotalDurationMs": 100,
                    "latencyBuckets": [
                        { "durationMs": 50, "count": 2 }
                    ]
                }),
            },
        ),
    ]);

    let consolidated = consolidate_load_metrics(&latest, LoadLatencySummary::default())
        .expect("expected consolidated metrics");

    assert_eq!(consolidated.total_sent, 4);
    assert_eq!(consolidated.avg_latency, 100);
    assert_eq!(consolidated.p95, 200);
    assert_eq!(consolidated.p99, 200);
}
```

Run:

```bash
cargo test -p previa-main consolidates_latency_from_runner_histograms
```

Expected before implementation: FAIL because avg/p95/p99 remain zero.

- [ ] **Step 2: Add parsed latency fields**

In `main/src/server/models.rs`, add:

```rust
#[derive(Debug, Clone)]
pub struct RunnerLoadLatencyBucket {
    pub duration_ms: u64,
    pub count: usize,
}
```

Extend `RunnerLoadMetricsPoint`:

```rust
pub latency_sample_count: Option<usize>,
pub latency_total_duration_ms: Option<u64>,
pub latency_buckets: Vec<RunnerLoadLatencyBucket>,
```

- [ ] **Step 3: Parse runner histogram fields**

In `main/src/server/utils.rs`, add a helper:

```rust
fn parse_latency_buckets(payload: &Value) -> Vec<crate::server::models::RunnerLoadLatencyBucket> {
    payload
        .get("latencyBuckets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let duration_ms = get_u64_field(item, "durationMs")?;
                    let count = get_usize_field(item, "count")?;
                    Some(crate::server::models::RunnerLoadLatencyBucket { duration_ms, count })
                })
                .collect()
        })
        .unwrap_or_default()
}
```

Populate the new fields in `parse_runner_load_metrics`:

```rust
latency_sample_count: get_usize_field(payload, "latencySampleCount"),
latency_total_duration_ms: get_u64_field(payload, "latencyTotalDurationMs"),
latency_buckets: parse_latency_buckets(payload),
```

- [ ] **Step 4: Prefer runner histograms when available**

In `main/src/server/execution/load_batch.rs`, add:

```rust
fn summarize_runner_latency(latest_by_node: &HashMap<String, RunnerLoadLine>) -> Option<LoadLatencySummary> {
    let mut accumulator = LoadLatencyAccumulator::default();

    for line in latest_by_node.values() {
        let Some(metrics) = parse_runner_load_metrics(&line.payload) else {
            continue;
        };

        for bucket in metrics.latency_buckets {
            accumulator.sample_count = accumulator.sample_count.saturating_add(bucket.count);
            accumulator.total_duration_ms = accumulator
                .total_duration_ms
                .saturating_add((bucket.duration_ms as u128).saturating_mul(bucket.count as u128));
            *accumulator.histogram.entry(bucket.duration_ms).or_insert(0) += bucket.count;
        }
    }

    (accumulator.sample_count > 0).then(|| summarize_load_latency(&accumulator))
}
```

At the start of `consolidate_load_metrics`, replace the passed latency summary only when runner histograms exist:

```rust
let latency = summarize_runner_latency(latest_by_node).unwrap_or(latency);
```

This keeps backward compatibility with old `durationMs` events.

- [ ] **Step 5: Verify main tests**

Run:

```bash
cargo test -p previa-main consolidates_latency_from_runner_histograms
cargo test -p previa-main load_batch
```

Expected: PASS.

---

## Task 3: Surface Pipeline Error Samples

**Files:**
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/metrics.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/remote-executor.ts`
- Modify: `app/src/lib/api-client.ts`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`
- Modify: `app/src/components/LoadTestResultsPanel.test.tsx`

- [ ] **Step 1: Add failing runner error sample test**

Add this test to `runner/src/server/metrics.rs`:

```rust
#[test]
fn snapshot_includes_deduped_error_samples() {
    let mut metrics = MetricsAccumulator::new();

    metrics.record_error_sample("create_user", Some(409), "HTTP 409 Conflict");
    metrics.record_error_sample("create_user", Some(409), "HTTP 409 Conflict");
    metrics.record_error_sample("get_created_user", Some(404), "HTTP 404 Not Found");

    let snapshot = metrics.snapshot(None, None);

    assert_eq!(snapshot.error_samples.len(), 2);
    assert_eq!(snapshot.error_samples[0].step_id, "create_user");
    assert_eq!(snapshot.error_samples[0].http_status, Some(409));
    assert_eq!(snapshot.error_samples[0].count, 2);
    assert_eq!(snapshot.error_samples[1].step_id, "get_created_user");
    assert_eq!(snapshot.error_samples[1].count, 1);
}
```

Run:

```bash
cargo test -p previa-runner snapshot_includes_deduped_error_samples
```

Expected before implementation: compile failure for missing `record_error_sample`/`error_samples`.

- [ ] **Step 2: Add runner error sample contract**

In `runner/src/server/models.rs`, add:

```rust
#[derive(Debug, Serialize, Clone, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadErrorSample {
    pub step_id: String,
    pub http_status: Option<u16>,
    pub error: String,
    pub count: usize,
}
```

Extend `LoadTestMetrics`:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub error_samples: Vec<LoadErrorSample>,
```

- [ ] **Step 3: Store bounded error samples**

In `runner/src/server/metrics.rs`, add:

```rust
error_samples: Vec<crate::server::models::LoadErrorSample>,
```

Initialize:

```rust
error_samples: Vec::new(),
```

Add:

```rust
pub fn record_error_sample(&mut self, step_id: &str, http_status: Option<u16>, error: &str) {
    if let Some(existing) = self.error_samples.iter_mut().find(|sample| {
        sample.step_id == step_id
            && sample.http_status == http_status
            && sample.error == error
    }) {
        existing.count = existing.count.saturating_add(1);
        return;
    }

    if self.error_samples.len() >= 10 {
        return;
    }

    self.error_samples.push(crate::server::models::LoadErrorSample {
        step_id: step_id.to_owned(),
        http_status,
        error: error.to_owned(),
        count: 1,
    });
}
```

When building `LoadTestMetrics`:

```rust
error_samples: self.error_samples.clone(),
```

- [ ] **Step 4: Record terminal failure details**

In `runner/src/server/wave_executor.rs`, change terminal recording to accept an optional result reference:

```rust
async fn record_terminal_pipeline(
    metrics: &Arc<tokio::sync::Mutex<MetricsAccumulator>>,
    cursor: PipelineCursor,
    success: bool,
    result: Option<&StepExecutionResult>,
) {
    let duration_ms = cursor.pipeline_started_at.elapsed().as_millis() as f64;
    let mut lock = metrics.lock().await;
    lock.update(duration_ms, success);

    if !success {
        if let Some(result) = result {
            let http_status = result.response.as_ref().map(|response| response.status);
            let error = result.error.as_deref().unwrap_or("pipeline failed");
            lock.record_error_sample(&result.step_id, http_status, error);
        }
    }
}
```

Update the call sites:

```rust
record_terminal_pipeline(metrics, cursor, false, Some(&result)).await;
record_terminal_pipeline(metrics, cursor, true, Some(&result)).await;
record_terminal_pipeline(metrics, cursor, false, None).await;
```

- [ ] **Step 5: Merge error samples into main snapshots**

In `main/src/server/execution/load_batch.rs`, add:

```rust
async fn merge_runner_error_samples(
    load_errors: &Arc<Mutex<Vec<String>>>,
    node: &str,
    payload: &Value,
) {
    let Some(samples) = payload.get("errorSamples").and_then(Value::as_array) else {
        return;
    };

    let mut lock = load_errors.lock().await;
    for sample in samples {
        if lock.len() >= 20 {
            break;
        }

        let step_id = sample.get("stepId").and_then(Value::as_str).unwrap_or("unknown_step");
        let error = sample.get("error").and_then(Value::as_str).unwrap_or("pipeline failed");
        let count = sample.get("count").and_then(Value::as_u64).unwrap_or(1);
        let status = sample
            .get("httpStatus")
            .and_then(Value::as_u64)
            .map(|value| format!(" HTTP {}", value))
            .unwrap_or_default();
        let message = format!("{} {}{} x{}: {}", node, step_id, status, count, error);

        if !lock.iter().any(|existing| existing == &message) {
            lock.push(message);
        }
    }
}
```

Call it immediately after each runner metrics payload is parsed in `forward_runner_stream_load_chunked`:

```rust
if event == "metrics" {
    if let Some(duration_ms) = parse_runner_duration_ms(&data) {
        let mut lock = load_latency.lock().await;
        lock.add_sample(duration_ms);
    }
    merge_runner_error_samples(&load_errors, &node, &data).await;
}
```

- [ ] **Step 6: Show load errors in the app panel**

In `app/src/types/load-test.ts`, add:

```ts
errors?: string[];
```

to `LoadTestMetrics`.

In `app/src/lib/remote-executor.ts`, set errors when building load metrics from snapshots:

```ts
const errors = pickStringArray(snapshot.errors);
```

and include:

```ts
errors,
```

In `app/src/lib/api-client.ts`, preserve stored history errors:

```ts
errors: Array.isArray(r.errors)
  ? r.errors.filter((item): item is string => typeof item === "string")
  : [],
```

In `app/src/components/LoadTestResultsPanel.tsx`, render:

```tsx
{metrics.errors && metrics.errors.length > 0 && (
  <div className="glass rounded-lg p-3 space-y-2">
    <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
      {t("loadTestResults.errorSamples", "Error samples")}
    </p>
    <div className="space-y-1">
      {metrics.errors.slice(0, 5).map((error, index) => (
        <p key={`${error}-${index}`} className="text-xs text-destructive break-words">
          {error}
        </p>
      ))}
    </div>
  </div>
)}
```

Add a test in `app/src/components/LoadTestResultsPanel.test.tsx`:

```tsx
it("renders load error samples", () => {
  render(
    <LoadTestResultsPanel
      state="completed"
      totalRequests={0}
      metrics={{
        totalSent: 10,
        totalSuccess: 0,
        totalError: 10,
        avgLatency: 20,
        p95: 30,
        p99: 40,
        rps: 5,
        latencyHistory: [],
        rpsHistory: [],
        runnerResourceHistory: [],
        startTime: 1,
        elapsedMs: 2,
        errors: ["runner-a create_user HTTP 409 x10: HTTP 409 Conflict"],
      }}
    />,
  );

  expect(screen.getByText("Error samples")).toBeInTheDocument();
  expect(screen.getByText(/create_user HTTP 409/)).toBeInTheDocument();
});
```

- [ ] **Step 7: Verify runner, main, and app**

Run:

```bash
cargo test -p previa-runner snapshot_includes_deduped_error_samples
cargo test -p previa-main load_batch
npm --prefix app test -- LoadTestResultsPanel
```

Expected: PASS.

---

## Task 4: Treat Expected Observer Cancellation As Non-Error

**Files:**
- Modify: `runner/src/server/wave_executor.rs`

- [ ] **Step 1: Add a drain report test**

Add a small report type near `drain_finished_tasks`:

```rust
#[derive(Debug, Default, PartialEq, Eq)]
struct DrainTaskReport {
    completed: usize,
    cancelled: usize,
    failed: usize,
}
```

Add this test to the `#[cfg(test)]` module in `runner/src/server/wave_executor.rs`:

```rust
#[tokio::test]
async fn drain_finished_tasks_reports_cancelled_tasks_without_failure() {
    let mut tasks = JoinSet::new();
    tasks.spawn(async {
        std::future::pending::<()>().await;
    });

    tasks.abort_all();

    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
    let report = drain_finished_tasks(&mut tasks).await;

    assert_eq!(report.cancelled, 1);
    assert_eq!(report.failed, 0);
    assert!(tasks.is_empty());
}
```

Run:

```bash
cargo test -p previa-runner drain_finished_tasks_reports_cancelled_tasks_without_failure
```

Expected before implementation: compile failure because `drain_finished_tasks` returns `()`.

- [ ] **Step 2: Return a report from `drain_finished_tasks`**

Change the function:

```rust
async fn drain_finished_tasks(tasks: &mut JoinSet<()>) -> DrainTaskReport {
    let mut report = DrainTaskReport::default();

    loop {
        match tokio::time::timeout(tokio::time::Duration::from_millis(0), tasks.join_next()).await {
            Ok(Some(Err(err))) if err.is_cancelled() => {
                report.cancelled = report.cancelled.saturating_add(1);
            }
            Ok(Some(Err(_err))) => {
                report.failed = report.failed.saturating_add(1);
            }
            Ok(Some(Ok(()))) => {
                report.completed = report.completed.saturating_add(1);
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    report
}
```

At call sites, consume the report and only log real failures:

```rust
let report = drain_finished_tasks(&mut tasks).await;
if report.failed > 0 {
    error!("wave response observer task failures: {}", report.failed);
}
if report.cancelled > 0 {
    tracing::debug!("wave response observer tasks cancelled: {}", report.cancelled);
}
```

Keep `tracing::error` for real failures.

- [ ] **Step 3: Verify cancellation handling**

Run:

```bash
cargo test -p previa-runner drain_finished_tasks_reports_cancelled_tasks_without_failure
cargo test -p previa-runner
```

Expected: PASS, and expected observer aborts are no longer logged as ERROR.

---

## Task 5: Full Verification With A Real Wave Run

**Files:**
- No extra source edits after Tasks 1-4.

- [ ] **Step 1: Run formatting and focused tests**

Run:

```bash
git diff --check
cargo test -p previa-runner
cargo test -p previa-main load_batch
cargo test -p previa-engine execution
npm --prefix app test -- LoadTestResultsPanel
npm --prefix app run build
```

Expected: all pass. Vite may keep existing chunk/dynamic import warnings.

- [ ] **Step 2: Run release build required by repo instructions**

Run:

```bash
cargo build --release
```

Expected: PASS.

- [ ] **Step 3: Restart local main and three runners with the release binaries**

Use the existing local process pattern for ports:

```bash
target/release/previa-main
target/release/previa-runner --port 5611
target/release/previa-runner --port 5612
target/release/previa-runner --port 5613
```

Then check:

```bash
curl -s http://127.0.0.1:5610/info
```

Expected:

```json
{
  "activeRunners": 3
}
```

- [ ] **Step 4: Execute the same CRUD Users wave load test**

Open:

```text
http://localhost:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Run the same wave profile used in the failing analysis.

- [ ] **Step 5: Validate acceptance criteria from the API**

Fetch latest history:

```bash
curl -s 'http://127.0.0.1:5610/api/v1/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/tests/load?pipelineIndex=0&limit=1'
```

Expected:

- `finalConsolidated.avgLatency > 0` when `finalConsolidated.totalSent > 0`
- `finalConsolidated.p95 > 0`
- `finalConsolidated.p99 > 0`
- `errors` contains at least one actionable sample if `totalError > 0`
- `finalConsolidated.curveAdherence` remains near the previous value, expected `>= 99`
- runner `finalLines[].payload.latencyBuckets` exists
- runner `finalLines[].payload.errorSamples` exists when failures happen

- [ ] **Step 6: Validate logs**

Check runner logs for this run:

```bash
rg 'wave response observer join error|wave response observer task failures|tasks cancelled' /tmp/previa-runner*.log
```

Expected:

- No `wave response observer join error: task ... was cancelled` at ERROR level.
- Real task failures, if any, still appear as `wave response observer task failures`.

- [ ] **Step 7: Commit and push**

Only after all verification passes:

```bash
git status --short
git add runner/src/server/models.rs runner/src/server/metrics.rs runner/src/server/wave_executor.rs main/src/server/models.rs main/src/server/utils.rs main/src/server/execution/load_batch.rs app/src/types/load-test.ts app/src/lib/remote-executor.ts app/src/lib/api-client.ts app/src/components/LoadTestResultsPanel.tsx app/src/components/LoadTestResultsPanel.test.tsx docs/superpowers/plans/2026-05-03-wave-load-diagnostics-corrections.md
git commit -m "fix: surface wave load diagnostics"
git push
```

Expected: branch `codex/wave-load-test` pushed.

---

## Acceptance Summary

The correction is complete when a wave load run can answer these questions directly from the UI/API:

- Did the runner follow the wave? Check `curveAdherence`, `httpStarted`, and `targetRpsLimit`.
- Did the target/backend keep up? Check `httpSendReturned`, `responseBodyCompleted`, `outstandingRequests`, and latency p95/p99.
- Why did pipelines fail? Check `errors` and runner `errorSamples`.
- Did grace shutdown generate false ERROR logs? It should not.

