# Operations

This guide covers the day-to-day commands used after a detached stack exists.

## `previa pull`

Pull published images:

```bash
previa pull
previa pull main
previa pull runner --version 0.0.7
```

## `previa status`

Show overall state:

```bash
previa status
previa status --main
previa status --runner 55880
previa status --json
```

State is derived from:

- PID liveness
- `GET /health` for local processes

Possible states:

- `running`
- `degraded`
- `stopped`

Human output example:

```text
default  running
main     running  12345  0.0.0.0:5588
runner   running  12346  127.0.0.1:55880
attached http://10.0.0.12:55880
```

JSON output includes:

- context name
- overall state
- runtime file path
- main process details
- local runners
- attached runners

## `previa list`

List all known contexts under `PREVIA_HOME/stacks`:

```bash
previa list
previa list --json
```

Typical human output:

```text
default  running
other    stopped
```

## `previa ps`

Show recorded local processes for a context:

```bash
previa ps
previa ps --context other --json
```

This shows recorded local process metadata such as:

- role
- state
- pid
- address and port
- health URL
- log path

## `previa logs`

Read logs from a detached runtime:

```bash
previa logs
previa logs --main
previa logs --runner 55880
previa logs --follow
previa logs --tail 20
previa logs -t
```

Notes:

- `--main` and `--runner` are mutually exclusive
- `-t` without a value means `10`
- `-t 0` fails
- without filters, logs include `main` and all local runners in port order

## `previa open`

Open the UI served by the selected detached `previa-main`:

```bash
previa open
previa open --context other
```

It opens:

```text
http://127.0.0.1:5588
```

If the recorded main address is `0.0.0.0` or `::`, `previa` normalizes it to
loopback before building the URL. The embedded app uses the opened origin as the
API base.

You can override the browser command with `PREVIA_OPEN_BROWSER`.

## `previa restart`

Restart a detached context using saved runtime config:

```bash
previa restart
previa restart --context other
```

Restart reuses the recorded runtime configuration for the detached context.

## `previa down`

Stop a detached context:

```bash
previa down
previa down --context other
previa down --runner 55880
previa down --all-contexts
```

Rules:

- `--all-contexts` and `--runner` are mutually exclusive
- `--runner` affects only local recorded runners
- attached runners are never stopped by `previa`
- removing the last local runners fails if no attached runners remain

## `previa version`

Show the compiled CLI version:

```bash
previa version
previa --version
```

## See Also

- [CLI commands](./cli-commands.md)
- [Getting started](./getting-started.md)
- [Home and contexts](./home-and-contexts.md)
- [Troubleshooting](./troubleshooting.md)
