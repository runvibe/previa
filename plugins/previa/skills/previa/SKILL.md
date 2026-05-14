---
name: previa
description: Work on Previa, the AI-first QA IDE and API testing runtime. Use when creating or debugging Previa pipelines, runners, MCP integration, CLI flows, repository changes, release/install workflows, or documentation.
---

# Previa

Use this skill when the user is working on Previa itself or wants help using Previa to design, run, or troubleshoot API workflows.

## Project Shape

- `previa` is the local CLI entrypoint.
- `main` is the orchestrator API for projects, specs, pipelines, history, proxying, queues, and MCP.
- `runner` executes E2E and load tests.
- `engine` contains reusable execution behavior.
- `app` is the browser IDE served by `previa-main`.

Keep HTTP transport and wiring in `routes/`, reusable business logic and integrations in `services/`, and data contracts or DB-facing structs in `models/`. Align request and response changes with OpenAPI definitions and existing migrations.

## Documentation Map

- Start with `README.md` for product positioning, install, quick start, and release links.
- Use `docs/previa/README.md` as the documentation index.
- Use `docs/previa/pipeline-authoring.md`, `docs/previa/spec-driven-testing.md`, and `docs/previa/examples-cookbook.md` for pipeline work.
- Use `docs/previa/api-workflows.md` for orchestrator API sequences.
- Use `docs/previa/mcp.md` for assistant and MCP integration.
- Use `docs/previa/remote-runners.md`, `docs/previa/e2e-queues.md`, and `docs/previa/wave-load-tests.md` for runtime execution flows.
- Use `PROJECT.md` for current project conventions and release workflow notes.

## Common Commands

```bash
cargo build --release
cargo test
previa up -d
previa status
previa open
previa mcp install codex --context default
```

After repository changes, run `cargo build --release`. If the release build succeeds, commit and push according to the repository workflow.

## Working Rules

- Prefer SQLx query macros with bound parameters for persistence changes.
- Reuse `migrations/` and avoid inline schema drift.
- Keep route handlers thin; move reusable behavior into focused service modules.
- Update docs and contracts when provider workflows or public behavior changes.
- For MCP work, remember the default endpoint is `http://localhost:5588/mcp`.
