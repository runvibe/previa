# Access Management and Authentication Design

## Goal

Add optional access management to Previa without changing the current default
open workflow.

When anonymous access is enabled, Previa behaves as it does today: every route
is available without a JWT or API token and the effective user has full access.
When anonymous access is disabled, `previa-main` requires authenticated access,
issues JWTs for app sessions, accepts fixed API tokens for CLI/MCP/direct API
clients, protects all non-health routes, and exposes user management for
administrators.

This feature spans:

- `main`: authentication, authorization, persistence, OpenAPI contracts.
- `app`: login screen, JWT-backed authenticated API client, user management UI.
- `previa`: `login`, `logout`, `whoami`, API token management, and Bearer token
  support for CLI HTTP calls.

## Non-Goals

V0 does not include SSO, OAuth, LDAP, MFA, password reset email flows,
project-level ACLs, or server-side JWT revocation. API tokens are included, but
they use the same fixed roles as users rather than per-resource scopes. The
design keeps JWT claims, API tokens, and user records small enough for those
features to be added in a later version without changing the core auth mode
contract.

## Auth Modes

Authentication is controlled by environment configuration at `previa-main`
startup.

```env
PREVIA_AUTH_ANONYMOUS=true
PREVIA_ROOT_USERNAME=root
PREVIA_ROOT_PASSWORD=change-me
PREVIA_JWT_SECRET=...
PREVIA_JWT_TTL_SECONDS=86400
```

### Anonymous Full Access

`PREVIA_AUTH_ANONYMOUS=true` enables the current Previa behavior.

- No JWT or API token is required.
- All API routes are available.
- The app opens directly.
- CLI commands continue to work without login.
- The effective principal is `anonymous` with full access.
- `POST /api/v1/auth/login` returns `409 conflict` with an `auth_disabled`
  error because login is not needed in anonymous mode.

This is the default mode to preserve backwards compatibility when no auth envs
are configured.

### Protected

`PREVIA_AUTH_ANONYMOUS=false` enables protected mode.

Public routes in protected mode:

- `GET /health`
- `POST /api/v1/auth/login`
- Static app assets needed to render the login screen.

Protected routes in protected mode:

- `/info`
- `/openapi.json`
- `/proxy`
- `/mcp` when MCP is enabled.
- All `/api/v1/**` routes except `/api/v1/auth/login`.
- All app data/API access after static asset delivery.

In protected mode, clients must send Bearer credentials:

```http
Authorization: Bearer <jwt>
```

JWTs are for browser app sessions. CLI, MCP, scripts, and direct API consumers
should use API tokens:

```http
Authorization: Bearer <api-token>
```

### Startup Validation

When protected mode is enabled, `previa-main` must fail startup if any required
root or JWT configuration is missing or blank:

- `PREVIA_ROOT_USERNAME`
- `PREVIA_ROOT_PASSWORD`
- `PREVIA_JWT_SECRET`

The failure message should name the missing env var and state that protected
mode requires it.

For local stacks managed by the `previa` CLI, protected mode generation is part
of the CLI implementation: when protected mode is requested and
`PREVIA_JWT_SECRET` is absent, the CLI generates a random secret and persists it
in the context `main.env`. Direct `previa-main` startup always requires explicit
configuration.

## Principals and Roles

The system has two kinds of principals:

- Environment root user: configured from env, never stored as a normal mutable
  user record.
- Database users: managed through the access management UI/API.
- API token principals: fixed tokens created by `root` or `admin`, stored only
  as hashes, and used by CLI, MCP, scripts, or direct API consumers without an
  interactive login.

V0 uses fixed roles:

- `root`: full access, includes user management.
- `admin`: full access, includes user management, cannot mutate the env root.
- `editor`: read/write projects, specs, env groups, pipelines, imports/exports,
  and executions.
- `operator`: read data, run/cancel executions, manage queues, inspect runners.
- `viewer`: read-only access.
- `anonymous`: full access only when anonymous mode is enabled.

The first implementation should keep role checks centralized in one permissions
module instead of spreading role strings through handlers.

## Backend Architecture

Keep transport concerns in routes/handlers, reusable logic in services, and data
contracts in models, following the existing project rules.

New modules:

```text
main/src/server/auth/config.rs
main/src/server/auth/mod.rs
main/src/server/auth/passwords.rs
main/src/server/auth/permissions.rs
main/src/server/auth/tokens.rs
main/src/server/db/users.rs
main/src/server/db/api_tokens.rs
main/src/server/handlers/auth.rs
main/src/server/handlers/api_tokens.rs
main/src/server/handlers/users.rs
main/src/server/middleware/auth.rs
main/src/server/middleware/authorize.rs
main/src/server/services/auth.rs
```

Responsibilities:

- `auth/config.rs`: parse envs, choose `AuthMode`, validate protected mode.
- `auth/passwords.rs`: hash and verify passwords with Argon2id.
- `auth/tokens.rs`: issue and verify app-session JWTs, generate API tokens, and
  hash API token secrets for lookup.
- `auth/permissions.rs`: map roles to permissions and route requirements.
- `services/auth.rs`: login, current user resolution, user CRUD rules, API
  token creation, token authentication, and token revocation rules.
- `db/users.rs`: SQLx persistence for database users.
- `db/api_tokens.rs`: SQLx persistence for hashed API tokens.
- `handlers/auth.rs`: request/response wiring for auth routes.
- `handlers/api_tokens.rs`: request/response wiring for API token management.
- `handlers/users.rs`: request/response wiring for user management.
- `middleware/auth.rs`: attach an effective principal to request extensions.
- `middleware/authorize.rs`: reject missing or insufficient principals.

`AppState` should gain an `auth` field containing immutable auth config and JWT
services. Runtime DB user lookups stay in `services/auth.rs`.

## Persistence

Add migrations for SQLite and Postgres.

Table: `users`

```sql
id TEXT PRIMARY KEY NOT NULL,
username TEXT NOT NULL UNIQUE,
password_hash TEXT NOT NULL,
role TEXT NOT NULL,
active INTEGER/BIGINT NOT NULL DEFAULT 1,
created_at TEXT NOT NULL,
updated_at TEXT NOT NULL,
created_at_ms INTEGER/BIGINT NOT NULL,
updated_at_ms INTEGER/BIGINT NOT NULL
```

SQLite should use `INTEGER`; Postgres migrations in this codebase use `BIGINT`
for boolean-like and timestamp integer fields.

Indexes:

- unique username.
- role if useful for future filtering.
- updated_at_ms for management UI ordering.

Do not store the env root password. The env root authenticates against the env
password and is represented as a synthetic principal.

Table: `api_tokens`

```sql
id TEXT PRIMARY KEY NOT NULL,
name TEXT NOT NULL,
token_prefix TEXT NOT NULL,
token_hash TEXT NOT NULL UNIQUE,
role TEXT NOT NULL,
created_by_user_id TEXT,
created_by_username TEXT NOT NULL,
active INTEGER/BIGINT NOT NULL DEFAULT 1,
last_used_at TEXT,
expires_at TEXT,
created_at TEXT NOT NULL,
updated_at TEXT NOT NULL,
created_at_ms INTEGER/BIGINT NOT NULL,
updated_at_ms INTEGER/BIGINT NOT NULL
```

API token records keep enough metadata for management and auditing but never
store the raw token value. `token_prefix` is a short display prefix such as
`pvk_abc123` so users can recognize which local secret they are using.

V0 supports non-expiring tokens by leaving `expires_at` null. The API accepts an
optional expiry when creating a token, but the default is fixed/non-expiring
because the primary use case is CLI and MCP automation without repeated login.

## API Contracts

New request/response models live in `main/src/server/models.rs` or a dedicated
auth models module re-exported into OpenAPI.

### Login

```http
POST /api/v1/auth/login
```

The login route is the only public credential exchange in protected mode.
Browser clients use it to get a short-lived JWT. CLI clients use it to bootstrap
a fixed API token and then use that API token for normal CLI requests.

App request:

```json
{
  "username": "root",
  "password": "secret",
  "clientKind": "app"
}
```

App response:

```json
{
  "tokenKind": "jwt",
  "token": "jwt",
  "expiresAt": "2026-05-14T12:00:00Z",
  "user": {
    "id": "root",
    "username": "root",
    "role": "root",
    "source": "env"
  }
}
```

CLI bootstrap request:

```json
{
  "username": "ana",
  "password": "secret",
  "clientKind": "api_token",
  "tokenName": "previa-cli-default"
}
```

CLI bootstrap response:

```json
{
  "tokenKind": "api_token",
  "token": "pvk_abc123.full-secret-only-shown-once",
  "record": {
    "id": "tok_123",
    "name": "previa-cli-default",
    "tokenPrefix": "pvk_abc123",
    "role": "editor",
    "active": true,
    "expiresAt": null,
    "createdByUsername": "ana",
    "lastUsedAt": null,
    "createdAt": "2026-05-13T10:00:00Z",
    "updatedAt": "2026-05-13T10:00:00Z"
  }
}
```

For `clientKind: "api_token"`, the created token role defaults to the
authenticated user's role and cannot exceed that user's role. The JWT returned
for `clientKind: "app"` is intended for the browser app session. JWT is not the
normal credential for CLI, MCP, scripts, or direct API clients.

Failures:

- `401 unauthorized` for invalid credentials.
- `403 forbidden` for inactive database users.

### API Tokens

API tokens are long-lived Bearer credentials for non-interactive clients. They
are the standard credential for the Previa CLI, MCP integrations, scripts, and
direct API consumers. They are accepted by the same protected routes as app
JWTs:

```http
Authorization: Bearer pvk_...
```

Only `root` and `admin` can create, list, disable, and delete API tokens through
the management routes below. `admin` cannot create or elevate a token to `root`.
Non-admin users can create their own CLI token only through
`POST /api/v1/auth/login` with `clientKind: "api_token"`.

```http
GET /api/v1/api-tokens
POST /api/v1/api-tokens
PATCH /api/v1/api-tokens/{tokenId}
DELETE /api/v1/api-tokens/{tokenId}
```

Create token request:

```json
{
  "name": "local-cli-default",
  "role": "operator",
  "expiresAt": null
}
```

Create token response:

```json
{
  "token": "pvk_abc123.full-secret-only-shown-once",
  "record": {
    "id": "tok_123",
    "name": "local-cli-default",
    "tokenPrefix": "pvk_abc123",
    "role": "operator",
    "active": true,
    "expiresAt": null,
    "createdByUsername": "root",
    "lastUsedAt": null,
    "createdAt": "2026-05-13T10:00:00Z",
    "updatedAt": "2026-05-13T10:00:00Z"
  }
}
```

List token response returns records only and never includes the raw token:

```json
[
  {
    "id": "tok_123",
    "name": "local-cli-default",
    "tokenPrefix": "pvk_abc123",
    "role": "operator",
    "active": true,
    "expiresAt": null,
    "createdByUsername": "root",
    "lastUsedAt": null,
    "createdAt": "2026-05-13T10:00:00Z",
    "updatedAt": "2026-05-13T10:00:00Z"
  }
]
```

Disabling or deleting a token must immediately prevent new requests from
authenticating with it.

### Current User

```http
GET /api/v1/auth/me
```

Response:

```json
{
  "id": "usr_123",
  "username": "ana",
  "role": "editor",
  "source": "database"
}
```

In anonymous mode this returns:

```json
{
  "id": "anonymous",
  "username": "anonymous",
  "role": "anonymous",
  "source": "anonymous"
}
```

### Users

```http
GET /api/v1/users
POST /api/v1/users
PATCH /api/v1/users/{userId}
DELETE /api/v1/users/{userId}
POST /api/v1/users/{userId}/password
```

Only `root` and `admin` can manage users.

Create user request:

```json
{
  "username": "ana",
  "password": "initial-secret",
  "role": "editor",
  "active": true
}
```

User record response:

```json
{
  "id": "usr_123",
  "username": "ana",
  "role": "editor",
  "active": true,
  "createdAt": "2026-05-13T10:00:00Z",
  "updatedAt": "2026-05-13T10:00:00Z"
}
```

Password hashes must never be serialized.

## Route Protection

The router should be structured so auth rules are easy to audit. Protected mode
must not rely on every handler remembering to check permissions manually.

Suggested grouping:

- Build public routes: `/health`, `/api/v1/auth/login`, static app fallback for
  login assets.
- Build API router: all `/api/v1/**` routes other than login.
- Apply `auth` middleware to the API router. The middleware accepts either an
  app-session JWT or an API token in the Bearer header.
- Apply permission layers by route group where practical:
  - read routes: `viewer`.
  - write routes: `editor`.
  - execution routes: `operator`.
  - user routes: `admin`.
  - API token routes: `admin`.

In anonymous mode, auth middleware attaches anonymous full-access principal and
authorization always passes.

SSE endpoints must accept Bearer tokens through regular headers. The app should
open SSE with fetch/readable stream if native `EventSource` cannot set headers.

## Frontend Architecture

New app files:

```text
app/src/lib/auth-client.ts
app/src/stores/useAuthStore.ts
app/src/pages/LoginPage.tsx
app/src/pages/AccessManagementPage.tsx
app/src/components/AuthGate.tsx
```

Changes:

- `api-client.ts` injects `Authorization: Bearer <token>` when a token exists.
- `api-client.ts` handles `401` by clearing auth state and routing to login.
- `App.tsx` wraps protected routes in `AuthGate`.
- `AppShell` adds an access management nav/action for `root` and `admin`.
- Login screen posts to `/api/v1/auth/login` without a token.
- Access management page lists users, creates users, edits role/status, resets
  passwords, and deletes users.
- Access management page also lists API tokens, creates new tokens, shows a new
  raw token exactly once, disables tokens, and deletes tokens.

When protected mode is active, the app should not call `/info` before login.
Instead, it should render login when no token exists and fetch `/auth/me` after
login.

In anonymous mode, `AuthGate` should allow entry without a token once it detects
anonymous access or when protected requests do not return `401`.

## CLI Architecture

Add CLI commands:

```text
previa login --context default
previa login --url http://localhost:5588
previa token use --context default --token-env PREVIA_API_TOKEN
previa token create --context default --name local-cli --role operator
previa token list --context default
previa token revoke --context default <TOKEN_ID>
previa logout --context default
previa whoami --context default
```

`previa login` is a convenience bootstrap for CLI use: it prompts for
username/password, asks `previa-main` to create an API token for the selected
context or URL, stores that API token locally, and does not keep the app-session
JWT as the CLI credential.

Local context API token storage:

```text
PREVIA_HOME/stacks/<context>/auth.json
```

Remote URL API token storage can use:

```text
PREVIA_HOME/auth/<url-hash>.json
```

Stored shape:

```json
{
  "baseUrl": "http://127.0.0.1:5588",
  "username": "root",
  "tokenKind": "api_token",
  "token": "pvk_abc123.full-secret-only-shown-once",
  "tokenId": "tok_123",
  "tokenPrefix": "pvk_abc123"
}
```

For automation, users can skip interactive login and set a fixed API token in a
local environment variable:

```bash
export PREVIA_API_TOKEN=pvk_abc123.full-secret-only-shown-once
previa status --context default
```

CLI credential resolution order:

1. `--token-env <ENV_NAME>` when provided
2. `PREVIA_API_TOKEN`
3. stored context API token from `auth.json`
4. stored remote URL API token from `PREVIA_HOME/auth/<url-hash>.json`

CLI HTTP helpers should:

- Load the API token for the selected context or URL.
- Attach `Authorization: Bearer <token>` to protected API calls.
- On `401`, print an actionable message:

```text
authentication required; run `previa login --context default`
```

`previa login` should prompt for a password without echoing, create/store an API
token, and discard any temporary app-session JWT used during bootstrap. If
adding a password prompt dependency is not desired in V0, support
`--password-stdin` and document that interactive masking will follow.

MCP install/print workflows should support API-token based configuration. When a
token env var is specified, the generated MCP command or config should pass that
environment variable through to the Previa MCP client process rather than
embedding the raw token in generated files.

## Security Notes

- Use Argon2id for database user password hashes.
- Store API tokens only as strong hashes; show the raw token only once at
  creation time.
- Compare secrets using constant-time comparison where available.
- Never log passwords, JWTs, or raw API tokens.
- JWT claims should include subject, username, role, source, issued-at, expiry,
  and issuer. JWTs are for the app session, not the normal CLI/MCP credential.
- API token authentication should produce a principal with subject, token id,
  token name, role, and source `api_token`.
- Use a short default TTL, configurable by env.
- Keep `RUNNER_AUTH_KEY` separate from user JWT auth. Main-runner auth protects
  runner transport; user auth protects clients accessing `previa-main`.
- CORS currently allows any origin. In protected mode, this remains acceptable
  for Bearer-token APIs only if tokens are not stored in cookies. V0 should use
  local storage or memory-backed app state, not cookies.

## Documentation

Update:

- `docs/previa/security.md`
- `docs/previa/cli-commands.md`
- `docs/previa/operations.md`
- `docs/previa/main-runner-auth.md` only to clarify that runner auth is
  separate from user access management.
- `PROJECT.md` if auth env conventions become part of operational workflow.

## Testing

Backend tests:

- Anonymous mode permits existing protected API routes without JWT or API token.
- Protected mode permits only `/health` and `/api/v1/auth/login` without a
  Bearer credential.
- Protected mode rejects `/info`, `/openapi.json`, `/proxy`, `/mcp`, and
  `/api/v1/projects` without a Bearer credential.
- Login succeeds for env root.
- Login fails for invalid env root credentials.
- Database user login succeeds with active user.
- Inactive user login returns forbidden.
- App login returns a JWT session token.
- CLI login bootstrap creates and stores an API token rather than persisting a
  JWT.
- API token creation returns the raw token once and stores only the hash.
- API token Bearer auth can access protected routes according to its role.
- Disabled, deleted, expired, and malformed API tokens are rejected.
- Role checks reject insufficient permissions.
- Password hashes are not returned by API.

Frontend tests:

- Login page submits credentials and stores token.
- Authenticated requests include Bearer token.
- Access management page creates API tokens and displays the new raw token once.
- Access management page lists token records without raw token values.
- `401` clears auth state and returns to login.
- Access management page is visible only to `root/admin`.
- Anonymous mode does not block current app workflows.

CLI tests:

- `previa login` stores an API token for a context.
- `PREVIA_API_TOKEN` is used before stored API token credentials.
- `previa token create` prints the raw token once.
- `previa logout` removes token.
- API helpers send Bearer token when present.
- `401` prints the login guidance.

## Release Criteria

- `cargo test`
- `npm test` in `app/`
- `npm run build` in `app/`
- `cargo build --release`
- Existing anonymous/default workflows still work without configuring auth.
- Protected mode requires login in app and CLI.
- Protected mode supports fixed API tokens for CLI, MCP, scripts, and direct API
  consumers.
- Protected mode leaves only `/health` and `/api/v1/auth/login` public.
