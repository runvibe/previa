# API Workflows

This guide maps the most important `previa-main` API workflows into practical sequences.

Base URL:

```text
http://127.0.0.1:5588
```

## 1. Create a Project

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects \
  -H 'content-type: application/json' \
  -d '{
    "name": "Users API",
    "description": "Project for users API validation",
    "pipelines": []
  }'
```

## 2. Add a Spec

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/specs \
  -H 'content-type: application/json' \
  -d @spec.json
```

Optional validation before persisting:

```bash
curl -sS http://127.0.0.1:5588/api/v1/specs/validate \
  -H 'content-type: application/json' \
  -d '{"source":"<openapi string or url>"}'
```

## 3. Add a Pipeline

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/pipelines \
  -H 'content-type: application/json' \
  -d @pipeline.json
```

## 4. Run an E2E Test

```bash
curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e \
  -H 'content-type: application/json' \
  -d "{\"pipelineId\":\"$PIPELINE_ID\",\"selectedBaseUrlKey\":\"hml\",\"specs\":[]}"
```

## 5. Run a Load Test

```bash
curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/load \
  -H 'content-type: application/json' \
  -d "{\"pipelineId\":\"$PIPELINE_ID\",\"selectedBaseUrlKey\":\"hml\",\"load\":{\"points\":[{\"atMs\":0,\"intensity\":10},{\"atMs\":60000,\"intensity\":80},{\"atMs\":120000,\"intensity\":30}],\"interpolation\":\"smooth\",\"runnerMaxRps\":600,\"gracePeriodMs\":30000},\"specs\":[]}"
```

See [Wave load tests](./wave-load-tests.md) for the load wave model and
diagnostics.

## 6. Run an E2E Queue

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e/queue \
  -H 'content-type: application/json' \
  -d "{\"pipelineIds\":[\"$PIPELINE_ID\"],\"selectedBaseUrlKey\":\"hml\",\"specs\":[]}"
```

## 7. Export a Project

```bash
curl -sS "http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/export?includeHistory=true"
```

## 8. Import a Project

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/import?includeHistory=true \
  -H 'content-type: application/json' \
  -d @project-export.json
```

The same import endpoint also accepts a SQLite database and imports every
project inside it:

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/import?includeHistory=true \
  -H 'content-type: application/vnd.sqlite3' \
  --data-binary @previa-projects.sqlite3
```

## 9. Export Selected Projects as SQLite

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/export \
  -H 'content-type: application/json' \
  -d '{"all":true,"projectIds":[],"includeHistory":true}' \
  -o previa-projects.sqlite3
```

## 10. Probe a Live Endpoint

```bash
curl -sS http://127.0.0.1:5588/proxy \
  -H 'content-type: application/json' \
  -d '{"method":"GET","url":"https://httpbin.org/status/200","headers":{}}'
```

## See Also

- [Pipeline authoring](./pipeline-authoring.md)
- [Spec-driven testing](./spec-driven-testing.md)
- [E2E queues](./e2e-queues.md)
