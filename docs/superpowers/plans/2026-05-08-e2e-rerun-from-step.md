# E2E Rerun From Step Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Rerun from here" action on E2E steps that restarts execution at the selected step while reusing all previously generated step results as template context.

**Architecture:** The frontend sends the selected `stepId` plus prior `StepExecutionResult` values to the orchestrator. The orchestrator validates the request, seeds E2E history with the prior step results, forwards a suffix execution request to the runner, and streams only newly executed step events back. The runner executes the original pipeline from `stepId` onward with an initial context map populated from prior results.

**Tech Stack:** Rust/Axum/SQLx orchestrator, `previa_runner` execution engine, SSE streaming, React/Vite/Zustand frontend, Vitest and Cargo tests.

---

## File Structure

- Modify `engine/src/execution/engine.rs`: add a public execution entrypoint that accepts `start_step_id` and an initial `HashMap<String, StepExecutionResult>`.
- Modify `engine/src/execution/mod.rs`: re-export the new engine entrypoint.
- Modify `runner/src/server/handlers/e2e.rs`: add the runner request model and route handler for suffix E2E execution.
- Modify `runner/src/server/mod.rs` or the runner router file that wires `/api/v1/tests/e2e`: register `/api/v1/tests/e2e/rerun-from-step`.
- Modify `main/src/server/models.rs`: add project/orchestrator request models for rerun-from-step.
- Create `main/src/server/services/e2e_rerun.rs`: validation and payload-building helpers; keeps rerun business rules outside routes.
- Modify `main/src/server/services/mod.rs`: export the service module.
- Modify `main/src/server/handlers/tests_e2e.rs`: add the project-scoped rerun handler.
- Modify `main/src/server/mod.rs`: register `/api/v1/projects/{projectId}/tests/e2e/rerun-from-step`.
- Modify `main/src/server/docs.rs`: include the new endpoint/model in OpenAPI generation.
- Modify `app/src/lib/api-client.ts`: add a typed `rerunE2eFromStep` client call.
- Modify `app/src/lib/remote-executor.ts`: add an SSE runner for rerun-from-step using the same event parser as normal E2E.
- Modify `app/src/stores/useExecutionHistoryStore.ts`: add `rerunFromStep`, preserve previous results before the selected step, and update suffix results from SSE.
- Modify `app/src/components/StepResultCard.tsx`: add the rerun button prop and render the action.
- Modify `app/src/pages/TestExecutionPage.tsx`: wire the rerun callback into every step card.
- Add or modify tests near the touched files:
  - `engine/src/execution/engine.rs` unit tests
  - `runner/src/server/handlers/e2e.rs` tests
  - `main/src/server/handlers/tests_e2e.rs` tests
  - `app/src/stores/useExecutionHistoryStore.test.ts` or nearest store test
  - `app/src/components/StepResultCard.test.tsx`

---

### Task 1: Engine Supports Suffix Execution With Seeded Context

**Files:**
- Modify: `engine/src/execution/engine.rs`
- Modify: `engine/src/execution/mod.rs`

- [ ] **Step 1: Write the failing engine test**

Add a test in `engine/src/execution/engine.rs` showing that a suffix execution can resolve `{{steps.login.response.body.token}}` from a prior result without executing the `login` step again.

```rust
#[tokio::test]
async fn executes_from_step_with_seeded_previous_results() {
    use crate::core::types::{Pipeline, PipelineStep, StepExecutionResult, StepRequest, StepResponse};
    use serde_json::json;
    use std::collections::HashMap;

    let server = httpmock::MockServer::start_async().await;
    let protected = server.mock_async(|when, then| {
        when.method("GET")
            .path("/protected")
            .header("authorization", "Bearer abc123");
        then.status(200).json_body(json!({ "ok": true }));
    }).await;

    let pipeline = Pipeline {
        id: Some("pipe-1".to_owned()),
        name: "Pipe".to_owned(),
        description: String::new(),
        steps: vec![
            PipelineStep {
                id: "login".to_owned(),
                name: "Login".to_owned(),
                description: String::new(),
                method: "POST".to_owned(),
                url: format!("{}/login", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                asserts: Vec::new(),
                delay: None,
                retry: None,
            },
            PipelineStep {
                id: "protected".to_owned(),
                name: "Protected".to_owned(),
                description: String::new(),
                method: "GET".to_owned(),
                url: format!("{}/protected", server.base_url()),
                headers: HashMap::from([(
                    "Authorization".to_owned(),
                    "Bearer {{steps.login.response.body.token}}".to_owned(),
                )]),
                body: None,
                operation_id: None,
                asserts: Vec::new(),
                delay: None,
                retry: None,
            },
        ],
    };

    let seeded = HashMap::from([(
        "login".to_owned(),
        StepExecutionResult {
            step_id: "login".to_owned(),
            status: "success".to_owned(),
            request: Some(StepRequest {
                method: "POST".to_owned(),
                url: format!("{}/login", server.base_url()),
                headers: HashMap::new(),
                body: None,
            }),
            response: Some(StepResponse {
                status: 200,
                status_text: "OK".to_owned(),
                headers: HashMap::new(),
                body: json!({ "token": "abc123" }),
            }),
            error: None,
            duration: Some(1),
            attempts: Some(1),
            attempt: Some(1),
            max_attempts: Some(1),
            assert_results: None,
        },
    )]);

    let mut started = Vec::new();
    let results = execute_pipeline_from_step_with_client_runtime_hooks(
        &reqwest::Client::new(),
        &pipeline,
        "protected",
        seeded,
        None,
        None,
        None,
        |step_id| started.push(step_id.to_owned()),
        |_| {},
        || false,
        |_| Box::pin(async { true }),
    ).await;

    assert_eq!(started, vec!["protected"]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].step_id, "protected");
    assert_eq!(results[0].status, "success");
    protected.assert_async().await;
}
```

- [ ] **Step 2: Run the engine test to verify RED**

Run:

```bash
cargo test -p previa-runner executes_from_step_with_seeded_previous_results
```

Expected: compile failure because `execute_pipeline_from_step_with_client_runtime_hooks` does not exist.

- [ ] **Step 3: Implement the minimal engine entrypoint**

In `engine/src/execution/engine.rs`, extract the existing loop internals so both normal and suffix executions use the same request preparation/sending path. Add this public function:

```rust
pub async fn execute_pipeline_from_step_with_client_runtime_hooks<FStart, FResult, FCancel, FGate>(
    client: &Client,
    pipeline: &Pipeline,
    start_step_id: &str,
    initial_context: HashMap<String, StepExecutionResult>,
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
    FGate: for<'a> FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> + Send,
{
    execute_pipeline_with_client_runtime_hooks_from_index(
        client,
        pipeline,
        pipeline
            .steps
            .iter()
            .position(|step| step.id == start_step_id)
            .unwrap_or(pipeline.steps.len()),
        initial_context,
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

Create a private helper that accepts `start_index` and `initial_context`, initializes `let mut context = initial_context;`, and loops over `pipeline.steps.iter().skip(start_index)`. Update the existing normal path to call this helper with `start_index = 0` and `HashMap::new()`.

- [ ] **Step 4: Re-export the new entrypoint**

In `engine/src/execution/mod.rs`, add:

```rust
execute_pipeline_from_step_with_client_runtime_hooks,
```

to the `pub use engine::{ ... }` list.

- [ ] **Step 5: Run the engine test to verify GREEN**

Run:

```bash
cargo test -p previa-runner executes_from_step_with_seeded_previous_results
```

Expected: PASS.

---

### Task 2: Runner Endpoint Executes From Step

**Files:**
- Modify: `runner/src/server/handlers/e2e.rs`
- Modify: runner router file that currently registers `/api/v1/tests/e2e`

- [ ] **Step 1: Write the failing runner test**

Add a test in `runner/src/server/handlers/e2e.rs` that posts to `/api/v1/tests/e2e/rerun-from-step` with a two-step pipeline, a prior `login` result, and `startStepId: "protected"`. Assert the SSE stream contains `step:start` and `step:result` for `protected`, but not for `login`.

```rust
#[tokio::test]
async fn rerun_from_step_streams_only_suffix_steps_with_prior_context() {
    let app = test_app();
    let payload = json!({
        "pipeline": pipeline_requiring_login_context(),
        "startStepId": "protected",
        "priorResults": {
            "login": successful_login_result("abc123")
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/tests/e2e/rerun-from-step")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("\"stepId\":\"login\""));
    assert!(text.contains("event: step:start"));
    assert!(text.contains("\"stepId\":\"protected\""));
    assert!(text.contains("event: pipeline:complete"));
}
```

- [ ] **Step 2: Run the runner test to verify RED**

Run:

```bash
cargo test -p previa-runner rerun_from_step_streams_only_suffix_steps_with_prior_context
```

Expected: FAIL with 404 or compile failure because the route/model does not exist.

- [ ] **Step 3: Add the runner request model**

In `runner/src/server/handlers/e2e.rs`, add:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct E2eRerunFromStepRequest {
    pub pipeline: Pipeline,
    pub start_step_id: String,
    #[serde(default)]
    pub prior_results: HashMap<String, StepExecutionResult>,
    pub selected_env_group_slug: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}
```

- [ ] **Step 4: Add the runner handler**

Add a handler that validates `start_step_id`, calls `execute_pipeline_from_step_with_client_runtime_hooks`, and emits the same SSE events as the full E2E handler:

```rust
pub async fn rerun_e2e_from_step(
    State(state): State<RunnerState>,
    payload: Result<Json<E2eRerunFromStepRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };
    if !payload.pipeline.steps.iter().any(|step| step.id == payload.start_step_id) {
        return bad_request_message_response("startStepId not found in pipeline");
    }

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(async move {
        let results = execute_pipeline_from_step_with_client_runtime_hooks(
            &state.client,
            &payload.pipeline,
            &payload.start_step_id,
            payload.prior_results,
            Some(payload.specs.as_slice()),
            Some(payload.env_groups.as_slice()),
            payload.selected_env_group_slug.as_deref(),
            |step_id| {
                let _ = send_sse_or_cancel(&tx, "step:start", json!({ "stepId": step_id }), &CancellationToken::new());
            },
            |result| {
                let _ = send_sse_or_cancel(&tx, "step:result", serde_json::to_value(result).unwrap_or(Value::Null), &CancellationToken::new());
            },
            || false,
            |_| Box::pin(async { true }),
        ).await;
        let failed = results.iter().filter(|result| result.status == "error").count();
        let total_duration = results.iter().filter_map(|result| result.duration).sum::<u128>();
        let _ = send_sse_or_cancel(
            &tx,
            "pipeline:complete",
            json!({
                "totalSteps": results.len(),
                "passed": results.len().saturating_sub(failed),
                "failed": failed,
                "totalDuration": total_duration
            }),
            &CancellationToken::new(),
        );
    });

    Sse::new(ReceiverStream::new(rx)).into_response()
}
```

Adjust names to match the actual runner state type and helper signatures in `runner/src/server/handlers/e2e.rs`; do not duplicate transport helpers if the file already exposes them.

- [ ] **Step 5: Register the runner route**

In the runner router file, add:

```rust
.route("/api/v1/tests/e2e/rerun-from-step", post(rerun_e2e_from_step))
```

- [ ] **Step 6: Run the runner test to verify GREEN**

Run:

```bash
cargo test -p previa-runner rerun_from_step_streams_only_suffix_steps_with_prior_context
```

Expected: PASS.

---

### Task 3: Orchestrator Validates And Forwards Rerun Requests

**Files:**
- Modify: `main/src/server/models.rs`
- Create: `main/src/server/services/e2e_rerun.rs`
- Modify: `main/src/server/services/mod.rs`
- Modify: `main/src/server/handlers/tests_e2e.rs`
- Modify: `main/src/server/mod.rs`
- Modify: `main/src/server/docs.rs`

- [ ] **Step 1: Write the failing orchestrator route test**

Add a test in `main/src/server/handlers/tests_e2e.rs` that posts to `/api/v1/projects/project-1/tests/e2e/rerun-from-step` and verifies the orchestrator forwards `startStepId` and `priorResults` to the runner route.

```rust
#[tokio::test]
async fn post_e2e_rerun_from_step_forwards_start_step_and_prior_results() {
    let (runner_url, received, _runner_task) = spawn_rerun_runner_server().await;
    let app = test_app(runner_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/projects/project-1/tests/e2e/rerun-from-step")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "pipelineId": "pipe-1",
                    "startStepId": "protected",
                    "priorResults": {
                        "login": successful_login_result("abc123")
                    }
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let forwarded = received.lock().await.clone().expect("forwarded payload");
    assert_eq!(forwarded["startStepId"], "protected");
    assert_eq!(forwarded["priorResults"]["login"]["response"]["body"]["token"], "abc123");
}
```

- [ ] **Step 2: Run the orchestrator test to verify RED**

Run:

```bash
cargo test -p previa-main post_e2e_rerun_from_step_forwards_start_step_and_prior_results
```

Expected: FAIL with 404 or compile failure because the endpoint/model does not exist.

- [ ] **Step 3: Add request models**

In `main/src/server/models.rs`, add:

```rust
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectE2eRerunFromStepRequest {
    pub pipeline_id: Option<String>,
    pub pipeline: Option<Pipeline>,
    pub start_step_id: String,
    #[serde(default)]
    pub prior_results: HashMap<String, previa_runner::StepExecutionResult>,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eRerunFromStepRunnerRequest {
    pub pipeline: Pipeline,
    pub start_step_id: String,
    pub prior_results: HashMap<String, previa_runner::StepExecutionResult>,
    pub selected_env_group_slug: Option<String>,
    pub specs: Vec<RuntimeSpec>,
    pub env_groups: Vec<RuntimeEnvGroup>,
}
```

- [ ] **Step 4: Add service validation**

Create `main/src/server/services/e2e_rerun.rs`:

```rust
use std::collections::HashMap;

use previa_runner::{Pipeline, StepExecutionResult};

pub fn validate_rerun_context(
    pipeline: &Pipeline,
    start_step_id: &str,
    prior_results: &HashMap<String, StepExecutionResult>,
) -> Result<(), String> {
    let start_index = pipeline
        .steps
        .iter()
        .position(|step| step.id == start_step_id)
        .ok_or_else(|| "startStepId not found in pipeline".to_owned())?;

    for step in pipeline.steps.iter().take(start_index) {
        match prior_results.get(&step.id) {
            Some(result) if result.status != "pending" && result.status != "running" => {}
            Some(_) => {
                return Err(format!(
                    "prior result for step '{}' is not completed",
                    step.id
                ));
            }
            None => {
                return Err(format!(
                    "prior result for step '{}' is required",
                    step.id
                ));
            }
        }
    }

    Ok(())
}
```

Export it in `main/src/server/services/mod.rs`:

```rust
pub mod e2e_rerun;
```

- [ ] **Step 5: Add the project route handler**

In `main/src/server/handlers/tests_e2e.rs`, add `run_e2e_rerun_from_step_for_project`. It should mirror `run_e2e_test_for_project`: load pipeline by `pipelineId`, resolve runtime specs/env groups, validate the prior results, seed the history accumulator with `priorResults` before forwarding, and call `forward_runner_stream` with endpoint path `/api/v1/tests/e2e/rerun-from-step`.

Use the existing scheduling and history logic from `start_e2e_execution`; if duplication grows beyond roughly one screen, extract shared orchestration into a parameterized helper in `main/src/server/execution/e2e.rs`:

```rust
enum E2eExecutionMode {
    Full,
    RerunFromStep {
        start_step_id: String,
        prior_results: HashMap<String, StepExecutionResult>,
    },
}
```

For history seeding, convert prior results to JSON values in pipeline order:

```rust
let seeded_steps = pipeline
    .steps
    .iter()
    .take_while(|step| step.id != start_step_id)
    .filter_map(|step| prior_results.get(&step.id))
    .filter_map(|result| serde_json::to_value(result).ok())
    .collect::<Vec<_>>();
```

Initialize `E2eHistoryAccumulator { steps: seeded_steps, summary: None, errors: Vec::new() }` before forwarding so snapshots and final history contain the complete combined run.

- [ ] **Step 6: Register the orchestrator route**

In `main/src/server/mod.rs`, add the route before `/api/v1/projects/{projectId}/tests/e2e/{test_id}` so it is not captured as a test id:

```rust
.route(
    "/api/v1/projects/{projectId}/tests/e2e/rerun-from-step",
    post(run_e2e_rerun_from_step_for_project),
)
```

- [ ] **Step 7: Update OpenAPI docs**

In `main/src/server/docs.rs`, add the new path and schemas:

```rust
crate::server::handlers::tests_e2e::run_e2e_rerun_from_step_for_project,
ProjectE2eRerunFromStepRequest,
E2eRerunFromStepRunnerRequest,
```

- [ ] **Step 8: Run orchestrator tests to verify GREEN**

Run:

```bash
cargo test -p previa-main post_e2e_rerun_from_step_forwards_start_step_and_prior_results
```

Expected: PASS.

---

### Task 4: Frontend API And Store Rerun Action

**Files:**
- Modify: `app/src/lib/api-client.ts`
- Modify: `app/src/lib/remote-executor.ts`
- Modify: `app/src/stores/useExecutionHistoryStore.ts`
- Test: `app/src/stores/useExecutionHistoryStore.test.ts`

- [ ] **Step 1: Write the failing store test**

Add a Vitest test that seeds results for `login` and `protected`, calls `rerunFromStep("protected")`, and asserts the store keeps `login` unchanged while replacing `protected` with streamed results.

```ts
it("reruns from the selected step while preserving previous step results", async () => {
  const loginResult = {
    stepId: "login",
    status: "success" as const,
    response: { status: 200, statusText: "OK", headers: {}, body: { token: "abc123" } },
  };
  const protectedResult = { stepId: "protected", status: "success" as const };

  useExecutionHistoryStore.setState({
    results: { login: loginResult, protected: protectedResult },
    running: false,
  });

  mockRunRemoteIntegrationFromStep.mockImplementation((_backend, _pipeline, startStepId, priorResults, callbacks) => {
    expect(startStepId).toBe("protected");
    expect(priorResults.login.response.body.token).toBe("abc123");
    callbacks.onStepStart("protected");
    callbacks.onStepResult("protected", {
      stepId: "protected",
      status: "success",
      response: { status: 200, statusText: "OK", headers: {}, body: { ok: true } },
    });
    callbacks.onComplete({ totalSteps: 1, passed: 1, failed: 0, totalDuration: 4 });
    return { cancel: vi.fn(), disconnect: vi.fn() };
  });

  await useExecutionHistoryStore.getState().rerunFromStep(
    pipeline,
    0,
    "project-1",
    "protected",
    "http://127.0.0.1:5610",
    [],
    [],
    "local",
  );

  const results = useExecutionHistoryStore.getState().results;
  expect(results.login).toEqual(loginResult);
  expect(results.protected.response?.body).toEqual({ ok: true });
});
```

- [ ] **Step 2: Run the store test to verify RED**

Run:

```bash
cd app && npm test -- useExecutionHistoryStore.test.ts
```

Expected: compile failure because `rerunFromStep` and the remote helper do not exist.

- [ ] **Step 3: Add the API client type**

In `app/src/lib/api-client.ts`, add:

```ts
export interface ProjectE2eRerunFromStepRequest {
  pipelineId?: string;
  pipeline?: Pipeline;
  startStepId: string;
  priorResults: Record<string, StepExecutionResult>;
  selectedBaseUrlKey?: string | null;
  selectedEnvGroupSlug?: string | null;
  pipelineIndex?: number;
  specs?: Array<{ slug: string; servers: Record<string, string> }>;
  envGroups?: RuntimeEnvGroup[];
}
```

- [ ] **Step 4: Add the remote SSE helper**

In `app/src/lib/remote-executor.ts`, add a `runRemoteIntegrationFromStep` function with the same callback contract as `runRemoteIntegrationTest`. It posts to:

```ts
const basePath = `${ensureApiPrefix(backendUrl)}/projects/${projectId}/tests/e2e/rerun-from-step`;
```

with body:

```ts
const body = {
  pipelineId: pipeline.id,
  startStepId,
  priorResults,
  selectedBaseUrlKey,
  selectedEnvGroupSlug,
  pipelineIndex,
  specs,
  envGroups,
};
```

Reuse the existing SSE parser and `dispatchIntegrationEvent` path so the store receives normal `step:start`, `step:result`, `execution:init`, and `pipeline:complete` events.

- [ ] **Step 5: Add store action**

In `app/src/stores/useExecutionHistoryStore.ts`, extend the interface:

```ts
rerunFromStep: (
  pipeline: Pipeline,
  pipelineIndex: number,
  projectId: string,
  startStepId: string,
  executionBackendUrl?: string,
  specs?: import("@/types/project").ProjectSpec[],
  envGroups?: import("@/types/project").ProjectEnvGroup[],
  selectedEnvGroupSlug?: string | null
) => Promise<"success" | "error">;
```

Implementation rules:

```ts
const startIndex = pipeline.steps.findIndex((step) => step.id === startStepId);
if (startIndex < 0) {
  toast.error("Step not found");
  return "error";
}
const currentResults = get().results;
const priorResults = Object.fromEntries(
  pipeline.steps
    .slice(0, startIndex)
    .filter((step) => {
      const status = currentResults[step.id]?.status;
      return status && status !== "pending" && status !== "running";
    })
    .map((step) => [step.id, currentResults[step.id]])
);
if (Object.keys(priorResults).length !== startIndex) {
  toast.error("Run the previous steps before rerunning from here");
  return "error";
}
```

Then initialize only suffix steps as `pending`, preserve prefix results as-is, stream events through `runRemoteIntegrationFromStep`, reload history at the end, and update `latestStatuses[pipelineIndex]`.

- [ ] **Step 6: Run store test to verify GREEN**

Run:

```bash
cd app && npm test -- useExecutionHistoryStore.test.ts
```

Expected: PASS.

---

### Task 5: Step UI Button And Page Wiring

**Files:**
- Modify: `app/src/components/StepResultCard.tsx`
- Modify: `app/src/pages/TestExecutionPage.tsx`
- Test: `app/src/components/StepResultCard.test.tsx`

- [ ] **Step 1: Write the failing component test**

Create or update `app/src/components/StepResultCard.test.tsx`:

```tsx
it("shows a rerun-from-step button and calls the handler", async () => {
  const onRerunFromStep = vi.fn();
  render(
    <StepResultCard
      step={step}
      result={{ stepId: step.id, status: "success" }}
      onRerunFromStep={onRerunFromStep}
    />
  );

  await userEvent.click(screen.getByRole("button", { name: /rerun from here/i }));
  expect(onRerunFromStep).toHaveBeenCalledWith(step.id);
});
```

- [ ] **Step 2: Run component test to verify RED**

Run:

```bash
cd app && npm test -- StepResultCard.test.tsx
```

Expected: compile failure because `onRerunFromStep` is not a prop.

- [ ] **Step 3: Add the card prop and buttons**

In `StepResultCardProps`, add:

```ts
onRerunFromStep?: (stepId: string) => void;
canRerunFromStep?: boolean;
```

Render a compact icon button in both list and grid variants near the existing `Code` action:

```tsx
{onRerunFromStep && (
  <TooltipProvider>
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-6 w-6 shrink-0"
          disabled={canRerunFromStep === false}
          aria-label="Rerun from here"
          onClick={(event) => {
            event.stopPropagation();
            event.preventDefault();
            onRerunFromStep(step.id);
          }}
        >
          <RotateCcw className="h-3.5 w-3.5" />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top" className="text-xs">Rerun from here</TooltipContent>
    </Tooltip>
  </TooltipProvider>
)}
```

- [ ] **Step 4: Wire page callback**

In `app/src/pages/TestExecutionPage.tsx`, add:

```ts
const handleRerunFromStep = useCallback(async (stepId: string) => {
  if (!selectedPipeline || selectedIndex === null) return;
  if (!executionBackendUrl) {
    toast.error(t("testExecution.configureServerUrl"));
    return;
  }
  await useExecutionHistoryStore.getState().rerunFromStep(
    selectedPipeline,
    selectedIndex,
    projectId,
    stepId,
    executionBackendUrl,
    specs,
    envGroups,
    effectiveSelectedEnvGroupSlug,
  );
  setChartRefreshKey((prev) => prev + 1);
}, [selectedPipeline, selectedIndex, executionBackendUrl, projectId, specs, envGroups, effectiveSelectedEnvGroupSlug, t]);
```

Pass it into every `StepResultCard` in mobile, graph, and list render branches:

```tsx
<StepResultCard
  step={step}
  result={results[step.id]}
  shouldCountdown={!!shouldCountdown}
  onAnalyzeWithAI={onAnalyzeStepWithAI}
  onGoToCode={onEditPipeline ? handleGoToCode : undefined}
  onRerunFromStep={handleRerunFromStep}
  canRerunFromStep={!running && !isBatchActive && !!executionBackendUrl}
/>
```

- [ ] **Step 5: Run component/page tests**

Run:

```bash
cd app && npm test -- StepResultCard.test.tsx TestExecutionPage
```

Expected: PASS.

---

### Task 6: Integration Verification And Release Checks

**Files:**
- No new files unless tests expose a targeted fix.

- [ ] **Step 1: Run frontend tests**

Run:

```bash
cd app && npm test
```

Expected: PASS.

- [ ] **Step 2: Run Rust tests**

Run:

```bash
cargo test
```

Expected: PASS.

- [ ] **Step 3: Run frontend build**

Run:

```bash
cd app && npm run build
```

Expected: PASS.

- [ ] **Step 4: Run required release build**

Run from repo root:

```bash
cargo build --release
```

Expected: PASS.

- [ ] **Step 5: Browser verification**

Open:

```text
http://127.0.0.1:5610/projects/019e0871-b9ca-79f2-801d-958364e1aefa/pipeline/019e0871-bac6-7771-b5a8-42d572ebd07f/integration-test
```

Manual checks:

- Run the full E2E test once.
- Click "Rerun from here" on the first step; the full pipeline should behave like a normal run.
- Click "Rerun from here" on a later step in a multi-step pipeline; prior cards stay populated and suffix cards move through pending/running/success or error.
- Confirm history records show the combined result set, not only the suffix.
- Confirm the button is disabled while a run or batch is active.

- [ ] **Step 6: Commit and push**

Run:

```bash
git status --short
git add engine/src/execution/engine.rs engine/src/execution/mod.rs runner/src/server/handlers/e2e.rs runner/src/server main/src/server/models.rs main/src/server/services/e2e_rerun.rs main/src/server/services/mod.rs main/src/server/handlers/tests_e2e.rs main/src/server/mod.rs main/src/server/docs.rs app/src/lib/api-client.ts app/src/lib/remote-executor.ts app/src/stores/useExecutionHistoryStore.ts app/src/components/StepResultCard.tsx app/src/pages/TestExecutionPage.tsx app/src/components/StepResultCard.test.tsx app/src/stores/useExecutionHistoryStore.test.ts
git commit -m "feat: rerun e2e from selected step"
git push
```

Expected: commit and push succeed. If some listed test files did not need changes, omit them from `git add`.

---

## Self-Review

- Spec coverage: The plan covers the selected-step button, reusing previously generated step results as context, suffix execution, SSE streaming, history preservation, UI wiring, and build verification.
- Placeholder scan: No placeholder markers remain. Steps name exact files, commands, expected results, and concrete interfaces.
- Type consistency: Frontend uses `startStepId`/`priorResults`; Rust request models use `start_step_id`/`prior_results` with `camelCase` serde. The same `StepExecutionResult` shape is used for context seeding in frontend, orchestrator, and runner.
