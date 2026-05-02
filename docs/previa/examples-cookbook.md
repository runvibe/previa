# Examples Cookbook

This guide collects small, reusable workflow examples for common Previa use cases.

## Smoke Test

```yaml
id: smoke-status
name: Smoke Status
steps:
  - id: health
    name: Health check
    method: GET
    url: https://httpbin.org/status/200
    headers: {}
    asserts:
      - field: status
        operator: equals
        expected: "200"
```

## CRUD Flow

```yaml
id: users-crud
name: Users CRUD
steps:
  - id: create_user
    name: Create user
    method: POST
    url: "{{specs.users.url.hml}}/users"
    headers:
      content-type: application/json
    body:
      name: "{{helpers.name}}"
      email: "{{helpers.email}}"
    asserts:
      - field: status
        operator: equals
        expected: "201"

  - id: get_user
    name: Get user
    method: GET
    url: "{{specs.users.url.hml}}/users/{{steps.create_user.id}}"
    headers: {}
    asserts:
      - field: status
        operator: equals
        expected: "200"

  - id: delete_user
    name: Delete user
    method: DELETE
    url: "{{specs.users.url.hml}}/users/{{steps.create_user.id}}"
    headers: {}
    asserts:
      - field: status
        operator: equals
        expected: "204"
```

## Regression Queue

Queue multiple stored pipelines in order:

```bash
curl -sS http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/e2e/queue \
  -H 'content-type: application/json' \
  -d '{"pipelineIds":["login","checkout","cleanup"],"selectedBaseUrlKey":"hml","specs":[]}'
```

## Wave Load Baseline

Wave load tests use elapsed time and intensity percentage. A runner treats
`100%` as its configured safe RPS capacity.

```bash
curl -N http://127.0.0.1:5588/api/v1/projects/$PROJECT_ID/tests/load \
  -H 'content-type: application/json' \
  -d '{"pipelineId":"users-crud","selectedBaseUrlKey":"hml","load":{"points":[{"atMs":0,"intensity":30},{"atMs":300000,"intensity":30}],"interpolation":"smooth","maxInFlight":200,"gracePeriodMs":30000},"specs":[]}'
```

## Repository Import

Import a directory of local pipeline files:

```bash
previa up -d -i ./tests/e2e -r -s app_e2e
```

## Remote Runner Attachment

```bash
RUNNER_AUTH_KEY=shared-secret previa up -d --runners 1 --attach-runner 10.0.0.12:55880
```

## MCP Connection

```toml
[mcp_servers.previa]
enabled = true
url = "http://localhost:5588/mcp"
```

## See Also

- [Pipeline authoring](./pipeline-authoring.md)
- [MCP integration](./mcp.md)
- [Remote runners](./remote-runners.md)
