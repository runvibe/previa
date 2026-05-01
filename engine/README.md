# `previa-engine`

> Rustdoc-style crate documentation.

## Crate Purpose

`previa-engine` is the core pipeline execution crate. It resolves templates, executes HTTP steps, evaluates assertions, and returns structured execution results.

## Public API

Exported functions:

- `execute_pipeline`
- `execute_pipeline_with_client`
- `execute_pipeline_with_hooks`
- `execute_pipeline_with_specs_hooks`
- `execute_pipeline_with_client_hooks`
- `render_template_value`
- `render_template_value_simple`

## Core Types

- `RuntimeSpec`
- `Pipeline`
- `PipelineStep`
- `StepAssertion`
- `StepExecutionResult`
- `StepRequest`
- `StepResponse`
- `AssertionResult`

## Execution Model

For each step:

1. Resolve templated `url`, `headers`, and `body`.
2. Apply `delay` (if configured).
3. Perform request.
4. Evaluate assertions.
5. Retry (if allowed by `retry` and assertion behavior).

`step.url` must always be an absolute URL (`http://` or `https://`).

## Template System

Supported expressions:

- `{{steps.<step_id>.<field>}}`
- `{{envs.current.<name>}}`
- `{{envs.<group_slug>.<name>}}`
- `{{specs.<slug>.url.<name>}}`
- legacy: `{{url.<slug>.<name>}}` (normalized)

Helpers:

- `{{helpers.uuid}}`
- `{{helpers.email}}`
- `{{helpers.name}}`
- `{{helpers.username}}`
- `{{helpers.number 10 99}}`
- `{{helpers.date}}`
- `{{helpers.boolean}}`
- `{{helpers.cpf}}`

## Assertion Operators

Currently implemented operators:

- `equals`
- `not_equals`
- `contains`
- `exists`
- `not_exists`
- `gt`
- `lt`

## Important Behavior Notes

- `GET` and `HEAD` do not send request body.
- `delay` is expressed in milliseconds and is applied before each attempt, including retries.
- `maxAttempts = retry + 1`.
- Assertion failures can trigger retry when `retry` allows additional attempts.
- `should_cancel` callback can interrupt execution.
- Unknown templates may remain unchanged.

## Rust Example

```rust,no_run
use previa_engine::execute_pipeline_with_specs_hooks;

# async fn demo(pipeline: previa_engine::Pipeline, specs: Vec<previa_engine::RuntimeSpec>) {
let results = execute_pipeline_with_specs_hooks(
    &pipeline,
    Some("prd"),
    Some(specs.as_slice()),
    |step_id| println!("step:start={step_id}"),
    |result| println!("step:result={}", result.step_id),
    || false,
).await;

println!("steps executed: {}", results.len());
# }
```

## Module Relationship

```text
main -> runner -> engine
```

## Common Pitfalls

- Empty `pipeline.steps` in upstream callers.
- Invalid assertion fields (for example missing `body.*`).
- Assuming unsupported operators such as `gte`, `lte`, `ne`, `not_contains`.
