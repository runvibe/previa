# Project Repository Workflow

This guide shows how to keep Previa configuration, specs, and pipelines inside an application repository and run repeatable smoke or E2E checks for that specific project.

## Goal

By the end of this workflow, you will have:

1. a project-local Previa runtime under `./.previa`
2. versioned pipeline files inside the application repository
3. a repeatable way to bootstrap one project and run smoke checks locally or in automation

## Recommended Repository Layout

```text
my-app/
  previa-compose.yaml
  previa/
    spec.json
  tests/
    e2e/
      smoke.previa.json
      login.previa.json
  .gitignore
```

Recommended `.gitignore` entry:

```gitignore
/.previa
```

Why this layout works well:

- `previa-compose.yaml` keeps stack wiring close to the app
- `tests/e2e/` keeps executable pipelines versioned with the code they validate
- `previa/spec.json` keeps spec-driven base URLs and OpenAPI data near the pipelines that use them
- `./.previa` isolates runtime state, logs, and the local database per repository

## 1. Start a Project-Local Stack

From the application repository root:

```bash
previa local up -d .
previa local status
previa local open
```

This keeps the detached stack for this repository under `./.previa` instead of mixing it with a shared global home.

## Push a Local Project to Remote

After creating or editing a project locally, push it to a remote Previa main:

```bash
previa local push --project my_app_smoke --to https://previa.example.com
```

If the project already exists on the remote, the command fails unless overwrite
is explicit:

```bash
previa local push \
  --project my_app_smoke \
  --to https://previa.example.com \
  --overwrite
```

`--overwrite` replaces the remote project with the local snapshot instead of
merging. Execution history is not included unless `--include-history` is passed.

## Export and Import a Local SQLite Snapshot

For bulk moves, export selected projects from a project-local context into a
SQLite database:

```bash
previa local export --all --output ./previa-projects.sqlite3
previa local export --project my_app_smoke --output ./my-app.sqlite3
```

Import the SQLite database into another local context with:

```bash
previa local import ./previa-projects.sqlite3
```

When a project name already exists in the target context, the imported copy gets
an `-imported`, `-imported-2`, `-imported-3` suffix instead of merging with or
overwriting the existing project.

## 2. Choose One of Two Pipeline Styles

There are two practical ways to keep pipelines in the repo.

## Self-Contained Pipelines

Use absolute URLs inside the pipeline itself.

Example:

```json
{
  "name": "Health Smoke",
  "description": "Basic smoke check against one environment",
  "steps": [
    {
      "id": "health",
      "name": "Health",
      "method": "GET",
      "url": "https://hml.example.com/health",
      "headers": {},
      "asserts": [
        {
          "field": "status",
          "operator": "equals",
          "expected": "200"
        }
      ]
    }
  ]
}
```

Use this style when:

- you want the shortest bootstrap flow
- the pipeline targets only one environment
- you want to use `previa up --import` directly

## Spec-Driven Pipelines

Use runtime spec URLs such as `{{specs.users.url.hml}}`.

Example:

```json
{
  "name": "Users Smoke",
  "description": "GET /users smoke using the project spec",
  "steps": [
    {
      "id": "list_users",
      "name": "List users",
      "method": "GET",
      "url": "{{specs.users.url.hml}}/users",
      "headers": {},
      "asserts": [
        {
          "field": "status",
          "operator": "equals",
          "expected": "200"
        }
      ]
    }
  ]
}
```

Use this style when:

- the same pipeline should run against `local`, `hml`, and `prd`
- you want one project-level source of truth for base URLs
- you want the IDE and MCP workflows to understand the same API model

## 3. Fast Path for Self-Contained Pipelines

If the pipeline files do not depend on `{{specs.<slug>.url.<name>}}`, you can import them directly from the repository:

```bash
previa --home ./.previa up -d -i ./tests/e2e -r -s my_app_smoke
```

Important behavior:

- `--import` requires `--detach`
- `--stack` becomes the newly created project name
- recursive import scans only `.previa`, `.previa.json`, `.previa.yaml`, and `.previa.yml`

After import, run a stored pipeline from the IDE or API.

Example:

```bash
PROJECT_ID="$(curl -sS http://127.0.0.1:5588/api/v1/projects | jq -r '.[] | select(.name=="my_app_smoke") | .id')"

PIPELINE_ID="$(curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines | jq -r '.[] | select(.name=="Health Smoke") | .id')"

curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e \
  -H 'content-type: application/json' \
  -d "{\"pipelineId\":\"$PIPELINE_ID\",\"selectedBaseUrlKey\":null,\"specs\":[]}"
```

## 4. Recommended Path for Spec-Driven Repositories

If your repo pipelines use `{{specs.<slug>.url.<name>}}`, bootstrap the project in this order:

1. start the local stack
2. create the project
3. add the project spec
4. sync pipelines into that project
5. run smoke or queue execution

This order matters because spec-driven templates must match runtime specs that already exist for the project.

## Find or Create the Project

```bash
PROJECT_ID="$(
  curl -sS http://127.0.0.1:5588/api/v1/projects \
    | jq -r '.[] | select(.name=="my_app_smoke") | .id' \
    | head -n 1
)"

if [ -z "$PROJECT_ID" ]; then
  PROJECT_ID="$(
    curl -sS http://127.0.0.1:5588/api/v1/projects \
      -H 'content-type: application/json' \
      -d '{"name":"my_app_smoke","description":"Repository-managed smoke project","pipelines":[]}' \
      | jq -r '.id'
  )"
fi
```

## Create or Update the Spec

Store the spec in the repository, for example at `previa/spec.json`, then create or update the project spec by slug:

```bash
SPEC_SLUG="$(jq -r '.slug' ./previa/spec.json)"
SPEC_ID="$(
  curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs" \
    | jq -r --arg slug "$SPEC_SLUG" '.[] | select(.slug==$slug) | .id' \
    | head -n 1
)"

if [ -n "$SPEC_ID" ]; then
  curl -sS -X PUT "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs/$SPEC_ID" \
    -H 'content-type: application/json' \
    -d @./previa/spec.json
else
  curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs" \
    -H 'content-type: application/json' \
    -d @./previa/spec.json
fi
```

Example spec file:

```json
{
  "slug": "users",
  "urls": [
    { "name": "local", "url": "http://127.0.0.1:3000" },
    { "name": "hml", "url": "https://hml.example.com" }
  ],
  "sync": false,
  "live": false,
  "spec": {
    "openapi": "3.0.3",
    "info": { "title": "Users API", "version": "1.0.0" },
    "paths": {}
  }
}
```

## Sync Pipelines from the Repo

For automation, JSON pipeline files are the simplest option because they can be sent directly with `curl`.

For repeatable automation, sync each repo file by pipeline name:

```bash
for file in ./tests/e2e/*.previa.json; do
  pipeline_name="$(jq -r '.name' "$file")"
  pipeline_id="$(
    curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines" \
      | jq -r --arg name "$pipeline_name" '.[] | select(.name==$name) | .id' \
      | head -n 1
  )"

  if [ -n "$pipeline_id" ]; then
    curl -sS -X PUT "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines/$pipeline_id" \
      -H 'content-type: application/json' \
      -d @"$file"
  else
    curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines" \
      -H 'content-type: application/json' \
      -d @"$file"
  fi
done
```

With this convention:

- the repository remains the source of truth for the pipeline definition
- the first sync creates missing pipelines
- the next sync updates the matching stored pipeline instead of duplicating it

## Export Stored Pipelines Back Into the Repo

If you already have stored pipelines in the local project and want to write them back into the repository:

```bash
previa --home ./.previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e
```

Useful variants:

```bash
previa --home ./.previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e \
  --overwrite

previa --home ./.previa export pipelines \
  --project my_app_smoke \
  --output-dir ./tests/e2e \
  --format json
```

Important behavior:

- exported files are direct pipeline objects, not full project bundles
- YAML is the default output format
- spec-driven pipelines still depend on matching project specs when reimported or executed later
- export fails on existing files unless `--overwrite` is set

## Run the Smoke Pipeline

If the pipeline uses a spec URL name like `hml`, pass the matching `selectedBaseUrlKey`:

```bash
curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e \
  -H 'content-type: application/json' \
  -d '{"pipelineId":"smoke","selectedBaseUrlKey":"hml","specs":[]}'
```

If the pipeline is self-contained and uses absolute URLs, `selectedBaseUrlKey` can be `null`.

## Run More Than One Stored Pipeline

For a small regression or smoke batch:

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e/queue \
  -H 'content-type: application/json' \
  -d '{"pipelineIds":["smoke","login"],"selectedBaseUrlKey":"hml","specs":[]}'
```

## Current Import Limitation

`previa up --import` is the best bootstrap path for self-contained pipelines, but it is not the best first step for a brand new spec-driven project.

Why:

- `--import` creates a new project from the provided stack name
- imported pipelines are validated during import
- spec-driven templates such as `{{specs.users.url.hml}}` depend on runtime specs that belong to the project

In practice, this means repo pipelines that depend on project specs are more reliable when bootstrapped with:

1. `POST /api/v1/projects`
2. `POST /api/v1/projects/{projectId}/specs` for first creation
3. `PUT /api/v1/projects/{projectId}/specs/{specId}` for spec updates
4. `POST /api/v1/projects/{projectId}/pipelines` for new entries
5. `PUT /api/v1/projects/{projectId}/pipelines/{pipelineId}` for updates

## Suggested Automation Script

A simple repository-local smoke script could look like this:

```bash
#!/usr/bin/env bash
set -euo pipefail

previa --home ./.previa up -d .

PROJECT_ID="$(
  curl -sS http://127.0.0.1:5588/api/v1/projects \
    | jq -r '.[] | select(.name=="my_app_smoke") | .id' \
    | head -n 1
)"

if [ -z "$PROJECT_ID" ]; then
  PROJECT_ID="$(
    curl -sS http://127.0.0.1:5588/api/v1/projects \
      -H 'content-type: application/json' \
      -d '{"name":"my_app_smoke","description":"Repository-managed smoke project","pipelines":[]}' \
      | jq -r '.id'
  )"
fi

SPEC_SLUG="$(jq -r '.slug' ./previa/spec.json)"
SPEC_ID="$(
  curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs" \
    | jq -r --arg slug "$SPEC_SLUG" '.[] | select(.slug==$slug) | .id' \
    | head -n 1
)"

if [ -n "$SPEC_ID" ]; then
  curl -sS -X PUT "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs/$SPEC_ID" \
    -H 'content-type: application/json' \
    -d @./previa/spec.json >/dev/null
else
  curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs" \
    -H 'content-type: application/json' \
    -d @./previa/spec.json >/dev/null
fi

for file in ./tests/e2e/*.previa.json; do
  pipeline_name="$(jq -r '.name' "$file")"
  pipeline_id="$(
    curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines" \
      | jq -r --arg name "$pipeline_name" '.[] | select(.name==$name) | .id' \
      | head -n 1
  )"

  if [ -n "$pipeline_id" ]; then
    curl -sS -X PUT "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines/$pipeline_id" \
      -H 'content-type: application/json' \
      -d @"$file" >/dev/null
  else
    curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines" \
      -H 'content-type: application/json' \
      -d @"$file" >/dev/null
  fi
done

curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e \
  -H 'content-type: application/json' \
  -d '{"pipelineId":"smoke","selectedBaseUrlKey":"hml","specs":[]}'
```

You can adapt the project lookup and creation logic if you want the script to reuse an existing project instead of creating a fresh one each time.

## Recommended Conventions

- use `previa --home ./.previa` inside application repositories
- keep `previa-compose.yaml` at the repo root when the stack is part of the project
- prefer `.previa.json` when you want API-driven sync from shell scripts
- match or update stored pipelines by name when you want repeatable automation
- keep spec `slug` and URL names short and stable
- reserve `previa up --import` for self-contained pipelines or one-off local imports

## See Also

- [Getting started](./getting-started.md)
- [Pipeline export](./pipeline-export.md)
- [Pipeline import](./pipeline-import.md)
- [Spec-driven testing](./spec-driven-testing.md)
- [API workflows](./api-workflows.md)
- [E2E queues](./e2e-queues.md)
