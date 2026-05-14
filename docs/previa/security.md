# Security

This guide covers the most important security behaviors in local and remote Previa deployments.

## Access Management

Previa starts in anonymous full-access mode by default. This preserves local
development behavior and does not require JWTs or API tokens.

For shared, remote, or exposed runtimes, enable protected mode:

```bash
printf '%s' 'change-me' | previa up -d --protected --root-username root --root-password-stdin
```

Protected mode requires:

- `PREVIA_AUTH_ANONYMOUS=false`
- `PREVIA_ROOT_USERNAME`
- `PREVIA_ROOT_PASSWORD`
- `PREVIA_JWT_SECRET`

Only `GET /health`, `POST /api/v1/auth/login`, and static app assets are public
in protected mode. Browser users receive JWTs. CLI, MCP, CI, scripts, and
direct API clients should use fixed API tokens.

Recommendations:

- use anonymous mode only for trusted local development
- keep root credentials, JWT secrets, and raw API tokens out of source control
- create named users for people and named API tokens for automation
- grant the least role needed, especially for long-lived API tokens
- revoke unused API tokens

## Main and Runner Authentication

Use `RUNNER_AUTH_KEY` to secure communication between `previa-main` and `previa-runner`.

Important rules:

- local-only stacks auto-generate a UUID v4 when no key is configured
- attached runners require an explicit shared key
- health and info endpoints on protected runners also require the key

## MCP Exposure

When `MCP_ENABLED=true`, `previa-main` exposes an MCP HTTP endpoint, usually at:

```text
http://localhost:5588/mcp
```

Recommendations:

- prefer local-only exposure during development
- be deliberate before exposing the MCP endpoint on public interfaces
- remember that MCP clients can operate against projects, pipelines, specs, queues, and transfers

## Port Exposure

Default local ports are:

- `5588` for `previa-main`
- `55880+` for local runners

Recommendations:

- prefer loopback addresses for local-only work
- expose remote runners intentionally and protect them with `RUNNER_AUTH_KEY`
- avoid publishing development ports broadly unless needed

## Project Data

`PREVIA_HOME` stores:

- runtime env files
- logs
- orchestrator database
- generated compose files
- runtime state

Recommendations:

- treat `PREVIA_HOME` as operational data
- avoid committing local runtime files into application repositories
- isolate experiments with `--home ./.previa` or another dedicated path

## Practical Checklist

- use the generated local key or define a shared `RUNNER_AUTH_KEY`
- require explicit auth for attached runners
- expose MCP only where intended
- keep remote runner ports scoped to trusted networks
- review project export/import payloads before sharing them

## See Also

- [Access management](./access-management.md)
- [Main and runner authentication](./main-runner-auth.md)
- [Remote runners](./remote-runners.md)
- [MCP integration](./mcp.md)
