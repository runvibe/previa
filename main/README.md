# `previa-main`

> Rustdoc-style crate documentation.

## Crate Purpose

`previa-main` is the orchestrator API. It routes execution to runners, aggregates SSE streams, and stores E2E/load history in SQLite or Postgres.

## Quick Start

### Start runner(s)

```bash
ADDRESS=0.0.0.0 PORT=55880 cargo run -p previa-runner
```

You can also download prebuilt binaries at: **https://previa.dev/downloads**

### Start orchestrator

```bash
ORCHESTRATOR_DATABASE_URL="sqlite://orchestrator.db" \
RUNNER_ENDPOINTS="http://127.0.0.1:55880" \
MCP_ENABLED=true \
PREVIA_APP_ENABLED=true \
ADDRESS=0.0.0.0 PORT=5588 \
cargo run -p previa-main
```

Postgres can be selected with the same environment variable:

```bash
ORCHESTRATOR_DATABASE_URL="postgres://previa:previa@127.0.0.1:5432/previa" \
RUNNER_ENDPOINTS="http://127.0.0.1:55880" \
cargo run -p previa-main
```

You can also download prebuilt binaries at: **https://previa.dev/downloads**

### Connect from UI

Use **https://previa.dev** and add:

```text
http://127.0.0.1:5588
```

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `ORCHESTRATOR_DATABASE_URL` | `sqlite://orchestrator.db` | Orchestrator database URL. Supports `sqlite://`, `postgres://`, and `postgresql://` |
| `RUNNER_RPS_PER_NODE` | `1000` | Per-node capacity hint for load planning |
| `RUNNER_ENDPOINTS` | empty | Runner endpoints CSV |
| `ADDRESS` | `0.0.0.0` | Bind address |
| `PORT` | `5588` | Bind port |
| `PREVIA_APP_ENABLED` | `false` | Enables the embedded React app on `/`, `/index`, and SPA routes outside reserved API/system paths |
| `PREVIA_AUTH_ANONYMOUS` | `true` | Enables anonymous full access when true or unset; set to `false` for protected mode |
| `PREVIA_ROOT_USERNAME` | unset | Environment root username required when protected mode is enabled |
| `PREVIA_ROOT_PASSWORD` | unset | Environment root password required when protected mode is enabled |
| `PREVIA_JWT_SECRET` | unset | JWT signing and API-token hashing secret required when protected mode is enabled |
| `PREVIA_JWT_TTL_SECONDS` | `86400` | App JWT lifetime in seconds |
| `MCP_ENABLED` | `false` | Habilita o endpoint MCP HTTP no `main` |
| `MCP_PATH` | `/mcp` | Caminho HTTP do servidor MCP quando habilitado |
| `RUST_LOG` | unset | Tracing filter |

Por padrao, `PREVIA_AUTH_ANONYMOUS` fica efetivamente ativo e todas as rotas
operam como o usuario `anonymous`, sem JWT ou API token. Quando
`PREVIA_AUTH_ANONYMOUS=false`, o `main` exige autenticacao em todas as rotas
protegidas; somente `GET /health`, `POST /api/v1/auth/login` e assets estaticos
do app ficam publicos. Veja
[`docs/previa/access-management.md`](../docs/previa/access-management.md).

Quando `MCP_ENABLED=true`, o `main` expõe um servidor MCP HTTP em `POST /mcp` por padrão.
Se precisar alterar o caminho, defina `MCP_PATH`.

Quando `PREVIA_APP_ENABLED=true`, o `main` serve o app React embutido em `/` e `/index`.
Rotas desconhecidas fora de `/api`, `/health`, `/info`, `/openapi.json`, `/proxy` e do caminho MCP
retornam `index.html` para o React Router resolver. Rotas `/api/...` continuam respondendo como API,
incluindo 404 em JSON.

## HTTP API Surface

Base URL: `http://localhost:5588`

Pipeline rule: every `step.url` must be an absolute URL (`http://` or `https://`).

### System

- `GET /health`
- `GET /info`
- `GET /openapi.json`

### Auth and Access

- `POST /api/v1/auth/login`
- `GET /api/v1/auth/me`
- `GET|POST /api/v1/users`
- `PATCH|DELETE /api/v1/users/{userId}`
- `GET|POST /api/v1/api-tokens`
- `PATCH|DELETE /api/v1/api-tokens/{tokenId}`

### Embedded App

- `GET /` when `PREVIA_APP_ENABLED=true`
- `GET /index` when `PREVIA_APP_ENABLED=true`
- `GET /<client-route>` when `PREVIA_APP_ENABLED=true` and the path is not reserved by the API/system routes

### Proxy

- `POST /proxy`

### Projects

- `GET /api/v1/projects`
- `POST /api/v1/projects`
- `GET /api/v1/projects/{projectId}`
- `PUT /api/v1/projects/{projectId}`
- `DELETE /api/v1/projects/{projectId}`

### Specs

- `POST /api/v1/specs/validate`
- `GET /api/v1/projects/{projectId}/specs`
- `POST /api/v1/projects/{projectId}/specs`
- `GET /api/v1/projects/{projectId}/specs/{specId}`
- `PUT /api/v1/projects/{projectId}/specs/{specId}`
- `DELETE /api/v1/projects/{projectId}/specs/{specId}`

### Pipelines

- `GET /api/v1/projects/{projectId}/pipelines`
- `POST /api/v1/projects/{projectId}/pipelines`
- `GET /api/v1/projects/{projectId}/pipelines/{pipelineId}`
- `PUT /api/v1/projects/{projectId}/pipelines/{pipelineId}`
- `DELETE /api/v1/projects/{projectId}/pipelines/{pipelineId}`

### E2E / Load Execution

- `POST /api/v1/projects/{projectId}/tests/e2e`
- `POST /api/v1/projects/{projectId}/tests/load`

Load tests should use the Wave payload shape with `load.points`,
`load.interpolation`, `load.runnerMaxRps`, and `load.gracePeriodMs`. See
[`docs/previa/wave-load-tests.md`](../docs/previa/wave-load-tests.md).

### Execution Stream / Cancel

- `GET /api/v1/projects/{projectId}/executions/{executionId}`
- `POST /api/v1/executions/{executionId}/cancel`

### History

- `GET|DELETE /api/v1/projects/{projectId}/tests/e2e`
- `GET|DELETE /api/v1/projects/{projectId}/tests/e2e/{test_id}`
- `GET|DELETE /api/v1/projects/{projectId}/tests/load`
- `GET|DELETE /api/v1/projects/{projectId}/tests/load/{test_id}`

## SSE Events

Primary events emitted by orchestration flows:

- `execution:init`
- `step:start`
- `step:result`
- `pipeline:complete`
- `metrics`
- `complete`
- `error`

Common context fields include node planning and runner metadata (`nodesFound`, `nodesUsed`, `runners`, `warning`, etc.).

## Error Contract

```json
{
  "error": "bad_request|not_found|service_unavailable|internal_server_error",
  "message": "description"
}
```

## Module Relationship

```text
main -> runner -> engine
```

## Common Pitfalls

- Missing `RUNNER_ENDPOINTS`.
- No active runners on `/health`.
- Empty pipeline steps in execution payloads.
