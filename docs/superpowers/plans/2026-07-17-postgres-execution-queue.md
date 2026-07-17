# Postgres Execution Queue Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every execution-time HTTP exchange between `previa-main` and `previa-runner` with a durable Postgres job and event queue for E2E and distributed load tests.

**Architecture:** `previa-main` writes executions and compatible jobs to Postgres; runners register, heartbeat, claim work with `FOR UPDATE SKIP LOCKED`, renew fenced leases, and append events/results. Main projection workers consume the append-only event log into durable snapshots, SSE, and existing history records. Postgres is the only runtime database; SQLite remains isolated in project import/export.

**Tech Stack:** Rust 2024, Tokio, Axum, SQLx 0.8 with Postgres and SQLite transfer support, PostgreSQL `LISTEN/NOTIFY`, React/Vite TypeScript, Docker Compose, Helm, Kubernetes/Karpenter.

## Global Constraints

- Postgres is mandatory for runtime state; reject SQLite runtime URLs.
- SQLite remains available only for project import/export files.
- Delivery is `at-least-once`; internal writes are idempotent and fenced by `runner_id`, `attempt`, `lease_epoch`, and `lease_token`.
- Queue tables are the source of truth; `LISTEN/NOTIFY` is only a wake-up hint and polling is mandatory.
- Claims use `FOR UPDATE SKIP LOCKED`.
- E2E produces one job; load produces multiple shards sharing one execution clock.
- No HTTP execution fallback remains in the runner.
- Runner HTTP keeps only `/health`, `/ready`, `/info`, and `/openapi.json`.
- Main and runner must enforce the same compiled queue protocol version.
- Transport stays in handlers/router wiring; reusable queue logic stays in focused services/modules.
- OpenAPI handlers, `main/src/server/models.rs`, `main/src/server/docs.rs`, and `app/src/lib/api-client.ts` must remain synchronized.
- Every behavioral task follows red-green TDD and ends with a focused commit.

---

### Task 1: Queue schema and configuration contracts

**Files:**
- Create: `main/migrations/postgres/202607170001_add_execution_queue.sql`
- Create: `main/src/server/queue/mod.rs`
- Create: `main/src/server/queue/config.rs`
- Create: `main/src/server/queue/models.rs`
- Modify: `main/src/server/mod.rs`
- Create: `runner/src/server/queue/mod.rs`
- Create: `runner/src/server/queue/config.rs`
- Create: `runner/src/server/queue/models.rs`
- Modify: `runner/src/server/mod.rs`
- Test: inline `#[cfg(test)]` modules in both `config.rs` files

**Interfaces:**
- Produces `main::server::queue::config::MainQueueConfig::from_env_values`.
- Produces `runner::server::queue::config::RunnerQueueConfig::from_env_values`.
- Produces shared wire concepts with identical serialized values: `ExecutionKind`, `ExecutionStatus`, `JobStatus`, `QueueProtocolVersion`.

- [x] **Step 1: Write failing configuration tests**

```rust
#[test]
fn defaults_match_queue_design() {
    let config = MainQueueConfig::from_env_values(&[
        ("DATABASE_URL", "postgres://postgres@localhost/previa"),
    ]).unwrap();
    assert_eq!(config.runner_stale_after, Duration::from_millis(15_000));
    assert_eq!(config.job_lease, Duration::from_millis(30_000));
    assert_eq!(config.job_max_attempts, 3);
    assert_eq!(config.retry_backoff_base, Duration::from_millis(1_000));
    assert_eq!(config.retry_backoff_max, Duration::from_millis(30_000));
}

#[test]
fn rejects_sqlite_runtime_database() {
    let error = MainQueueConfig::from_env_values(&[
        ("DATABASE_URL", "sqlite://previa.db"),
    ]).unwrap_err();
    assert!(error.contains("Postgres"));
}

#[test]
fn runner_defaults_and_cross_field_validation() {
    let config = RunnerQueueConfig::from_env_values(&[
        ("PREVIA_QUEUE_DATABASE_URL", "postgres://runner@localhost/previa"),
    ]).unwrap();
    assert_eq!(config.event_batch_size, 200);
    assert_eq!(config.event_buffer_max, 5_000);
    assert!(RunnerQueueConfig::from_env_values(&[
        ("PREVIA_QUEUE_DATABASE_URL", "postgres://runner@localhost/previa"),
        ("PREVIA_QUEUE_EVENT_BATCH_SIZE", "500"),
        ("PREVIA_QUEUE_EVENT_BUFFER_MAX", "200"),
    ]).is_err());
}
```

- [x] **Step 2: Run tests and verify they fail because queue modules do not exist**

Run:

```bash
PREVIA_MAIN_SKIP_APP_BUILD=1 cargo test -p previa-main queue::config
cargo test -p previa-runner queue::config
```

Expected: compile failure for missing `queue` modules and config types.

- [x] **Step 3: Add exact configuration structs and parsers**

```rust
pub const QUEUE_PROTOCOL_VERSION: i32 = 1;

#[derive(Debug, Clone)]
pub struct MainQueueConfig {
    pub database_url: String,
    pub runner_stale_after: Duration,
    pub job_lease: Duration,
    pub job_max_attempts: u32,
    pub projection_lease: Duration,
    pub projection_poll_interval: Duration,
    pub maintenance_interval: Duration,
    pub retry_backoff_base: Duration,
    pub retry_backoff_max: Duration,
    pub event_retention: Duration,
    pub runner_retention: Duration,
}

#[derive(Debug, Clone)]
pub struct RunnerQueueConfig {
    pub database_url: String,
    pub heartbeat_interval: Duration,
    pub lease_renew_interval: Duration,
    pub poll_interval: Duration,
    pub event_flush_interval: Duration,
    pub event_batch_size: usize,
    pub event_buffer_max: usize,
}
```

Implement a private `EnvSource` map and numeric helpers that enforce every range from the spec. `DATABASE_URL` and `PREVIA_QUEUE_DATABASE_URL` must accept only `postgres://` or `postgresql://`.

- [x] **Step 4: Add the Postgres migration**

Create the tables `queue_protocol`, `runner_instances`, `executions`, `execution_jobs`, `execution_events`, and `execution_snapshots` with the columns, checks, unique constraints, and indexes from the spec. Seed protocol version atomically:

```sql
INSERT INTO queue_protocol (id, protocol_version, updated_at)
VALUES (1, 1, CURRENT_TIMESTAMP)
ON CONFLICT (id) DO UPDATE
SET protocol_version = EXCLUDED.protocol_version,
    updated_at = EXCLUDED.updated_at;
```

Use `TEXT CHECK (...)` for execution/job states and `JSONB NOT NULL` for request, requirements, payload, event, and snapshot JSON.
Enable `pgcrypto` for token hashing and generated fencing tokens:

```sql
CREATE EXTENSION IF NOT EXISTS pgcrypto;
```

- [x] **Step 5: Run focused tests and migration validation**

Run:

```bash
PREVIA_MAIN_SKIP_APP_BUILD=1 cargo test -p previa-main queue::config
cargo test -p previa-runner queue::config
sqlx migrate info --source main/migrations/postgres
```

Expected: all queue configuration tests pass and SQLx lists `202607170001` as a valid pending migration.

- [x] **Step 6: Commit**

```bash
git add main/migrations/postgres/202607170001_add_execution_queue.sql \
  main/src/server/queue main/src/server/mod.rs \
  runner/src/server/queue runner/src/server/mod.rs
git commit -m "feat: add postgres execution queue schema"
```

### Task 2: Restricted Postgres queue functions and repository

**Files:**
- Create: `main/migrations/postgres/202607170002_add_execution_queue_functions.sql`
- Create: `main/src/server/queue/repository.rs`
- Modify: `main/src/server/queue/mod.rs`
- Create: `main/tests/postgres_queue.rs`

**Interfaces:**
- Produces `QueueRepository::connect(database_url, max_connections)`.
- Produces `enqueue_execution`, `cancel_execution`, `claim_projection`, `read_events_after`, `store_snapshot`, `reap_expired_jobs`.
- SQL functions consumed by runners: `queue_register_runner`, `queue_heartbeat_runner`, `queue_claim_job`, `queue_renew_job_lease`, `queue_publish_events`, `queue_complete_job`, `queue_fail_job`, `queue_acknowledge_cancellation`, `queue_read_control`.

- [x] **Step 1: Add failing real-Postgres integration tests**

Use `PREVIA_TEST_POSTGRES_URL`; create a unique schema per test with UUID suffix and set `search_path`. Test:

```rust
#[tokio::test]
async fn concurrent_workers_claim_distinct_jobs() {
    let harness = PostgresHarness::new().await;
    let execution = harness.enqueue_load_with_shards(2).await;
    let (left, right) = tokio::join!(
        harness.claim("runner-a"),
        harness.claim("runner-b"),
    );
    assert_ne!(left.unwrap().job_id, right.unwrap().job_id);
    assert_eq!(harness.active_jobs(execution).await, 2);
}

#[tokio::test]
async fn stale_lease_epoch_cannot_publish_or_finish() {
    let harness = PostgresHarness::new().await;
    let first = harness.claim_seeded_job("runner-a").await;
    harness.expire_and_requeue(first.job_id).await;
    let second = harness.claim("runner-b").await.unwrap();
    assert!(harness.publish(first.fencing(), 1).await.is_err());
    assert!(harness.finish(first.fencing()).await.is_err());
    assert!(harness.finish(second.fencing()).await.is_ok());
}
```

- [x] **Step 2: Run and verify red**

Run:

```bash
PREVIA_TEST_POSTGRES_URL=postgres://postgres:postgres@127.0.0.1:5432/previa_test \
  PREVIA_MAIN_SKIP_APP_BUILD=1 cargo test -p previa-main --test postgres_queue
```

Expected: compile failure for missing repository/harness.

- [x] **Step 3: Implement restricted SQL functions**

Each runner function must be `SECURITY DEFINER SET search_path = public, pg_temp`, require `runner_id` plus `runner_session_token`, compare `digest(token, 'sha256')`, and revoke table access from the runner role. `queue_claim_job` must:

```sql
SELECT j.id
FROM execution_jobs j
JOIN executions e ON e.id = j.execution_id
JOIN runner_instances r ON r.id = p_runner_id
WHERE j.status = 'queued'
  AND j.available_at <= CURRENT_TIMESTAMP
  AND e.desired_status = 'running'
  AND j.pool = r.pool
  AND j.kind = ANY (r.supported_kinds)
  AND (j.requirements_json = '{}'::jsonb
       OR (r.labels_json || r.capabilities_json) @> j.requirements_json)
  AND (
    (j.kind = 'e2e' AND (
      SELECT count(*) FROM execution_jobs active
      WHERE active.runner_id = r.id
        AND active.kind = 'e2e'
        AND active.status IN ('leased', 'running')
    ) < r.max_e2e_slots)
    OR
    (j.kind = 'load' AND (
      SELECT count(*) FROM execution_jobs active
      WHERE active.runner_id = r.id
        AND active.kind = 'load'
        AND active.status IN ('leased', 'running')
    ) < r.max_load_slots)
  )
ORDER BY j.priority DESC, j.created_at ASC
FOR UPDATE OF j SKIP LOCKED
LIMIT 1;
```

Then increment `attempt` and `lease_epoch`, generate a fresh UUID token, set lease fields, and return the complete immutable payload.

- [x] **Step 4: Implement `QueueRepository`**

Use `sqlx::PgPool`. Keep all SQL in `repository.rs`. Main-side methods use bound parameters and transactions; no handler calls SQL directly.

- [x] **Step 5: Run concurrency and privilege tests**

Run the integration command from Step 2. Expected: claim, fencing, idempotency, cancellation, protocol, and privilege tests all pass.

- [x] **Step 6: Commit**

```bash
git add main/migrations/postgres/202607170002_add_execution_queue_functions.sql \
  main/src/server/queue/repository.rs main/src/server/queue/mod.rs \
  main/tests/postgres_queue.rs
git commit -m "feat: add fenced postgres job claims"
```

### Task 3: Runner registration, heartbeat, claim loop, and event buffer

**Files:**
- Create: `runner/src/server/queue/repository.rs`
- Create: `runner/src/server/queue/heartbeat.rs`
- Create: `runner/src/server/queue/event_buffer.rs`
- Create: `runner/src/server/queue/worker.rs`
- Modify: `runner/src/server/queue/mod.rs`
- Modify: `runner/src/server/state.rs`
- Modify: `runner/src/main.rs`
- Modify: `runner/Cargo.toml`

**Interfaces:**
- Produces `RunnerQueueRepository`.
- Produces `RunnerWorker::run(cancel_token)`.
- Produces `EventBuffer::push(QueueEvent)` and `flush`.
- Consumes a callback `JobExecutor: async fn(ClaimedJob, EventSink, CancellationToken) -> JobOutcome`.

- [x] **Step 1: Write failing worker tests**

Create fake repository tests proving:

```rust
#[tokio::test]
async fn lost_notify_is_recovered_by_polling() { /* fake returns a job on second poll */ }

#[tokio::test]
async fn lease_renew_failure_cancels_executor() { /* renew returns false */ }

#[tokio::test]
async fn event_buffer_flushes_at_batch_size_and_interval() { /* paused time */ }
```

- [x] **Step 2: Run and verify red**

Run `cargo test -p previa-runner queue::`. Expected: missing worker/repository/event buffer symbols.

- [x] **Step 3: Implement runner repository and identity**

Connect with `PgPoolOptions`, register once, keep the opaque token only in memory, and call the restricted SQL functions. Redact database URLs from all errors.

- [x] **Step 4: Implement heartbeat and worker loop**

The worker loop uses `PgListener` for `previa_jobs` and `previa_control`, plus `tokio::time::interval(config.poll_interval)`. A claimed job starts:

- a lease renewal task;
- a cancellation/control task;
- the executor;
- a bounded event buffer.

Whichever first invalidates the lease cancels the executor and prevents terminal writes with stale fencing.

- [x] **Step 5: Implement buffered event publication**

Assign monotonically increasing `seq` per attempt. Flush at batch size or timer. Reject serialized payloads over `1 MiB`. Stop accepting events when the configured buffer maximum is reached.

- [x] **Step 6: Run runner tests**

Run:

```bash
cargo test -p previa-runner queue::
```

Expected: registration, polling fallback, renewal, cancellation, and buffer tests pass.

- [x] **Step 7: Commit**

```bash
git add runner/Cargo.toml runner/src/main.rs runner/src/server/state.rs \
  runner/src/server/queue
git commit -m "feat: run postgres queue worker"
```

### Task 4: E2E job execution

**Files:**
- Create: `runner/src/server/queue/e2e_executor.rs`
- Modify: `runner/src/server/queue/worker.rs`
- Modify: `runner/src/server/queue/mod.rs`
- Modify: `main/src/server/execution/e2e.rs`
- Modify: `main/src/server/handlers/tests_e2e.rs`
- Test: inline modules and `main/tests/postgres_queue_e2e.rs`

**Interfaces:**
- Produces `E2eJobPayload` and `E2eQueueExecutor`.
- Main `start_e2e_execution` enqueues and returns a durable execution context without selecting runner endpoints.

- [x] **Step 1: Write failing E2E queue tests**

Test that POST creation inserts one E2E job, no runner endpoint is required, step events are persisted, retry increments attempt, and a terminal result creates one history row.

- [x] **Step 2: Run red tests**

Run:

```bash
PREVIA_TEST_POSTGRES_URL=postgres://postgres:postgres@127.0.0.1:5432/previa_test \
  PREVIA_MAIN_SKIP_APP_BUILD=1 cargo test -p previa-main --test postgres_queue_e2e
cargo test -p previa-runner e2e_executor
```

- [x] **Step 3: Implement immutable E2E payload**

Payload contains pipeline, specs, env groups, selected URLs, prior results for rerun, `execution_id`, `job_id`, and `attempt`. It contains all data required by `previa-engine`; the runner never reads project tables.

- [x] **Step 4: Adapt runner engine hooks**

Emit deterministic `execution:running`, `step:start`, `step:result`, error, and terminal events through `EventSink`. Return `JobOutcome::Completed(result_json)`, `Failed { retryable, error }`, or `Cancelled`.

- [x] **Step 5: Replace main E2E dispatch**

Remove runner registry collection, scheduler in-memory acquire/release, and HTTP forwarding from `start_e2e_execution`. Enqueue execution/job transactionally and create the `ExecutionCtx` from durable snapshot updates.

- [x] **Step 6: Run E2E tests**

Expected: focused main/runner tests pass and no E2E start path opens runner HTTP.

- [x] **Step 7: Commit**

```bash
git add runner/src/server/queue/e2e_executor.rs runner/src/server/queue \
  main/src/server/execution/e2e.rs main/src/server/handlers/tests_e2e.rs \
  main/tests/postgres_queue_e2e.rs
git commit -m "feat: execute e2e jobs from postgres"
```

### Task 5: Distributed load shards

**Files:**
- Create: `runner/src/server/queue/load_executor.rs`
- Modify: `runner/src/server/queue/worker.rs`
- Modify: `runner/src/server/queue/mod.rs`
- Modify: `main/src/server/execution/load.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Test: `main/tests/postgres_queue_load.rs` and inline runner tests

**Interfaces:**
- Produces `LoadShardJobPayload`.
- Reuses `apply_runner_telemetry_line`, latency consolidation, and existing wave metrics.

- [x] **Step 1: Write failing shard tests**

Test exact split of RPS/wave across three shards, concurrent claims, aggregated buckets, and retry after the global clock advances without replaying expired slots.

- [x] **Step 2: Run red tests**

Run focused main Postgres integration test and `cargo test -p previa-runner load_executor`.

- [x] **Step 3: Implement shard payload**

Include `execution_started_at_ms`, shard index/count, assigned RPS, wave profile, global deadline, grace period, pipeline/spec/env snapshots, and reservation labels.

- [x] **Step 4: Implement load executor**

Adapt existing classic/wave execution functions to an internal executor that publishes aggregated `RunnerLoadLine` buckets. On retry compute current elapsed time from the global execution clock and skip expired dispatch slots.

- [x] **Step 5: Replace main HTTP polling**

Delete `forward_runner_polled_load_chunked`, telemetry URL construction, ack handling, and polling envs. Keep pure consolidation functions in `load_batch.rs`; feed them from Postgres events.

- [x] **Step 6: Run load tests**

Expected: split, wave, retry clock, consolidation, cancellation, and history compatibility tests pass.

- [x] **Step 7: Commit**

```bash
git add runner/src/server/queue/load_executor.rs runner/src/server/queue \
  main/src/server/execution/load.rs main/src/server/execution/load_batch.rs \
  main/tests/postgres_queue_load.rs
git commit -m "feat: distribute load through postgres shards"
```

### Task 6: Durable projection, SSE, history, cancellation, and maintenance

**Files:**
- Create: `main/src/server/queue/projector.rs`
- Create: `main/src/server/queue/dispatcher.rs`
- Create: `main/src/server/queue/retention.rs`
- Modify: `main/src/server/queue/mod.rs`
- Modify: `main/src/server/state.rs`
- Modify: `main/src/main.rs`
- Modify: `main/src/server/handlers/executions.rs`
- Modify: `main/src/server/services/execution_summary.rs`
- Test: `main/tests/postgres_queue_recovery.rs`

**Interfaces:**
- Produces background `QueueRuntime::start`.
- Produces durable snapshot subscription used by SSE and summary handlers.

- [x] **Step 1: Write failing recovery tests**

Cover lost notify, main restart from `last_event_id`, projection lease takeover, cancellation before/after claim, exponential retry, dead letter, and retention prerequisites.

- [x] **Step 2: Run red tests**

Run `cargo test -p previa-main --test postgres_queue_recovery` with the Postgres test URL.

- [x] **Step 3: Implement projector**

Use a projection lease in `execution_snapshots`; read ordered events after checkpoint; apply event and checkpoint in one transaction; publish the resulting snapshot to in-process subscribers. History insert uses execution ID idempotency.

- [x] **Step 4: Implement dispatcher and maintenance**

Implement cancel desired state, expired lease reaper, retry promotion, stale runner marking, dead letter transition, and advisory-lock ownership. Backoff is:

```rust
let shift = attempt.saturating_sub(1).min(31);
let delay = base.saturating_mul(1_u32 << shift).min(max);
```

- [x] **Step 5: Implement retention**

Delete events only for terminal, fully projected executions with final history older than retention. Delete inactive runners only with no active jobs and after runner retention.

- [x] **Step 6: Wire startup and handlers**

Start listener/projector/maintenance tasks after migrations. Cancellation updates Postgres. SSE reads persisted snapshot first and subscribes to subsequent projection changes.

- [x] **Step 7: Run recovery tests**

Expected: all recovery/cancellation/retention tests pass.

- [x] **Step 8: Commit**

```bash
git add main/src/main.rs main/src/server/state.rs main/src/server/queue \
  main/src/server/handlers/executions.rs \
  main/src/server/services/execution_summary.rs \
  main/tests/postgres_queue_recovery.rs
git commit -m "feat: project durable execution events"
```

### Task 7: Remove runner execution HTTP API

**Files:**
- Modify: `runner/src/server/mod.rs`
- Modify: `runner/src/server/docs.rs`
- Modify: `runner/src/server/handlers/mod.rs`
- Delete: `runner/src/server/handlers/e2e.rs`
- Delete: `runner/src/server/handlers/load.rs`
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/state.rs`
- Delete: `runner/src/server/load_execution.rs`
- Modify: `runner/src/lib.rs`

**Interfaces:**
- Runner HTTP surface temporarily contains health/readiness/info/OpenAPI and reservation lifecycle endpoints; Task 10 moves reservation lifecycle to Postgres and removes the temporary endpoints.

- [x] **Step 1: Write failing route/OpenAPI tests**

Assert old execution routes return `404` and OpenAPI excludes `/tests/e2e`, `/tests/load`, `/telemetry`, `/status`, and `/cancel`, while health/info routes remain.

- [x] **Step 2: Run red tests**

Run `cargo test -p previa-runner server::`.

- [x] **Step 3: Remove execution handlers and in-memory load registry**

Move reusable engine calls into queue executors before deleting handler modules. Keep reservation endpoints only if the Kubernetes plugin still calls them during this task; Task 10 removes or replaces remaining direct runner control.

- [x] **Step 4: Update runner models/docs**

Remove HTTP-only request/response models after queue payload equivalents compile. Update OpenAPI paths/components.

- [x] **Step 5: Run runner tests**

Expected: route/OpenAPI, engine, wave, metrics, queue, and reservation tests pass.

- [x] **Step 6: Commit**

```bash
git add -A runner/src
git commit -m "refactor: remove runner execution http api"
```

### Task 8: Make Postgres the only main runtime database

**Files:**
- Modify: `main/src/server/db/pool.rs`
- Modify: all `main/src/server/db/*.rs`
- Modify: `main/src/main.rs`
- Modify: `main/src/server/services/sqlite_transfer.rs`
- Modify: `main/Cargo.toml`
- Modify: all main tests using `sqlite::memory:`
- Delete: runtime-only `main/migrations/sqlite/*.sql`

**Interfaces:**
- `DbPool` wraps `PgPool` and retains `query`, `sql`, `begin`, and `pool` helpers with Postgres arguments.
- `sqlite_transfer.rs` owns a separate `SqlitePool` used only for portable files.

- [ ] **Step 1: Add failing database-mode tests**

Assert Postgres URLs connect/migrate, SQLite runtime URLs fail, and SQLite project export/import still round-trips.

- [ ] **Step 2: Convert `DbPool` to `PgPool`**

Remove `Any`, `DatabaseKind::Sqlite`, and runtime driver switching. Convert placeholders to native `$1` forms or retain the existing question-mark rewrite in the wrapper while returning `sqlx::query::Query<Postgres, PgArguments>`.

- [ ] **Step 3: Convert DB modules and tests**

Use bound Postgres queries. Replace SQLite-memory harnesses with isolated Postgres schemas via the shared test harness. Keep SQLite dependencies only behind the transfer service.

- [ ] **Step 4: Isolate transfer SQLite**

Use `SqlitePoolOptions` inside `sqlite_transfer.rs`; create the portable schema explicitly and never expose this pool in `AppState`.

- [ ] **Step 5: Run main test suite**

Run:

```bash
PREVIA_TEST_POSTGRES_URL=postgres://postgres:postgres@127.0.0.1:5432/previa_test \
  PREVIA_MAIN_SKIP_APP_BUILD=1 cargo test -p previa-main
```

Expected: all main tests pass on Postgres, including SQLite transfer tests.

- [ ] **Step 6: Commit**

```bash
git add -A main
git commit -m "refactor: require postgres runtime storage"
```

### Task 9: API, UI, MCP, and observability alignment

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/docs.rs`
- Modify: `main/src/server/handlers/executions.rs`
- Modify: `main/src/server/handlers/runners.rs`
- Modify: `main/src/server/mcp/service.rs`
- Modify: `app/src/lib/api-client.ts`
- Modify: relevant app stores/components/tests

**Interfaces:**
- External execution status union is `queued | leased | running | retrying | cancel_requested | completed | failed | cancelled`.
- Runner records expose safe queue health/capacity fields.

- [ ] **Step 1: Write failing OpenAPI/client/UI tests**

Assert all states deserialize, queue diagnostics appear, secrets never appear, and MCP execution status/summary reads durable snapshots.

- [ ] **Step 2: Run red tests**

Run:

```bash
PREVIA_MAIN_SKIP_APP_BUILD=1 cargo test -p previa-main server::docs
python3 scripts/check_openapi_client_contract.py
cd app && npm test
```

- [ ] **Step 3: Update models, handlers, OpenAPI, client, and UI**

Keep the contract synchronized in one change. Add queue depth, oldest eligible age, retries, dead letters, projection lag, event backlog, runner heartbeat, pool, slots, and effective non-secret configuration.

- [ ] **Step 4: Update MCP**

Execution tools continue targeting main. Remove assumptions about runner endpoint streams and expose durable state/summary.

- [ ] **Step 5: Run contract and frontend tests**

Expected: docs tests, contract checker, Vitest, and TypeScript build pass.

- [ ] **Step 6: Commit**

```bash
git add main/src/server app/src
git commit -m "feat: expose durable queue execution state"
```

### Task 10: CLI, Compose, Helm, and Kubernetes plugin

**Files:**
- Modify: `previa/src/compose.rs`
- Modify: `previa/src/config.rs`
- Modify: `previa/src/diagnostics.rs`
- Modify: `previa-compose.yaml`
- Modify: `.env.example`
- Modify: `charts/previa/values.yaml`
- Modify: `charts/previa/templates/main-deployment.yaml`
- Modify: `charts/previa/templates/plugin-deployment.yaml`
- Modify: `charts/previa/templates/plugin-rbac.yaml`
- Modify: `kubernetes-plugin/src/services/runner_resources.rs`
- Modify: `kubernetes-plugin/src/services/runner_health.rs`
- Modify: related tests

**Interfaces:**
- `previa up` always provisions Postgres and distinct main/runner credentials.
- Provisioned runners receive `PREVIA_QUEUE_DATABASE_URL`, protocol, pool, labels, and slots.

- [ ] **Step 1: Write failing generated-config tests**

Assert generated Compose contains Postgres healthcheck/volume, main `DATABASE_URL`, runner restricted URL, dependency ordering, and no `RUNNER_AUTH_KEY` execution transport. Assert Helm renders distinct secrets.

- [ ] **Step 2: Run red tests**

Run CLI compose/config tests and Kubernetes plugin resource tests.

- [ ] **Step 3: Implement local Postgres provisioning**

Generate a `postgres:17` service with persistent volume and healthcheck. Generate credentials once into context env files with restrictive permissions. Make main/runners depend on healthy Postgres.

- [ ] **Step 4: Update Helm and plugin**

Require external Postgres secret refs. Inject runner queue secret only into runner pods. Readiness uses registration/heartbeat rather than execution endpoint discovery.

Replace `/internal/reservation/rearm` and `/internal/reservation/release` calls
with Postgres desired-state updates on `runner_instances`. The runner observes
`ready` or `draining` through `previa_control`; after this path passes plugin
tests, remove both reservation routes from `runner/src/server/mod.rs` and their
OpenAPI components so the final runner HTTP surface is exactly `/health`,
`/ready`, `/info`, and `/openapi.json`.

- [ ] **Step 5: Update diagnostics**

`previa doctor` validates Postgres reachability, protocol, migrations, and restricted runner role without printing credentials.

- [ ] **Step 6: Run infrastructure tests**

Expected: CLI, chart rendering, and plugin tests pass.

- [ ] **Step 7: Commit**

```bash
git add previa previa-compose.yaml .env.example charts kubernetes-plugin
git commit -m "feat: deploy postgres execution queue"
```

### Task 11: Documentation, CI, compatibility cleanup, and full verification

**Files:**
- Modify: `README.md`
- Modify: `PROJECT.md`
- Modify: `AGENTS.md` only if provider workflow changed
- Modify: relevant `docs/previa/*.md`
- Modify: `.github/workflows/release.yaml`
- Create: `.github/workflows/ci.yaml`
- Modify: `CHANGELOG.md`
- Remove obsolete HTTP polling configuration/docs

**Interfaces:**
- CI provides a real Postgres service and the queue test URL.
- User docs explain breaking upgrade, SQLite export/import, and protocol compatibility.

- [ ] **Step 1: Add CI workflow**

Use `postgres:17`, health checks, Rust 1.90+, Node install, app dependencies, and commands:

```bash
cargo test --workspace
python3 scripts/check_openapi_client_contract.py
cd app && npm test
cd app && npm run build
cargo build --release
```

- [ ] **Step 2: Update documentation**

Document mandatory Postgres, env defaults, queue states, runner role, Compose/Helm secrets, backup/migration, rollback boundary, and removed HTTP endpoints. Remove references to `runner_polling`, telemetry ack, and SQLite runtime.

- [ ] **Step 3: Run placeholder and stale-reference scans**

Run:

```bash
rg -n "runner_polling|telemetry/ack|sqlite::memory:|RUNNER_AUTH_KEY" \
  main runner previa charts docs README.md PROJECT.md .env.example
```

Expected: only migration/changelog text or explicitly retained non-execution auth references.

- [ ] **Step 4: Run full validation**

Run with real Postgres:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/check_openapi_client_contract.py
cd app && npm ci && npm test && npm run build
cd ..
cargo build --release
```

Expected: every command exits zero.

- [ ] **Step 5: Commit**

```bash
git add README.md PROJECT.md AGENTS.md docs .github CHANGELOG.md
git commit -m "docs: document postgres execution queue"
```

- [ ] **Step 6: Push and update the draft PR**

```bash
git push -u origin codex/postgres-execution-queue
```

Update the PR body with architecture, migration impact, test evidence, and explicit breaking-change notice. Keep it draft until the full validation and review complete.
