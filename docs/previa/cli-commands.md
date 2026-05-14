# CLI Commands

This guide explains every command currently exposed by the `previa` CLI.

For fast day-to-day usage, see [Operations cheatsheet](./operations-cheatsheet.md). For deeper runtime behavior, see [Up and runtime](./up-and-runtime.md) and [Operations](./operations.md).

## Command Overview

Top-level help:

```bash
previa --help
previa help
previa help up
```

Current commands:

- `login`
- `logout`
- `whoami`
- `token`
- `init`
- `local`
- `up`
- `mcp`
- `runner`
- `pull`
- `down`
- `restart`
- `status`
- `list`
- `ps`
- `logs`
- `open`
- `export`
- `version`
- `help`

## Global Option

`previa` supports one global option:

```text
--home <PATH>
```

Resolution order for the runtime home is:

1. `--home <PATH>`
2. `PREVIA_HOME`
3. `$HOME/.previa`

Example:

```bash
previa --home ./.previa up -d
previa --home ./.previa status
```

For repository-local workflows, prefer the `local` command:

```bash
previa local up -d
previa local status
previa local open
previa local down
```

`previa local ...` uses `./.previa` as the runtime home unless an explicit
global `--home <PATH>` is provided.

## `previa login`

Authenticates to a protected `previa-main` and stores a fixed API token for CLI,
MCP, and direct API usage.

```text
previa login (--context <CONTEXT> | --url <URL>) --username <USERNAME> --password-stdin
```

Important options:

- `--context <CONTEXT>`: select a detached local context, default `default`
- `--url <URL>`: authenticate to an explicit remote `previa-main` URL instead
  of a local context
- `--username <USERNAME>`: user or environment root username
- `--password-stdin`: read the password from standard input

Examples:

```bash
printf '%s' 'change-me' | previa login --context default --username root --password-stdin
printf '%s' 'change-me' | previa login --url https://previa.example.com --username root --password-stdin
previa whoami --context default
```

Notes:

- `previa login` requests `clientKind=api_token`, not an app JWT
- the token is stored under the selected context config directory, or under
  `PREVIA_HOME/auth/` when `--url` is used
- `PREVIA_API_TOKEN` takes precedence over the stored token when set
- in anonymous mode, login is unnecessary and the server rejects login with a
  conflict response

## `previa logout`

Removes a locally stored API token.

```text
previa logout [--context <CONTEXT> | --url <URL>]
```

Examples:

```bash
previa logout --context default
previa logout --url https://previa.example.com
```

## `previa whoami`

Shows the current authenticated principal for a protected context or URL.

```text
previa whoami [--context <CONTEXT> | --url <URL>]
```

Examples:

```bash
previa whoami --context default
PREVIA_API_TOKEN='pvk_...' previa whoami --url https://previa.example.com
```

## `previa token`

Manages fixed API tokens for CLI, MCP, CI, scripts, and direct API usage.

```text
previa token list [--context <CONTEXT> | --url <URL>] [--json]
previa token create [--context <CONTEXT> | --url <URL>] --name <NAME> [--role <ROLE>]
previa token revoke [--context <CONTEXT> | --url <URL>] <TOKEN_ID>
previa token use [--context <CONTEXT> | --url <URL>] --token-env <ENV_NAME>
```

Supported roles are `root`, `admin`, `editor`, `operator`, and `viewer`.

Examples:

```bash
previa token list --context default
previa token list --context default --json
previa token create --context default --name ci --role operator
previa token create --url https://previa.example.com --name mcp --role editor
previa token revoke --context default token_123
previa token use --context default --token-env PREVIA_API_TOKEN
```

Notes:

- `token create` prints the raw token only once; store it when it is created
- `token use` stores an existing token from an environment variable in the same
  local auth file used by `previa login`
- listing, creating, and revoking tokens require a stored token or
  `PREVIA_API_TOKEN`

## `previa local`

Runs common Previa commands with a project-local runtime home.

```text
previa local <COMMAND>
```

Supported commands:

- `up`
- `push`
- `import`
- `export`
- `runner`
- `down`
- `status`
- `logs`
- `open`

Examples:

```bash
previa local up -d
previa local push --project my_app --to https://previa.example.com
previa local push --project my_app --to https://previa.example.com --overwrite
previa local runner list
previa local runner add 10.0.0.12:55880 --name staging-a
previa local status
previa local open
previa local logs
previa local down
```

Notes:

- `local up` keeps the same behavior as `up`; pass `-d` or `--detach` for detached mode.
- `previa --home ./custom local status` uses `./custom`, because explicit `--home` wins.

### `previa local runner`

Runs the same runner registry commands as `previa runner`, using the project-local runtime home `./.previa`.

Examples:

```bash
previa local runner list
previa local runner add 10.0.0.12:55880 --name staging-a
previa local runner disable staging-a
previa local runner enable staging-a
previa local runner remove staging-a
```

### `previa local push`

Pushes a project from the project-local context to a remote `previa-main`.

```text
previa local push --project <ID_OR_NAME> --to <REMOTE_URL> [OPTIONS]
```

Important options:

- `--project <ID_OR_NAME>`: local project selector, by ID or exact name
- `--to <REMOTE_URL>`: remote `previa-main` base URL
- `--overwrite`: replace an existing remote project instead of failing
- `--include-history`: include E2E and load history in the pushed snapshot
- `--remote-project-id <PROJECT_ID>`: select the remote project to replace explicitly

Behavior:

- without `--overwrite`, `push` creates the remote project only when no matching remote project exists
- with `--overwrite`, `push` deletes the matched remote project and imports the local snapshot
- matching checks the local project ID first, then exact project name
- by default, execution history is not pushed

Examples:

```bash
previa local push --project my_app --to https://previa.example.com
previa local push --project my_app --to https://previa.example.com --overwrite
previa local push --project my_app --to https://previa.example.com --overwrite --include-history
previa local push --project my_app --to https://previa.example.com --remote-project-id prj_123 --overwrite
```

### `previa local export`

Exports selected projects from the project-local context to a SQLite database.

```text
previa local export (--all | --project <PROJECT_ID>...) --output <DB_SQLITE3> [OPTIONS]
```

Important options:

- `--all`: export every project in the local context
- `--project <PROJECT_ID>`: export one selected project; repeat for multiple projects
- `--output <DB_SQLITE3>`: destination SQLite file
- `--overwrite`: replace an existing destination file
- `--no-history`: omit E2E and load history from the export

Examples:

```bash
previa local export --all --output ./previa-projects.sqlite3
previa local export --project project_a --project project_b --output ./selected.sqlite3
previa local export --all --output ./previa-projects.sqlite3 --overwrite --no-history
```

### `previa local import`

Imports every project from a SQLite database into the project-local context.

```text
previa local import <DB_SQLITE3> [OPTIONS]
```

If an imported project name already exists, Previa keeps both projects and
renames the imported project with `-imported`, `-imported-2`, and so on.

Important options:

- `--no-history`: omit E2E and load history while importing
- `--context <CONTEXT>`: import into a specific local context

Examples:

```bash
previa local import ./previa-projects.sqlite3
previa local import ./previa-projects.sqlite3 --no-history
```

## `previa init`

Creates a starter `previa-compose.yaml` in the current directory.

```text
previa init [OPTIONS]
```

Important options:

- `--force`: overwrite an existing `previa-compose.yaml`

Examples:

```bash
previa init
previa init --force
previa up -d .
```

Notes:

- the command always writes `previa-compose.yaml` in the current working directory
- by default it fails if the file already exists
- the generated file is a valid compose source for `previa up`

## `previa up`

Starts a Previa stack for one context.

```text
previa up [OPTIONS] [SOURCE]
```

Main uses:

- start a Docker-backed stack
- start a binary-backed stack with `--bin` on Linux
- apply a `previa-compose` source
- attach remote runners
- import pipelines after startup

Important options:

- `--context <CONTEXT>`: selects the context name, default `default`
- `[SOURCE]`: compose source directory or file
- `--main-address <ADDR>` and `--main-port <PORT>`: override main bind target
- `--runner-address <ADDR>` and `--runner-port-range <START:END>`: override local runner binds
- `--runners <N>`: number of local runners to start
- `--attach-runner <RUNNER>`: attach an existing runner endpoint; may be repeated
- `--import <PATH>`: import one file or a directory of pipeline files after startup
- `--recursive`: required when recursively importing a directory
- `--stack <STACK>`: required when using `--import`
- `--dry-run`: prints the planned runtime without starting it
- `-d, --detach`: starts the stack in detached mode
- `--protected`: disables anonymous access and enables access management
- `--anonymous`: explicitly keeps anonymous full access enabled
- `--root-username <USERNAME>`: sets the environment root username for
  protected mode, default `root`
- `--root-password-stdin`: reads the environment root password from standard
  input for protected mode
- `--bin`: Linux-only, uses local binaries instead of container images
- `--version <TAG>`: image tag for compose-backed runtimes, default is the current CLI version

Examples:

```bash
previa up
previa up -d
previa up -d --bin
previa up ./
previa up ./previa-compose.yaml
previa up --context other -p 6688 -P 56880:56889 --runners 2
previa up -d --attach-runner 10.0.0.12:55880
previa up -d --import ./tests/e2e -r --stack app_e2e
printf '%s' 'change-me' | previa up -d --protected --root-username root --root-password-stdin
previa up -d --anonymous
previa up --dry-run
```

Notes:

- detached mode writes runtime state and unlocks `status`, `logs`, `ps`, `restart`, and `down`
- `--dry-run` cannot be combined with `--detach`
- `--version` is not used with `--bin`
- with `--bin`, runtime binary resolution prefers `<workspace>/target/debug`, then `<workspace>/target/release`, and only then `PREVIA_HOME/bin`
- when `--bin` cannot find a compatible local runtime binary in any of those locations, `previa` can bootstrap one into `PREVIA_HOME/bin`
- if a workspace build exists, an older copy in `PREVIA_HOME/bin` is ignored in favor of the workspace binary
- when only local runners are used and `RUNNER_AUTH_KEY` is missing, `previa up` generates one automatically
- when `--attach-runner` is used, `RUNNER_AUTH_KEY` is required
- `--protected` persists `PREVIA_AUTH_ANONYMOUS=false`, `PREVIA_ROOT_USERNAME`,
  `PREVIA_ROOT_PASSWORD`, and a generated `PREVIA_JWT_SECRET` in `main.env`
- `--anonymous` persists `PREVIA_AUTH_ANONYMOUS=true` in `main.env`
- `--protected` requires `--root-password-stdin` unless the selected context
  already has `PREVIA_ROOT_PASSWORD` in `main.env`
- on macOS and Windows, the control binary is supported but `previa up` does not expose `--bin`

See also:

- [Up and runtime](./up-and-runtime.md)
- [Compose source](./compose.md)
- [Pipeline import](./pipeline-import.md)
- [Main and runner authentication](./main-runner-auth.md)
- [Access management](./access-management.md)

## `previa runner`

Manages the dynamic runner registry stored by `previa-main`.

```text
previa runner <COMMAND>
```

Supported commands:

- `list`: list registered runners
- `add <ENDPOINT>`: add or update a runner endpoint
- `enable <ID_ENDPOINT_OR_NAME>`: enable a runner
- `disable <ID_ENDPOINT_OR_NAME>`: disable a runner
- `remove <ID_ENDPOINT_OR_NAME>`: remove a runner

Important options:

- `--context <CONTEXT>`: selects the detached context, default `default`
- `--json`: available on `list`
- `--name <NAME>`: available on `add`
- `--disabled`: registers the runner disabled on `add`

Examples:

```bash
previa runner list
previa runner list --json
previa runner add 10.0.0.12:55880 --name staging-a
previa runner add http://10.0.0.13:55880 --disabled
previa runner disable staging-a
previa runner enable 10.0.0.12:55880
previa runner remove staging-a
```

Behavior:

- endpoints are normalized to include `http://` when no scheme is provided
- selectors can match the runner ID, endpoint, or name
- `enable` and `disable` update the persistent `enabled` flag stored in the `runners` table
- disabled runners stay registered, but `previa-main` ignores them before probing `/health`
- enabled runners are probed with `/health` before execution; only healthy runners are selected
- the command talks to the selected detached `previa-main`; start the context first with `previa up -d`
- `previa local runner ...` uses the same registry commands with project-local home `./.previa`

## `previa mcp`

Installs, removes, inspects, or prints MCP client configuration for supported tools.

```text
previa mcp install <target> [OPTIONS]
previa mcp uninstall <target> [OPTIONS]
previa mcp status <target> [OPTIONS]
previa mcp print <target> [OPTIONS]
```

Supported targets in the current Linux-first release:

- `codex`
- `cursor`
- `claude-desktop`
- `claude-code`
- `warp`
- `copilot-vscode`

Important options:

- `--scope global|project`: defaults to `global`
- `--name <SERVER_NAME>`: defaults to `previa`
- `--context <CONTEXT>`: resolves the MCP URL from a detached Previa context
- `--url <MCP_URL>`: bypasses context lookup and uses an explicit MCP URL
- `--force`: replaces a conflicting named entry for `install`
- `--no-verify`: skips the `OPTIONS` check against the MCP endpoint during `install`

Examples:

```bash
previa mcp install codex --context default
previa mcp install cursor --scope project --url http://localhost:5588/mcp
previa mcp status copilot-vscode --scope project
previa mcp print claude-code --context default
previa mcp uninstall warp
```

Notes:

- `install` and `print` resolve the URL from `--url` or a detached context, defaulting to context `default`
- `claude-desktop` is manual-only in this version; use `previa mcp print claude-desktop`
- `warp` writes a Previa-managed Oz-compatible JSON file under `PREVIA_HOME/clients/warp/`
- `claude-code` is driven through the external `claude mcp ...` CLI
- global vs project scope depends on the target client

See also:

- [MCP integration](./mcp.md)
- [Troubleshooting](./troubleshooting.md)

## `previa pull`

Pulls published runtime images.

```text
previa pull [OPTIONS] [TARGET]
```

Targets:

- `main`
- `runner`
- `all` (default)

Important options:

- `--version <TAG>`: image tag to pull, default is the current CLI version

Examples:

```bash
previa pull
previa pull main
previa pull runner --version 0.0.7
previa pull all
```

This command is mainly useful for compose-backed runtimes.

## `previa down`

Stops a detached context, or selected local runners inside it.

```text
previa down [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context to stop, default `default`
- `--all-contexts`: stops every detached context under `PREVIA_HOME/stacks`
- `--runner <RUNNER>`: stops only the selected local runner; may be repeated

Examples:

```bash
previa down
previa down --context other
previa down --runner 55880
previa down --all-contexts
```

Notes:

- `--all-contexts` and `--runner` are mutually exclusive
- attached runners are never stopped by `previa`
- `--runner` only affects locally recorded runners
- removing the last local runner fails if no attached runner remains

## `previa restart`

Restarts a detached context using its saved runtime configuration.

```text
previa restart [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context to restart, default `default`
- `--version <TAG>`: only supported for compose-backed runtimes

Examples:

```bash
previa restart
previa restart --context other
previa restart --version 0.0.7
```

Notes:

- restart requires an existing detached context
- for `--bin`, restart ignores image tags and reuses the saved local runtime shape

## `previa status`

Shows the current health and state for one context.

```text
previa status [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context to inspect, default `default`
- `--main`: show only the main process
- `--runner <RUNNER>`: show only the selected runner
- `--json`: render machine-readable output

Examples:

```bash
previa status
previa status --main
previa status --runner 55880
previa status --json
```

Notes:

- `--main` and `--runner` are mutually exclusive
- state is derived from runtime metadata plus health probing when possible

## `previa list`

Lists every known context under `PREVIA_HOME/stacks`.

```text
previa list [OPTIONS]
```

Important options:

- `--json`: render machine-readable output

Examples:

```bash
previa list
previa list --json
```

Typical output shows the context name, current state, and backing runtime file.

## `previa ps`

Shows recorded local process metadata for one context.

```text
previa ps [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context to inspect, default `default`
- `--json`: render machine-readable output

Examples:

```bash
previa ps
previa ps --context other
previa ps --json
```

Typical fields include role, pid, state, address, port, health URL, and log path.

## `previa logs`

Reads logs from a detached runtime.

```text
previa logs [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context to inspect, default `default`
- `--main`: show only main logs
- `--runner <RUNNER>`: show only one runner log
- `--follow`: stream logs
- `-t, --tail [<N>]`: tail mode; when used without a value it defaults to `10`

Examples:

```bash
previa logs
previa logs --main
previa logs --runner 55880
previa logs --follow
previa logs -t
previa logs --tail 50
```

Notes:

- `--main` and `--runner` are mutually exclusive
- `-t 0` is invalid
- without filters, `previa` shows main plus all local runners
- for compose-backed runtimes, logs come from Docker Compose
- for binary-backed runtimes, logs come from files under the context log directory

## `previa open`

Opens the Previa UI served by the selected detached `previa-main`.

```text
previa open [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context to open, default `default`

Examples:

```bash
previa open
previa open --context other
```

Runtime behavior:

- builds a URL like `http://127.0.0.1:5588`
- normalizes unspecified bind addresses such as `0.0.0.0` and `::` to loopback
- opens the default browser
- prints the final URL to stdout
- if the browser launcher fails, exits with error, highlights the failure in red, and still prints the final URL for manual opening

If the main runtime is bound to `0.0.0.0` or `::`, `previa` normalizes it to loopback for the browser URL.

## `previa export pipelines`

Exports stored pipelines from one detached context into local files.

```text
previa export pipelines [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context to inspect, default `default`
- `--project <ID_OR_NAME>`: required project selector
- `--output-dir <PATH>`: required destination directory
- `--pipeline <ID_OR_NAME>`: export only selected pipelines; may be repeated
- `--format <yaml|json>`: output format, default `yaml`
- `--overwrite`: replace existing files instead of failing

Examples:

```bash
previa export pipelines --project my_app_smoke --output-dir ./tests/e2e
previa export pipelines --context other --project project-users --output-dir ./tests/e2e --format json
previa export pipelines --project my_app_smoke --output-dir ./tests/e2e --pipeline smoke --pipeline login
previa export pipelines --project my_app_smoke --output-dir ./tests/e2e --overwrite
```

Notes:

- export requires an existing detached context
- exported files are direct pipeline objects, not project bundles
- project selection accepts project ID first, then exact name matching
- if more than one project matches the provided exact name, the command fails and asks for a project ID
- if any target file already exists, the command fails unless `--overwrite` is set

See also:

- [Pipeline export](./pipeline-export.md)
- [Project repository workflow](./project-repository-workflow.md)

## `previa version`

Prints the compiled CLI version.

```text
previa version
previa --version
```

Examples:

```bash
previa version
previa --version
```

## `previa help`

Shows built-in command help from the CLI parser.

```text
previa help
previa help up
previa help logs
```

This is the fastest way to confirm the exact flags supported by the binary you are running.

## See Also

- [Getting started](./getting-started.md)
- [Operations cheatsheet](./operations-cheatsheet.md)
- [Operations](./operations.md)
- [Up and runtime](./up-and-runtime.md)
