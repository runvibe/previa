# Wave Isolated HTTP Fire Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep the wave's real HTTP send starts close to the configured timeline by isolating the fire path from response/body/assertion/pipeline-continuation work.

**Architecture:** Split the current wave sender into three responsibilities: fire workers accept ready requests on the sender runtime, fire tasks start the real HTTP request and wait only for the `reqwest::send()` result, and observer workers on a separate runtime complete body reading, assertions, metrics, and pipeline continuation. `sendStarted` remains the wave dispatch acceptance metric, `httpStarted` becomes the real pre-`.send()` metric, and `senderLaggedStarts` remains a diagnostic for starts that entered the fire path after their tick window.

**Tech Stack:** Rust, Tokio, reqwest, mpsc channels, Previa engine HTTP execution helpers, Axum/SSE load telemetry, SQLite history, React/TypeScript load-test UI.

---

## File Structure

- Modify `engine/src/execution/http_step.rs`
  - Extract "start HTTP request" from "read body/evaluate assertions".
  - Add a public `StartedHttpStep` struct.
  - Add `start_prepared_http_step_with_hooks(...)`.
  - Add `complete_started_http_step_with_hook(...)`.
  - Keep `send_prepared_http_step_with_hooks(...)` as the compatibility wrapper used by non-wave code.

- Modify `engine/src/execution/mod.rs` and `engine/src/lib.rs`
  - Re-export the new split HTTP helpers and `StartedHttpStep`.

- Modify `runner/src/server/wave_sender.rs`
  - Change `ObserverCommand` so it carries either a completed start error result or a started HTTP response.
  - Fire workers must spawn HTTP start tasks immediately after accepting a ready request.
  - Fire tasks call `start_prepared_http_step_with_hooks(...)` and send the start outcome to the observer channel.
  - Observer workers call `complete_started_http_step_with_hook(...)` for successful HTTP starts.
  - Move observer work onto a dedicated OS thread/runtime.

- Modify `runner/src/server/wave_metrics_actor.rs` and `runner/src/server/metrics.rs`
  - No schema rename required.
  - Keep existing lifecycle counters.
  - If needed, add tests proving `httpStarted` can now differ from `sendStarted` without lowering open-loop acceptance.

- Modify `main/src/server/execution/load_batch.rs`
  - No behavioral change expected.
  - Keep `curveAdherence` based on `sendStarted`.

- Modify tests:
  - `engine/src/execution/http_step.rs`
  - `runner/src/server/wave_sender.rs`
  - Optional: `runner/src/server/wave_metrics_actor.rs`
  - Optional: `main/src/server/execution/load_batch.rs`

---

## Design Notes

The current implementation fixed total load loss, but not timeline fidelity:

```text
scheduledStarts:     106902
sendStarted:         106902
httpStarted:         106902
senderLaggedStarts:   59072
curveAdherence:       70.87
```

This means the system eventually starts every request, but many starts happen outside their intended second. The next correction is not to replay more aggressively; it is to remove response/body work from the runtime that needs to keep timing.

The intended lifecycle after this change:

```text
scheduler thread
  -> dispatcher/prepare thread
  -> sender fire runtime
       records SendStarted
       spawns HTTP start task
       records HttpStarted immediately before reqwest.send()
       sends StartedHttpStep or start error to observer channel
  -> observer runtime
       reads body
       evaluates assertions
       records HttpSendReturned / ResponseBodyCompleted / HttpCompleted / PipelineFinished
       emits WaveObserverEvent back to dispatcher
```

`response_in_flight` should continue to represent requests whose final observation/pipeline result is pending. It is incremented when the fire path accepts the request and decremented after the observer finishes, including send errors.

---

### Task 1: Add Split HTTP Start/Complete Helpers in Engine

**Files:**
- Modify: `engine/src/execution/http_step.rs`
- Modify: `engine/src/execution/mod.rs`
- Modify: `engine/src/lib.rs`

- [ ] **Step 1: Write a failing engine test for start vs body split**

Add this test to `engine/src/execution/http_step.rs` inside `mod tests`:

```rust
#[tokio::test]
async fn split_http_helpers_start_send_before_body_completion() {
    let server = httpmock::MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/users");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({"ok": true}));
        })
        .await;

    let client = reqwest::Client::new();
    let step = PipelineStep {
        id: "get-users".to_owned(),
        name: "GET users".to_owned(),
        description: None,
        method: "GET".to_owned(),
        url: format!("{}/users", server.base_url()),
        headers: HashMap::new(),
        body: None,
        operation_id: None,
        delay: None,
        retry: None,
        asserts: vec![],
    };
    let context = HashMap::new();
    let prepared = prepare_http_step(&step, &context, None, None, None, 1, 1)
        .expect("step should prepare");
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));

    let started_events = std::sync::Arc::clone(&events);
    let returned_events = std::sync::Arc::clone(&events);
    let started = start_prepared_http_step_with_hooks(
        &client,
        prepared,
        &step,
        || false,
        move || {
            let events = std::sync::Arc::clone(&started_events);
            async move {
                events.lock().expect("events lock").push("started");
            }
        },
        move || {
            let events = std::sync::Arc::clone(&returned_events);
            async move {
                events.lock().expect("events lock").push("returned");
            }
        },
    )
    .await
    .expect("send should not be cancelled")
    .expect("send should return response, not error result");

    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        ["started", "returned"]
    );

    let body_events = std::sync::Arc::clone(&events);
    let result = complete_started_http_step_with_hook(
        started,
        &step,
        &context,
        None,
        None,
        None,
        || false,
        move || {
            let events = std::sync::Arc::clone(&body_events);
            async move {
                events.lock().expect("events lock").push("body");
            }
        },
    )
    .await
    .expect("body completion should not be cancelled");

    assert_eq!(result.status, "success");
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        ["started", "returned", "body"]
    );
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test -p previa-engine split_http_helpers_start_send_before_body_completion
```

Expected: FAIL to compile because `start_prepared_http_step_with_hooks`, `complete_started_http_step_with_hook`, and `StartedHttpStep` do not exist yet.

- [ ] **Step 3: Add the split result type**

In `engine/src/execution/http_step.rs`, after `PreparedHttpStep`, add:

```rust
#[derive(Debug)]
pub struct StartedHttpStep {
    pub request: StepRequest,
    pub response: reqwest::Response,
    started_at: Instant,
    attempt: usize,
    max_attempts: usize,
}
```

This type intentionally owns the `reqwest::Response` so it can cross from the fire runtime to the observer runtime.

- [ ] **Step 4: Add the start helper**

Add this function near `send_prepared_http_step_with_hooks`:

```rust
pub async fn start_prepared_http_step_with_hooks<
    FCancel,
    FStart,
    FStartFuture,
    FSend,
    FSendFuture,
>(
    client: &Client,
    prepared: PreparedHttpStep,
    step: &PipelineStep,
    mut should_cancel: FCancel,
    mut on_send_started: FStart,
    mut on_send_returned: FSend,
) -> Option<Result<StartedHttpStep, StepExecutionResult>>
where
    FCancel: FnMut() -> bool,
    FStart: FnMut() -> FStartFuture,
    FStartFuture: Future<Output = ()>,
    FSend: FnMut() -> FSendFuture,
    FSendFuture: Future<Output = ()>,
{
    let mut request_builder = client.request(prepared.method.clone(), prepared.url.clone());

    for (key, value) in &prepared.request.headers {
        request_builder = request_builder.header(key, value);
    }

    if let Some(body) = prepared.request.body.as_ref() {
        if !prepared.request.method.eq_ignore_ascii_case("GET")
            && !prepared.request.method.eq_ignore_ascii_case("HEAD")
        {
            request_builder = request_builder.json(body);
        }
    }

    let request = prepared.request.clone();
    if should_cancel() {
        return None;
    }
    on_send_started().await;
    let Some(send_result) = await_with_cancel(request_builder.send(), &mut should_cancel).await
    else {
        return None;
    };
    on_send_returned().await;

    match send_result {
        Ok(response) => Some(Ok(StartedHttpStep {
            request,
            response,
            started_at: prepared.started_at,
            attempt: prepared.attempt,
            max_attempts: prepared.max_attempts,
        })),
        Err(err) => {
            let result = step_result(
                step,
                request,
                None,
                Some(err.to_string()),
                "error",
                prepared.started_at,
                prepared.attempt,
                prepared.max_attempts,
                None,
            );
            log_step_response(&step.id, None, result.error.as_deref());
            Some(Err(result))
        }
    }
}
```

- [ ] **Step 5: Add the completion helper**

Move the existing `Ok(response)` body processing from `send_prepared_http_step_with_hooks` into:

```rust
pub async fn complete_started_http_step_with_hook<FCancel, FBody, FBodyFuture>(
    started: StartedHttpStep,
    step: &PipelineStep,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    mut should_cancel: FCancel,
    mut on_body_completed: FBody,
) -> Option<StepExecutionResult>
where
    FCancel: FnMut() -> bool,
    FBody: FnMut() -> FBodyFuture,
    FBodyFuture: Future<Output = ()>,
{
    let response = started.response;
    let status = response.status();
    let status_text = status.canonical_reason().unwrap_or("").to_owned();
    let mut headers = HashMap::new();
    for (key, value) in response.headers() {
        headers.insert(
            key.as_str().to_owned(),
            value.to_str().unwrap_or_default().to_owned(),
        );
    }

    let content_type = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
        .unwrap_or("");

    let body = if content_type.contains("application/json") {
        let Some(body_result) = await_with_cancel(response.json::<Value>(), &mut should_cancel).await
        else {
            return None;
        };
        on_body_completed().await;
        match body_result {
            Ok(value) => value,
            Err(err) => {
                let result = step_result(
                    step,
                    started.request,
                    None,
                    Some(err.to_string()),
                    "error",
                    started.started_at,
                    started.attempt,
                    started.max_attempts,
                    None,
                );
                log_step_response(&step.id, None, result.error.as_deref());
                return Some(result);
            }
        }
    } else {
        let Some(body_result) = await_with_cancel(response.text(), &mut should_cancel).await else {
            return None;
        };
        on_body_completed().await;
        Value::String(body_result.unwrap_or_default())
    };

    let http_error =
        (!status.is_success()).then(|| format!("HTTP {} {}", status.as_u16(), status_text));
    let mut result = step_result(
        step,
        started.request,
        Some(StepResponse {
            status: status.as_u16(),
            status_text: status_text.clone(),
            headers,
            body,
        }),
        http_error.clone(),
        "success",
        started.started_at,
        started.attempt,
        started.max_attempts,
        None,
    );

    let has_status_assert = has_status_assertion(step);
    let assert_results = evaluate_assertions(
        step,
        &result,
        context,
        specs,
        env_groups,
        selected_env_group_slug,
    );
    let assertion_failed = assert_results.iter().any(|result| !result.passed);
    if !assert_results.is_empty() {
        if assertion_failed {
            result.status = "error".to_owned();
            let failed_count = assert_results.iter().filter(|result| !result.passed).count();
            result.error = Some(match result.error {
                Some(err) => format!("{} | {} assertion(s) failed", err, failed_count),
                None => format!("{} assertion(s) failed", failed_count),
            });
        } else if http_error.is_some() {
            if has_status_assert {
                result.status = "success".to_owned();
                result.error = None;
            } else {
                result.status = "error".to_owned();
            }
        }
        result.assert_results = Some(assert_results);
    } else if http_error.is_some() {
        result.status = "error".to_owned();
    }

    log_step_response(&step.id, result.response.as_ref(), result.error.as_deref());
    Some(result)
}
```

Use the existing imports already present in the file.

- [ ] **Step 6: Rewrite the compatibility wrapper**

Rewrite `send_prepared_http_step_with_hooks(...)` so it calls the new start helper and completion helper:

```rust
let started = start_prepared_http_step_with_hooks(
    client,
    prepared,
    step,
    || should_cancel(),
    on_send_started,
    on_send_returned,
)
.await?;

match started {
    Ok(started) => {
        complete_started_http_step_with_hook(
            started,
            step,
            context,
            specs,
            env_groups,
            selected_env_group_slug,
            should_cancel,
            on_body_completed,
        )
        .await
    }
    Err(result) => Some(result),
}
```

- [ ] **Step 7: Re-export new helpers**

In `engine/src/execution/mod.rs`, export:

```rust
pub use http_step::{
    PreparedHttpStep, StartedHttpStep, complete_started_http_step_with_hook, prepare_http_step,
    send_prepared_http_step, send_prepared_http_step_with_hooks,
    start_prepared_http_step_with_hooks,
};
```

In `engine/src/lib.rs`, export:

```rust
pub use execution::{
    PreparedHttpStep, StartedHttpStep, complete_started_http_step_with_hook, execute_pipeline,
    prepare_http_step, send_prepared_http_step, send_prepared_http_step_with_hooks,
    start_prepared_http_step_with_hooks,
};
```

- [ ] **Step 8: Run engine tests**

Run:

```bash
cargo test -p previa-engine
```

Expected: all engine tests pass.

---

### Task 2: Introduce Started Observer Commands in Runner

**Files:**
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Write a failing test for fire/observer split**

Add this test inside `runner/src/server/wave_sender.rs` tests:

```rust
#[tokio::test]
async fn fire_worker_starts_http_before_observer_reads_body() {
    let server = httpmock::MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(httpmock::Method::GET).path("/ok");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!({"ok": true}));
        })
        .await;

    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
    let (observer_tx, mut observer_rx) = mpsc::unbounded_channel();
    let ready_to_send = Arc::new(AtomicUsize::new(0));
    let response_in_flight = Arc::new(AtomicUsize::new(0));
    let token = tokio_util::sync::CancellationToken::new();
    let started = Instant::now();
    let client = Arc::new(Client::new());

    let mut request = test_ready_wave_request(1, started, 0, 60_000);
    request.step.url = format!("{}/ok", server.base_url());
    request.prepared = previa_runner::prepare_http_step(
        &request.step,
        &request.context,
        None,
        None,
        None,
        1,
        1,
    )
    .expect("mock request should prepare");

    ready_to_send.fetch_add(1, Ordering::SeqCst);
    request_tx.send(request).expect("request should enqueue");
    drop(request_tx);

    run_fire_only_sender_for_test(
        client,
        started,
        metric_tx.clone(),
        Arc::clone(&response_in_flight),
        Arc::clone(&ready_to_send),
        request_rx,
        observer_tx,
        token,
    )
    .await;

    assert!(observer_rx.try_recv().is_ok());

    let mut http_started = 0usize;
    let mut body_completed = 0usize;
    while let Ok(event) = metric_rx.try_recv() {
        if matches!(event, WaveMetricEvent::HttpStarted { .. }) {
            http_started += 1;
        }
        if matches!(event, WaveMetricEvent::ResponseBodyCompleted { .. }) {
            body_completed += 1;
        }
    }

    assert_eq!(http_started, 1);
    assert_eq!(body_completed, 0);
    assert_eq!(response_in_flight.load(Ordering::SeqCst), 1);
}
```

This will require updating `run_fire_only_sender_for_test` to accept `Arc<Client>`.

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test -p previa-runner fire_worker_starts_http_before_observer_reads_body
```

Expected: FAIL to compile because fire workers do not yet start HTTP or emit observer commands with started HTTP responses.

- [ ] **Step 3: Add observer command variants**

Change:

```rust
struct ObserverCommand<C> {
    request: ReadyWaveRequest<C>,
}
```

To:

```rust
enum ObserverCommand<C> {
    Started {
        request: ReadyWaveRequest<C>,
        started: previa_runner::StartedHttpStep,
    },
    StartError {
        cursor: C,
        result: StepExecutionResult,
    },
}
```

Add imports:

```rust
use previa_runner::{
    PipelineStep, PreparedHttpStep, RuntimeEnvGroup, RuntimeSpec, StartedHttpStep,
    StepExecutionResult, complete_started_http_step_with_hook,
    send_prepared_http_step_with_hooks, start_prepared_http_step_with_hooks,
};
```

After the migration, remove `send_prepared_http_step_with_hooks` from this file if it is no longer used.

- [ ] **Step 4: Replace `observe_ready_request` with `observe_started_request`**

Remove the direct call to `send_prepared_http_step_with_hooks` from observer code and add:

```rust
async fn observe_started_request<C>(
    started_at: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    request: ReadyWaveRequest<C>,
    started_http: StartedHttpStep,
    token: tokio_util::sync::CancellationToken,
) -> Option<WaveObserverEvent<C>>
where
    C: Send + 'static,
{
    let metrics_for_body = metric_tx.clone();
    let result = complete_started_http_step_with_hook(
        started_http,
        &request.step,
        &request.context,
        Some(request.specs.as_slice()),
        Some(request.env_groups.as_slice()),
        request.selected_env_group_slug.as_deref(),
        || token.is_cancelled(),
        move || {
            let metric_tx = metrics_for_body.clone();
            async move {
                let _ = metric_tx.send(WaveMetricEvent::ResponseBodyCompleted {
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                    count: 1,
                });
            }
        },
    )
    .await?;

    let (network_tx_bytes, network_rx_bytes) =
        estimate_results_network_bytes(std::slice::from_ref(&result));
    if result.request.is_some() {
        let _ = metric_tx.send(WaveMetricEvent::HttpCompleted(1));
    }
    let _ = metric_tx.send(WaveMetricEvent::NetworkBytes {
        tx: network_tx_bytes,
        rx: network_rx_bytes,
    });

    Some(WaveObserverEvent {
        cursor: request.cursor,
        result,
    })
}
```

- [ ] **Step 5: Handle start errors in observer**

Add:

```rust
async fn observe_start_error<C>(
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    cursor: C,
    result: StepExecutionResult,
) -> Option<WaveObserverEvent<C>>
where
    C: Send + 'static,
{
    let (network_tx_bytes, network_rx_bytes) =
        estimate_results_network_bytes(std::slice::from_ref(&result));
    if result.request.is_some() {
        let _ = metric_tx.send(WaveMetricEvent::HttpCompleted(1));
    }
    let _ = metric_tx.send(WaveMetricEvent::NetworkBytes {
        tx: network_tx_bytes,
        rx: network_rx_bytes,
    });

    Some(WaveObserverEvent { cursor, result })
}
```

- [ ] **Step 6: Update `run_observer_request`**

Change its input from a `ReadyWaveRequest<C>` to `ObserverCommand<C>`:

```rust
async fn run_observer_request<C>(
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    command: ObserverCommand<C>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let result = match command {
        ObserverCommand::Started { request, started: started_http } => {
            observe_started_request(started, metric_tx, request, started_http, token).await
        }
        ObserverCommand::StartError { cursor, result } => {
            observe_start_error(metric_tx, cursor, result).await
        }
    };

    response_in_flight.fetch_sub(1, Ordering::SeqCst);
    if let Some(event) = result {
        let _ = observer_tx.send(event);
    }
}
```

- [ ] **Step 7: Run runner tests**

Run:

```bash
cargo test -p previa-runner fire_worker_starts_http_before_observer_reads_body
```

Expected: still fails because fire workers do not yet produce `ObserverCommand::Started`.

---

### Task 3: Start HTTP in Fire Tasks

**Files:**
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Add a fire start helper**

Add:

```rust
async fn start_ready_request<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    observer_tx: mpsc::UnboundedSender<ObserverCommand<C>>,
    request: ReadyWaveRequest<C>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let metrics_for_start = metric_tx.clone();
    let metrics_for_send = metric_tx.clone();
    let prepared = request.prepared.clone();
    let step = request.step.clone();
    let start_result = start_prepared_http_step_with_hooks(
        client.as_ref(),
        prepared,
        &step,
        || token.is_cancelled(),
        move || {
            let metric_tx = metrics_for_start.clone();
            async move {
                let _ = metric_tx.send(WaveMetricEvent::HttpStarted {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
        },
        move || {
            let metric_tx = metrics_for_send.clone();
            async move {
                let _ = metric_tx.send(WaveMetricEvent::HttpSendReturned {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
        },
    )
    .await;

    let Some(start_result) = start_result else {
        response_in_flight.fetch_sub(1, Ordering::SeqCst);
        return;
    };

    let command = match start_result {
        Ok(started_http) => ObserverCommand::Started {
            request,
            started: started_http,
        },
        Err(result) => ObserverCommand::StartError {
            cursor: request.cursor,
            result,
        },
    };
    let _ = observer_tx.send(command);
}
```

- [ ] **Step 2: Pass `client` into fire workers**

Change `run_sender_worker` signature from:

```rust
async fn run_sender_worker<C>(
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    mut worker_rx: mpsc::UnboundedReceiver<SenderWorkerCommand<C>>,
    observer_tx: mpsc::UnboundedSender<ObserverCommand<C>>,
    token: tokio_util::sync::CancellationToken,
)
```

To:

```rust
async fn run_sender_worker<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    mut worker_rx: mpsc::UnboundedReceiver<SenderWorkerCommand<C>>,
    observer_tx: mpsc::UnboundedSender<ObserverCommand<C>>,
    token: tokio_util::sync::CancellationToken,
)
```

Update all call sites, including `run_fire_only_sender_for_test`.

- [ ] **Step 3: Spawn start tasks from the fire worker**

Replace:

```rust
if observer_tx.send(ObserverCommand { request }).is_err() {
    response_in_flight.fetch_sub(1, Ordering::SeqCst);
    break;
}
```

With:

```rust
let start_task = start_ready_request(
    Arc::clone(&client),
    started,
    metric_tx.clone(),
    Arc::clone(&response_in_flight),
    observer_tx.clone(),
    request,
    token.clone(),
);
tokio::spawn(start_task);
```

This is the critical change: fire workers return to the channel immediately after spawning the HTTP start task.

- [ ] **Step 4: Keep response-in-flight decrement in observer**

Do not decrement `response_in_flight` in the fire task when the observer channel accepts a command. Only decrement in `run_observer_request`.

If `start_prepared_http_step_with_hooks` returns `None` due to cancellation, `start_ready_request` must decrement:

```rust
response_in_flight.fetch_sub(1, Ordering::SeqCst);
```

Only do this on cancellation/no-command, because no observer will receive the command.

- [ ] **Step 5: Update tests expecting fire-only observer commands**

Existing tests that count observer commands should still pass, but they may need to wait until HTTP start tasks finish. Use condition-based waiting:

```rust
timeout(Duration::from_secs(1), async {
    loop {
        if observer_rx.try_recv().is_ok() {
            break;
        }
        tokio::task::yield_now().await;
    }
})
.await
.expect("observer command should be produced");
```

Do not use fixed sleeps.

- [ ] **Step 6: Run runner sender tests**

Run:

```bash
cargo test -p previa-runner wave_sender
```

Expected: all wave sender tests pass.

---

### Task 4: Move Observer Loop to a Dedicated Runtime Thread

**Files:**
- Modify: `runner/src/server/wave_sender.rs`

- [ ] **Step 1: Add observer worker count config**

Add:

```rust
fn observer_worker_count() -> usize {
    std::env::var("RUNNER_WAVE_OBSERVER_WORKERS")
        .or_else(|_| std::env::var("RUNNER_WAVE_OBSERVER_THREADS"))
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(2)
                .clamp(2, 16)
        })
}
```

- [ ] **Step 2: Add observer handle**

Add:

```rust
struct WaveObserverHandle {
    join: std::thread::JoinHandle<()>,
}

impl WaveObserverHandle {
    fn stop(self) {
        if let Err(err) = self.join.join() {
            tracing::error!("wave observer thread panicked: {:?}", err);
        }
    }
}
```

- [ ] **Step 3: Add `spawn_wave_observer_thread`**

Add:

```rust
fn spawn_wave_observer_thread<C>(
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    observer_command_rx: mpsc::UnboundedReceiver<ObserverCommand<C>>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    token: tokio_util::sync::CancellationToken,
) -> WaveObserverHandle
where
    C: Send + 'static,
{
    let join = std::thread::Builder::new()
        .name("previa-wave-observer".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(observer_worker_count())
                .thread_name("previa-wave-observer")
                .enable_all()
                .build()
                .expect("failed to build previa wave observer runtime");
            runtime.block_on(run_observer_loop(
                started,
                metric_tx,
                response_in_flight,
                observer_command_rx,
                observer_tx,
                token,
            ));
        })
        .expect("failed to spawn previa wave observer thread");

    WaveObserverHandle { join }
}
```

- [ ] **Step 4: Remove observer task from sender runtime**

In `WaveSender::run`, replace:

```rust
let observer = tokio::spawn(run_observer_loop(...));
```

With:

```rust
let observer = spawn_wave_observer_thread(
    self.started,
    self.metric_tx.clone(),
    Arc::clone(&self.response_in_flight),
    observer_command_rx,
    self.observer_tx.clone(),
    self.token.clone(),
);
```

At shutdown, replace `observer.abort()` / `observer.await` with:

```rust
drop(observer_command_tx);
observer.stop();
```

The observer loop already listens to cancellation and channel close.

- [ ] **Step 5: Update tests for observer runtime**

Add:

```rust
#[test]
fn observer_worker_count_uses_positive_env_value() {
    let previous_workers = std::env::var("RUNNER_WAVE_OBSERVER_WORKERS").ok();
    let previous_threads = std::env::var("RUNNER_WAVE_OBSERVER_THREADS").ok();
    unsafe {
        std::env::set_var("RUNNER_WAVE_OBSERVER_WORKERS", "3");
        std::env::remove_var("RUNNER_WAVE_OBSERVER_THREADS");
    }

    assert_eq!(observer_worker_count(), 3);

    unsafe {
        match previous_workers {
            Some(value) => std::env::set_var("RUNNER_WAVE_OBSERVER_WORKERS", value),
            None => std::env::remove_var("RUNNER_WAVE_OBSERVER_WORKERS"),
        }
        match previous_threads {
            Some(value) => std::env::set_var("RUNNER_WAVE_OBSERVER_THREADS", value),
            None => std::env::remove_var("RUNNER_WAVE_OBSERVER_THREADS"),
        }
    }
}
```

If existing sender env tests use a mutex, reuse that mutex or create a second static mutex to avoid concurrent env mutation.

- [ ] **Step 6: Run runner tests**

Run:

```bash
cargo test -p previa-runner
```

Expected: all runner tests pass.

---

### Task 5: Validate Metrics Semantics

**Files:**
- Modify if needed: `runner/src/server/metrics.rs`
- Modify if needed: `runner/src/server/wave_metrics_actor.rs`
- Modify if needed: `main/src/server/execution/load_batch.rs`

- [ ] **Step 1: Confirm no schema rename is needed**

Run:

```bash
rg -n "httpStarted|sendStarted|senderLaggedStarts|curveAdherence" runner/src main/src app/src
```

Expected:

- `curveAdherence` remains based on `sendStarted`.
- UI still displays `Starts atrasados no sender`.
- No label says "drops" or "descartes".

- [ ] **Step 2: Add or update metric test if needed**

If no existing test covers `sendStarted` vs `httpStarted`, add this to `runner/src/server/metrics.rs`:

```rust
#[test]
fn lifecycle_can_show_http_started_after_send_started() {
    let mut metrics = MetricsAccumulator::new();

    metrics.record_planned_at(1_000, 1);
    metrics.record_send_started_at(1_000);
    metrics.record_http_start_at(2_000);

    let snapshot = metrics.snapshot(None, None);

    assert_eq!(snapshot.lifecycle_buckets.len(), 2);
    assert_eq!(snapshot.lifecycle_buckets[0].elapsed_ms, 1_000);
    assert_eq!(snapshot.lifecycle_buckets[0].send_started, 1);
    assert_eq!(snapshot.lifecycle_buckets[0].http_started, 0);
    assert_eq!(snapshot.lifecycle_buckets[1].elapsed_ms, 2_000);
    assert_eq!(snapshot.lifecycle_buckets[1].send_started, 0);
    assert_eq!(snapshot.lifecycle_buckets[1].http_started, 1);
}
```

- [ ] **Step 3: Run metrics-related tests**

Run:

```bash
cargo test -p previa-runner metrics wave_metrics_actor
cargo test -p previa-main load_batch
```

Expected: all selected tests pass.

---

### Task 6: Full Verification and Local Stack Restart

**Files:**
- No source files changed.

- [ ] **Step 1: Run backend tests**

Run:

```bash
cargo test -p previa-engine
cargo test -p previa-runner
cargo test -p previa-main
```

Expected: all tests pass.

- [ ] **Step 2: Run frontend verification**

Run:

```bash
npm --prefix app test -- LoadTestResultsPanel remote-executor api-client
cd app && ./node_modules/.bin/tsc --noEmit
npm --prefix app run build
```

Expected: all commands pass. Existing Vite chunk-size warnings are acceptable.

- [ ] **Step 3: Run release build**

Run:

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] **Step 4: Restart local stack with new binaries**

Run:

```bash
for port in 5610 5611 5612 5613; do
  lsof -ti tcp:$port | xargs -r kill
done
sleep 1
cd /Users/assis/projects/previa
screen -S previa-wave -X quit || true
screen -dmS previa-wave zsh -lc '
  cd /Users/assis/projects/previa
  RUST_LOG=info PORT=5611 target/release/previa-runner > /tmp/previa-runner-5611.log 2>&1 &
  RUST_LOG=info PORT=5612 target/release/previa-runner > /tmp/previa-runner-5612.log 2>&1 &
  RUST_LOG=info PORT=5613 target/release/previa-runner > /tmp/previa-runner-5613.log 2>&1 &
  RUST_LOG=info PREVIA_APP_ENABLED=1 ORCHESTRATOR_DATABASE_URL=sqlite:///private/tmp/previa-verify-5610.db PORT=5610 RUNNER_ENDPOINTS=http://127.0.0.1:5611,http://127.0.0.1:5612,http://127.0.0.1:5613 target/release/previa-main > /tmp/previa-main-5610.log 2>&1
'
sleep 2
curl -s http://127.0.0.1:5610/info | jq '{activeRunners,totalRunners,runners:[.runners[].runtime.pid]}'
```

Expected:

```json
{
  "activeRunners": 3,
  "totalRunners": 3
}
```

- [ ] **Step 5: Run the same wave test from UI**

Open:

```text
http://127.0.0.1:5610/projects/019de1a7-4dfd-7662-8b53-a305e5714ca5/pipeline/019de1a7-4dfd-7662-8b53-a317b9bdbe23/load-test
```

Use:

```text
0s -> 10%
59.4s -> 49%
120s -> 80%
interpolation: Step
```

- [ ] **Step 6: Analyze latest result**

Run:

```bash
sqlite3 /private/tmp/previa-verify-5610.db \
  "select final_consolidated_json from load_history order by finished_at_ms desc limit 1" \
  | jq '{scheduledStarts,sendStarted,httpStarted,httpSendReturned,responseBodyCompleted,senderLaggedStarts,readyRequests,outstandingRequests,activePipelines,curveAdherence,totalSent,totalSuccess,totalError,avgLatency,p95,p99}'
```

Expected:

- `scheduledStarts == sendStarted`
- `httpStarted` should be much closer to `sendStarted` per second than before.
- `senderLaggedStarts` should be materially lower than the previous `59072` for the same wave on the same machine.
- `readyRequests` should be lower than the previous `7893`.
- `curveAdherence` should be higher than the previous `70.87%`.

- [ ] **Step 7: Analyze 10-second lifecycle windows**

Run:

```bash
sqlite3 /private/tmp/previa-verify-5610.db \
  "select final_consolidated_json from load_history order by finished_at_ms desc limit 1" \
  | jq -r '
    [.lifecycleBuckets[] | {bin: ((.elapsedMs/10000)|floor*10), planned:(.planned//0), sendStarted:(.sendStarted//0), httpStarted:(.httpStarted//0), sendReturned:(.httpSendReturned//0), body:(.responseBodyCompleted//0), late:(.senderLagged//0)}] |
    group_by(.bin)[] |
    {sec:.[0].bin, planned:map(.planned)|add, sendStarted:map(.sendStarted)|add, httpStarted:map(.httpStarted)|add, sendReturned:map(.sendReturned)|add, body:map(.body)|add, late:map(.late)|add} |
    [.sec,.planned,.sendStarted,.httpStarted,.sendReturned,.body,.late] | @tsv'
```

Expected:

- `sendStarted` tracks `planned` per 10-second window.
- `httpStarted` tracks `planned` per 10-second window unless the machine/runtime is saturated.
- Any remaining lag is shown as `late`, not hidden as dropped starts.

---

### Task 7: Commit and Push

**Files:**
- All modified files from previous tasks.

- [ ] **Step 1: Review diff**

Run:

```bash
git status --short
git diff --stat
git diff -- engine/src/execution/http_step.rs runner/src/server/wave_sender.rs
```

Expected: diff only contains the split HTTP fire/observer architecture, tests, and required exports.

- [ ] **Step 2: Commit**

Run:

```bash
git add engine/src/execution/http_step.rs engine/src/execution/mod.rs engine/src/lib.rs runner/src/server/wave_sender.rs runner/src/server/metrics.rs runner/src/server/wave_metrics_actor.rs main/src/server/execution/load_batch.rs docs/superpowers/plans/2026-05-06-wave-isolated-http-fire-runtime.md
git commit -m "Isolate wave HTTP fire path from observation"
```

If some listed files were not modified, remove them from `git add`.

- [ ] **Step 3: Push**

Run:

```bash
git push origin codex/wave-load-test
```

Expected: branch pushes successfully.

---

## Self-Review

- Spec coverage: The plan addresses the current evidence: total starts are preserved, but timing slips because fire and observation still compete. It moves real HTTP start to fire tasks and body/assertion work to a dedicated observer runtime.
- Placeholder scan: No TODO/TBD placeholders remain.
- Type consistency: `StartedHttpStep`, `start_prepared_http_step_with_hooks`, `complete_started_http_step_with_hook`, `ObserverCommand::Started`, and `ObserverCommand::StartError` are named consistently through the plan.
- Risk note: If `reqwest::Response` does not satisfy the required `Send + 'static` bounds when crossing the channel, fall back to keeping response body completion inside the HTTP start task but move those tasks onto a separate fire runtime from the sender channel readers. The preferred design should be attempted first because it creates the cleanest separation.
