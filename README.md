<p align="center">
  <img src="assets/logo.png" alt="Previa logo" width="220">
</p>

# Previa

[![Release](https://img.shields.io/github/v/release/runvibe/previa?display_name=tag&cacheSeconds=300)](https://github.com/runvibe/previa/releases)
[![Build](https://img.shields.io/github/actions/workflow/status/runvibe/previa/release.yaml?branch=main&label=build&cacheSeconds=300)](https://github.com/runvibe/previa/actions/workflows/release.yaml)
[![License](https://img.shields.io/github/license/runvibe/previa?cacheSeconds=300)](https://github.com/runvibe/previa/blob/main/LICENSE)
[![Stars](https://img.shields.io/github/stars/runvibe/previa?style=social&cacheSeconds=300)](https://github.com/runvibe/previa/stargazers)

**The first AI-First IDE for QA. Test, design, and validate APIs with AI assistance from your desktop, CI/CD, or your favorite AI assistant.**

Previa is a platform for simulating, executing, and tracing real end-to-end API operations so you can see what happened, where a failure occurred, and why.

The `previa` CLI is the local entry point for running a Previa stack on your machine. It starts `previa-main`, manages local and attached `previa-runner` instances, opens the IDE in the browser, and helps you bootstrap projects from pipeline files.

## Quick Links

- [Website](https://previa.dev)
- [Documentation](docs/previa/README.md)
- [Releases](https://github.com/runvibe/previa/releases)
- [Security policy](SECURITY.md)

## What Is Previa?

Previa combines local runtime operations with project-scoped API testing workflows:

- `previa` runs and manages the local stack
- `previa-main` is the orchestrator API for projects, specs, pipelines, history, proxying, queues, and MCP
- `previa-runner` executes E2E and load tests
- the browser IDE is served by `previa-main`; external builds can point to a main API with `VITE_PREVIA_API_BASE_URL`

In practice, the flow looks like this:

```text
previa CLI -> previa-main -> previa-runner -> target API
```

## Why Previa Exists

I created Previa to make end-to-end and load testing simple enough for any AI to understand, generate, and execute through pipelines. The goal was to build a testing system that could become the preferred runtime for AI-first development workflows.

I have been building with an AI-first mindset since 2025, when I started using tools like Codex and Claude Code heavily in day-to-day development. Over time, I kept running into the same bottleneck: tests were often missing, brittle, misleading, or easy for AI assistants to fake with weak assertions that looked correct but did not really protect real user flows.

That led to a simple idea: end-to-end testing should live outside the application as an independent runtime that any team, developer, or AI assistant can use to verify whether a real workflow broke. And once that runtime already understands the system, it should also make load testing just as easy, whether through a few clicks in the IDE or a prompt sent from an AI assistant.

*Philippe Assis*

## AI-First Workflow

Previa is designed to work well in AI-first development loops:

- the CLI starts a real test runtime outside your application codebase
- the IDE gives you a visual place to inspect specs, pipelines, executions, and failures
- the HTTP API lets CI/CD and automation trigger the same workflows
- the MCP server lets assistants inspect, generate, validate, and troubleshoot using the same runtime

The main idea is simple: your assistant should not have to guess whether a workflow still works. It should be able to ask Previa to run it.

## Install

Install the CLI with:

```bash
curl -fsSL https://raw.githubusercontent.com/runvibe/previa/main/install.sh | sh
```

On Windows, use:

```powershell
irm https://raw.githubusercontent.com/runvibe/previa/main/install.ps1 | iex
```

The installers detect Linux, macOS, or Windows, install the matching `previa` control binary under the default user home, and configure `PREVIA_HOME`. macOS releases publish native `amd64` and Apple Silicon `arm64` control binaries.

## Quick Start

`-d` is the short form of `--detach`.

The shortest happy path is:

```text
install -> up -> open -> create or import a pipeline -> run tests
```

Start a Docker-backed stack:

```bash
previa up -d
```

This is the general cross-platform path when Docker is available.

Start a binary-backed stack without Docker:

```bash
previa up -d --bin
```

This path is Linux-only and aimed at local runtime development.
On macOS and Windows, the control binary is supported, but the `--bin` feature is not exposed.

Inspect the runtime and open the IDE:

```bash
previa status
previa open
```

Inside an application repository, use the project-local workflow to keep state
under `./.previa`:

```bash
previa local up -d
previa local status
previa local open
```

Push a local project to a remote Previa main:

```bash
previa local push --project my_app --to https://previa.example.com
previa local push --project my_app --to https://previa.example.com --overwrite
```

Move one or more project-local projects as a SQLite database:

```bash
previa local export --all --output ./previa-projects.sqlite3
previa local export --project project_id --output ./project.sqlite3
previa local import ./previa-projects.sqlite3
```

### Local wave load target

For validating wave load-test behavior without an external API, start the deterministic local target stack:

```bash
scripts/start-local-load-target-stack.sh
```

The script starts:

- Previa main on `http://127.0.0.1:5610`
- three runners on `5611`, `5612`, and `5613`
- the deterministic load target on `http://127.0.0.1:5620`

It also creates a project named `Local Load Target Reference` and prints the load-test URL. During or after a run, inspect the target-side counters:

```bash
curl -fsS http://127.0.0.1:5620/metrics | jq
```

Use this target to compare the configured wave, runner HTTP RPS, and target-side received RPS without depending on DNS, gateway behavior, or a remote application.

`previa open` launches:

```text
http://127.0.0.1:5588
```

The embedded app uses `window.location.origin` as the API base. When the app is built as an external artifact, set `VITE_PREVIA_API_BASE_URL` at build time to point it at a `previa-main`.

PWA service worker precaching is disabled by default so the embedded app cannot keep stale local bundles after a `previa-main` rebuild. If you intentionally want a standalone PWA build, set `VITE_PREVIA_ENABLE_PWA=true` during the app build.

Se o comando de browser falhar, o CLI mantém a saída em erro, destaca a mensagem em vermelho e ainda imprime a URL final para abrir manualmente.

## Documentation

Start here for the full documentation hub:

- [Previa docs index](docs/previa/README.md)

Recommended first reads:

- [Getting started](docs/previa/getting-started.md)
- [Minimal happy path](docs/previa/minimal-happy-path.md)
- [Architecture at a glance](docs/previa/architecture.md)
- [Runtime modes](docs/previa/runtime-modes.md)
- [Release and install](docs/previa/release-install.md)
- [MCP integration](docs/previa/mcp.md)
- [Operations cheatsheet](docs/previa/operations-cheatsheet.md)
- [Contributing](CONTRIBUTING.md)

## Release Scope

The release workflow supports four manual publishing scopes:

- `linux`: GitHub Release assets for Linux plus Docker, crates.io, and release metadata publishing
- `mac`: GitHub Release assets for macOS only
- `windows`: GitHub Release asset for Windows only
- `all`: Linux full publishing plus release metadata and macOS and Windows release assets

Today, macOS and Windows release assets contain only the `previa` control binary. macOS publishes both `amd64` and `arm64`; Linux continues to publish `previa`, `previa-main`, and `previa-runner`.

## Community

- [Contributing guide](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

## License

Previa is released under the MIT License.
