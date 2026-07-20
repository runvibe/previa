# Response Extraction and Cobran SDX Smoke Design

## Status

Approved for implementation on 2026-07-20.

## Objective

Allow a Previa E2E pipeline to extract a value from an HTTP response and reuse
it in later steps. The first production use case is an autonomous smoke test of
the Cobran API at `gateway.sdx.autob/v1/cobran`, including its passwordless
authentication through the SDX Auth service and its boleto integration with the
production Autobanking Core.

The smoke uses company `7`, the agreed test customer CPF, and installment
`d0e313aa-ea2a-41b5-8e69-38804c2ac567`. The CPF is intentionally not repeated
in this design document; it is supplied only in the private pipeline created in
Previa.

## Verified Runtime Contracts

The following contracts were verified against the live services before this
design was written:

- `GET https://api.autobanking.com.br/api/customer/{cpf}/7` returns `200` with
  `data.email` and `data.fullName`.
- `GET https://api.autobanking.com.br/api/boletos/{cpf}?companyId=7` returns
  `200` with one operation and twelve installments for the selected fixture.
- `GET https://api.autobanking.com.br/api/boletos/{cpf}/{installmentKey}` returns
  `200` and the selected installment is currently `open`.
- The Core serializes monetary fields as strings even though the current Cobran
  OpenAPI schema describes them as numbers. The smoke must expose, rather than
  hide, any incompatibility this creates in the Cobran adapter.
- `POST http://gateway.sdx.autob/v1/cobran/api/auth/signin` returns `202` and
  sends the passwordless code to Mailpit in the `auth` namespace.
- Mailpit is reachable from the Previa runners at
  `http://mailpit.auth.svc.cluster.local:8025`. Its message detail response has
  `Text` and `HTML` fields, and both contain the six-digit code.
- `http://previa.sdx.autob` and the Postgres execution queue are healthy. The
  queue had ready runners when this design was approved.

## Problem

Previa can currently interpolate a field from a previous JSON response with a
template such as `{{steps.login.token}}`. It cannot derive a smaller value from
a string field. Mailpit returns the passwordless email body as one string, so a
pipeline can assert that a code is present but cannot pass that code to the
Cobran verification endpoint.

Generating a token outside the pipeline would make the test manual and
short-lived. Persisting a refresh token would make repeated executions depend
on rotated credentials. Neither option is a true autonomous smoke test.

## Decision

Add declarative response extractions to `PipelineStep`. An extraction selects a
response field, applies a regular expression, and stores one capture group in
the step execution result. Later steps reference the value through a dedicated
`extracts` template root.

Example:

```json
{
  "id": "read-login-email",
  "name": "Read login email",
  "method": "GET",
  "url": "http://mailpit.auth.svc.cluster.local:8025/api/v1/message/{{steps.list-login-emails.messages.0.ID}}",
  "extracts": [
    {
      "name": "code",
      "field": "body.HTML",
      "regex": "<strong>[[:space:]]*([0-9]{6})[[:space:]]*</strong>",
      "group": 1,
      "required": true
    }
  ]
}
```

The extracted value is then available as:

```text
{{extracts.read-login-email.code}}
```

The response body remains available through the existing `steps` root, so all
current pipelines remain compatible. A dedicated root avoids shadowing normal
response properties and preserves scalar response interpolation unchanged.

## Contract

### Step extraction

`PipelineStep` gains an optional `extracts` array. Each entry contains:

- `name`: required identifier, unique within the step; lowercase letters,
  digits, `_`, and `-` are accepted.
- `field`: required assertion-style source path. The initial version supports
  `body` and `body.<json-path>`, including array indexes.
- `regex`: required Rust-compatible regular expression.
- `group`: optional capture-group index, default `1`. Group `0` explicitly
  selects the entire match.
- `required`: optional boolean, default `true`.

The regular expression is compiled during pipeline validation. Invalid
patterns, duplicate names, invalid names, and references to a capture group
that does not exist are rejected before execution where that can be determined
statically.

### Execution semantics

Extractions run after the response body is decoded and before assertions are
evaluated.

- A successful match stores the selected capture as a string.
- If a required extraction cannot resolve its source, match its expression, or
  select its configured group, the step fails with an error that identifies the
  extraction but does not include the extracted value or full response body.
- A missing optional extraction is omitted from `extracts` and does not fail
  the step.
- Retries repeat extraction against each new response. Only the terminal
  attempt is retained.
- Extracted values are serialized in `StepExecutionResult` so queued E2E jobs,
  reruns from a later step, history, and SSE all preserve template context.

`StepExecutionResult` gains an optional `extracts` map. Template context keeps
the current response bodies under `steps` and builds a parallel extraction map:

```json
{
  "steps": {
    "read-login-email": {
      "...response fields": "..."
    }
  },
  "extracts": {
    "read-login-email": {
      "code": "captured value"
    }
  }
}
```

### Validation

The main API validates extraction definitions when a pipeline is created,
updated, imported, or submitted inline. Template validation accepts
`{{extracts.<earlierStep>.<name>}}` only when the referenced earlier step
declares that extraction. Unknown extraction names and forward references are
rejected.

The runner repeats structural validation defensively before execution so a
queued payload cannot bypass the contract.

### Sensitive data

The feature is generic and does not assume that an extracted value is secret.
However, the first use case extracts an OTP and receives access and refresh
tokens. Logging must never print extracted values. Existing execution-history
responses may contain request and response bodies, so the Cobran smoke project
must remain private and must not be shared publicly.

No OTP, bearer token, customer email, or customer name is added to source
control.

## Cobran Smoke Pipeline

Create a private project named `cobran-sdx-e2e-smoke` and a pipeline named
`Cobran SDX passwordless boleto flow`. Its steps are:

1. Call the production Core customer endpoint and assert `200`, `data.email`
   exists, and `data.fullName` exists. This verifies the fixture independently
   before exercising the BFF.
2. Call the production Core boleto list and assert `200`, a non-empty operation
   list, and the selected installment key. This gives a clear upstream failure
   if later Cobran calls fail.
3. Call the production Core installment detail and assert `200`, the selected
   key, and `open` status.
4. Call Cobran signin and assert `202`.
5. Search Mailpit by the customer email returned by the Core preflight, with a
   short retry window, and select the newest matching message. The smoke does
   not delete or mutate messages in the shared mailbox.
6. Fetch that message by the returned ID, assert `200`, verify its recipient,
   and extract the six-digit code from `body.HTML`.
7. Call Cobran verify with the CPF and extracted code; assert `200` and capture
   `data.accessToken` through ordinary response interpolation.
8. Call `GET /api/boletos` with the bearer token; assert `200`, the operation is
   present, and the known installment is returned.
9. Call `GET /api/boletos/{installmentKey}` with the bearer token; assert `200`,
    the exact installment key, and `open` status.

The pipeline uses the public production Core URLs exactly as requested. Cobran
continues to use the SDX Auth service through its configured internal route.
Mailpit is called only through its internal Kubernetes service.

## Failure Attribution

The direct Core preflight steps are intentionally part of the same pipeline.
They separate fixture drift or upstream downtime from Cobran adapter failures:

- Core preflight fails: fixture, production Core, or network problem.
- Signin/email steps fail: Cobran-to-Auth or Auth-to-Mailpit problem.
- Verify fails: extraction, challenge, or Auth verification problem.
- Core passes but authenticated Cobran boleto step fails: Cobran mapping or
  serialization problem, including the known monetary string/number mismatch.

## Code Boundaries

- `engine/src/core/types.rs`: extraction and result contracts.
- `engine/src/execution/`: extraction evaluation and result population.
- `engine/src/template/resolve.rs`: expose `extracts` without changing normal
  response-body paths.
- `main/src/server/validation/pipelines.rs`: definition and reference
  validation.
- `main/src/server/docs.rs`: generated OpenAPI components remain synchronized.
- `main/src/server/mcp/service.rs`: pipeline creation guidance documents the
  new contract even though MCP routes are not required for execution.
- `app/src/lib/api-client.ts` and relevant UI pipeline types/editors: keep the
  TypeScript contract synchronized without requiring the UI to author
  extractions in the first version.

No persistence migration is needed because pipelines and execution results are
already serialized as JSON.

## Testing

Implementation tests must cover:

- extraction from a string response body and a nested JSON string field;
- default and explicit capture groups;
- optional and required no-match behavior;
- invalid regex, duplicate name, invalid name, invalid group, unknown
  extraction reference, and forward-reference validation;
- interpolation of an extracted value into a later request body;
- retries and queued execution serialization;
- preservation of existing `{{steps.<id>.<field>}}` behavior;
- absence of extracted values from logs.

Required repository verification:

- focused engine and main validation tests;
- `cargo test -p previa-main server::docs`;
- `python3 scripts/check_openapi_client_contract.py`;
- `npm test`;
- `cargo test`;
- `cargo build --release`.

After release and SDX deployment, create the private project and pipeline through
the live Previa API, confirm queue diagnostics show ready runners, execute the
pipeline, and inspect every terminal step. A successful enqueue alone is not
completion.

## Rollout and Compatibility

The schema change is additive. Pipelines without `extracts` behave exactly as
before. Main and runner should still be released together because queued job
payloads and result serialization gain new fields.

The feature is complete only when the released main and runner are deployed to
SDX and the live Cobran smoke finishes successfully, or when it produces a
verified application defect with the exact failing step and response contract.
