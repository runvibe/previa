# `previa-runner`

> Rustdoc-style crate documentation.

## Crate Purpose

`previa-runner` is the remote execution API. It receives E2E/load requests, runs pipelines through `previa-engine`, and streams SSE events.

## Runtime Configuration

| Variable | Default | Description |
|---|---|---|
| `ADDRESS` | `0.0.0.0` | Bind address |
| `PORT` | `7373` | Bind port |
| `RUST_LOG` | unset | Tracing filter |

## Quick Start

```bash
ADDRESS=0.0.0.0 PORT=55880 RUST_LOG=info cargo run -p previa-runner
```

You can also download prebuilt binaries at: **https://previa.dev/downloads**

## HTTP API Surface

Base URL: `http://localhost:55880`

Pipeline rule: every `step.url` must be an absolute URL (`http://` or `https://`).

- `GET /health`
- `GET /info`
- `GET /openapi.json`
- `POST /api/v1/tests/e2e`
- `POST /api/v1/tests/load`

## Request Models

### E2E

```json
{
  "pipeline": { "name": "E2E", "steps": [] },
  "selectedBaseUrlKey": null,
  "specs": []
}
```

### Load

```json
{
  "pipeline": { "name": "Load", "steps": [] },
  "load": {
    "points": [
      { "atMs": 0, "intensity": 10 },
      { "atMs": 60000, "intensity": 80 },
      { "atMs": 120000, "intensity": 30 }
    ],
    "interpolation": "smooth",
    "runnerMaxRps": 600,
    "gracePeriodMs": 30000
  },
  "selectedBaseUrlKey": null,
  "specs": []
}
```

The legacy `config.totalRequests` shape may still be accepted for compatibility,
but new load tests should use the Wave `load` shape.

## SSE Event Contracts

### E2E sequence

1. `execution:init`
2. `step:start`
3. `step:result`
4. `pipeline:complete`

### Load sequence

1. `execution:init`
2. `metrics` (repeated)
3. `complete`

## Transaction Header

`x-transaction-id` is propagated and echoed by middleware.

## Error Contract

```json
{
  "error": "bad_request",
  "message": "description"
}
```

## Curl Example

```bash
curl -N http://localhost:55880/api/v1/tests/e2e \
  -H 'content-type: application/json' \
  -d '{"pipeline":{"name":"E2E","steps":[{"id":"s1","name":"Status","method":"GET","url":"https://httpbin.org/status/200","headers":{},"body":null,"asserts":[]}]},"selectedBaseUrlKey":null,"specs":[]}'
```

## Module Relationship

```text
main -> runner -> engine
```
