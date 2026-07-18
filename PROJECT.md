# Project Notes

## Release and Install Workflow

- Treat project env groups as runtime configuration for executions. OpenAPI specs remain contract metadata; new pipelines should prefer `{{envs.current.<entry>}}` for selectable runtime URLs and reserve `{{specs.<slug>.url.<env>}}` for spec-bound server references.

- The orchestrator API contract is generated from Rust handlers and models via `utoipa` in `main/src/server/docs.rs`, then served from `/openapi.json`. When adding or changing API routes, update the handler annotation, the `docs.rs` path/component list, and the TypeScript client in `app/src/lib/api-client.ts`.

- Postgres is mandatory for operational state and the execution queue.
  `DbPool::connect` rejects SQLite. SQLite access is private to
  `services/sqlite_transfer.rs` for portable project import/export only.
  Main/runner execution communication must use the fenced Postgres queue;
  runner HTTP is operational-only (`/health`, `/ready`, `/info`,
  `/openapi.json`).

- After API client or OpenAPI route changes, validate:
  - `cargo test -p previa-main server::docs`
  - `python3 scripts/check_openapi_client_contract.py`
  - `npm test`

- Keep GitHub Release asset names aligned with installer platform slugs:
  - Linux: `previa-linux-amd64`, `previa-linux-arm64`
  - macOS: `previa-macos-amd64`, `previa-macos-arm64`
  - Windows: `previa-windows-amd64.exe`
- Keep `scripts/generate_release_metadata.py` in sync with `.github/workflows/release.yaml` whenever release matrix entries change.
- Keep `install.sh` architecture detection aligned with published Unix release assets.
- After release workflow or installer changes, validate:
  - `sh -n install.sh`
  - `python3 scripts/test_release_metadata.py`
  - `cargo build --release`
  - `cargo test`
