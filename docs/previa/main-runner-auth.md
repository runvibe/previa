# Main and Runner Authentication (legacy)

> This page describes the pre-Postgres execution transport. Current releases
> authenticate runner queue access with the restricted
> `PREVIA_QUEUE_DATABASE_URL` role. Main no longer sends executions, control
> messages, or telemetry acknowledgements to runner HTTP endpoints. See
> [Postgres execution queue](./postgres-execution-queue.md).

Previa can secure communication between `previa-main` and `previa-runner` using `RUNNER_AUTH_KEY`.

## What It Does

When the runner is configured with `RUNNER_AUTH_KEY`, the orchestrator must send the same raw value in the `Authorization` header for:

- `/health`
- `/info`
- `/api/v1/tests/e2e`
- `/api/v1/tests/load`

## Local Runners

When a stack uses only local runners and no `RUNNER_AUTH_KEY` is configured, `previa up` generates a UUID v4 automatically and persists it to:

- `PREVIA_HOME/stacks/<context>/config/main.env`
- `PREVIA_HOME/stacks/<context>/config/runner.env`

This keeps local main/runner communication exclusive by default and makes the same key available on restart.

## Attached Runners

When `--attach-runner` is used, `RUNNER_AUTH_KEY` is required.

Example:

```bash
RUNNER_AUTH_KEY=shared-secret previa up -d --attach-runner 10.0.0.12:55880
```

If you omit the key in this scenario, `previa up` fails with:

```text
RUNNER_AUTH_KEY is required when using --attach-runner
```

## Precedence

For local startup, the effective `RUNNER_AUTH_KEY` is resolved in this order:

1. process environment
2. compose `main.env`
3. compose `runners.local.env`
4. context-scoped `main.env`
5. context-scoped `runner.env`

## Examples

Direct runner startup:

```bash
RUNNER_AUTH_KEY=shared-secret ADDRESS=0.0.0.0 PORT=55880 cargo run -p previa-runner
```

Direct main startup:

```bash
RUNNER_AUTH_KEY=shared-secret \
RUNNER_ENDPOINTS=http://127.0.0.1:55880 \
MCP_ENABLED=true \
cargo run -p previa-main
```

Compose input:

```yaml
version: 1
main:
  env:
    RUNNER_AUTH_KEY: shared-secret
runners:
  local:
    count: 1
    env:
      RUNNER_AUTH_KEY: shared-secret
```

## See Also

- [Remote runners](./remote-runners.md)
- [Runtime modes](./runtime-modes.md)
- [Troubleshooting](./troubleshooting.md)
