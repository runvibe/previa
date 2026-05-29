# Troubleshooting

This guide covers common local issues when working with `previa`.

## Context Already Running

Error:

```text
context '<name>' is already running
```

What it means:

- the selected context already has active local processes

What to do:

```bash
previa status --context <name>
previa down --context <name>
```

## No Detached Runtime

Error:

```text
no detached runtime exists for context '<name>'
```

What it means:

- the context was never started with `--detach`
- or it has already been stopped

## Docker-Backed Startup

Use `previa doctor` first when the Docker-backed runtime does not start:

```bash
previa doctor
```

If Docker is running but the default ports are busy, start on alternate ports:

```bash
previa up -d --main-port 5688 --runner-port-range 56880:56979
```

If startup fails because runtime images are missing or stale, pull them before
starting again:

```bash
previa pull all
```

## Docker Compose Is Not Available

Error examples:

```text
failed to find Docker Compose
```

or

```text
docker compose up failed
```

What it means:

- `previa` could not use Docker Compose on your machine
- the CLI supports both `docker compose` and `docker-compose`

What to do:

- confirm that either `docker compose version` or `docker-compose version` works in your shell
- if only `docker-compose` is available, `previa` will use it automatically
- if neither command works, install Docker Compose and try `previa up` again

## Runner Selector Did Not Match

Error:

```text
runner selector '<value>' did not match any local runner
```

What it means:

- the selector used in `status`, `logs`, or `down --runner` did not match any
  local runner in the recorded runtime

## Requested Runner Count Exceeds Port Range

Error:

```text
requested local runner count exceeds the configured port range
```

What it means:

- the configured `-P/--runner-port-range` cannot fit the requested
  `--runners` count

## Unexpected `--bin` Behavior Inside the Workspace

This is a common source of confusion when using `--bin` inside the workspace.

`previa` resolves local binaries from:

1. workspace `target/debug`
2. workspace `target/release`
3. `PREVIA_HOME/bin`

By default, `previa up --bin` tries to keep `previa-main` and `previa-runner`
aligned with the current CLI version. That means:

- matching workspace binaries are preferred when present
- if no compatible workspace binary exists, a matching binary under `PREVIA_HOME/bin` is reused
- if no compatible local binary exists anywhere, `previa` downloads one into `PREVIA_HOME/bin`

Typical workaround:

```bash
cargo build -p previa-main -p previa-runner
previa --home ./.previa-dev up --detach --bin
```

## Pipeline Import Failures

Check these first:

- `--stack` is present
- `--detach` is present
- the file suffix is one of `.previa`, `.previa.json`, `.previa.yaml`, `.previa.yml`
- recursive mode points to a directory
- non-recursive mode points to a file
- the file content is a direct `Pipeline` object

If import fails after startup, the runtime remains running. You can inspect it
with:

```bash
previa status
previa logs
```

## `401 Unauthorized` From a Runner

If a runner is configured with `RUNNER_AUTH_KEY`, `previa-main` must send the
same value in the `Authorization` header.

Check these first:

- `RUNNER_AUTH_KEY` is the same on `main` and the runner
- the value is present in the environment used by `previa up`
- attached external runners are using the same shared key
- the runtime binaries in use match the current CLI version

When auth is enabled, even `/health` and `/info` require the key. A mismatch
there makes the runner appear unhealthy or unavailable.

## Path and Home Confusion

To isolate everything inside a repo, prefer:

```bash
previa --home ./.previa up --detach
```

This avoids mixing project-local experimentation with the default
`$HOME/.previa`.

## See Also

- [Home and contexts](./home-and-contexts.md)
- [Up and runtime](./up-and-runtime.md)
- [Pipeline import](./pipeline-import.md)
