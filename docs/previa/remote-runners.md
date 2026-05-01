# Remote Runners

Remote runners let one `previa-main` orchestrate execution against runner processes outside the local stack.

## Runners Are Required for Tests

`previa-main` is the API and orchestrator. It does not execute E2E or load-test pipelines by itself. Test execution is always delegated to one or more `previa-runner` processes.

Before running tests, make sure at least one runner is:

- registered in the active `previa-main` context;
- enabled in the runner registry;
- reachable from `previa-main`;
- returning success from `/health`.

If no enabled runner is healthy, the app cannot start E2E or load tests. The runner icon in the app shows an alert in that state, and the `/runners` page explains that a runner must be started or fixed before tests can run.

## Start a Runner

Example:

```bash
RUNNER_AUTH_KEY=shared-secret \
ADDRESS=0.0.0.0 \
PORT=55880 \
cargo run -p previa-runner
```

You can also run a downloaded `previa-runner` binary with the same environment variables.

## Attach It to a Local Stack

Use `--attach-runner` with the same shared key:

```bash
RUNNER_AUTH_KEY=shared-secret previa up -d --attach-runner 10.0.0.12:55880
```

Accepted attached runner formats:

- `55880` -> `http://127.0.0.1:55880`
- `10.0.0.12:55880` -> `http://10.0.0.12:55880`
- `10.0.0.12` -> `http://10.0.0.12:55880`

## Dynamic Runner Registry

`previa-main` stores runners in its database. On startup, endpoints from `RUNNER_ENDPOINTS` are automatically inserted or updated in that registry and enabled.

The registry has a persistent `enabled` flag. A disabled runner remains stored for later use, but `previa-main` ignores it before checking health.

You can manage the registry without restarting the context:

```bash
previa runner list
previa runner add 10.0.0.12:55880 --name staging-a
previa runner disable staging-a
previa runner enable staging-a
previa runner remove staging-a
```

For project-local contexts, use:

```bash
previa local runner list
previa local runner add 10.0.0.12:55880 --name staging-a
```

Before each execution, `previa-main` reads enabled runners from the registry, checks `/health` and `/info`, marks failed runners as unhealthy, and runs only with enabled runners whose `/health` responds successfully.

## Mixed Topologies

You can combine local and attached runners:

```bash
RUNNER_AUTH_KEY=shared-secret previa up -d --runners 1 --attach-runner 10.0.0.12:55880
```

In that case:

- local runners inherit the same `RUNNER_AUTH_KEY`
- `previa-main` sends that key to all enabled registered runners

## Important Rule

Attached runners always require `RUNNER_AUTH_KEY`.

If the key on `previa-main` does not match the key on the remote runner, the runner appears unhealthy and execution cannot start successfully.

## See Also

- [Main and runner authentication](./main-runner-auth.md)
- [Operations](./operations.md)
