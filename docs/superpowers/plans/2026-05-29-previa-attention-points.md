# Previa Attention Points Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve the project analysis attention points around project instructions, OpenAPI/client drift, SQLx persistence guidance, and frontend test noise.

**Architecture:** Keep the current Rust and React architecture intact. Add small guardrails and documentation that match the existing `handlers/`, `services/`, `models`, `utoipa`, and `sqlx::Any` patterns instead of forcing a broad refactor.

**Tech Stack:** Rust 2024, Axum, SQLx, Utoipa, React, Vite, Vitest, Python 3.

---

### Task 1: Align Project Guidance

**Files:**
- Modify: `AGENTS.md`
- Modify: `PROJECT.md`
- Modify: `docs/previa/architecture.md`

- [x] Update project instructions to name `handlers/`, generated OpenAPI, and portable SQLx usage.
- [x] Document API contract validation expectations in `PROJECT.md`.
- [x] Document the live OpenAPI and persistence source of truth in the architecture guide.

### Task 2: Guard OpenAPI Route Drift

**Files:**
- Modify: `main/src/server/docs.rs`

- [x] Add a unit test that asserts critical paths exist in the generated OpenAPI document.
- [x] Run `cargo test -p previa-main server::docs`.

### Task 3: Guard TypeScript Client Drift

**Files:**
- Create: `scripts/check_openapi_client_contract.py`

- [x] Add a script that compares known TypeScript client paths with generated OpenAPI paths.
- [x] Run `python3 scripts/check_openapi_client_contract.py`.

### Task 4: Reduce Frontend Test Noise

**Files:**
- Modify focused React test/setup or component files as needed.

- [x] Remove expected-error console noise from tests.
- [x] Fix Radix dialog accessibility warnings where they are from local markup.
- [x] Run `npm test`.

### Task 5: Final Validation and Publish

- [x] Run `cargo test --workspace`.
- [x] Run `npm test` from `app/`.
- [x] Run `cargo build --release`.
- [x] Commit and push the branch when release build succeeds.
