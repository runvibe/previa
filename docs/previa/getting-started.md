# Getting Started

This guide covers the shortest path to a working local Previa stack.

## Fast Path

If you want the shortest operator flow, it is:

1. install `previa`
2. start a local stack with `previa up -d`
3. open the IDE with `previa open`
4. create or import a pipeline
5. run E2E or load tests from the IDE, API, or MCP-connected assistant

## Install

On Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/runvibe/previa/main/install.sh | sh
```

On Windows:

```powershell
irm https://raw.githubusercontent.com/runvibe/previa/main/install.ps1 | iex
```

The installer detects the local OS, places `previa` under the default Previa home, and configures
`PREVIA_HOME`.

## First Local Stack

Start the default context in detached mode:

```bash
previa up --detach
```

Check status:

```bash
previa status
```

Open the UI served by your local `previa-main`:

```bash
previa open
```

This opens:

```text
http://127.0.0.1:5588
```

The embedded app uses `window.location.origin` as the API base. External builds use `VITE_PREVIA_API_BASE_URL` when it is defined.

Service worker precaching is disabled by default to avoid stale embedded bundles after local rebuilds. Set `VITE_PREVIA_ENABLE_PWA=true` only when building a standalone PWA artifact.

From there, you can:

- create a project and add specs
- create or import pipelines
- run E2E and load tests
- inspect failures and history

Stop the stack:

```bash
previa down
```

## Work Inside a Repo

Use the project-local workflow:

```bash
previa local up --detach
previa local status
previa local open
previa local down
```

This keeps runtime state, logs, and database files inside the repository.

This is shorthand for passing `--home ./.previa` to the regular commands.

## Optional: Pull a Specific Image Tag

```bash
previa pull all --version 0.0.7
previa up --detach --version 0.0.7
```

By default, `previa up` and `previa pull` use the same version tag as the running `previa` CLI.

## What Gets Created

When you start a detached stack, `previa` writes files under:

```text
$PREVIA_HOME/stacks/<context>/
```

Notably:

- `config/main.env`
- `config/runner.env`
- `data/main/orchestrator.db`
- `run/docker-compose.generated.yaml`
- `run/state.json`

## See Also

- [Home and contexts](./home-and-contexts.md)
- [Up and runtime](./up-and-runtime.md)
- [Release and install](./release-install.md)
- [Operations](./operations.md)
