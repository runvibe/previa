# Agents

- Document and adjust each agent as provider workflows change.
- **Architecture**: keep transport concerns inside `handlers/` and router
  wiring, move reusable logic into dedicated modules, and align data contracts
  with generated OpenAPI and `models` structs.
- **Project structure**: `handlers/` handles HTTP transport and wiring,
  `services/` owns reusable business logic and integrations, `models` defines
  data contracts and DB-facing structs; keep modules small and focused.
- **Separation rule**: always split handlers from services and models; do not
  mix request/response handling with business logic or data structs.
- **API contracts**: Previa currently generates OpenAPI through `utoipa` in
  `main/src/server/docs.rs` and serves it at `/openapi.json`; keep route
  annotations, models, and the TypeScript API client in sync.
- **Persistence**: prefer SQLx with bound parameters, reuse `migrations/`, and
  avoid inline schema drift. The main API supports SQLite and Postgres through
  `sqlx::Any`, so use `DbPool::query`, `DbPool::sql`, or `QueryBuilder` for
  backend-portable SQL; use SQLx macros only where they fit that portability.
- **Processes**: discuss changes via PROJECT.md conventions, open pull requests
  with review context, and keep agents.md current when workflows shift.
- **Release build & push**: after finishing any change run
  `cargo build --release`; if the release build succeeds, commit the changes and
  push to the remote.
