# Import and Export

This guide maps each Previa import/export path to its format, scope, and best
use case.

## Quick Matrix

| Workflow | Commands or endpoints | Formats | Scope | Best for |
| --- | --- | --- | --- | --- |
| Pipeline files | `previa export pipelines`, `previa up --import`, `previa local up --import` | YAML, JSON | Direct pipeline objects only | Keeping test pipelines in an application repository |
| Project SQLite snapshot | `previa local export`, `previa local import`, `POST /api/v1/projects/export`, `POST /api/v1/projects/import` | SQLite database | One or more projects with pipelines, specs, env groups, and optional history | Backups, local migration, bulk transfer |
| Project JSON bundle API | `GET /api/v1/projects/{projectId}/export`, `POST /api/v1/projects/import` | JSON envelope | One project with pipelines, specs, env groups, and optional history | API automation and integrations |
| Local push | `previa local push` | No user-facing file | One local project copied to a remote `previa-main` | Publishing a local project to a shared remote environment |

## Pipeline Files

Pipeline file import/export is the repository-friendly path. Use it when the
pipeline definitions should live beside the application code, for example under
`tests/e2e`, `qa/pipelines`, or `.previa/pipelines`.

Export every stored pipeline from a project as YAML:

```bash
previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e
```

Export as JSON:

```bash
previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e \
  --format json
```

Export only selected pipelines:

```bash
previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e \
  --pipeline smoke \
  --pipeline login
```

Use a project-local runtime home when the stack belongs to the repository:

```bash
previa --home ./.previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e \
  --format yaml \
  --overwrite
```

Export writes one file per pipeline.

Extensions:

- YAML: `.previa.yaml`
- JSON: `.previa.json`

The file name is based on `pipeline.id` when present, otherwise on a slugified
pipeline name, otherwise on `pipeline-<position>`.

Import a single pipeline file into a new local project:

```bash
previa up --detach \
  --import ./tests/e2e/login.previa.yaml \
  --stack my_app_smoke
```

Import a directory recursively:

```bash
previa up --detach \
  --import ./tests/e2e \
  --recursive \
  --stack my_app_smoke
```

Project-local equivalent:

```bash
previa local up --detach \
  --import ./tests/e2e \
  --recursive \
  --stack my_app_smoke
```

Supported import suffixes:

- `.previa`
- `.previa.json`
- `.previa.yaml`
- `.previa.yml`

Rules and limitations:

- Each imported file must contain one direct `Pipeline` object, not a project
  export bundle.
- `--import` requires `--detach` and `--stack`.
- `--recursive` is required when importing a directory.
- Import creates a new project named by `--stack`; it fails if that project name
  already exists.
- Pipeline file import/export does not include project specs, env groups,
  execution history, project tags, sharing rules, users, tokens, or runner
  registrations.
- Spec-driven pipelines can be exported as files, but later execution still
  requires compatible project specs and environment data to exist in the target
  project.

## Project SQLite Snapshot

SQLite import/export is the full local snapshot path. Use it for backup,
bulk migration between local contexts, or moving a complete project with its
supporting project data.

Export one project from the project-local context:

```bash
previa local export \
  --project project_id \
  --output ./my-app.sqlite3
```

Export multiple projects:

```bash
previa local export \
  --project project_a \
  --project project_b \
  --output ./selected-projects.sqlite3
```

Export every project:

```bash
previa local export \
  --all \
  --output ./previa-projects.sqlite3
```

Overwrite an existing destination and omit history:

```bash
previa local export \
  --all \
  --output ./previa-projects.sqlite3 \
  --overwrite \
  --no-history
```

Import the SQLite snapshot:

```bash
previa local import ./previa-projects.sqlite3
```

Import without history:

```bash
previa local import ./previa-projects.sqlite3 --no-history
```

Use a non-default local context:

```bash
previa local export \
  --context staging \
  --project project_id \
  --output ./staging-project.sqlite3

previa local import ./staging-project.sqlite3 --context staging
```

The SQLite snapshot includes:

- project metadata, including name, description, tags, timestamps, and legacy
  project spec JSON
- stored pipelines
- project OpenAPI specs
- project env groups
- E2E and load-test history, unless omitted with `--no-history`

The SQLite snapshot does not represent runtime state such as running processes,
runner registrations, local auth files, API tokens, or remote environment
configuration outside the project database rows.

On import, Previa imports every project in the SQLite file. If a project name
already exists in the target context, the imported project is renamed with
`-imported`, `-imported-2`, and so on instead of merging or overwriting.

Equivalent API export:

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/export \
  -H 'content-type: application/json' \
  -d '{"all":true,"projectIds":[],"includeHistory":true}' \
  -o previa-projects.sqlite3
```

Equivalent API import:

```bash
curl -sS 'http://127.0.0.1:5588/api/v1/projects/import?includeHistory=true' \
  -H 'content-type: application/vnd.sqlite3' \
  --data-binary @previa-projects.sqlite3
```

SQLite API import also accepts these content types:

- `application/vnd.sqlite3`
- `application/x-sqlite3`
- `application/octet-stream`

## Project JSON Bundle API

The project JSON bundle is the API-level project transfer format. It is useful
for integrations that want a readable JSON payload, but there is no direct CLI
command that writes this bundle to disk.

Export one project as JSON:

```bash
curl -sS 'http://127.0.0.1:5588/api/v1/projects/project_id/export?includeHistory=true' \
  -o project-export.json
```

Import the JSON bundle:

```bash
curl -sS 'http://127.0.0.1:5588/api/v1/projects/import?includeHistory=true' \
  -H 'content-type: application/json' \
  -d @project-export.json
```

The payload is a `ProjectExportEnvelope` with format
`previa.project.export.v1`.

The JSON bundle includes:

- one project
- project metadata, including tags and timestamps
- stored pipelines
- project OpenAPI specs
- project env groups
- E2E and load-test history when `includeHistory=true`

Import behavior differs from SQLite import:

- JSON project import preserves the project ID from the payload.
- If that project ID already exists, import returns a conflict.
- It does not auto-rename project names.

## Local Push

`previa local push` publishes one project from a project-local context to a
remote `previa-main`. It is the right path when the target is a shared Previa
environment rather than a file.

Create the remote project when no matching project exists:

```bash
previa local push \
  --project my_app_smoke \
  --to https://previa.example.com
```

Replace an existing remote project:

```bash
previa local push \
  --project my_app_smoke \
  --to https://previa.example.com \
  --overwrite
```

Include execution history:

```bash
previa local push \
  --project my_app_smoke \
  --to https://previa.example.com \
  --overwrite \
  --include-history
```

Select the remote project explicitly:

```bash
previa local push \
  --project my_app_smoke \
  --to https://previa.example.com \
  --remote-project-id prj_123 \
  --overwrite
```

By default, `local push` does not include execution history. It matches the
remote project by local project ID first, then by exact project name. With
`--overwrite`, it deletes the matched remote project before importing the local
snapshot.

## Choosing a Format

Use pipeline YAML or JSON when:

- the pipelines should be reviewed in pull requests
- the application repository should bootstrap its own smoke or E2E tests
- each file should be a small, portable pipeline definition

Use SQLite when:

- you need the closest local backup of one or more projects
- you need project specs, env groups, and optional execution history
- you are moving work between local Previa contexts

Use the project JSON bundle API when:

- an integration needs to inspect or transform a complete project payload
- a human-readable project transfer document is more useful than a SQLite file
- preserving the project ID is required

Use `local push` when:

- a local project should be promoted to a remote Previa instance
- the destination is an environment URL, not a file
- overwrite behavior should be explicit and auditable

## Related Commands

```bash
# Pipeline files
previa export pipelines --project my_app --output-dir ./tests/e2e --format yaml
previa export pipelines --project my_app --output-dir ./tests/e2e --format json
previa up -d --import ./tests/e2e -r --stack my_app

# Project-local SQLite snapshots
previa local export --project project_id --output ./project.sqlite3
previa local export --all --output ./previa-projects.sqlite3
previa local import ./previa-projects.sqlite3

# Remote publication
previa local push --project my_app --to https://previa.example.com --overwrite
```

## See Also

- [Pipeline import](./pipeline-import.md)
- [Pipeline export](./pipeline-export.md)
- [Project repository workflow](./project-repository-workflow.md)
- [API workflows](./api-workflows.md)
- [CLI commands](./cli-commands.md)
