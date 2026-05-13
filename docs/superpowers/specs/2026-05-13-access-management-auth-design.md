# Access Management and Authentication Design

## Goal

Add optional access management to Previa without changing the current default
open workflow.

When anonymous access is enabled, Previa behaves as it does today: every route
is available without a JWT and the effective user has full access. When
anonymous access is disabled, `previa-main` requires username/password login,
issues JWTs, protects all non-health routes, and exposes user management for
administrators.

This feature spans:

- `main`: authentication, authorization, persistence, OpenAPI contracts.
- `app`: login screen, authenticated API client, user management UI.
- `previa`: `login`, `logout`, `whoami`, and Bearer token support for CLI HTTP
  calls.

## Non-Goals

V0 does not include SSO, OAuth, LDAP, MFA, password reset email flows, scoped API
tokens, project-level ACLs, or server-side JWT revocation. The design keeps JWT
claims and user records small enough for those features to be added in a later
version without changing the core auth mode contract.

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

- No JWT is required.
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

In protected mode, clients must send:

```http
Authorization: Bearer <jwt>
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
main/src/server/handlers/auth.rs
main/src/server/handlers/users.rs
main/src/server/middleware/auth.rs
main/src/server/middleware/authorize.rs
main/src/server/services/auth.rs
```

Responsibilities:

- `auth/config.rs`: parse envs, choose `AuthMode`, validate protected mode.
- `auth/passwords.rs`: hash and verify passwords with Argon2id.
- `auth/tokens.rs`: issue and verify JWTs.
- `auth/permissions.rs`: map roles to permissions and route requirements.
- `services/auth.rs`: login, current user resolution, user CRUD rules.
- `db/users.rs`: SQLx persistence for database users.
- `handlers/auth.rs`: request/response wiring for auth routes.
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

## API Contracts

New request/response models live in `main/src/server/models.rs` or a dedicated
auth models module re-exported into OpenAPI.

### Login

```http
POST /api/v1/auth/login
```

Request:

```json
{
  "username": "root",
  "password": "secret"
}
```

Response:

```json
{
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

Failures:

- `401 unauthorized` for invalid credentials.
- `403 forbidden` for inactive database users.

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
- Apply `auth` middleware to the API router.
- Apply permission layers by route group where practical:
  - read routes: `viewer`.
  - write routes: `editor`.
  - execution routes: `operator`.
  - user routes: `admin`.

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
previa logout --context default
previa whoami --context default
```

Local context token storage:

```text
PREVIA_HOME/stacks/<context>/auth.json
```

Remote URL token storage can use:

```text
PREVIA_HOME/auth/<url-hash>.json
```

Stored shape:

```json
{
  "baseUrl": "http://127.0.0.1:5588",
  "username": "root",
  "token": "jwt",
  "expiresAt": "2026-05-14T12:00:00Z"
}
```

CLI HTTP helpers should:

- Load the token for the selected context or URL.
- Attach `Authorization: Bearer <token>` to protected API calls.
- On `401`, print an actionable message:

```text
authentication required; run `previa login --context default`
```

`previa login` should prompt for a password without echoing. If adding a
password prompt dependency is not desired in V0, support `--password-stdin` and
document that interactive masking will follow.

## Security Notes

- Use Argon2id for database user password hashes.
- Compare secrets using constant-time comparison where available.
- Never log passwords or JWTs.
- JWT claims should include subject, username, role, source, issued-at, expiry,
  and issuer.
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

- Anonymous mode permits existing protected API routes without JWT.
- Protected mode permits only `/health` and `/api/v1/auth/login` without JWT.
- Protected mode rejects `/info`, `/openapi.json`, `/proxy`, `/mcp`, and
  `/api/v1/projects` without JWT.
- Login succeeds for env root.
- Login fails for invalid env root credentials.
- Database user login succeeds with active user.
- Inactive user login returns forbidden.
- Role checks reject insufficient permissions.
- Password hashes are not returned by API.

Frontend tests:

- Login page submits credentials and stores token.
- Authenticated requests include Bearer token.
- `401` clears auth state and returns to login.
- Access management page is visible only to `root/admin`.
- Anonymous mode does not block current app workflows.

CLI tests:

- `previa login` stores token for a context.
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
- Protected mode leaves only `/health` and `/api/v1/auth/login` public.
