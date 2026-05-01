# `previa` v1 Specification

## Summary

`previa` is the local operations CLI for Previa. Version 1 is Linux-first and
local-only: it runs and manages a local Previa context through Docker Compose
and exposes the local `previa` version.

This document is implementation-ready. Anything not defined here is out of
scope for v1 and must not be invented during implementation.

## Product Goals

- Bootstrap a local context with one `previa-main` and multiple
  `previa-runner` processes.
- Allow attaching existing runner endpoints that are already running.
- Support foreground and detached execution modes.
- Persist all `previa`-generated files under `PREVIA_HOME`.
- Reuse the current environment-variable contract already supported by the
  binaries.
- Provide machine-readable status and listing output for automation.
- Provide context-scoped log access for detached execution.
- Detect unhealthy processes using both PID liveness and HTTP health probes.
- Expose the local `previa` version.
- Pull published container images for `previa-main` and `previa-runner`.

## Non-Goals

- Installing binaries for Linux, macOS, or Windows in v1.
- Updating binaries in v1.
- Remote provisioning over SSH.
- Fleet or cluster management across multiple hosts.
- Automatic runner registration in external control planes.
- Native service managers such as `systemd`, `launchd`, or Windows Service
  Manager.
- Checksum or signature verification in v1.

## Command Surface

The v1 CLI surface is fixed to the commands below:

```text
previa [--home <path>] up [--context <context-name>] [<source>] [--main-address <address>] [--main-port, -p <port>] [--runner-address <address>] [--runner-port-range, -P <start:end>] [--runners <N>] [--attach-runner, -a <address|address:port|port> ...] [--import, -i <path>] [--recursive, -r] [--stack, -s <name>] [--dry-run] [-d, --detach] [--version <tag>]
previa [--home <path>] pull [main|runner|all] [--version <version>]
previa [--home <path>] down [--context <context-name>] [--all-contexts] [--runner <address|address:port|port> ...]
previa [--home <path>] restart [--context <context-name>] [--version <tag>]
previa [--home <path>] status [--context <context-name>] [--main] [--runner <address|address:port|port>] [--json]
previa [--home <path>] list [--json]
previa [--home <path>] ps [--context <context-name>] [--json]
previa [--home <path>] logs [--context <context-name>] [--main] [--runner <address|address:port|port>] [--follow] [--tail, -t [<lines>]]
previa [--home <path>] open [--context <context-name>]
previa version
```

No additional v1 commands are required beyond the surface listed above.

### Command Semantics

#### `previa [--home <path>] up [--context <context-name>] [<source>] [--main-address <address>] [--main-port, -p <port>] [--runner-address <address>] [--runner-port-range, -P <start:end>] [--runners <N>] [--attach-runner, -a <address|address:port|port> ...] [--import, -i <path>] [--recursive, -r] [--stack, -s <name>] [--dry-run]`

- Bootstraps a local context on the current host.
- Executes exactly one `previa-main` process.
- Accepts `--context <context-name>` to identify the local execution context that `up`,
  `down`, `restart`, `status`, and `ps` will manage.
- When omitted, `--context` defaults to `default`.
- `context-name` must match `[A-Za-z0-9][A-Za-z0-9._-]*`.
- Optionally accepts a positional `<source>` that points to a
  `previa-compose.json`, `previa-compose.yaml`, or `previa-compose.yml`
  document.
- `<source>` may be `.`, a directory path, or an explicit file path.
- When `<source>` is `.` or a directory path, `up` must search that directory
  in this exact order:
  - `previa-compose.yaml`
  - `previa-compose.yml`
  - `previa-compose.json`
- When `<source>` is a file path, the file extension must be `.json`, `.yaml`,
  or `.yml`.
- When a compose file is resolved, `up` must load configuration from it before
  applying CLI flag overrides.
- Optionally overrides the `previa-main` listen address through
  `--main-address <address>`.
- Optionally overrides the `previa-main` listen port through
  `--main-port <port>` or `-p <port>`.
- Optionally spawns the number of local `previa-runner` processes declared by
  `--runners <N>`.
- Optionally overrides the local runner bind address through
  `--runner-address <address>`.
- Optionally overrides the local runner port allocation window through
  `--runner-port-range <start:end>` or `-P <start:end>`.
- Optionally attaches one or more existing runner targets provided through
  repeated `--attach-runner <selector>` or `-a <selector>` flags.
- Optionally imports pipelines from a local file or directory through
  `--import <path>` or `-i <path>`.
- Accepts `--recursive` or `-r` only together with `--import` to scan a
  directory recursively for pipeline files.
- Accepts `--stack <name>` or `-s <name>` together with `--import` to name the
  new project created by the import.
- `--attach-runner <selector>` accepts:
  - `port`, for example `55880`
  - `address:port`, for example `10.0.0.12:55880`
  - `address`, for example `10.0.0.12`
- `port` is normalized to `http://127.0.0.1:<port>`.
- `address:port` is normalized to `http://<address>:<port>`.
- `address` is normalized to `http://<address>:55880`.
- `up` must persist attached runners in normalized full-URL form.
- Effective configuration precedence is:
  - CLI flags
  - compose file values from `<source>`
  - the context-scoped `main.env` and `runner.env`
  - built-in defaults from this specification
- Requires at least one runner source overall: either `--runners <N>` greater
  than `0`, at least one `--attach-runner` / `-a`, or both.
- When omitted, the effective `previa-main` `ADDRESS` comes from
  the context-scoped `main.env`, or `0.0.0.0` when that file or variable is
  absent.
- When present, `--main-address <address>` overrides the effective
  `previa-main` `ADDRESS`.
- When omitted, `--main-port` / `-p` defaults to the effective `PORT` value from
  the context-scoped `main.env`, or `5588` when that file or variable is absent.
- When present, `--main-port <port>` / `-p <port>` must be an integer from `1`
  to `65535`.
- When `--runners` is omitted, it defaults to `1`.
- When present, `--runners <N>` must be an integer greater than or equal to
  `0`.
- When omitted, `--runner-port-range` / `-P` defaults to `55880:55979`.
- When present, `--runner-port-range <start:end>` / `-P <start:end>` must:
  - parse as two integer ports from `1` to `65535`
  - satisfy `start <= end`
  - provide at least as many distinct ports as the requested local runner count
- When omitted, the effective local runner `ADDRESS` comes from
  the context-scoped `runner.env`, or `127.0.0.1` when that file or variable is
  absent.
- When present, `--runner-address <address>` overrides the effective local
  runner `ADDRESS`.
- Accepts `--dry-run` to validate configuration and print the resolved context
  plan without spawning child processes or mutating runtime state.
- In this version, `--import` requires `--detach`.
- `--import` must not be combined with `--dry-run`.
- `--stack` is required whenever `--import` is present.
- Without `--recursive`, `--import <path>` must resolve to a single file whose
  name ends with `.previa`, `.previa.json`, `.previa.yaml`, or `.previa.yml`.
- With `--recursive`, `--import <path>` must resolve to a directory and `up`
  must recursively collect only files ending with `.previa`, `.previa.json`,
  `.previa.yaml`, or `.previa.yml`.
- In recursive mode, files outside those suffixes are ignored.
- Every collected import file must parse as a direct `Pipeline` object in JSON
  or YAML according to its suffix; any candidate file that fails to parse or
  validate must fail the command and report the offending path.
- After the local `previa-main` becomes healthy, `up` must send the collected
  pipelines to the local API endpoint `/api/v1/projects/import/pipelines`.
- The backend import must create a new project named by `--stack` and persist
  all pipelines atomically.
- If the import fails after the runtime has started, the runtime remains
  running and the CLI returns the import error.
- Accepts `-d` and `--detach` to leave the spawned processes running in
  background.
- Before validating local bind availability or spawning any child process, `up`
  must fail clearly when the selected context already has running recorded
  processes.
- That error must identify the selected context and list the recorded services
  with their addresses and ports.
- Uses the lowest port in the effective runner port range for the first local
  runner and increments sequentially for each additional local runner.
- Builds `RUNNER_ENDPOINTS` for `previa-main` by concatenating:
  - the local runner processes started by the command, in local port order
  - the attached runner endpoints provided via `--attach-runner` / `-a`, in
    CLI order after normalization
- Example:
  `http://127.0.0.1:55880,http://127.0.0.1:55881,http://10.0.0.12:55880`
- Starts `previa-main` after all local runner processes have been spawned.
- Fails before spawning any child process when the effective local
  `previa-main` or local runner bind target is already in use.
- Without `--dry-run`, when the effective local `previa-main` port is already
  in use, `up` must interactively prompt the operator to accept `port + 100`
  as the replacement main port or rerun with `-p <port>`.
- Without `--dry-run`, when the effective local runner port range contains an
  in-use local bind target, `up` must interactively prompt the operator to
  accept the entire runner range shifted by `+100` ports or rerun with
  `-P <start:end>`.
- In those prompts, pressing Enter with no typed value must be treated as `yes`.
- Starts `previa-main` with `ADDRESS` and `PORT` overridden to the effective
  `--main-address` and `--main-port` / `-p` values when provided.
- With `--dry-run`, resolves compose input, validates selectors, validates
  context-scoped paths, validates port allocation, validates that requested
  local bind targets are available, prints the effective context plan, and exits
  successfully without acquiring the stack lock.
- With `--dry-run`, must fail for the same validation errors that a real `up`
  invocation would surface before process creation.
- Without `-d` or `--detach`, runs all child processes in foreground and
  multiplexes their stdout and stderr to the current terminal session.
- Without `-d` or `--detach`, stops all child processes when the command
  receives `SIGINT` or `SIGTERM`.
- With `-d` or `--detach`, writes the runtime file for the selected context name
  under `PREVIA_HOME/stacks/<context-name>/run/` and then exits successfully.
- With `-d` or `--detach`, redirects `previa-main` stdout and stderr to
  `PREVIA_HOME/stacks/<context-name>/logs/main.log`.
- With `-d` or `--detach`, redirects each local runner stdout and stderr to
  `PREVIA_HOME/stacks/<context-name>/logs/runners/<port>.log`.
- Does not rewrite the context-scoped `main.env` or `runner.env`.

#### `previa pull [main|runner|all] [--version <version>]`

- Pulls published container images using the local Docker CLI.
- Accepts `main`, `runner`, or `all` as the optional target selector.
- When omitted, the target defaults to `all`.
- Accepts `--version <version>` to override the image tag.
- When omitted, `--version` defaults to `latest`.
- Resolves repositories exactly as:
  - `main` -> `ghcr.io/runvibe/main`
  - `runner` -> `ghcr.io/runvibe/runner`
- `all` must pull both repositories sequentially using the same resolved tag.
- Must fail with a clear error when the Docker CLI is unavailable in `PATH`.
- Must not require local `previa-main` or `previa-runner` binaries to exist.

#### `previa down [--context <context-name>] [--all-contexts] [--runner <address|address:port|port> ...]`

- Stops a local detached context started by `previa up --detach`.
- Accepts `--context <context-name>` and defaults to `default` when omitted.
- Accepts `--all-contexts` to stop every detached context recorded under
  `PREVIA_HOME/stacks/`.
- Reads the runtime file for the selected context name.
- Without `--runner`, sends a termination signal to the recorded
  `previa-main` PID and to every recorded local `previa-runner` PID.
- Without `--runner`, waits for the recorded local processes to exit and
  removes the selected stack runtime file after shutdown completes.
- With one or more `--runner <selector>` flags, sends termination signals only
  to the matching recorded local runner PIDs.
- With one or more `--runner <selector>` flags, rewrites the selected context
  runtime file after removing the stopped local runner entries and preserving
  the `previa-main` PID plus any remaining local runners and attached runner
  endpoints.
- With `--all-contexts`, ignores per-context selection and stops every detached
  context that currently has a runtime file.
- With `--all-contexts`, removes each selected context runtime file after the
  recorded local processes exit.
- `--all-contexts` and `--runner <selector>` are mutually exclusive in v1.
- `--runner <selector>` accepts:
  - `port`, for example `55880`
  - `address:port`, for example `127.0.0.1:55880`
  - `address`, for example `127.0.0.1`
- Matching rules:
  - `port` matches local runner entries with the same `port`
  - `address:port` matches local runner entries with both the same address and
    the same port
  - `address` matches all local runner entries with the same address
- The runtime file must store the local runner bind address for each runner
  entry so that selector matching is deterministic.
- Partial runner shutdown must fail if none of the requested selectors match a
  local runner entry in the runtime file.
- Partial runner shutdown must fail if it would leave the context with zero
  runner sources overall, meaning no remaining local runners and no attached
  runner endpoints.
- Fails with a clear error if no detached runtime file exists.
- Does not send termination signals to attached runner endpoints because they
  are not child processes of `previa`.

#### `previa restart [--context <context-name>]`

- Restarts a detached local context previously started by `previa up --detach`.
- Accepts `--context <context-name>` and defaults to `default` when omitted.
- Reads the runtime file for the selected context name.
- Stops the recorded local processes using the same behavior as `previa down`.
- Starts a new detached context using the same effective configuration recorded in
  the runtime file:
  - the recorded `main.address`
  - the recorded `main.port`
  - the local runner count from the recorded local runner entries
  - the local runner `address` values from the recorded local runner entries
  - the recorded `runner_port_range`
  - the attached runner endpoints from `attached_runners`
  - the recorded compose `source` path when present
- Rewrites the selected context runtime file with the new PIDs after the new
  context starts successfully.
- Fails with a clear error if no detached runtime file exists.
- Does not send termination signals to attached runner endpoints.

#### `previa status [--context <context-name>] [--main] [--runner <address|address:port|port>] [--json]`

- Reports the status of the detached local context managed by `previa up`.
- Accepts `--context <context-name>` and defaults to `default` when omitted.
- Reads the runtime file for the selected context name when it exists.
- Without filters, checks both PID liveness and HTTP health for the recorded
  `previa-main` and local `previa-runner` processes and reports the overall
  stack status.
- HTTP health probes must use `GET /health` against the recorded `address` and
  `port` of each local process and treat HTTP `200 OK` as healthy.
- With `--main`, reports only the status of the recorded `previa-main` PID and
  health probe result.
- With `--runner <selector>`, reports only the status of the recorded local
  runner or local runners that match the given selector, including PID and
  health probe results.
- `--main` and `--runner <selector>` are mutually exclusive in v1.
- Without filters, prints `stopped` when the runtime file does not exist.
- Without filters, prints `running` with the recorded PIDs, ports, and attached
  runner endpoints when all recorded local processes are alive and all local
  health probes return `200 OK`.
- Without filters, prints `degraded` when the runtime file exists but one or
  more recorded local PIDs are no longer alive, or when one or more local
  health probes fail.
- With `--main`, prints `running`, `degraded`, or `stopped` for the
  `previa-main` process.
- With `--runner <selector>`, prints `running`, `degraded`, or `stopped` for
  the selected local runner or runners.
- `--runner <selector>` accepts:
  - `port`, for example `55880`
  - `address:port`, for example `127.0.0.1:55880`
  - `address`, for example `127.0.0.1`
- `status --runner <selector>` must fail clearly when the requested selector
  does not match any local runner entry in the runtime file.
- Accepts `--json` to print a stable machine-readable JSON document instead of
  human-readable text.
- `status --json` must include the selected context name, overall state, runtime
  file path, `main`, `runners`, and `attached_runners`.
- `status --json` must use the exact schema defined in the `Status JSON Schema`
  section of this specification.
- Does not interact with native service managers.

#### `previa list [--json]`

- Lists the context names currently known under `PREVIA_HOME/stacks/`.
- Prints one context per line with its current status.
- Context status is derived from the same runtime-state rules used by
  `previa status`.
- `list` must inspect context directories and their runtime files under
  `PREVIA_HOME/stacks/`.
- `list` must include contexts with runtime state even if their processes are no
  longer alive, in which case they appear as `degraded` or `stopped` according
  to the recorded state.
- Accepts `--json` to print a stable machine-readable JSON array instead of
  human-readable text.
- `list --json` must use the exact schema defined in the `List JSON Schema`
  section of this specification.

#### `previa ps [--context <context-name>] [--json]`

- Lists the local processes tracked for a detached context.
- Accepts `--context <context-name>` and defaults to `default` when omitted.
- Reads the runtime file for the selected context name when it exists.
- Without `--json`, prints one row per tracked local process.
- `ps` must include the detached `previa-main` process and all detached local
  `previa-runner` processes.
- `ps` must not represent attached runner endpoints as local processes because
  they are externally managed.
- Each printed process row must include:
  - `role`, fixed to `main` or `runner`
  - `pid`
  - `address`
  - `port`
  - `state`
  - `health_url`
  - `log_path`
- `state` is derived from the same PID liveness and health-probe rules used by
  `status`.
- When no detached runtime file exists, `ps` prints no process rows and exits
  successfully.
- Accepts `--json` to print a stable machine-readable JSON array instead of
  human-readable text.
- `ps --json` must use the exact schema defined in the `PS JSON Schema`
  section of this specification.

#### `previa logs [--context <context-name>] [--main] [--runner <address|address:port|port>] [--follow] [--tail, -t [<lines>]]`

- Reads detached context logs from context-scoped log files.
- Accepts `--context <context-name>` and defaults to `default` when omitted.
- With no selector, prints the `previa-main` log followed by all local runner
  logs in ascending local runner port order.
- With `--main`, prints only `PREVIA_HOME/stacks/<context-name>/logs/main.log`.
- With `--runner <selector>`, prints only the matching local runner log file or
  files.
- Accepts `--tail <lines>` / `-t <lines>` to print only the last `N` lines from
  each selected log file.
- Accepts `--tail` / `-t` with no value as a shorthand for the last `10` lines.
- `--main` and `--runner <selector>` are mutually exclusive in v1.
- `--runner <selector>` uses the same selector grammar and matching rules as
  `status` and `down`.
- Accepts `--follow` to stream appended log lines until interrupted by the
  operator.
- When `--tail` / `-t` is combined with `--follow`, `logs` prints only the
  tailed suffix first and then streams appended lines.
- `logs` is only defined for detached contexts because foreground contexts already
  stream directly to the calling terminal.
- `logs` must fail clearly when no detached runtime file exists for the
  selected context name.

#### `previa open [--context <context-name>]`

- Opens the Previa UI in the user's default browser for the selected detached
  context.
- Accepts `--context <context-name>` and defaults to `default` when omitted.
- Reads the runtime file for the selected context name.
- Uses the recorded detached `main.address` and `main.port` to construct the
  local `previa-main` URL.
- The opened URL must be the recorded `previa-main` URL after loopback
  normalization, for example `http://127.0.0.1:5588`.
- The `main` URL must use the `http` scheme.
- When the recorded `main.address` is an unspecified bind address such as
  `0.0.0.0` or `::`, `open` must normalize it to the local loopback address
  before opening it.
- Fails clearly when no detached runtime file exists for the selected context
  name.
- Prints the opened UI URL to stdout after the browser launch succeeds.

#### `previa version`

- Prints the `previa` binary version.
- Does not inspect running processes.

## Filesystem Layout

v1 uses `PREVIA_HOME` as the base directory for all `previa`-generated
files.

- Environment variable:
  - `PREVIA_HOME`
- Global CLI override:
  - `--home <path>`
- Resolution precedence:
  1. `--home <path>`
  2. `PREVIA_HOME`
  3. `$HOME/.previa`
- Default value when `PREVIA_HOME` is not set:
  - `$HOME/.previa`
- Directory layout:
  - `PREVIA_HOME/bin/previa-main`
  - `PREVIA_HOME/bin/previa-runner`
  - `PREVIA_HOME/stacks/<context-name>/config/main.env`
  - `PREVIA_HOME/stacks/<context-name>/config/runner.env`
  - `PREVIA_HOME/stacks/<context-name>/data/main/orchestrator.db`
  - `PREVIA_HOME/stacks/<context-name>/logs/main.log`
  - `PREVIA_HOME/stacks/<context-name>/logs/runners/<port>.log`
  - `PREVIA_HOME/stacks/<context-name>/run/state.json`
  - `PREVIA_HOME/stacks/<context-name>/run/lock`

Any `previa` command that writes files must create parent directories as
needed.

In v1, context naming isolates runtime state, generated config, and generated data
under `PREVIA_HOME/stacks/<context-name>/`.

## Runtime State

### Detached Runtime File

Path pattern: `PREVIA_HOME/stacks/<context-name>/run/state.json`

Lock path pattern: `PREVIA_HOME/stacks/<context-name>/run/lock`

Schema:

```json
{
  "name": "default",
  "mode": "detached",
  "started_at": "2026-03-11T16:25:00Z",
  "source": "/workspace/demo/previa-compose.yaml",
  "main": {
    "pid": 41021,
    "address": "0.0.0.0",
    "port": 5588,
    "log_path": "/home/assis/.previa/stacks/default/logs/main.log"
  },
  "runner_port_range": {
    "start": 55880,
    "end": 55979
  },
  "attached_runners": ["http://10.0.0.12:55880"],
  "runners": [
    {
      "address": "127.0.0.1",
      "pid": 41022,
      "port": 55880,
      "log_path": "/home/assis/.previa/stacks/default/logs/runners/55880.log"
    },
    {
      "address": "127.0.0.1",
      "pid": 41023,
      "port": 55881,
      "log_path": "/home/assis/.previa/stacks/default/logs/runners/55881.log"
    }
  ]
}
```

Rules:

- `previa up --detach --context <context-name>` must fail if the runtime file for
  that context already exists.
- The runtime file is written only after all child processes have been spawned
  successfully.
- The runtime file must be written atomically by writing a temporary file in
  the stack-specific `run/` directory and renaming it into place.
- `up`, `down`, and `restart` must acquire an exclusive lock on the context lock
  file before mutating runtime state or process state for that context.
- Locking is per context name; operations against different context names may
  proceed concurrently.
- `status`, `list`, and `ps` do not require an exclusive lock.
- `previa down` reads this file, terminates the recorded local processes,
  waits for them to stop, and then removes the file when stopping the full
  stack.
- `previa down --runner <selector>` rewrites this file after removing the
  selected local runner entries.
- `previa restart` reads this file, stops the recorded local processes, and
  uses the recorded main address, main port, local runner addresses, local
  runner count, `runner_port_range`, `attached_runners`, and compose `source`
  path when present to launch a new detached context.
- The runtime file must persist the effective context name in `name`.
- The runtime file must persist the resolved compose file path in `source` when
  `up` started from a compose file.
- The runtime file must persist `log_path` for the detached `previa-main`
  process and for each detached local runner.
- `previa status` reads this file and reports `running`, `degraded`, or
  `stopped` based on file presence, PID liveness, and local `GET /health`
  probe results.
- The runtime file must persist attached runner endpoints for status reporting
  and `RUNNER_ENDPOINTS` introspection in normalized full-URL form.
- If one or more recorded local PIDs no longer exist, `down` continues shutting
  down the remaining recorded local processes and still removes the runtime
  file.

## Health Model

- `previa status`, `list`, and `ps` must probe detached local processes
  using `GET /health`.
- The `previa-main` health URL is
  `http://<main.address>:<main.port>/health`.
- Each local runner health URL is
  `http://<runner.address>:<runner.port>/health`.
- A probe result is `healthy` only when the HTTP request succeeds and returns
  `200 OK`.
- A process is:
  - `running` when its PID is alive and its health probe is `healthy`
  - `degraded` when its PID is alive but its health probe is not `healthy`
  - `stopped` when its PID is no longer alive
- Attached runner endpoints are not process-managed by `previa`, so v1 does
  not include them in the overall stack health calculation beyond reporting
  them as configured endpoints.

## Status JSON Schema

`previa status --json` must emit a single JSON object with this exact shape:

```json
{
  "name": "default",
  "state": "running",
  "runtime_file": "/home/assis/.previa/stacks/default/run/state.json",
  "main": {
    "state": "running",
    "pid": 41021,
    "address": "0.0.0.0",
    "port": 5588,
    "health_url": "http://0.0.0.0:5588/health",
    "log_path": "/home/assis/.previa/stacks/default/logs/main.log"
  },
  "runners": [
    {
      "state": "running",
      "pid": 41022,
      "address": "127.0.0.1",
      "port": 55880,
      "health_url": "http://127.0.0.1:55880/health",
      "log_path": "/home/assis/.previa/stacks/default/logs/runners/55880.log"
    }
  ],
  "attached_runners": ["http://10.0.0.12:55880"]
}
```

Rules:

- `state` values are fixed to `running`, `degraded`, or `stopped`.
- `runtime_file` must always be an absolute path.
- `main` must be `null` when no detached runtime file exists.
- `runners` must be an empty array when no detached runtime file exists.
- `attached_runners` must be an empty array when no detached runtime file
  exists.

## List JSON Schema

`previa list --json` must emit a JSON array of stack entries with this exact
shape:

```json
[
  {
    "name": "default",
    "state": "running",
    "runtime_file": "/home/assis/.previa/stacks/default/run/state.json"
  },
  {
    "name": "api",
    "state": "degraded",
    "runtime_file": "/home/assis/.previa/stacks/api/run/state.json"
  }
]
```

Rules:

- Each entry must contain `name`, `state`, and `runtime_file`.
- `state` values are fixed to `running`, `degraded`, or `stopped`.
- `runtime_file` must always be an absolute path.

## PS JSON Schema

`previa ps --json` must emit a JSON array of tracked local process entries
with this exact shape:

```json
[
  {
    "role": "main",
    "state": "running",
    "pid": 41021,
    "address": "0.0.0.0",
    "port": 5588,
    "health_url": "http://0.0.0.0:5588/health",
    "log_path": "/home/assis/.previa/stacks/default/logs/main.log"
  },
  {
    "role": "runner",
    "state": "running",
    "pid": 41022,
    "address": "127.0.0.1",
    "port": 55880,
    "health_url": "http://127.0.0.1:55880/health",
    "log_path": "/home/assis/.previa/stacks/default/logs/runners/55880.log"
  }
]
```

Rules:

- Each entry must contain `role`, `state`, `pid`, `address`, `port`,
  `health_url`, and `log_path`.
- `role` values are fixed to `main` or `runner`.
- `state` values are fixed to `running`, `degraded`, or `stopped`.
- When no detached runtime file exists, `ps --json` must emit an empty array.

## Configuration Model

`previa` must reuse the environment variables already supported by the
existing binaries.

### `previa-compose`

Supported filenames:

- `previa-compose.yaml`
- `previa-compose.yml`
- `previa-compose.json`

Supported top-level schema:

- `version: integer` required
- `main.address: string` optional
- `main.port: integer` optional
- `main.env: object<string, string>` optional
- `runners.local.address: string` optional
- `runners.local.count: integer` optional
- `runners.local.port_range.start: integer` optional
- `runners.local.port_range.end: integer` optional
- `runners.local.env: object<string, string>` optional
- `runners.attach: string[]` optional

Example YAML:

```yaml
version: 1

main:
  address: 0.0.0.0
  port: 6688
  env:
    ORCHESTRATOR_DATABASE_URL: sqlite:///home/assis/.previa/stacks/default/data/main/orchestrator.db
    RUST_LOG: info

runners:
  local:
    address: 127.0.0.1
    count: 3
    port_range:
      start: 56000
      end: 56009
    env:
      RUST_LOG: info
  attach:
    - 10.0.0.12:55880
    - 10.0.0.13
```

Example JSON:

```json
{
  "version": 1,
  "main": {
    "address": "0.0.0.0",
    "port": 6688,
    "env": {
      "ORCHESTRATOR_DATABASE_URL": "sqlite:///home/assis/.previa/stacks/default/data/main/orchestrator.db",
      "RUST_LOG": "info"
    }
  },
  "runners": {
    "local": {
      "address": "127.0.0.1",
      "count": 3,
      "port_range": {
        "start": 56000,
        "end": 56009
      },
      "env": {
        "RUST_LOG": "info"
      }
    },
    "attach": ["10.0.0.12:55880", "10.0.0.13"]
  }
}
```

Rules:

- `version` is required and must equal `1` in v1.
- `main.address` maps to the `ADDRESS` environment variable for
  `previa-main`.
- `main.port` is equivalent to `--main-port` / `-p`.
- `main.env` injects additional environment variables into the `previa-main`
  child process.
- `runners.local.address` maps to the `ADDRESS` environment variable for
  spawned local `previa-runner` processes.
- `runners.local.count` is equivalent to `--runners`.
- `runners.local.port_range.start` and `runners.local.port_range.end` together are
  equivalent to `--runner-port-range` / `-P`.
- `runners.local.env` injects additional environment variables into spawned
  local `previa-runner` child processes.
- `runners.attach` entries use the same selector grammar as
  `--attach-runner` / `-a`.
- `runners.local` and `runners.attach` are independent; a compose file may
  define only local runners, only attached runners, or both.
- CLI flags always override values loaded from the compose file.
- Effective environment variable precedence for each child process is:
  - CLI-derived values that `previa` must control directly
  - compose `env`
  - the stack-scoped `main.env` or `runner.env`
  - built-in defaults from this specification
- `RUNNER_ENDPOINTS` is always controlled by `previa` and must not be taken
  from `main.env` in the compose file.
- The compose file is read-only input. `previa` must never rewrite it.

### `main.env`

Path pattern: `PREVIA_HOME/stacks/<context-name>/config/main.env`

Default content:

```dotenv
ADDRESS=0.0.0.0
PORT=5588
ORCHESTRATOR_DATABASE_URL=sqlite://$HOME/.previa/stacks/<context-name>/data/main/orchestrator.db
RUNNER_ENDPOINTS=http://127.0.0.1:55880
RUST_LOG=info
```

Notes:

- `ORCHESTRATOR_DATABASE_URL` must use an absolute path inside
  `PREVIA_HOME/stacks/<context-name>/data/main/orchestrator.db`.
- `up` reads this file when present and must not rewrite it.

### `runner.env`

Path pattern: `PREVIA_HOME/stacks/<context-name>/config/runner.env`

Default content:

```dotenv
ADDRESS=127.0.0.1
PORT=55880
RUST_LOG=info
```

Notes:

- `up` reads this file when present and must not rewrite it.

## Runtime Rules

`previa up` is the v1 bootstrap command for local development, single-host
evaluation, and hybrid local-plus-remote runner attachment.

Rules:

- It is local-only and does not provision remote hosts.
- It uses the installed binaries from `PREVIA_HOME/bin`.
- It may resolve a `previa-compose` document from `.`, a directory path, or an
  explicit file path passed as the positional `<source>`.
- It accepts `--context <context-name>` to scope runtime state and detached context
  control.
- The effective context name defaults to `default`.
- It must resolve context-scoped generated paths under
  `PREVIA_HOME/stacks/<context-name>/`.
- It always executes one `previa-main`.
- It accepts `--main-address <address>` to override the `ADDRESS` environment
  variable passed to the `previa-main` child process.
- It accepts `--main-port <port>` / `-p <port>` to override the `PORT`
  environment variable passed to the `previa-main` child process.
- It executes exactly the local runner count declared by the operator in
  `--runners <N>`.
- It accepts `--runner-address <address>` to override the `ADDRESS`
  environment variable passed to spawned local `previa-runner` processes.
- It accepts `--runner-port-range <start:end>` / `-P <start:end>` to define the
  inclusive local port interval available for spawned runners.
- It may attach existing runner targets declared through repeated
  `--attach-runner <selector>` or `-a <selector>` flags.
- It accepts `--dry-run` to validate and render the effective stack plan
  without starting processes.
- It may load `main.address`, `main.port`, `main.env`, `runners.local.address`,
  `runners.local.count`, `runners.local.port_range`, `runners.local.env`, and
  `runners.attach` from a compose file.
- It must reject `up` if `--runners 0` is combined with no
  `--attach-runner` / `-a`.
- `previa-main` binds to the configured `ADDRESS` and `PORT` from
  the context-scoped `main.env` when present, except that `PORT` is overridden by
  `--main-port <port>` / `-p <port>` when provided.
- `previa-main` may also take its effective `ADDRESS` from `main.address` in a
  compose file and additional environment variables from `main.env`.
- Each local spawned runner binds to the effective runner `ADDRESS` from
  the context-scoped `runner.env` or `runners.local.address` in a compose file,
  uses ports from the effective runner port range in ascending order, and may
  receive additional environment variables from `runners.local.env`.
- The effective runner `ADDRESS` defaults to `127.0.0.1`.
- The effective runner port range defaults to `55880:55979`.
- `up` must fail before spawning any local child process when the requested
  local runner count exceeds the capacity of the effective runner port range.
- The command must override `RUNNER_ENDPOINTS` for the `previa-main` child
  process so that it points to all local spawned runners followed by all
  attached runner endpoints after selector normalization.
- Attached runner endpoints are treated as externally managed and are never
  spawned, restarted, or terminated by `previa`.
- If a compose file is used, `up` must resolve it to an absolute path before
  recording it in runtime state.
- When `--detach` is used, stdout and stderr must be redirected to context-scoped
  log files under `PREVIA_HOME/stacks/<context-name>/logs/`.
- Detached runtime state for different context names must be isolated from one
  another by using separate runtime files.
- Different context names must use different context-scoped config, runtime, and
  generated data paths by default.
- If any child process fails during startup, the command must terminate the
  remaining local children and exit with a non-zero status.

## Error Handling

The implementation must surface explicit user-facing errors for:

- Missing `PREVIA_HOME/bin/previa-main`.
- Missing `PREVIA_HOME/bin/previa-runner` when local runners are requested.
- Invalid `--attach-runner <selector>` / `-a <selector>` value.
- Invalid `--context <context-name>` value.
- Invalid `--main-address <address>` value.
- Invalid `--runner-address <address>` value.
- Missing compose file when `<source>` is provided.
- Unsupported compose file extension when `<source>` is a file path.
- Invalid YAML or JSON in a compose file.
- Missing `version` in a compose file.
- Unsupported compose file `version`.
- Invalid compose file schema.
- Invalid `--main-port <port>` / `-p <port>` value.
- Invalid `--runner-port-range <start:end>` / `-P <start:end>` value.
- Requested local runner count exceeds the effective runner port range
  capacity.
- Requested local bind target already in use.
- Invalid use of `--dry-run` together with `--detach`.
- Existing detached runtime file for the selected context name during
  `up --detach`.
- Lock contention on the selected context name during `up`, `down`, or
  `restart`.
- Missing detached runtime file for the selected context name during `down`.
- Unknown local runner selector during `down --runner <selector>`.
- Attempted `down --runner <selector>` that would leave the stack with zero runner
  sources.
- Mutually exclusive `down --all-contexts` and `down --runner <selector>`.
- Missing detached runtime file for the selected context name during `restart`.
- Mutually exclusive `status --main` and `status --runner <selector>`.
- Unknown local runner selector during `status --runner <selector>`.
- Failure to format or emit `ps --json` according to the documented schema.
- Mutually exclusive `logs --main` and `logs --runner <selector>`.
- Unknown local runner selector during `logs --runner <selector>`.
- Missing detached runtime file for the selected context name during `logs`.
- Missing detached runtime file for the selected context name during `open`.
- Health probe failure due to invalid local status target URL construction.
- Failure to read or follow a detached log file.
- Permission failures when writing inside `PREVIA_HOME`.
- Failure to spawn `previa-main` or one of the local `previa-runner`
  processes.

## Test Plan

The implementation is complete only when these scenarios are covered:

1. `version` prints the `previa` binary version without requiring network.
2. `up --context api .` uses `api` as the context name.
3. `up .` resolves `./previa-compose.yaml`, `./previa-compose.yml`, or
   `./previa-compose.json` using the documented lookup order.
4. `up /workspace/demo` resolves a compose file from that directory using the
   documented lookup order.
5. `up /workspace/demo/previa-compose.yaml` reads that exact file.
6. `up /workspace/demo/previa-compose.yaml` applies compose settings for
   `version`, main address, main port, main `env`, local runner address, local
   runner count, local runner port range, local runner `env`, and attached
   runners.
7. `up /workspace/demo/previa-compose.yaml -p 7788 --runners 2` lets the CLI flags
    override the compose file values.
8. `up /workspace/demo/previa-compose.yaml` fails clearly when `version` is
   missing.
9. `up /workspace/demo/previa-compose.yaml` fails clearly when `version` is not
   `1`.
10. `up --main-address 0.0.0.0 --runner-address 127.0.0.1 --runners 3` starts one
    `previa-main` and three local runners with the requested bind addresses.
11. `up --runners 3` starts one `previa-main`, three local runners, and injects
   `RUNNER_ENDPOINTS=http://127.0.0.1:55880,http://127.0.0.1:55881,http://127.0.0.1:55882`
   into the `previa-main` child process.
12. `up -p 6688 --runners 1` starts `previa-main` with `PORT=6688`.
13. `up -P 56000:56002 --runners 3` starts local runners on ports
   `56000`, `56001`, and `56002`.
14. `up -P 56000:56001 --runners 3` fails validation before spawning
   any local child process because the range capacity is insufficient.
15. `up` fails before spawning any child process when a requested local main or
    runner bind target is already in use.
16. `up --runners 1 -a 10.0.0.12:55880` injects
   `RUNNER_ENDPOINTS=http://127.0.0.1:55880,http://10.0.0.12:55880`
   into the `previa-main` child process.
17. `up --runners 0 -a 10.0.0.12:55880` is valid and starts only `previa-main`
   locally while attaching the remote runner endpoint.
18. `up --runners 0` with no attached runner fails validation before spawning any
   process.
19. `up -a 55880` normalizes the attached runner target to
   `http://127.0.0.1:55880`.
20. `up -a 10.0.0.12` normalizes the attached runner target to
   `http://10.0.0.12:55880`.
21. `up -a 10.0.0.12:55880` normalizes the attached runner target to
   `http://10.0.0.12:55880`.
22. `up -a bad:value:123` fails clearly because the attached runner selector is
    invalid.
23. `up --dry-run --context api /workspace/demo/previa-compose.yaml` prints the
    resolved effective stack plan and writes no runtime, lock, or log files.
24. `up --dry-run --detach` fails clearly because dry-run cannot detach.
25. `up /workspace/demo/previa-compose.yaml --detach --context api` writes the
    resolved absolute compose file path to
    `PREVIA_HOME/stacks/api/run/state.json`.
26. `up --runners 3 --detach` writes
    `PREVIA_HOME/stacks/default/run/state.json` with the
   `previa-main` PID and the three runner PIDs, then exits without stopping the
   spawned processes.
27. Detached runtime state persists the context name, effective main address,
   main port,
   runner addresses, runner port range, attached runner endpoints, and log
   paths when `up --detach` is used.
28. `up --detach` redirects `previa-main` output to
    `PREVIA_HOME/stacks/default/logs/main.log`.
29. `up --detach` redirects a local runner on port `55880` to
    `PREVIA_HOME/stacks/default/logs/runners/55880.log`.
30. `up --context api --detach` and `up --context jobs --detach` can coexist because
    they use different runtime files.
31. `up --context api --detach` fails clearly when
    `PREVIA_HOME/stacks/api/run/state.json` already exists.
32. Concurrent mutating operations against the same context name fail clearly on
    lock contention through `PREVIA_HOME/stacks/api/run/lock`.
33. `status --context api` reports `running` when all PIDs in
    `PREVIA_HOME/stacks/api/run/state.json` are alive.
34. `status --context api` reports `degraded` when the `previa-main` PID is alive
    but `GET /health` does not return `200 OK`.
35. `status --context api` reports `degraded` when a local runner PID is alive
    but its `GET /health` does not return `200 OK`.
36. `status --json --context api` prints a stable JSON document for context `api`
    matching the documented schema.
37. `status` without `--context` targets the `default` context.
38. `status` reports `degraded` when the runtime file exists but one or more
    recorded local PIDs are no longer alive.
39. `status` reports `stopped` when no detached runtime file exists for the
    selected context name.
40. `status --main` reports only the status of the recorded `previa-main`
    process, including its health result.
41. `status --runner 55880` reports the status of the recorded local runner on
    port `55880`.
42. `status --runner 127.0.0.1:55880` reports the status of the recorded local
    runner bound to `127.0.0.1:55880`.
43. `status --runner 127.0.0.1` reports the status of all recorded local
    runners bound to `127.0.0.1`.
44. `status --runner 55880` fails clearly when the selector does not match any
    local runner entry in the runtime file.
45. `status --main --runner 55880` fails clearly because the filters are
    mutually exclusive.
46. `list` reports all known context names under `PREVIA_HOME/stacks/` with their
    current states.
47. `list --json` prints a stable JSON array matching the documented schema.
48. `ps --context api` prints one row for the detached `previa-main` process and
    one row per detached local runner process tracked in the runtime file.
49. `ps --json --context api` prints a stable JSON array matching the documented
    schema.
50. `ps` prints no rows and exits successfully when no detached runtime file
    exists for the selected context.
51. `logs --context api --main` reads
    `PREVIA_HOME/stacks/api/logs/main.log`.
52. `logs --context api --runner 127.0.0.1:55880` reads the matching local runner
    log file.
53. `logs --context api --tail 20` prints only the last 20 lines of the selected
    log file or files.
54. `logs --context api --follow` streams appended log lines until interrupted.
55. `logs --main --runner 55880` fails clearly because the filters are mutually
    exclusive.
56. `open --context api` opens the recorded detached `previa-main` URL directly.
57. `down --context api` reads `PREVIA_HOME/stacks/api/run/state.json`, terminates the
    recorded local processes, waits for shutdown, and removes the runtime file.
58. `down` without `--context` targets the `default` context.
59. `down` fails clearly when no detached runtime file exists for the selected
    context name.
60. `down --runner 55880` stops only the recorded local runner on port `55880`
    and rewrites the selected stack runtime file with the remaining runner
    entries.
61. `down --runner 127.0.0.1:55880` stops only the recorded local runner bound
    to `127.0.0.1:55880`.
62. `down --runner 127.0.0.1` stops all recorded local runners bound to
    `127.0.0.1`.
63. `down --runner 55880 --runner 55881` stops only the selected local runners
    and preserves `previa-main` plus any remaining local runners and attached
    runner endpoints.
64. `down --runner 55880` fails clearly when the selector does not match any
    local runner entry in the runtime file.
65. `down --runner 55880` fails clearly if removing that runner would leave the
    context with zero runner sources overall.
66. `down` does not attempt to terminate attached runner endpoints.
67. `down --all-contexts` stops every detached context with a runtime file under
    `PREVIA_HOME/stacks/`.
68. `restart --context api` reads `PREVIA_HOME/stacks/api/run/state.json`, stops the
    detached local processes, starts a new detached context with the same runner
    topology, and rewrites the runtime file with new PIDs.
69. `restart` without `--context` targets the `default` context.
70. `restart` preserves the recorded main port and runner port range from the
   runtime file.
71. `restart` fails clearly when no detached runtime file exists for the
    selected context name.
72. `up --detach` fails clearly when
    `PREVIA_HOME/stacks/default/run/state.json` already exists.
73. Any file generated by `previa` is written under `PREVIA_HOME`.

## Rollback and Recovery

- Automatic rollback is out of scope for v1.
- If `up` fails before detached runtime state is written, the command must
  terminate already spawned child processes before exiting.
- If `down` encounters one or more missing local PIDs, it must continue
  processing the remaining recorded local processes and then remove the runtime
  file.
- If `down --runner <selector>` stops some requested local runners and then fails
  before rewriting the runtime file, the operator must reconcile the runtime
  file manually before the next `status`, `down`, or `restart`.
- If `restart` fails after stopping the previous detached context but before the
  new detached context is fully ready, the operator must rerun `previa up` or
  `previa restart` manually.

## Security and Known Risks

- No checksum verification is available in v1.
- No signature verification is available in v1.

## Implementation Notes

- The crate is named `previa`.
- It should remain separate from HTTP transport concerns and reuse dedicated
  modules for runtime state persistence, process spawning, endpoint
  validation, health probing, log access, and teardown behavior.
- The CLI must target the existing `previa-main` and `previa-runner` contracts
  without requiring changes to those binaries for v1.
