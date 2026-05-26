# Pipeline Import

This guide documents pipeline import through `previa up`.

## Overview

`previa` can import local pipeline files after the runtime starts and the local
`previa-main` becomes healthy.

Single file:

```bash
previa up --detach --import ./api-smoke.previa.yaml --stack smoke_tests
```

Recursive directory import:

```bash
previa up --detach -i ./tests/e2e -r -s app_e2e
```

## Required Flags

When `--import` is used:

- `--stack` is required
- `--detach` is required
- `--dry-run` is not allowed

## Supported Files

Accepted suffixes:

- `.previa`
- `.previa.json`
- `.previa.yaml`
- `.previa.yml`

Rules:

- without `--recursive`, `--import` must point to a single supported file
- with `--recursive`, `--import` must point to a directory
- recursive import scans only files with the supported suffixes

## File Format

Each file must contain a direct `Pipeline` object, not a project bundle.

Expected top-level shape:

- `id` optional
- `name` required
- `description` optional
- `steps` required

Each step uses the pipeline step schema understood by `previa-runner`.

## Example

```yaml
id: api-smoke
name: API Smoke Test
description: Example pipeline for local import.
steps:
  - id: get_status
    name: Check 200
    method: GET
    url: https://httpbin.org/status/200
    headers: {}
    asserts:
      - field: status
        operator: equals
        expected: "200"
```

## Runtime Behavior

Import happens after the stack starts.

If import succeeds:

- a new local project is created with the name from `--stack`
- the imported pipelines are stored under that project

If import fails:

- `previa up` returns an error
- the runtime that already started remains running

## Common Local Workflow

Inside an app repo:

```text
my-app/
  previa-compose.yaml
  tests/
    e2e/
      login.previa.yml
      checkout.previa.yml
```

Example flow:

```bash
previa --home ./.previa up --detach
previa --home ./.previa up --detach -i tests/e2e -r -s app_e2e
previa --home ./.previa open
```

## See Also

- [Import and export](./import-export.md)
- [Getting started](./getting-started.md)
- [Up and runtime](./up-and-runtime.md)
- [Project repository workflow](./project-repository-workflow.md)
- [Troubleshooting](./troubleshooting.md)
