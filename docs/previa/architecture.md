# Architecture At a Glance

Previa is made of four main parts:

- `previa`: the local CLI used to start and operate a stack
- `previa-main`: the orchestrator API for projects, specs, pipelines, history, proxying, queues, MCP, and the optional embedded app
- `previa-runner`: the execution API for E2E and load requests
- `previa-engine`: the execution core that resolves templates, performs HTTP steps, and evaluates assertions

## End-to-End Flow

```text
previa CLI -> previa-main -> previa-runner -> previa-engine -> target API
```

Typical operator flow:

1. Start a local stack with `previa up -d`
2. Open the IDE with `previa open`
3. Create projects, specs, and pipelines through the IDE, API, or MCP
4. Run E2E tests, load tests, or E2E queues through `previa-main`
5. Inspect history, logs, and runtime state

## Ports and Default Interfaces

By default:

- `previa-main` listens on `0.0.0.0:5588`
- local `previa-runner` instances start at `127.0.0.1:55880`
- the MCP endpoint is `http://localhost:5588/mcp` when enabled
- the embedded app is served by `previa-main` on `/` and `/index` when `PREVIA_APP_ENABLED=true`
- `previa open` opens the selected `previa-main` URL directly
- the embedded app uses `window.location.origin` as the API base; external builds use `VITE_PREVIA_API_BASE_URL`; standalone PWA precaching is opt-in with `VITE_PREVIA_ENABLE_PWA=true`

## Feature Map

CLI:

- start and stop local stacks
- inspect status, processes, logs, and contexts
- import local pipeline files

IDE:

- connect to a local `previa-main`
- manage projects, specs, pipelines, and executions visually
- can be served directly by `previa-main` when the embedded app is enabled

HTTP API:

- create and update projects, specs, and pipelines
- run E2E, load, and queue workflows
- export and import project bundles

MCP:

- expose the same Previa platform capabilities to AI assistants
- support project inspection, pipeline authoring, failure triage, queue operations, and migrations

## See Also

- [Minimal happy path](./minimal-happy-path.md)
- [Runtime modes](./runtime-modes.md)
- [MCP integration](./mcp.md)
