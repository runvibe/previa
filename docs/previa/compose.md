# Compose Source

This guide documents the `previa-compose` input format used by `previa up`.

## Supported Filenames

- `previa-compose.yaml`
- `previa-compose.yml`
- `previa-compose.json`

To create a starter YAML file in the current directory:

```bash
previa init
```

## Using a Compose Source

Examples:

```bash
previa up .
previa up ./environments/dev
previa up ./previa-compose.yaml
```

`SOURCE` may be:

- `.`
- a directory
- an explicit compose file path

When `SOURCE` is `.` or a directory, the lookup order is:

1. `previa-compose.yaml`
2. `previa-compose.yml`
3. `previa-compose.json`

## Configuration Precedence

`previa up` resolves configuration in this order:

1. CLI flags
2. compose source values
3. `main.env` and `runner.env` from the selected context
4. built-in defaults

`previa` also injects runtime-managed values such as:

- `PREVIA_CONTEXT`
- `DATABASE_URL`
- `PREVIA_QUEUE_DATABASE_URL`

The generated runtime always includes Postgres 17, persistent storage, a
healthcheck, and distinct main/runner credentials. SQLite is only used by the
explicit project import/export commands.

## Supported Shape

Top-level fields:

- `version`
- `main.address`
- `main.port`
- `main.env`
- `runners.local.address`
- `runners.local.count`
- `runners.local.port_range.start`
- `runners.local.port_range.end`
- `runners.local.env`
- `runners.attach`

`version` is required and must be `1`.

## Example YAML

```yaml
version: 1
main:
  address: 0.0.0.0
  port: 5588
  env:
    RUST_LOG: info
runners:
  local:
    address: 127.0.0.1
    count: 2
    port_range:
      start: 55880
      end: 55889
    env:
      RUST_LOG: info
  attach:
    - 10.0.0.12:55880
```

## Notes

- `runners.local.count` maps to `--runners`
- `runners.attach` uses the same selector grammar as `--attach-runner`
- CLI flags always override compose values
- the compose source is read-only input
- `previa` never rewrites the input compose file

Execution transport is authenticated by the restricted Postgres runner role,
not by an HTTP runner auth key.

For the exact schema and compatibility rules, see the spec.

## See Also

- [Up and runtime](./up-and-runtime.md)
- [Home and contexts](./home-and-contexts.md)
- [CLI specification](../specs/previa-v1.md)
