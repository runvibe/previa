# Minimal Happy Path

This is the shortest practical path from zero to a working local Previa flow.

## 1. Install

```bash
curl -fsSL https://raw.githubusercontent.com/runvibe/previa/main/install.sh | sh
```

## 2. Start a Local Stack

Docker-backed:

```bash
previa up -d
```

Binary-backed on Linux:

```bash
previa up -d --bin
```

## 3. Open the IDE

```bash
previa open
```

This opens:

```text
http://127.0.0.1:5588
```

The embedded app uses the same origin as its API base.

## 4. Create a Project Workflow

From the IDE, API, or MCP:

1. create a project
2. add an OpenAPI spec with one or more runtime base URLs
3. create a pipeline that uses either absolute URLs or `{{specs.<slug>.url.<name>}}`

## 5. Run a Test

Run an E2E execution:

```bash
curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e \
  -H 'content-type: application/json' \
  -d "{\"pipelineId\":\"$PIPELINE_ID\",\"selectedBaseUrlKey\":\"hml\",\"specs\":[]}"
```

Run a load test:

```bash
curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/load \
  -H 'content-type: application/json' \
  -d "{\"pipelineId\":\"$PIPELINE_ID\",\"selectedBaseUrlKey\":\"hml\",\"config\":{\"totalRequests\":1000,\"concurrency\":20,\"rampUpSeconds\":10},\"specs\":[]}"
```

## 6. Inspect the Runtime

```bash
previa status
previa logs
```

## See Also

- [Architecture at a glance](./architecture.md)
- [Runtime modes](./runtime-modes.md)
- [Pipeline import](./pipeline-import.md)
