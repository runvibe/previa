# Pipeline Export

This guide documents pipeline export through the `previa` CLI.

## Overview

`previa export pipelines` exports stored project pipelines from a detached local context into repository-friendly files.

Default usage:

```bash
previa export pipelines --project my_app_smoke --output-dir ./tests/e2e
```

By default, the command:

- reads the detached runtime state for the selected context
- resolves the target project by ID or exact name
- loads stored pipelines from the local `previa-main`
- writes one file per pipeline as `*.previa.yaml`
- fails if an output file already exists

## Command Shape

```text
previa export pipelines [OPTIONS]
```

Important options:

- `--context <CONTEXT>`: context name, default `default`
- `--project <ID_OR_NAME>`: required project selector
- `--output-dir <PATH>`: required destination directory
- `--pipeline <ID_OR_NAME>`: optional repeated pipeline filter
- `--format <yaml|json>`: output format, default `yaml`
- `--overwrite`: replace existing files instead of failing

## Examples

Export every stored pipeline as YAML:

```bash
previa export pipelines --project my_app_smoke --output-dir ./tests/e2e
```

Export as JSON:

```bash
previa export pipelines --project my_app_smoke --output-dir ./tests/e2e --format json
```

Export only selected pipelines:

```bash
previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e \
  --pipeline smoke \
  --pipeline login
```

Overwrite existing files:

```bash
previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e \
  --overwrite
```

## File Naming

The output file base name is:

- `pipeline.id` when present and non-empty
- otherwise a slugified version of `pipeline.name`
- otherwise `pipeline-<position>`

Extensions:

- YAML: `.previa.yaml`
- JSON: `.previa.json`

## Safety Behavior

Before writing files, `previa`:

- resolves the full export set
- checks for duplicate generated file paths
- checks whether target files already exist

If any of those checks fail, no pipeline file is written.

## Important Notes

- exported files contain direct pipeline objects, not project export bundles
- `previa export pipelines` requires a detached context; it does not start one automatically
- project name selection is exact-match only
- if more than one project shares the same name, export fails and asks for a project ID
- if you export spec-driven pipelines, the files remain valid, but later execution still depends on matching project specs

## See Also

- [Import and export](./import-export.md)
- [CLI commands](./cli-commands.md)
- [Project repository workflow](./project-repository-workflow.md)
- [Pipeline import](./pipeline-import.md)
