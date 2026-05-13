# Access Management Auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement protected-mode access management with app-session JWTs and fixed API tokens for CLI, MCP, scripts, and direct API access.

**Architecture:** Keep route handlers thin, add auth services and DB modules, and enforce route access through middleware. Anonymous mode remains the default and bypasses JWT/API-token requirements with full access.

**Tech Stack:** Rust 2024, Axum, SQLx Any, SQLite/Postgres migrations, Vite React, Zustand, Vitest, Clap CLI.

---

### Task 1: Backend Auth Foundation

**Files:**
- Create: `main/src/server/auth/config.rs`
- Create: `main/src/server/auth/mod.rs`
- Create: `main/src/server/auth/permissions.rs`
- Create: `main/src/server/auth/tokens.rs`
- Create: `main/src/server/auth/passwords.rs`
- Modify: `main/Cargo.toml`
- Modify: `main/src/server/state.rs`
- Modify: `main/src/main.rs`

- [ ] Write failing unit tests for auth mode parsing, protected-mode validation, role permissions, JWT issue/verify, API token hash/verify, and password hash/verify.
- [ ] Run the targeted Rust tests and confirm they fail because the modules do not exist.
- [ ] Implement the auth foundation modules.
- [ ] Run targeted tests and confirm they pass.

### Task 2: Backend Persistence and Auth Routes

**Files:**
- Create: `main/migrations/sqlite/202605140001_add_access_management.sql`
- Create: `main/migrations/postgres/202605140001_add_access_management.sql`
- Create: `main/src/server/db/users.rs`
- Create: `main/src/server/db/api_tokens.rs`
- Create: `main/src/server/services/auth.rs`
- Create: `main/src/server/handlers/auth.rs`
- Create: `main/src/server/handlers/api_tokens.rs`
- Create: `main/src/server/handlers/users.rs`
- Modify: `main/src/server/db/mod.rs`
- Modify: `main/src/server/handlers/mod.rs`
- Modify: `main/src/server/mod.rs`
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/docs.rs`

- [ ] Write failing route tests for anonymous login conflict, app login JWT, CLI login API token bootstrap, `/auth/me`, user CRUD, and API token CRUD.
- [ ] Run targeted tests and confirm the new behavior fails before implementation.
- [ ] Implement migrations, DB helpers, services, models, routes, and OpenAPI registration.
- [ ] Run targeted tests and confirm they pass.

### Task 3: Route Protection and Roles

**Files:**
- Create: `main/src/server/middleware/auth.rs`
- Create: `main/src/server/middleware/authorize.rs`
- Modify: `main/src/server/middleware/mod.rs`
- Modify: `main/src/server/mod.rs`

- [ ] Write failing integration tests proving protected mode allows only `/health` and `/api/v1/auth/login` without credentials.
- [ ] Write failing integration tests proving editor cannot mutate runners, operator can run/cancel executions, viewer cannot mutate data, and API tokens obey their role.
- [ ] Implement auth/authorization middleware and route grouping.
- [ ] Run targeted route tests and confirm they pass.

### Task 4: CLI API Token Support

**Files:**
- Create: `previa/src/auth.rs`
- Modify: `previa/src/cli.rs`
- Modify: `previa/src/lib.rs`
- Modify: CLI HTTP helpers in `previa/src/local_push.rs`, `previa/src/export.rs`, `previa/src/runner_cli.rs`, and `previa/src/pipeline_import.rs`

- [ ] Write failing CLI tests for `previa login`, `logout`, `whoami`, `token create/list/revoke`, `PREVIA_API_TOKEN`, and `--token-env`.
- [ ] Implement auth storage and Bearer injection.
- [ ] Run targeted CLI tests and confirm they pass.

### Task 5: Frontend Auth and Access Management

**Files:**
- Create: `app/src/lib/auth-client.ts`
- Create: `app/src/stores/useAuthStore.ts`
- Create: `app/src/pages/LoginPage.tsx`
- Create: `app/src/pages/AccessManagementPage.tsx`
- Create: `app/src/components/AuthGate.tsx`
- Modify: `app/src/lib/api-client.ts`
- Modify: `app/src/App.tsx`
- Modify: `app/src/components/AppShell.tsx`

- [ ] Write failing Vitest tests for login, Bearer injection, 401 handling, access page visibility, user management, and API token one-time display.
- [ ] Implement the app auth gate, login flow, access management page, and client headers.
- [ ] Run targeted Vitest tests and confirm they pass.

### Task 6: Full Verification and Finish

- [ ] Run `cargo test`.
- [ ] Run `npm test` in `app/`.
- [ ] Run `npm run build` in `app/`.
- [ ] Run `cargo build --release`.
- [ ] Commit and push `codex/access-management`.
