# End-to-End Walkthrough

This guide shows one complete Previa workflow from stack startup to test execution and failure inspection.

## Goal

By the end of this walkthrough, you will have:

1. started a local Previa stack
2. created a project
3. added an API spec
4. added a pipeline
5. run an E2E test
6. inspected the execution result

## 1. Start Previa

Use the default Docker-backed path:

```bash
previa up -d
previa open
```

This opens:

```text
http://127.0.0.1:5588
```

## 2. Create a Project

Create a project through the IDE or the API:

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects \
  -H 'content-type: application/json' \
  -d '{
    "name": "Users API",
    "description": "Example end-to-end walkthrough",
    "pipelines": []
  }'
```

Save the returned `id` as `PROJECT_ID`.

## 3. Add a Spec

Create a minimal project spec with a named runtime URL such as `hml`:

```json
{
  "slug": "users",
  "urls": [
    { "name": "hml", "url": "https://hml.example.com" }
  ],
  "sync": false,
  "live": false,
  "spec": {
    "openapi": "3.0.3",
    "info": { "title": "Users API", "version": "1.0.0" },
    "paths": {
      "/users": {
        "get": {
          "responses": {
            "200": { "description": "ok" }
          }
        }
      }
    }
  }
}
```

Send it to:

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs \
  -H 'content-type: application/json' \
  -d @spec.json
```

This uses the same spec shape described in [Spec-driven testing](./spec-driven-testing.md).

## 4. Add a Pipeline

Create a pipeline that uses the spec base URL:

```json
{
  "name": "Users Smoke",
  "description": "Simple GET /users validation",
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

Send it to:

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines \
  -H 'content-type: application/json' \
  -d @pipeline.json
```

Save the returned `id` as `PIPELINE_ID`.

## 5. Run an E2E Test

Run the stored pipeline:

```bash
curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e \
  -H 'content-type: application/json' \
  -d "{\"pipelineId\":\"$PIPELINE_ID\",\"selectedBaseUrlKey\":\"hml\",\"specs\":[]}"
```

You can do the same from:

- the browser IDE
- an MCP-connected AI assistant
- your own automation through the HTTP API

## 6. Inspect the Result

After the execution, inspect:

- the execution timeline
- the step that failed, if any
- request and response details
- assertions that passed or failed

Useful runtime checks from the CLI:

```bash
previa status
previa logs
```

## 7. What to Do Next

Once this flow works, the next practical steps are:

- add more CRUD or auth steps to the pipeline
- create a regression queue
- run a load test for the same pipeline
- connect an AI assistant through MCP and ask it to inspect or improve the workflow

## See Also

- [Getting started](./getting-started.md)
- [API workflows](./api-workflows.md)
- [Examples cookbook](./examples-cookbook.md)
- [MCP integration](./mcp.md)
