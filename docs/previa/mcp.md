# MCP Integration

Previa can expose an MCP server from `previa-main`, allowing AI assistants to inspect projects, author pipelines, operate queues, validate specs, probe live APIs, and run execution workflows against your local stack.

## Enable MCP

Direct startup:

```bash
MCP_ENABLED=true cargo run -p previa-main
```

With compose input:

```yaml
version: 1
main:
  env:
    MCP_ENABLED: "true"
    MCP_PATH: /mcp
runners:
  local:
    count: 1
```

Then:

```bash
previa up -d .
```

## Endpoint

Default endpoint:

```text
http://localhost:5588/mcp
```

If you override the main port or `MCP_PATH`, update the URL accordingly.

## `previa mcp`

The CLI can help wire the Previa MCP endpoint into supported clients:

```bash
previa mcp install codex --context default
previa mcp status codex
previa mcp print claude-code --context default
previa mcp uninstall cursor --scope project
```

Protected contexts need an API token. If you already ran
`previa login --context default`, `previa mcp install` reuses the saved token.
You can also reference an environment variable instead of writing the raw token
to client config:

```bash
previa mcp install codex --context default --token-env PREVIA_API_TOKEN
previa mcp print cursor --context default --token-env PREVIA_API_TOKEN
```

In anonymous mode, MCP behaves like the rest of Previa: no bearer token is
required and the effective role is `anonymous` with full access. In protected
mode, MCP requests must include `Authorization: Bearer <api-token>`.

Current target matrix for the Linux-first release:

- `codex`
  Global: `~/.codex/config.toml`
  Project: `.codex/config.toml`

- `cursor`
  Global: `~/.cursor/mcp.json`
  Project: `.cursor/mcp.json`

- `copilot-vscode`
  Global: `~/.config/Code/User/mcp.json`
  Project: `.vscode/mcp.json`

- `claude-code`
  Managed through the external `claude mcp ...` CLI with global and project scope support.

- `warp`
  Global only in this version.
  Previa writes an Oz-compatible JSON file under `PREVIA_HOME/clients/warp/`.

- `claude-desktop`
  Manual-only in this version.
  Use `previa mcp print claude-desktop` for the URL and guidance.

## Codex Example

```toml
[mcp_servers.previa]
enabled = true
url = "http://localhost:5588/mcp"

[mcp_servers.previa.headers]
Authorization = "Bearer ${PREVIA_API_TOKEN}"
```

On the same machine, `localhost` is usually the right host even if `previa-main` is bound to `0.0.0.0`.

## What the Server Exposes

Today, the Previa MCP server exposes:

- tools
- prompts
- resources via `resources/list` and `resources/read`

It does not currently expose MCP resource templates as a separate capability layer.

### Available Resources

Today, `resources/list` includes:

- `previa://openapi`
- `previa://projects/<project-id>`
- `previa://projects/<project-id>/pipelines`
- `previa://projects/<project-id>/pipelines/id:<pipeline-id>`
- `previa://projects/<project-id>/pipelines/index:<index>`
- `previa://projects/<project-id>/specs`
- `previa://projects/<project-id>/specs/<spec-id>`

`resources/read` returns JSON content for these URIs, which makes project metadata, saved pipelines, specs, and the live OpenAPI document directly readable by MCP-aware clients such as Codex.

## Built-In Prompts

These prompts are available through `prompts/list` and `prompts/get`.

- `default`
  General operational prompt for pipeline authoring, execution analysis, and safe repair planning.

- `previa_pipeline_author`
  Detailed authoring prompt for valid Previa pipelines, including schemas, template rules, and examples.

- `project_onboarding_guide`
  Helps an assistant discover the current project, specs, pipelines, and context before changing anything.

- `pipeline_failure_triage`
  Focused on investigating failing E2E and load executions and proposing the next safe action.

- `openapi_spec_ingestion_advisor`
  Helps validate OpenAPI content and turn it into project specs safely.

- `pipeline_repair_planner`
  Plans evidence-based changes for a failing or outdated pipeline before applying updates.

- `load_test_designer`
  Helps choose sensible load parameters and explain tradeoffs before running a load test.

- `queue_orchestrator`
  Helps create, inspect, monitor, and cancel E2E queues for a project.

- `http_probe_assistant`
  Guides live HTTP inspection through `proxy_request` before making persistent pipeline changes.

- `project_migration_assistant`
  Supports exporting, reviewing, and importing project bundles between environments.

- `safe_change_reviewer`
  Reviews risky create, update, delete, and import actions before execution.

- `spec_to_pipeline_bootstrap`
  Turns project specs into an initial executable pipeline design.

## Prompt Aliases

These legacy names are still accepted by `prompts/get`:

- `pipeline_test_assistant` -> `default`
- `pipeline_creation_specialist` -> `previa_pipeline_author`

## Available Tools

### System and Discovery

- `health`
  Returns a simple orchestrator health payload.

- `get_info`
  Returns runner registration and health information.

- `get_openapi_document`
  Returns the orchestrator OpenAPI document.

- `get_pipeline_creation_guide`
  Returns a built-in guide for Previa pipeline structure and supported templates.

### Projects and Transfers

- `list_projects`
- `get_project`
- `create_project`
- `update_project`
- `delete_project`
- `export_project`
- `import_project`

Use these to inspect project state, create or update metadata, and move project bundles between environments.

### Pipelines

- `list_project_pipelines`
- `get_project_pipeline`
- `create_project_pipeline`
- `update_project_pipeline`
- `delete_project_pipeline`

Use these for stored project pipelines, not just one-off execution payloads.

### Specs and OpenAPI

- `list_project_specs`
- `get_project_spec`
- `create_project_spec`
- `update_project_spec`
- `delete_project_spec`
- `validate_openapi`

Use these when the assistant needs to understand or manage the spec layer behind `{{specs.<slug>.url.<name>}}`.

### E2E and Load History

- `list_e2e_history`
- `get_e2e_test`
- `delete_e2e_history`
- `delete_e2e_test`
- `list_load_history`
- `get_load_test`
- `delete_load_history`
- `delete_load_test`

These are useful for diagnosis, cleanup, and reporting.

### Execution and Queues

- `run_project_e2e_test`
- `run_project_load_test`
- `get_execution`
- `cancel_execution`
- `create_project_e2e_queue`
- `get_current_project_e2e_queue`
- `get_project_e2e_queue`
- `cancel_project_e2e_queue`

These are the tools that let an assistant actively run and operate test workflows.

### Live HTTP Inspection

- `proxy_request`

Use this to inspect live endpoint behavior, headers, auth, payloads, redirects, or SSE without immediately changing stored project assets.

## What an Assistant Can Do Well

With the current MCP surface, an assistant can:

- inspect project state before acting
- create or repair pipelines
- validate or ingest OpenAPI specs
- run E2E and load workflows
- operate queue-based regression sequences
- inspect failures using history and execution data
- probe live HTTP behavior
- move projects across environments

## Suggested First Prompts

After connecting your assistant, try prompts like:

- inspect this project and summarize its specs and pipelines
- use `project_onboarding_guide` and tell me the safest next step
- use `previa_pipeline_author` to create a CRUD pipeline for my users spec
- use `pipeline_failure_triage` on the latest failing execution
- use `queue_orchestrator` to run these pipeline IDs in sequence
- use `http_probe_assistant` to inspect this endpoint before editing my pipeline

## Practical Advice

- use specs before hardcoding environment URLs when possible
- prefer `project_onboarding_guide` before risky writes in an unfamiliar project
- prefer `proxy_request` when you need evidence from the target API first
- use `safe_change_reviewer` before deletes, imports, or broad updates

## See Also

- [Architecture at a glance](./architecture.md)
- [Main and runner authentication](./main-runner-auth.md)
- [E2E queues](./e2e-queues.md)
- [Proxy](./proxy.md)
