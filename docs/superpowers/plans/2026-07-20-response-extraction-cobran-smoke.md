# Response Extraction and Cobran SDX Smoke Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add reusable regular-expression extraction to Previa E2E steps, release it to SDX, and run a private end-to-end Cobran passwordless boleto smoke.

**Architecture:** The engine owns extraction contracts and evaluation, while the main API validates complete pipeline definitions and template references. Extracted values are stored on `StepExecutionResult` and exposed through a parallel `extracts.<stepId>.<name>` template root without changing existing `steps.*` behavior. The live smoke uses production Core preflights, SDX Cobran/Auth, and the cluster-internal Mailpit API.

**Tech Stack:** Rust 2024, serde, regex, reqwest, Axum/utoipa, TypeScript/Zod/React, PostgreSQL execution queue, Kubernetes, GitHub Actions.

## Global Constraints

- Follow red-green-refactor for every behavior change.
- Keep transport in handlers, reusable execution policy in the engine, and API contracts in models/types.
- Preserve all existing `{{steps.<stepId>.<fieldPath>}}` interpolation.
- Never log extracted values, OTPs, bearer tokens, customer email, or customer name.
- Keep the live project and pipeline private.
- Use `http://previa.sdx.autob`, `http://gateway.sdx.autob/v1/cobran`, `http://mailpit.auth.svc.cluster.local:8025`, production Core, company `7`, and the approved fixture only.
- A successful enqueue is not completion; inspect every terminal E2E step.
- Run `cargo build --release` after modifications, and push successful commits.

---

### Task 1: Engine extraction contracts and validation

**Files:**
- Modify: `engine/src/core/types.rs`
- Create: `engine/src/extractions.rs`
- Modify: `engine/src/lib.rs`
- Modify: Rust test literals containing `PipelineStep` and `StepExecutionResult`

**Interfaces:**
- Produces: `StepExtraction { name, field, regex, group, required }`.
- Produces: `validate_step_extractions(step: &PipelineStep) -> Vec<String>`.
- Produces: `evaluate_step_extractions(step: &PipelineStep, result: &StepExecutionResult) -> Result<HashMap<String, String>, String>`.
- Adds: `PipelineStep.extracts: Vec<StepExtraction>` with serde default.
- Adds: `StepExecutionResult.extracts: HashMap<String, String>` with serde default and empty-map omission.

- [ ] **Step 1: Write failing extraction contract tests**

Add unit tests in `engine/src/extractions.rs` that construct a response with
`body.HTML`, then assert:

```rust
assert_eq!(
    evaluate_step_extractions(&step, &result).unwrap().get("code"),
    Some(&"123456".to_owned())
);
```

Add separate tests for group `0`, a missing optional match, a missing required
match, invalid regex, duplicate names, invalid names, invalid source paths, and
a configured capture group outside `regex.captures_len()`.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test -p previa-engine extractions -- --nocapture`

Expected: compilation fails because `StepExtraction`, the fields, and the
evaluation functions do not exist.

- [ ] **Step 3: Add the minimal contracts and evaluator**

Add the contract to `engine/src/core/types.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StepExtraction {
    pub name: String,
    pub field: String,
    pub regex: String,
    #[serde(default = "default_extraction_group")]
    pub group: usize,
    #[serde(default = "default_extraction_required")]
    pub required: bool,
}
```

Add `extracts` to `PipelineStep` and `StepExecutionResult`. Implement a focused
`engine/src/extractions.rs` module that resolves `body` or `body.<path>`, turns
the source into a string, compiles the regex, and returns captures without ever
including captured content in an error.

Re-export `StepExtraction`, `evaluate_step_extractions`, and
`validate_step_extractions` from `engine/src/lib.rs`. Add empty `extracts`
values to existing Rust struct literals mechanically.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run: `cargo test -p previa-engine extractions -- --nocapture`

Expected: all extraction unit tests pass.

- [ ] **Step 5: Commit the contract slice**

```bash
git add engine runner main
git commit -m "feat(engine): add response extraction contracts"
```

### Task 2: Execution and template interpolation

**Files:**
- Modify: `engine/src/execution/engine.rs`
- Modify: `engine/src/execution/http_step.rs`
- Modify: `engine/src/template/resolve.rs`
- Modify: `engine/src/execution/logging.rs` only if tests show extracted values can enter logs

**Interfaces:**
- Consumes: `evaluate_step_extractions` and `StepExecutionResult.extracts` from Task 1.
- Produces: template resolution for `{{extracts.<stepId>.<name>}}`.
- Preserves: existing response-body context under `steps`.

- [ ] **Step 1: Write failing template-context tests**

In `engine/src/template/resolve.rs`, add a result with response body
`{"token":"existing"}` and extracts `{"code":"123456"}`. Assert both:

```rust
assert_eq!(rendered["old"], "existing");
assert_eq!(rendered["new"], "123456");
```

using `{{steps.login.token}}` and `{{extracts.login.code}}` respectively. Add a
scalar-response test proving `{{steps.login}}` remains unchanged.

- [ ] **Step 2: Run template tests and verify RED**

Run: `cargo test -p previa-engine template::resolve -- --nocapture`

Expected: the `extracts` template remains unresolved.

- [ ] **Step 3: Build the parallel extraction context**

Extend `build_template_context` to add:

```rust
root.insert("extracts".to_owned(), Value::Object(extracts_map));
```

where each step ID maps to its extraction string map. Do not modify the
existing `steps_map` construction.

- [ ] **Step 4: Run template tests and verify GREEN**

Run: `cargo test -p previa-engine template::resolve -- --nocapture`

Expected: old and new interpolation tests pass.

- [ ] **Step 5: Write failing HTTP execution tests**

Add one classic engine test and one prepared/queued HTTP-step test. Each server
returns an HTML or JSON string containing a code. Assert the result stores only
the configured capture and that a later request body receives that value. Add
a required-no-match execution test asserting the step status is `error`, the
next step is not called, and the error does not contain the response body.

- [ ] **Step 6: Run execution tests and verify RED**

Run: `cargo test -p previa-engine execution -- --nocapture`

Expected: results have no extracted value and the chained request is unresolved.

- [ ] **Step 7: Evaluate extractions before assertions in both paths**

In both `engine.rs` and `http_step.rs`, run extraction after response decoding
and before assertion evaluation. On success assign `result.extracts`; on a
required extraction error set the step to `error`, preserve the HTTP response,
skip assertions, and allow the existing retry/finalization logic to handle the
terminal attempt.

- [ ] **Step 8: Run execution and engine tests and verify GREEN**

Run: `cargo test -p previa-engine --lib -- --nocapture`

Expected: extraction, chaining, retry, assertion, and legacy template tests pass.

- [ ] **Step 9: Commit execution support**

```bash
git add engine
git commit -m "feat(engine): interpolate extracted response values"
```

### Task 3: Main API pipeline validation

**Files:**
- Modify: `main/src/server/validation/pipelines.rs`
- Modify: `main/src/server/services/pipeline_import.rs` tests if fixtures require the new field
- Modify: handler/MCP tests containing `PipelineStep` literals

**Interfaces:**
- Consumes: `validate_step_extractions` and each earlier step's declared extraction names.
- Produces: validation of `{{extracts.<earlierStep>.<name>}}` in URLs, headers, bodies, and assertion expected values.

- [ ] **Step 1: Write failing validation tests**

Add tests covering a valid prior extraction reference, unknown extraction
name, unknown step, forward reference, missing name segment, and invalid
extraction definition. Use a two-step pipeline where the second body contains:

```json
{"code":"{{extracts.email.code}}"}
```

- [ ] **Step 2: Run main validation tests and verify RED**

Run: `cargo test -p previa-main server::validation::pipelines -- --nocapture`

Expected: `extracts` is rejected as an unknown template root.

- [ ] **Step 3: Track extraction declarations during validation**

Maintain `HashMap<String, HashSet<String>>` for prior step extraction names.
Validate each step's extraction definitions before validating its templates,
but expose names to following steps only after the current step is complete.
Accept only `extracts.<stepId>.<name>` and return precise errors for malformed,
unknown, or forward references.

- [ ] **Step 4: Run validation tests and verify GREEN**

Run: `cargo test -p previa-main server::validation::pipelines -- --nocapture`

Expected: all old and new pipeline validation tests pass.

- [ ] **Step 5: Run import and handler regression tests**

Run: `cargo test -p previa-main pipeline -- --nocapture`

Expected: pipeline create, update, import, inline execution, and MCP validation tests pass.

- [ ] **Step 6: Commit API validation**

```bash
git add main
git commit -m "feat(main): validate response extraction templates"
```

### Task 4: OpenAPI, TypeScript contract, UI parsing, and guidance

**Files:**
- Modify: `main/src/server/docs.rs`
- Modify: `main/src/server/mcp/service.rs`
- Modify: `app/src/types/pipeline.ts`
- Modify: `app/src/lib/pipeline-schema.ts`
- Modify: `app/src/lib/api-client.ts` if explicit mapping omits extractions
- Test: relevant Rust docs tests and app schema/client tests

**Interfaces:**
- Consumes: `StepExtraction`, `PipelineStep.extracts`, and `StepExecutionResult.extracts`.
- Produces: generated OpenAPI and TypeScript contracts that round-trip extraction definitions and results.

- [ ] **Step 1: Write failing TypeScript schema/client tests**

Add a pipeline fixture with:

```ts
extracts: [{
  name: "code",
  field: "body.HTML",
  regex: "<strong>([0-9]{6})</strong>",
  group: 1,
  required: true,
}]
```

Assert the Zod parser and API mapper preserve every field.

- [ ] **Step 2: Run app tests and verify RED**

Run: `npm test -- --run`

Expected: extraction data is stripped or rejected by the current client schema.

- [ ] **Step 3: Synchronize public contracts**

Add `StepExtraction` and result extraction types to TypeScript, extend the Zod
schema, preserve extracts in API mappings, register the Rust schema in
`docs.rs`, and update the pipeline creation guide with the exact
`{{extracts.<stepId>.<name>}}` syntax and ordering rule.

- [ ] **Step 4: Run contract checks and verify GREEN**

Run:

```bash
cargo test -p previa-main server::docs
python3 scripts/check_openapi_client_contract.py
npm test -- --run
```

Expected: all commands exit `0`.

- [ ] **Step 5: Commit contract synchronization**

```bash
git add main app
git commit -m "feat: expose response extractions in pipeline contracts"
```

### Task 5: Repository verification and release publication

**Files:**
- Modify: release metadata/version files only through the repository's existing release workflow if required
- Inspect: `.github/workflows/release.yaml`
- Inspect: deployment source referenced by the SDX Argo application

**Interfaces:**
- Consumes: all implementation commits.
- Produces: a published main/runner image pair containing the same extraction contract version.

- [ ] **Step 1: Run formatting and focused checks**

Run:

```bash
cargo fmt --all -- --check
cargo test -p previa-engine --lib
cargo test -p previa-main server::docs
python3 scripts/check_openapi_client_contract.py
npm test -- --run
```

Expected: all commands exit `0`.

- [ ] **Step 2: Run complete verification**

Run:

```bash
cargo test
cargo build --release
```

Expected: all test suites pass and the release workspace build exits `0`.

- [ ] **Step 3: Review the final diff and commit residual formatting**

Run `git diff --check`, inspect `git diff origin/main...HEAD`, and commit only
intentional remaining changes with a specific message.

- [ ] **Step 4: Push the implementation branch**

Run: `git push origin codex/cobran-sdx-e2e-extraction`

Expected: the remote branch advances to the verified implementation commit.

- [ ] **Step 5: Publish using the repository release workflow**

Use the existing release mechanism without inventing a parallel image build.
Monitor the GitHub Actions run until the matching `previa-main` and
`previa-runner` images are present in GitHub Packages.

### Task 6: SDX rollout and live Cobran smoke

**Files:**
- Modify: the SDX deployment source that pins `previa-main` and `previa-runner`
- Create through live API: private Previa project and pipeline

**Interfaces:**
- Consumes: the published image version from Task 5.
- Produces: a terminal live E2E execution with step-by-step evidence.

- [ ] **Step 1: Update and synchronize SDX**

Pin both Previa workloads to the same published version, commit and push the
deployment change, synchronize the Argo application, and wait for healthy
rollouts.

- [ ] **Step 2: Verify the live version and runner gate**

Check deployment images, `GET http://previa.sdx.autob/health`, OpenAPI version,
and `GET /api/v1/queue/diagnostics`. Continue only with at least one ready
runner and no rollout degradation.

- [ ] **Step 3: Create or update the private smoke project**

Use `POST /api/v1/projects` and
`POST /api/v1/projects/{projectId}/pipelines` to create
`cobran-sdx-e2e-smoke` / `Cobran SDX passwordless boleto flow`. The pipeline
contains the nine approved steps and the fixture CPF only in this private live
object, not source control.

- [ ] **Step 4: Launch and monitor the E2E run**

Call `POST /api/v1/projects/{projectId}/tests/e2e`, record the execution/test
IDs, and poll the execution/history APIs until terminal. Inspect each request,
status, assertion, extraction presence, and downstream response without
printing secrets or PII.

- [ ] **Step 5: Report the live outcome**

If all steps pass, report the project, pipeline, execution IDs, deployed image,
and terminal success. If a step fails, report the exact subsystem and sanitized
contract evidence; retain the private pipeline for rerun after correction.
