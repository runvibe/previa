# Glossary

This glossary defines the main terms used across the Previa docs.

## `previa`

The CLI used to start, inspect, and operate a local Previa stack.

## `previa-main`

The orchestrator API. It manages projects, specs, pipelines, executions, queues, proxying, history, and MCP.

## `previa-runner`

The runtime worker that executes E2E and load tests.

## `previa-engine`

The execution core that resolves templates, performs HTTP steps, and evaluates assertions.

## context

An isolated local runtime under `PREVIA_HOME/stacks/<context>`. A context has its own config, logs, state, and database.

## stack

A running or planned local Previa runtime for one context. In practice, this usually means one `previa-main` plus local and/or attached runners.

## project

The main logical container for specs, pipelines, executions, and history inside `previa-main`.

## spec

An API description, usually OpenAPI-based, associated with a project. Specs can provide named base URLs such as `hml` or `prd`.

## pipeline

A sequence of HTTP steps and assertions used to validate a workflow.

## step

One executable unit inside a pipeline, such as a `GET`, `POST`, or `DELETE` request plus assertions.

## E2E test

An execution of a pipeline to validate a real end-to-end workflow.

## load test

A pipeline execution repeated under a timeline-based wave of load intensity.
The wave maps elapsed time to an intensity percentage, and runners translate
that percentage into local request flow using their configured safe capacity.

## queue

An ordered set of E2E pipeline executions that Previa runs and tracks as one larger workflow.

## attached runner

A runner endpoint not started by the local stack, but attached with `--attach-runner`.

## local runner

A runner process or container started directly by `previa up`.

## `PREVIA_HOME`

The root directory where Previa stores binaries, contexts, logs, runtime state, and local databases.

## MCP

Model Context Protocol. In Previa, this is the integration that lets AI assistants inspect and operate the platform through exposed tools and prompts.

## IDE

The browser UI served by `previa-main`. Embedded builds use `window.location.origin` as the API base; external builds use `VITE_PREVIA_API_BASE_URL`. Service worker precaching is disabled by default unless `VITE_PREVIA_ENABLE_PWA=true` is set for a standalone PWA build.

## See Also

- [Architecture at a glance](./architecture.md)
- [Home and contexts](./home-and-contexts.md)
- [MCP integration](./mcp.md)
